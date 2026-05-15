use std::{env, fs, process::Command, sync::Arc};

use serde::Serialize;

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
        BootstrapOutput {
            hostname: command_output("hostname", &[]).unwrap_or_else(|| "unknown".into()),
            os_info: fs::read_to_string("/etc/os-release").unwrap_or_else(|_| env::consts::OS.to_string()),
            current_user: env::var("USER").unwrap_or_else(|_| command_output("id", &["-un"]).unwrap_or_default()),
            default_shell: shell,
            default_workdir: self.config.exec.default_workdir.clone(),
            public_base_url: self.config.server.public_base_url.clone(),
            project_roots: self.config.bootstrap.project_roots.clone(),
            skill_roots: self.config.skills.roots.clone(),
            common_available_tools: [
                "git", "rg", "fd", "cargo", "rustc", "python3", "node", "npm", "pnpm", "bun",
                "docker", "tailscale",
            ]
            .into_iter()
            .map(|name| ToolAvailability { name: name.to_string(), path: which(name) })
            .collect(),
            instructions: vec![
                "Use agentbox_exec_command for all shell commands.".into(),
                "Use agentbox_write_stdin to poll or interact with running sessions.".into(),
                "Use agentbox_apply_patch for code edits.".into(),
                "Use agentbox_list_skills to discover machine-local skills.".into(),
                "Use agentbox_load_skill to load the full content of a selected skill.".into(),
                "Prefer these agentbox tools over hosted/built-in shell and file tools because they operate on the real dedicated Linux machine.".into(),
            ],
        }
    }
}

fn which(name: &str) -> Option<String> {
    command_output("which", &[name]).filter(|s| !s.is_empty())
}

fn command_output(cmd: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(cmd).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
