use std::{
    collections::HashMap,
    env,
    io::{Read, Write},
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::Context;
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use rand::Rng;
use serde::Deserialize;
use thiserror::Error;
use tokio::{
    io::AsyncReadExt,
    process::Command,
    sync::{Mutex, Notify},
};
use uuid::Uuid;

use crate::{
    config::ExecConfig,
    truncation::{OutputShape, truncate_head_tail},
};

#[derive(Debug, Deserialize)]
pub struct ExecCommandInput {
    pub cmd: String,
    pub workdir: Option<String>,
    pub shell: Option<String>,
    pub tty: Option<bool>,
    pub login: Option<bool>,
    pub yield_time_ms: Option<u64>,
    pub max_output_tokens: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct WriteStdinInput {
    pub session_id: u64,
    #[serde(default)]
    pub chars: String,
    pub yield_time_ms: Option<u64>,
    pub max_output_tokens: Option<usize>,
}

#[derive(Debug, Error)]
pub enum ExecError {
    #[error("cmd contains a NUL byte")]
    NulByte,
    #[error("maximum running process count reached")]
    TooManyProcesses,
    #[error("unknown session id {0}")]
    UnknownSession(u64),
    #[error(
        "stdin is closed for this session; rerun exec_command with tty=true to keep stdin open"
    )]
    StdinClosed,
    #[error("{0}")]
    Other(String),
}

#[derive(Clone)]
pub struct ProcessManager {
    config: ExecConfig,
    sessions: Arc<Mutex<HashMap<u64, Arc<Session>>>>,
    next_id: Arc<AtomicU64>,
}

struct Session {
    id: u64,
    tty: bool,
    output: Mutex<Vec<u8>>,
    notify: Notify,
    exit_code: Mutex<Option<i32>>,
    last_used: Mutex<Instant>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    child: Mutex<Option<tokio::process::Child>>,
    pty_pid: Mutex<Option<u32>>,
}

pub struct ShellPlan {
    pub shell: String,
    pub flag: String,
    pub cmd: String,
}

impl ProcessManager {
    pub fn new(mut config: ExecConfig) -> Self {
        if env::var("agentbox_DETERMINISTIC_SESSION_IDS")
            .ok()
            .as_deref()
            == Some("1")
        {
            config.deterministic_session_ids = true;
        }
        Self {
            config,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(AtomicU64::new(1)),
        }
    }

    pub fn shell_plan(&self, input_shell: Option<&str>, login: bool, cmd: &str) -> ShellPlan {
        let shell = input_shell
            .map(ToOwned::to_owned)
            .or_else(|| self.config.default_shell.clone())
            .or_else(|| env::var("SHELL").ok())
            .unwrap_or_else(|| "/bin/bash".to_string());
        ShellPlan {
            shell,
            flag: if login { "-lc" } else { "-c" }.to_string(),
            cmd: cmd.to_string(),
        }
    }

    pub async fn exec_command(&self, input: ExecCommandInput) -> Result<OutputShape, ExecError> {
        if input.cmd.contains('\0') {
            return Err(ExecError::NulByte);
        }
        self.cleanup_expired().await;
        if self.sessions.lock().await.len() >= self.config.max_processes {
            return Err(ExecError::TooManyProcesses);
        }
        let tty = input.tty.unwrap_or(false);
        if tty {
            self.exec_tty(input).await
        } else {
            self.exec_non_tty(input).await
        }
    }

