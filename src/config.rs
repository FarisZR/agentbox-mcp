use std::{env, fs, net::IpAddr, path::Path};

use anyhow::Context;
use clap::Parser;
use serde::Deserialize;

#[derive(Debug, Parser)]
pub struct Cli {
    #[arg(long, env = "agentbox_MCP_CONFIG")]
    pub config: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub server: ServerConfig,
    pub tools: ToolsConfig,
    pub exec: ExecConfig,
    pub auth: AuthConfig,
    pub skills: SkillsConfig,
    pub bootstrap: BootstrapConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub bind: String,
    pub public_base_url: Option<String>,
    pub mcp_path: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ToolsConfig {
    pub prefix: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ExecConfig {
    pub default_workdir: String,
    pub default_shell: Option<String>,
    pub login_default: bool,
    pub default_yield_time_ms: u64,
    pub min_yield_time_ms: u64,
    pub max_yield_time_ms: u64,
    pub default_max_output_tokens: usize,
    pub hard_max_output_tokens: usize,
    pub max_processes: usize,
    pub session_idle_ttl_seconds: u64,
    pub overlay_codex_env: bool,
    pub deterministic_session_ids: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AuthConfig {
    pub mode: AuthMode,
    pub static_bearer: StaticBearerConfig,
    pub oauth: OAuthConfig,
    pub fake_oauth: FakeOAuthConfig,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    None,
    #[default]
    StaticBearer,
    FakeOAuth,
    OAuthJwks,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct StaticBearerConfig {
    pub token_env: String,
    pub token: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct OAuthConfig {
    pub resource: String,
    pub issuer: String,
    pub jwks_url: String,
    pub audience: String,
    pub required_scopes: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct FakeOAuthConfig {
    pub client_id: String,
    pub client_credential_env: String,
    pub client_credential: Option<String>,
    pub allowed_redirect_uri_prefixes: Vec<String>,
    pub allowed_redirect_uris: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SkillsConfig {
    pub enabled: bool,
    pub roots: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct BootstrapConfig {
    pub project_roots: Vec<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:8787".to_string(),
            public_base_url: None,
            mcp_path: "/mcp".to_string(),
        }
    }
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            prefix: "agentbox_".to_string(),
        }
    }
}

impl Default for ExecConfig {
    fn default() -> Self {
        Self {
            default_workdir: env::var("HOME").unwrap_or_else(|_| "/tmp".to_string()),
            default_shell: None,
            login_default: true,
            default_yield_time_ms: 1000,
            min_yield_time_ms: 50,
            max_yield_time_ms: 30000,
            default_max_output_tokens: 6000,
            hard_max_output_tokens: 50000,
            max_processes: 64,
            session_idle_ttl_seconds: 86400,
            overlay_codex_env: true,
            deterministic_session_ids: false,
        }
    }
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            mode: AuthMode::StaticBearer,
            static_bearer: StaticBearerConfig::default(),
            oauth: OAuthConfig::default(),
            fake_oauth: FakeOAuthConfig::default(),
        }
    }
}

impl Default for StaticBearerConfig {
    fn default() -> Self {
        Self {
            token_env: "agentbox_MCP_TOKEN".to_string(),
            token: None,
        }
    }
}

impl Default for OAuthConfig {
    fn default() -> Self {
        Self {
            resource: "https://agentbox.example.com".to_string(),
            issuer: "https://auth.example.com".to_string(),
            jwks_url: "https://auth.example.com/.well-known/jwks.json".to_string(),
            audience: "https://agentbox.example.com".to_string(),
            required_scopes: vec!["agentbox:exec".to_string()],
        }
    }
}

impl Default for FakeOAuthConfig {
    fn default() -> Self {
        Self {
            client_id: "chatgpt-agentbox".to_string(),
            client_credential_env: "agentbox_FAKE_OAUTH_CLIENT_CREDENTIAL".to_string(),
            client_credential: None,
            allowed_redirect_uri_prefixes: vec!["https://chatgpt.com/connector/oauth/".to_string()],
            allowed_redirect_uris: vec![
                "https://chatgpt.com/connector_platform_oauth_redirect".to_string(),
            ],
        }
    }
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            roots: vec!["~/.agents".to_string(), "/opt/agentbox/skills".to_string()],
        }
    }
}

impl Config {
    pub fn load(path: Option<&str>) -> anyhow::Result<Self> {
        let env_path = env::var("agentbox_MCP_CONFIG").ok();
        let selected_path = path.or(env_path.as_deref());
        let mut cfg = if let Some(path) = selected_path {
            let text = fs::read_to_string(path).with_context(|| format!("read config {path}"))?;
            toml::from_str::<Self>(&text).with_context(|| format!("parse config {path}"))?
        } else if Path::new("agentbox-mcp.toml").exists() {
            let text = fs::read_to_string("agentbox-mcp.toml")?;
            toml::from_str::<Self>(&text).context("parse agentbox-mcp.toml")?
        } else {
            Self::default()
        };
        if let Ok(bind) = env::var("agentbox_MCP_BIND") {
            cfg.server.bind = bind;
        }
        if let Ok(url) = env::var("agentbox_MCP_PUBLIC_BASE_URL") {
            cfg.server.public_base_url = Some(url);
        }
        if env::var("agentbox_DETERMINISTIC_SESSION_IDS")
            .ok()
            .as_deref()
            == Some("1")
        {
            cfg.exec.deterministic_session_ids = true;
        }
        Ok(cfg)
    }

    pub fn warn_if_insecure_auth(&self) {
        if self.auth.mode == AuthMode::None {
            let host = self.server.bind.split(':').next().unwrap_or_default();
            let local = host
                .parse::<IpAddr>()
                .map(|ip| ip.is_loopback())
                .unwrap_or(host == "localhost");
            if !local {
                tracing::warn!(
                    "auth mode is none while bind address is not localhost; do not expose this endpoint"
                );
            }
        }
    }
}

pub fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/")
        && let Ok(home) = env::var("HOME")
    {
        return format!("{home}/{rest}");
    }
    path.to_string()
}
