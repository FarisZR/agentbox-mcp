use std::sync::Arc;

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    apply_patch::{self, ApplyPatchInput},
    auth::AuthLayer,
    bootstrap::Bootstrapper,
    config::Config,
    exec::{ExecCommandInput, ProcessManager, WriteStdinInput},
    skills::{ListSkillsInput, LoadSkillInput, SkillCatalog},
};

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub manager: Arc<ProcessManager>,
    pub auth: Arc<AuthLayer>,
    pub skills: Arc<SkillCatalog>,
    pub bootstrap: Arc<Bootstrapper>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i64,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

pub fn build_router(state: AppState) -> Router {
    let mcp_path = state.config.server.mcp_path.clone();
    Router::new()
        .route("/healthz", get(healthz))
        .route("/.well-known/oauth-protected-resource", get(oauth_resource))
        .route(&mcp_path, post(mcp_post).get(mcp_get))
        .with_state(state)
}

async fn healthz() -> impl IntoResponse {
    Json(json!({"status": "ok"}))
}

async fn oauth_resource(State(state): State<AppState>) -> impl IntoResponse {
    Json(state.auth.protected_resource_metadata())
}

async fn mcp_get() -> impl IntoResponse {
    (
        StatusCode::METHOD_NOT_ALLOWED,
        "SSE stream is not implemented; use POST Streamable HTTP",
    )
}

async fn mcp_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<JsonRpcRequest>,
) -> impl IntoResponse {
    if let Err(err) = state.auth.check(&headers) {
        return err.into_response();
    }
    let id = req.id.clone().unwrap_or(Value::Null);
    if req.jsonrpc != "2.0" {
        return Json(error(id, -32600, "invalid jsonrpc version", None)).into_response();
    }
    if req.id.is_none() {
        tracing::info!(method = %req.method, notification = true, "mcp request");
        return (StatusCode::ACCEPTED, "").into_response();
    }
    tracing::info!(method = %req.method, "mcp request");
    let result = match req.method.as_str() {
        "initialize" => Ok(initialize_result()),
        "tools/list" => Ok(json!({"tools": tool_defs(&state.config)})),
        "tools/call" => call_tool(state, req.params).await,
        _ => Err(jsonrpc_err(
            -32601,
            format!("unknown method {}", req.method),
            None,
        )),
    };
    match result {
        Ok(value) => Json(JsonRpcResponse {
            jsonrpc: "2.0",
            id,
            result: Some(value),
            error: None,
        })
        .into_response(),
        Err(err) => Json(JsonRpcResponse {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(err),
        })
        .into_response(),
    }
}

async fn call_tool(state: AppState, params: Value) -> Result<Value, JsonRpcError> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| jsonrpc_err(-32602, "tools/call missing name", None))?;
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let prefix = &state.config.tools.prefix;
    let bare = name.strip_prefix(prefix).unwrap_or(name);
    match bare {
        "exec_command" => {
            let input: ExecCommandInput = serde_json::from_value(args).map_err(invalid_params)?;
            let out = state.manager.exec_command(input).await.map_err(tool_err)?;
            Ok(
                json!({"content":[{"type":"text","text": serde_json::to_string(&out).unwrap_or_default()}], "structuredContent": out}),
            )
        }
        "write_stdin" => {
            let input: WriteStdinInput = serde_json::from_value(args).map_err(invalid_params)?;
            let out = state.manager.write_stdin(input).await.map_err(tool_err)?;
            Ok(
                json!({"content":[{"type":"text","text": serde_json::to_string(&out).unwrap_or_default()}], "structuredContent": out}),
            )
        }
        "apply_patch" => {
            let input: ApplyPatchInput = serde_json::from_value(args).map_err(invalid_params)?;
            let out = apply_patch::apply(input, &state.config.exec.default_workdir).await;
            Ok(
                json!({"content":[{"type":"text","text": out.output.clone()}], "structuredContent": out}),
            )
        }
        "bootstrap" => {
            let out = state.bootstrap.profile();
            Ok(
                json!({"content":[{"type":"text","text": serde_json::to_string_pretty(&out).unwrap_or_default()}], "structuredContent": out}),
            )
        }
        "list_skills" => {
            if !state.config.skills.enabled {
                return Err(jsonrpc_err(-32602, "skill tools are disabled", None));
            }
            let input: ListSkillsInput = serde_json::from_value(args).map_err(invalid_params)?;
            let out = state.skills.list(input);
            Ok(
                json!({"content":[{"type":"text","text": serde_json::to_string_pretty(&out).unwrap_or_default()}], "structuredContent": out}),
            )
        }
        "load_skill" => {
            if !state.config.skills.enabled {
                return Err(jsonrpc_err(-32602, "skill tools are disabled", None));
            }
            let input: LoadSkillInput = serde_json::from_value(args).map_err(invalid_params)?;
            let out = state.skills.load(input).map_err(tool_err)?;
            Ok(
                json!({"content":[{"type":"text","text": out.content.clone()}], "structuredContent": out}),
            )
        }
        _ => Err(jsonrpc_err(-32602, format!("unknown tool {name}"), None)),
    }
}