    async fn exec_non_tty(&self, input: ExecCommandInput) -> Result<OutputShape, ExecError> {
        let start = Instant::now();
        let login = input.login.unwrap_or(self.config.login_default);
        let shell = self.shell_plan(input.shell.as_deref(), login, &input.cmd);
        let workdir = input
            .workdir
            .unwrap_or_else(|| self.config.default_workdir.clone());
        let mut cmd = Command::new(&shell.shell);
        cmd.arg(&shell.flag)
            .arg(&shell.cmd)
            .current_dir(&workdir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        self.overlay_env(&mut cmd);
        tracing::info!(cmd = %input.cmd, %workdir, tty = false, "exec start");
        let mut child = cmd
            .spawn()
            .map_err(|e| ExecError::Other(format!("spawn failed: {e}")))?;
        let stdout = child
            .stdout
            .take()
            .context("stdout missing")
            .map_err(|e| ExecError::Other(e.to_string()))?;
        let stderr = child
            .stderr
            .take()
            .context("stderr missing")
            .map_err(|e| ExecError::Other(e.to_string()))?;
        let session = self.new_session(false).await?;
        {
            let mut child_slot = session.child.lock().await;
            *child_slot = Some(child);
        }
        spawn_pipe_reader(stdout, session.clone());
        spawn_pipe_reader(stderr, session.clone());
        spawn_child_waiter(session.clone(), self.sessions.clone());
        self.sessions
            .lock()
            .await
            .insert(session.id, session.clone());
        let out = self.collect_until(&session, input.yield_time_ms).await;
        let status = *session.exit_code.lock().await;
        if status.is_some() {
            self.sessions.lock().await.remove(&session.id);
        }
        Ok(self.shape(
            session.id,
            start.elapsed(),
            status,
            out,
            input.max_output_tokens,
        ))
    }

    async fn exec_tty(&self, input: ExecCommandInput) -> Result<OutputShape, ExecError> {
        let start = Instant::now();
        let login = input.login.unwrap_or(self.config.login_default);
        let shell = self.shell_plan(input.shell.as_deref(), login, &input.cmd);
        let workdir = input
            .workdir
            .unwrap_or_else(|| self.config.default_workdir.clone());
        let session = self.new_session(true).await?;
        let pty_system = NativePtySystem::default();
        let pair = pty_system
            .openpty(PtySize {
                rows: 24,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| ExecError::Other(format!("open pty failed: {e}")))?;
        let mut builder = CommandBuilder::new(shell.shell);
        builder.arg(shell.flag);
        builder.arg(shell.cmd);
        builder.cwd(PathBuf::from(&workdir));
        if self.config.overlay_codex_env {
            for (k, v) in codex_env() {
                builder.env(k, v);
            }
        }
        tracing::info!(cmd = %input.cmd, %workdir, tty = true, "exec start");
        let mut child = pair
            .slave
            .spawn_command(builder)
            .map_err(|e| ExecError::Other(format!("pty spawn failed: {e}")))?;
        *session.pty_pid.lock().await = child.process_id();
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| ExecError::Other(e.to_string()))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| ExecError::Other(e.to_string()))?;
        *session.writer.lock().await = Some(writer);
        self.sessions
            .lock()
            .await
            .insert(session.id, session.clone());
        let reader_session = session.clone();
        let handle = tokio::runtime::Handle::current();
        std::thread::spawn(move || {
            let mut buf = [0_u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let bytes = buf[..n].to_vec();
                        let session = reader_session.clone();
                        let _jh = handle.spawn(async move {
                            session.output.lock().await.extend(bytes);
                            session.notify.notify_waiters();
                        });
                    }
                    Err(_) => break,
                }
            }
        });
        let wait_session = session.clone();
        let sessions = self.sessions.clone();
        let handle = tokio::runtime::Handle::current();
        std::thread::spawn(move || {
            let code = child.wait().map(|s| s.exit_code() as i32).unwrap_or(-1);
            let _jh = handle.spawn(async move {
                *wait_session.exit_code.lock().await = Some(code);
                wait_session.notify.notify_waiters();
                tracing::info!(session_id = wait_session.id, exit_code = code, "exec exit");
                let _ = sessions;
            });
        });
        let out = self.collect_until(&session, input.yield_time_ms).await;
        let status = *session.exit_code.lock().await;
        if status.is_some() {
            self.sessions.lock().await.remove(&session.id);
        }
        Ok(self.shape(
            session.id,
            start.elapsed(),
            status,
            out,
            input.max_output_tokens,
        ))
    }

    pub async fn write_stdin(&self, input: WriteStdinInput) -> Result<OutputShape, ExecError> {
        let start = Instant::now();
        let session = self
            .sessions
            .lock()
            .await
            .get(&input.session_id)
            .cloned()
            .ok_or(ExecError::UnknownSession(input.session_id))?;
        *session.last_used.lock().await = Instant::now();
        if !input.chars.is_empty() {
            if !session.tty {
                return Err(ExecError::StdinClosed);
            }
            if input.chars.contains('\u{3}')
                && let Some(pid) = *session.pty_pid.lock().await
            {
                unsafe {
                    libc::kill(-(pid as i32), libc::SIGINT);
                    libc::kill(pid as i32, libc::SIGINT);
                }
            }
            let mut writer = session.writer.lock().await;
            let writer = writer
                .as_mut()
                .ok_or_else(|| ExecError::Other("pty writer is closed".to_string()))?;
            writer
                .write_all(input.chars.as_bytes())
                .map_err(|_| ExecError::Other("failed to write to pty".to_string()))?;
            writer.flush().ok();
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let out = self.collect_until(&session, input.yield_time_ms).await;
        let status = *session.exit_code.lock().await;
        if status.is_some() {
            self.sessions.lock().await.remove(&session.id);
        }
        Ok(self.shape(
            session.id,
            start.elapsed(),
            status,
            out,
            input.max_output_tokens,
        ))
    }

    async fn new_session(&self, tty: bool) -> Result<Arc<Session>, ExecError> {
        let id = if self.config.deterministic_session_ids {
            self.next_id.fetch_add(1, Ordering::SeqCst)
        } else {
            rand::rng().random_range(1..=u64::MAX)
        };
        Ok(Arc::new(Session {
            id,
            tty,
            output: Mutex::new(Vec::new()),
            notify: Notify::new(),
            exit_code: Mutex::new(None),
            last_used: Mutex::new(Instant::now()),
            writer: Mutex::new(None),
            child: Mutex::new(None),
            pty_pid: Mutex::new(None),
        }))
    }

    async fn collect_until(&self, session: &Session, yield_time_ms: Option<u64>) -> Vec<u8> {
        let ms = yield_time_ms
            .unwrap_or(self.config.default_yield_time_ms)
            .clamp(self.config.min_yield_time_ms, self.config.max_yield_time_ms);
        let deadline = Instant::now() + Duration::from_millis(ms);
        loop {
            let exited = session.exit_code.lock().await.is_some();
            if exited || Instant::now() >= deadline {
                break;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            let _ = tokio::time::timeout(
                remaining.min(Duration::from_millis(50)),
                session.notify.notified(),
            )
            .await;
        }
        let mut output = session.output.lock().await;
        std::mem::take(&mut *output)
    }

    fn shape(
        &self,
        session_id: u64,
        elapsed: Duration,
        exit_code: Option<i32>,
        bytes: Vec<u8>,
        max_output_tokens: Option<usize>,
    ) -> OutputShape {
        let text = String::from_utf8_lossy(&bytes).to_string();
        let max = max_output_tokens
            .unwrap_or(self.config.default_max_output_tokens)
            .min(self.config.hard_max_output_tokens);
        let (output, original_token_count) = truncate_head_tail(&text, max);
        OutputShape {
            chunk_id: Some(Uuid::new_v4().to_string()),
            wall_time_seconds: elapsed.as_secs_f64(),
            exit_code,
            session_id: exit_code.is_none().then_some(session_id),
            original_token_count,
            output,
        }
    }

    fn overlay_env(&self, cmd: &mut Command) {
        if self.config.overlay_codex_env {
            cmd.envs(codex_env());
        }
    }

    async fn cleanup_expired(&self) {
        let ttl = Duration::from_secs(self.config.session_idle_ttl_seconds);
        let sessions = self.sessions.lock().await;
        let mut expired = Vec::new();
        for (id, session) in sessions.iter() {
            if session.last_used.lock().await.elapsed() > ttl {
                expired.push(*id);
            }
        }
        drop(sessions);
        for id in expired {
            if let Some(session) = self.sessions.lock().await.remove(&id)
                && let Some(mut child) = session.child.lock().await.take()
            {
                let _ = child.kill().await;
            }
        }
    }
}

fn spawn_pipe_reader<R>(mut reader: R, session: Arc<Session>)
where
    R: tokio::io::AsyncRead + Send + Unpin + 'static,
{
    tokio::spawn(async move {
        let mut buf = [0_u8; 8192];
        loop {
            match reader.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    session.output.lock().await.extend(&buf[..n]);
                    session.notify.notify_waiters();
                }
                Err(_) => break,
            }
        }
    });
}

