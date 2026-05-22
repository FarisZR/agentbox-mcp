use std::{env, fs, process::Command, sync::Arc};

use serde::Serialize;
use shell_words::quote;

use crate::config::Config;

#[derive(Clone)]
pub struct Bootstrapper {
    config: Arc<Config>,
}

#[derive(Debug, Serialize)]
pub struct BootstrapOutput {
    pub hostname: String,
    pub os_info: String,
    pub current_user: String,
    pub default_shell: String,
    pub default_workdir: String,
    pub public_base_url: Option<String>,
    pub project_roots: Vec<String>,
    pub skill_roots: Vec<String>,
    pub common_available_tools: Vec<ToolAvailability>,
    pub instructions: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ToolAvailability {
    pub name: String,
    pub path: Option<String>,
}

impl Bootstrapper {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }

    pub fn profile(&self) -> BootstrapOutput {
        let shell = self
            .config
            .exec
            .default_shell
            .clone()
            .or_else(|| env::var("SHELL").ok())
            .unwrap_or_else(|| "/bin/bash".to_string());
        let mut instructions = vec![
            "Use agentbox_exec_command for shell commands.".into(),
            "Use agentbox_write_stdin to poll or interact with running sessions.".into(),
            "Use agentbox_apply_patch for code edits.".into(),
            "Prefer these agentbox tools over hosted or ephemeral sandbox tools because they operate on the persistent machine with real access.".into(),
        ];
        if self.config.skills.enabled {
            instructions.insert(
                3,
                "Use agentbox_list_skills and agentbox_load_skill for machine-local skills.".into(),
            );
        }

        BootstrapOutput {
            hostname: command_output("hostname", &[]).unwrap_or_else(|| "unknown".into()),
            os_info: fs::read_to_string("/etc/os-release")
                .unwrap_or_else(|_| env::consts::OS.to_string()),
            current_user: env::var("USER")
                .unwrap_or_else(|_| command_output("id", &["-un"]).unwrap_or_default()),
            default_shell: shell.clone(),
            default_workdir: self.config.exec.default_workdir.clone(),
            public_base_url: self.config.server.public_base_url.clone(),
            project_roots: self.config.bootstrap.project_roots.clone(),
            skill_roots: if self.config.skills.enabled {
                self.config.skills.roots.clone()
            } else {
                Vec::new()
            },
            common_available_tools: [
                "git",
                "rg",
                "fd",
                "cargo",
                "rustc",
                "python3",
                "node",
                "npm",
                "pnpm",
                "bun",
                "docker",
                "tailscale",
            ]
            .into_iter()
            .map(|name| ToolAvailability {
                name: name.to_string(),
                path: self.which(name, &shell),
            })
            .collect(),
            instructions,
        }
    }

    fn which(&self, name: &str, shell: &str) -> Option<String> {
        login_shell_output(
            shell,
            &format!("command -v -- {}", quote(name)),
            &self.config.exec.default_workdir,
            self.config.exec.login_default,
        )
        .filter(|s| !s.is_empty())
    }
}

fn command_output(cmd: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(cmd).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn login_shell_output(shell: &str, cmd: &str, workdir: &str, login: bool) -> Option<String> {
    let output = Command::new(shell)
        .arg(if login { "-lc" } else { "-c" })
        .arg(cmd)
        .current_dir(workdir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tempfile::TempDir;

    use super::*;
    use crate::config::{Config, ExecConfig};

    #[test]
    fn login_shell_tool_detection_sees_user_profile_path() {
        let tmp = TempDir::new().unwrap();
        let bin = tmp.path().join("bin");
        fs::create_dir(&bin).unwrap();
        let tool = bin.join("agentbox-test-tool");
        fs::write(&tool, "#!/bin/sh\n").unwrap();

        let shell = tmp.path().join("shell");
        fs::write(
            &shell,
            format!(
                "#!/bin/sh\nif [ \"$1\" = \"-lc\" ]; then shift; PATH=\"{}:$PATH\" exec /bin/sh -c \"$1\"; fi\nexec /bin/sh \"$@\"\n",
                bin.display()
            ),
        )
        .unwrap();
        let mut perms = fs::metadata(&shell).unwrap().permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut tool_perms = fs::metadata(&tool).unwrap().permissions();
            tool_perms.set_mode(0o755);
            fs::set_permissions(&tool, tool_perms).unwrap();
            perms.set_mode(0o755);
            fs::set_permissions(&shell, perms).unwrap();
        }

        let mut config = Config {
            exec: ExecConfig {
                default_shell: Some(shell.display().to_string()),
                default_workdir: tmp.path().display().to_string(),
                login_default: true,
                ..ExecConfig::default()
            },
            ..Config::default()
        };
        config.skills.enabled = false;
        let bootstrap = Bootstrapper::new(Arc::new(config));

        assert_eq!(
            bootstrap.which("agentbox-test-tool", shell.to_str().unwrap()),
            Some(bin.join("agentbox-test-tool").display().to_string())
        );
    }
}
