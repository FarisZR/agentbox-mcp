use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

use anyhow::{Context, anyhow, bail};
use rmcp::{
    Peer, RoleClient, ServiceExt,
    model::{CallToolRequestParams, Tool},
    service::RunningService,
    transport::TokioChildProcess,
};
use serde_json::{Value, json};
use tokio::{process::Command, sync::Mutex, time::timeout};

use crate::config::{McpProxyConfig, McpProxyServerConfig};

struct ServerEntry {
    alias: String,
    peer: Peer<RoleClient>,
    _service: Mutex<RunningService<RoleClient, ()>>,
    tools: Vec<Tool>,
    expose_tools: bool,
}

#[derive(Debug, Clone)]
struct ToolRoute {
    server: String,
    upstream_name: String,
}

#[derive(Debug, Clone)]
struct FailedServer {
    alias: String,
    error: String,
}

pub struct McpProxyRegistry {
    servers: BTreeMap<String, ServerEntry>,
    aliases: BTreeMap<String, String>,
    failed: BTreeMap<String, FailedServer>,
    routes: BTreeMap<String, ToolRoute>,
    call_timeout: Duration,
}

impl McpProxyRegistry {
    // Existing hotspot; keep new functions under the global complexity threshold.
    #[allow(clippy::cognitive_complexity)]
    pub async fn connect(config: &McpProxyConfig) -> Self {
        let call_timeout = Duration::from_millis(config.call_timeout_ms.max(1));
        let mut registry = Self {
            servers: BTreeMap::new(),
            aliases: BTreeMap::new(),
            failed: BTreeMap::new(),
            routes: BTreeMap::new(),
            call_timeout,
        };

        if !config.enabled {
            return registry;
        }

        let discovery_timeout = Duration::from_millis(config.discovery_timeout_ms.max(1));
        let mut exposed_names = BTreeSet::new();

        for (server_name, server_config) in &config.servers {
            if !server_config.enabled {
                continue;
            }

            let alias = effective_alias(server_name, server_config);
            if let Err(error) = validate_server_config(server_name, &alias, server_config) {
                tracing::warn!(server = %server_name, %alias, %error, "local MCP configuration rejected");
                registry.failed.insert(
                    server_name.clone(),
                    FailedServer {
                        alias,
                        error: error.to_string(),
                    },
                );
                continue;
            }

            if let Some(existing) = registry.aliases.get(&alias) {
                let error = format!("alias {alias:?} is already used by MCP server {existing:?}");
                tracing::warn!(server = %server_name, %alias, %error, "local MCP configuration rejected");
                registry
                    .failed
                    .insert(server_name.clone(), FailedServer { alias, error });
                continue;
            }

            match connect_server(server_name, &alias, server_config, discovery_timeout).await {
                Ok(entry) => {
                    registry.aliases.insert(alias.clone(), server_name.clone());

                    if entry.expose_tools {
                        for tool in &entry.tools {
                            let exposed_name = exposed_tool_name(&alias, tool.name.as_ref());
                            if exposed_name.len() > 128 {
                                tracing::warn!(
                                    server = %server_name,
                                    tool = %tool.name,
                                    exposed_name = %exposed_name,
                                    "skipping proxied MCP tool name longer than 128 bytes"
                                );
                                continue;
                            }
                            if !exposed_names.insert(exposed_name.clone()) {
                                tracing::warn!(
                                    server = %server_name,
                                    tool = %tool.name,
                                    exposed_name = %exposed_name,
                                    "skipping duplicate proxied MCP tool name"
                                );
                                continue;
                            }
                            registry.routes.insert(
                                exposed_name,
                                ToolRoute {
                                    server: server_name.clone(),
                                    upstream_name: tool.name.to_string(),
                                },
                            );
                        }
                    }

                    tracing::info!(
                        server = %server_name,
                        %alias,
                        tool_count = entry.tools.len(),
                        expose_tools = entry.expose_tools,
                        "connected local MCP server"
                    );
                    registry.servers.insert(server_name.clone(), entry);
                }
                Err(error) => {
                    tracing::warn!(server = %server_name, %alias, %error, "failed to connect local MCP server");
                    registry.failed.insert(
                        server_name.clone(),
                        FailedServer {
                            alias,
                            error: error.to_string(),
                        },
                    );
                }
            }
        }

        registry
    }