fn spawn_child_waiter(session: Arc<Session>, sessions: Arc<Mutex<HashMap<u64, Arc<Session>>>>) {
    tokio::spawn(async move {
        let code = {
            let mut child = session.child.lock().await;
            if let Some(child) = child.as_mut() {
                child.wait().await.ok().and_then(|s| s.code()).unwrap_or(-1)
            } else {
                -1
            }
        };
        *session.exit_code.lock().await = Some(code);
        session.notify.notify_waiters();
        tracing::info!(session_id = session.id, exit_code = code, "exec exit");
        let _ = sessions;
    });
}

fn codex_env() -> Vec<(&'static str, &'static str)> {
    vec![
        ("NO_COLOR", "1"),
        ("TERM", "dumb"),
        ("LANG", "C.UTF-8"),
        ("LC_CTYPE", "C.UTF-8"),
        ("LC_ALL", "C.UTF-8"),
        ("COLORTERM", ""),
        ("PAGER", "cat"),
        ("GIT_PAGER", "cat"),
        ("GH_PAGER", "cat"),
        ("CODEX_CI", "1"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manager() -> ProcessManager {
        let cfg = ExecConfig {
            deterministic_session_ids: true,
            ..ExecConfig::default()
        };
        ProcessManager::new(cfg)
    }

    #[test]
    fn shell_plan_uses_one_cmd_argument() {
        let m = manager();
        for cmd in [
            "hello world",
            "echo \"a b\"",
            "echo a\n echo b",
            "echo a; echo b",
        ] {
            let plan = m.shell_plan(Some("/bin/bash"), true, cmd);
            assert_eq!(plan.shell, "/bin/bash");
            assert_eq!(plan.flag, "-lc");
            assert_eq!(plan.cmd, cmd);
        }
        assert_eq!(m.shell_plan(Some("sh"), false, "x").flag, "-c");
    }

    #[tokio::test]
    async fn rejects_nul() {
        let err = manager()
            .exec_command(ExecCommandInput {
                cmd: "a\0b".into(),
                workdir: None,
                shell: None,
                tty: None,
                login: None,
                yield_time_ms: None,
                max_output_tokens: None,
            })
            .await
            .unwrap_err();
        assert!(matches!(err, ExecError::NulByte));
    }
}