fn initialize_result() -> Value {
    json!({
        "protocolVersion": "2025-06-18",
        "capabilities": {"tools": {}},
        "serverInfo": {"name": "agentbox-mcp", "version": env!("CARGO_PKG_VERSION")},
        "instructions": "Dedicated unsandboxed Linux agentbox MCP. Authenticate at the MCP entrance; tools run on the real machine."
    })
}

fn tool_defs(config: &Config) -> Vec<Value> {
    let prefix = &config.tools.prefix;
    let mut tools = vec![
        tool(
            prefix,
            "exec_command",
            "Run a shell command on the persistent machine with real access.",
            exec_input_schema(),
            Some(exec_output_schema()),
            false,
            true,
            true,
        ),
        tool(
            prefix,
            "write_stdin",
            "Write input to, or poll output from, a real-access persistent machine session.",
            write_input_schema(),
            Some(exec_output_schema()),
            false,
            true,
            true,
        ),
        tool(
            prefix,
            "apply_patch",
            "Apply a patch to files on the persistent machine with real access.",
            obj_schema(vec![
                ("patch", "string", true),
                ("workdir", "string", false),
            ]),
            Some(apply_patch_output_schema()),
            false,
            true,
            true,
        ),
        tool(
            prefix,
            "bootstrap",
            "Return information about the persistent machine with real access.",
            obj_schema(vec![]),
            Some(bootstrap_output_schema()),
            true,
            false,
            false,
        ),
    ];

    if config.skills.enabled {
        tools.extend([
            tool(
                prefix,
                "list_skills",
                "List skills available on the persistent machine.",
                obj_schema(vec![
                    ("query", "string", false),
                    ("include_paths", "boolean", false),
                    ("max_results", "number", false),
                ]),
                Some(list_skills_output_schema()),
                true,
                false,
                false,
            ),
            tool(
                prefix,
                "load_skill",
                "Load skill instructions from the persistent machine.",
                obj_schema(vec![("skill", "string", true)]),
                Some(load_skill_output_schema()),
                true,
                false,
                false,
            ),
        ]);
    }

    tools
}

#[allow(clippy::too_many_arguments)]
fn tool(
    prefix: &str,
    name: &str,
    description: &str,
    input_schema: Value,
    output_schema: Option<Value>,
    read_only: bool,
    open_world: bool,
    destructive: bool,
) -> Value {
    let mut value = json!({
        "name": format!("{prefix}{name}"),
        "description": description,
        "inputSchema": input_schema,
        "annotations": {
            "readOnlyHint": read_only,
            "openWorldHint": open_world,
            "destructiveHint": destructive
        }
    });
    if let Some(schema) = output_schema {
        value["outputSchema"] = schema;
    }
    value
}

fn exec_input_schema() -> Value {
    obj_schema(vec![
        ("cmd", "string", true),
        ("workdir", "string", false),
        ("shell", "string", false),
        ("tty", "boolean", false),
        ("login", "boolean", false),
        ("yield_time_ms", "number", false),
        ("max_output_tokens", "number", false),
    ])
}

fn write_input_schema() -> Value {
    obj_schema(vec![
        ("session_id", "number", true),
        ("chars", "string", false),
        ("yield_time_ms", "number", false),
        ("max_output_tokens", "number", false),
    ])
}

fn obj_schema(fields: Vec<(&str, &str, bool)>) -> Value {
    let mut props = serde_json::Map::new();
    let mut req = Vec::new();
    for (name, ty, required) in fields {
        props.insert(name.to_string(), json!({"type": ty}));
        if required {
            req.push(Value::String(name.to_string()));
        }
    }
    json!({"type":"object", "properties": props, "required": req, "additionalProperties": false})
}