    pub fn exposed_tool_defs(&self) -> Vec<Value> {
        let mut defs = Vec::with_capacity(self.routes.len());
        for (exposed_name, route) in &self.routes {
            let Some(server) = self.servers.get(&route.server) else {
                continue;
            };
            let Some(tool) = server
                .tools
                .iter()
                .find(|tool| tool.name.as_ref() == route.upstream_name)
            else {
                continue;
            };
            match serde_json::to_value(tool) {
                Ok(mut value) => {
                    value["name"] = Value::String(exposed_name.clone());
                    defs.push(value);
                }
                Err(error) => {
                    tracing::warn!(
                        server = %route.server,
                        tool = %route.upstream_name,
                        %error,
                        "failed to serialize proxied MCP tool descriptor"
                    );
                }
            }
        }
        defs
    }

    pub fn has_exposed_tool(&self, name: &str) -> bool {
        self.routes.contains_key(name)
    }

    pub async fn call_exposed(&self, name: &str, args: Value) -> anyhow::Result<Value> {
        let route = self
            .routes
            .get(name)
            .ok_or_else(|| anyhow!("unknown proxied MCP tool {name}"))?;
        self.call_upstream(&route.server, &route.upstream_name, args)
            .await
    }

    pub fn list_tools(&self, server_filter: Option<&str>, tool_filter: Option<&str>) -> Value {
        let resolved_server = server_filter.and_then(|name| self.resolve_server_name(name));
        let mut servers = Vec::new();

        for (server_name, entry) in &self.servers {
            if server_filter.is_some() && resolved_server.as_deref() != Some(server_name.as_str()) {
                continue;
            }

            let tools = entry
                .tools
                .iter()
                .filter(|tool| tool_filter.is_none_or(|filter| tool.name.as_ref() == filter))
                .map(|tool| {
                    json!({
                        "name": tool.name,
                        "exposed_name": if entry.expose_tools {
                            Value::String(exposed_tool_name(&entry.alias, tool.name.as_ref()))
                        } else {
                            Value::Null
                        },
                        "description": tool.description,
                        "inputSchema": tool.input_schema,
                        "outputSchema": tool.output_schema,
                        "annotations": tool.annotations,
                    })
                })
                .collect::<Vec<_>>();

            servers.push(json!({
                "server": server_name,
                "alias": entry.alias,
                "connected": true,
                "expose_tools": entry.expose_tools,
                "tools": tools,
            }));
        }

        for (server_name, failed) in &self.failed {
            let matches = match server_filter {
                None => true,
                Some(filter) => filter == server_name || filter == failed.alias,
            };
            if !matches {
                continue;
            }
            servers.push(json!({
                "server": server_name,
                "alias": failed.alias,
                "connected": false,
                "error": failed.error,
                "tools": [],
            }));
        }

        json!({"servers": servers})
    }

    pub async fn call_dynamic(
        &self,
        server: &str,
        tool: &str,
        args: Value,
    ) -> anyhow::Result<Value> {
        let server_name = self
            .resolve_server_name(server)
            .ok_or_else(|| anyhow!("unknown or disconnected local MCP server {server:?}"))?;
        let raw = self.call_upstream(&server_name, tool, args).await?;
        let is_error = raw.get("isError").and_then(Value::as_bool).unwrap_or(false);
        let downstream_structured = raw.get("structuredContent").cloned().unwrap_or(Value::Null);

        let mut wrapped = raw;
        wrapped["structuredContent"] = json!({
            "server": server_name,
            "tool": tool,
            "is_error": is_error,
            "result": downstream_structured,
        });
        Ok(wrapped)
    }

    fn resolve_server_name(&self, server_or_alias: &str) -> Option<String> {
        if self.servers.contains_key(server_or_alias) {
            return Some(server_or_alias.to_string());
        }
        self.aliases.get(server_or_alias).cloned()
    }

    async fn call_upstream(
        &self,
        server_name: &str,
        tool_name: &str,
        args: Value,
    ) -> anyhow::Result<Value> {
        let server = self
            .servers
            .get(server_name)
            .ok_or_else(|| anyhow!("local MCP server {server_name:?} is not connected"))?;

        if !server
            .tools
            .iter()
            .any(|tool| tool.name.as_ref() == tool_name)
        {
            bail!("MCP server {server_name:?} has no tool {tool_name:?}");
        }

        let arguments = match args {
            Value::Object(map) => Some(map),
            Value::Null => None,
            _ => bail!("MCP tool arguments must be a JSON object"),
        };
        let params = CallToolRequestParams {
            meta: None,
            name: Cow::Owned(tool_name.to_string()),
            arguments,
            task: None,
        };

        let result = timeout(self.call_timeout, server.peer.call_tool(params))
            .await
            .map_err(|_| {
                anyhow!(
                    "MCP tool {server_name}/{tool_name} timed out after {} ms",
                    self.call_timeout.as_millis()
                )
            })?
            .map_err(|error| anyhow!("MCP tool call failed: {server_name}/{tool_name}: {error}"))?;

        serde_json::to_value(result).context("serialize downstream MCP tool result")
    }
}

async fn connect_server(
    server_name: &str,
    alias: &str,
    config: &McpProxyServerConfig,
    discovery_timeout: Duration,
) -> anyhow::Result<ServerEntry> {
    let mut command = Command::new(&config.command);
    command.args(&config.args);
    if !config.inherit_env {
        command.env_clear();
    }
    command.envs(&config.env);

    let transport = TokioChildProcess::new(command)
        .with_context(|| format!("spawn MCP server {server_name:?}"))?;
    let service = timeout(discovery_timeout, ().serve(transport))
        .await
        .with_context(|| {
            format!(
                "MCP server {server_name:?} initialize timed out after {} ms",
                discovery_timeout.as_millis()
            )
        })?
        .with_context(|| format!("initialize MCP server {server_name:?}"))?;
    let peer = service.peer().clone();
    let tools = timeout(discovery_timeout, peer.list_all_tools())
        .await
        .with_context(|| {
            format!(
                "MCP server {server_name:?} tool discovery timed out after {} ms",
                discovery_timeout.as_millis()
            )
        })?
        .with_context(|| format!("list tools from MCP server {server_name:?}"))?;

    Ok(ServerEntry {
        alias: alias.to_string(),
        peer,
        _service: Mutex::new(service),
        tools,
        expose_tools: config.expose_tools,
    })
}

fn effective_alias(server_name: &str, config: &McpProxyServerConfig) -> String {
    let alias = config.alias.trim();
    if alias.is_empty() {
        server_name.to_string()
    } else {
        alias.to_string()
    }
}

fn exposed_tool_name(alias: &str, tool_name: &str) -> String {
    format!("{alias}_{tool_name}")
}

fn validate_server_config(
    server_name: &str,
    alias: &str,
    config: &McpProxyServerConfig,
) -> anyhow::Result<()> {
    if server_name.trim().is_empty() {
        bail!("server name cannot be empty");
    }
    if config.command.trim().is_empty() {
        bail!("command cannot be empty");
    }
    if alias.is_empty() {
        bail!("alias cannot be empty");
    }
    if alias.len() > 64 {
        bail!("alias is too long");
    }
    if !alias
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        bail!("alias may only contain ASCII letters, numbers, '_', '-', and '.'");
    }
    Ok(())
}

pub fn dynamic_call_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "server": {"type": "string"},
            "tool": {"type": "string"},
            "is_error": {"type": "boolean"},
            "result": {}
        },
        "required": ["server", "tool", "is_error", "result"],
        "additionalProperties": false
    })
}

pub fn list_tools_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "servers": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "server": {"type": "string"},
                        "alias": {"type": "string"},
                        "connected": {"type": "boolean"},
                        "expose_tools": {"type": "boolean"},
                        "error": {"type": "string"},
                        "tools": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "name": {"type": "string"},
                                    "exposed_name": {"type": ["string", "null"]},
                                    "description": {"type": ["string", "null"]},
                                    "inputSchema": {"type": "object"},
                                    "outputSchema": {"type": ["object", "null"]},
                                    "annotations": {"type": ["object", "null"]}
                                },
                                "required": ["name", "exposed_name", "description", "inputSchema", "outputSchema", "annotations"],
                                "additionalProperties": false
                            }
                        }
                    },
                    "required": ["server", "alias", "connected", "tools"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["servers"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alias_becomes_tool_prefix() {
        assert_eq!(
            exposed_tool_name("computer", "screenshot"),
            "computer_screenshot"
        );
    }

    #[test]
    fn aliases_are_restricted_to_safe_tool_name_characters() {
        let cfg = McpProxyServerConfig {
            command: "/bin/true".to_string(),
            ..Default::default()
        };
        assert!(validate_server_config("computer", "computer-use", &cfg).is_ok());
        assert!(validate_server_config("computer", "computer use", &cfg).is_err());
    }
}