fn exec_output_schema() -> Value {
    json!({
        "type":"object",
        "properties": {
            "chunk_id": {"type":"string"},
            "wall_time_seconds": {"type":"number"},
            "exit_code": {"type":"number"},
            "session_id": {"type":"number"},
            "original_token_count": {"type":"number"},
            "output": {"type":"string"}
        },
        "required": ["wall_time_seconds", "output"],
        "additionalProperties": false
    })
}

fn apply_patch_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "status": {"type": "string", "enum": ["completed", "failed"]},
            "output": {"type": "string"}
        },
        "required": ["status", "output"],
        "additionalProperties": false
    })
}

fn bootstrap_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "hostname": {"type": "string"},
            "os_info": {"type": "string"},
            "current_user": {"type": "string"},
            "default_shell": {"type": "string"},
            "default_workdir": {"type": "string"},
            "public_base_url": {"type": ["string", "null"]},
            "project_roots": {"type": "array", "items": {"type": "string"}},
            "skill_roots": {"type": "array", "items": {"type": "string"}},
            "common_available_tools": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string"},
                        "path": {"type": ["string", "null"]}
                    },
                    "required": ["name"],
                    "additionalProperties": false
                }
            },
            "instructions": {"type": "array", "items": {"type": "string"}}
        },
        "required": [
            "hostname",
            "os_info",
            "current_user",
            "default_shell",
            "default_workdir",
            "project_roots",
            "skill_roots",
            "common_available_tools",
            "instructions"
        ],
        "additionalProperties": false
    })
}

fn list_skills_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "skill_roots": {"type": "array", "items": {"type": "string"}},
            "skills": {
                "type": "array",
                "items": skill_meta_schema()
            }
        },
        "required": ["skill_roots", "skills"],
        "additionalProperties": false
    })
}

fn load_skill_output_schema() -> Value {
    let mut schema = skill_meta_schema();
    if let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) {
        properties.insert("content".to_string(), json!({"type": "string"}));
    }
    if let Some(required) = schema.get_mut("required").and_then(Value::as_array_mut) {
        required.push(Value::String("content".to_string()));
    }
    schema
}

fn skill_meta_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "name": {"type": "string"},
            "title": {"type": "string"},
            "description": {"type": "string"},
            "tags": {"type": "array", "items": {"type": "string"}},
            "path": {"type": ["string", "null"]},
            "instruction_file": {"type": ["string", "null"]}
        },
        "required": ["name", "title", "description", "tags"],
        "additionalProperties": false
    })
}

fn error(id: Value, code: i64, message: &str, data: Option<Value>) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(jsonrpc_err(code, message, data)),
    }
}

fn jsonrpc_err(code: i64, message: impl Into<String>, data: Option<Value>) -> JsonRpcError {
    JsonRpcError {
        code,
        message: message.into(),
        data,
    }
}

fn invalid_params(err: serde_json::Error) -> JsonRpcError {
    jsonrpc_err(-32602, err.to_string(), None)
}

fn tool_err(err: impl std::fmt::Display) -> JsonRpcError {
    jsonrpc_err(-32000, err.to_string(), None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_descriptions_are_short_but_identify_persistent_machine() {
        let config = Config::default();
        let tools = tool_defs(&config);
        for tool in tools {
            let description = tool
                .get("description")
                .and_then(Value::as_str)
                .expect("tool description");
            assert!(
                description.len() <= 90,
                "description too long: {description}"
            );
            if tool
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| name != "agentbox_list_skills" && name != "agentbox_load_skill")
            {
                assert!(
                    description.contains("persistent machine"),
                    "description should mention persistent machine: {description}"
                );
                assert!(
                    description.contains("real access") || description.contains("real-access"),
                    "description should distinguish the environment: {description}"
                );
            }
        }
    }

    #[test]
    fn skill_tools_can_be_disabled() {
        let mut config = Config::default();
        config.skills.enabled = false;
        let names = tool_defs(&config)
            .into_iter()
            .map(|tool| {
                tool.get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string()
            })
            .collect::<Vec<_>>();
        assert!(!names.contains(&"agentbox_list_skills".to_string()));
        assert!(!names.contains(&"agentbox_load_skill".to_string()));
        assert!(names.contains(&"agentbox_exec_command".to_string()));
    }
}
