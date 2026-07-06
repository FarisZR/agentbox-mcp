use std::{
    collections::HashMap,
    env,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::{
    Form, Json, Router,
    extract::{Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    apply_patch::{self, ApplyPatchInput},
    auth::AuthLayer,
    bootstrap::Bootstrapper,
    config::{AuthMode, Config},
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
    pub fake_oauth_codes: Arc<FakeOAuthCodes>,
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

#[derive(Default)]
pub struct FakeOAuthCodes {
    inner: Mutex<HashMap<String, FakeOAuthCode>>,
}

#[derive(Debug)]
struct FakeOAuthCode {
    redirect_uri: String,
    expires_at: Instant,
    code_challenge: String,
    scope: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FakeOAuthAuthorizeQuery {
    response_type: Option<String>,
    client_id: Option<String>,
    redirect_uri: Option<String>,
    state: Option<String>,
    code_challenge: Option<String>,
    code_challenge_method: Option<String>,
    scope: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FakeOAuthTokenForm {
    grant_type: String,
    client_id: Option<String>,
    #[serde(rename = "client_secret")]
    client_credential: Option<String>,
    code: Option<String>,
    redirect_uri: Option<String>,
    code_verifier: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
enum FakeOAuthTokenError {
    InvalidGrant,
    InvalidRequest(&'static str),
}

impl FakeOAuthCodes {
    const CODE_TTL: Duration = Duration::from_secs(300);

    fn issue(&self, redirect_uri: String, code_challenge: String, scope: Option<String>) -> String {
        let code = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let now = Instant::now();
        let mut codes = self.inner.lock().expect("fake OAuth code store poisoned");
        codes.retain(|_, existing| existing.expires_at > now);
        codes.insert(
            code.clone(),
            FakeOAuthCode {
                redirect_uri,
                expires_at: now + Self::CODE_TTL,
                code_challenge,
                scope,
            },
        );
        code
    }

    fn consume(
        &self,
        code: &str,
        redirect_uri: &str,
        code_verifier: Option<&str>,
    ) -> Result<Option<String>, FakeOAuthTokenError> {
        let now = Instant::now();
        let mut codes = self.inner.lock().expect("fake OAuth code store poisoned");
        codes.retain(|_, existing| existing.expires_at > now);
        let Some(stored) = codes.remove(code) else {
            return Err(FakeOAuthTokenError::InvalidGrant);
        };
        if redirect_uri != stored.redirect_uri {
            return Err(FakeOAuthTokenError::InvalidGrant);
        }
        let Some(verifier) = code_verifier else {
            return Err(FakeOAuthTokenError::InvalidRequest("missing code_verifier"));
        };
        if !verify_pkce_s256(verifier, &stored.code_challenge) {
            return Err(FakeOAuthTokenError::InvalidGrant);
        }
        Ok(stored.scope)
    }
}

pub fn build_router(state: AppState) -> Router {
    let mcp_path = state.config.server.mcp_path.clone();
    Router::new()
        .route("/healthz", get(healthz))
        .route("/.well-known/oauth-protected-resource", get(oauth_resource))
        .route(
            "/.well-known/oauth-authorization-server",
            get(oauth_authorization_server),
        )
        .route("/oauth/authorize", get(oauth_authorize))
        .route("/oauth/token", post(finish_login))
        .route(&mcp_path, post(mcp_post).get(mcp_get))
        .with_state(state)
}

async fn healthz() -> impl IntoResponse {
    Json(json!({"status": "ok"}))
}

async fn oauth_resource(State(state): State<AppState>) -> impl IntoResponse {
    Json(state.auth.protected_resource_metadata())
}

async fn oauth_authorization_server(State(state): State<AppState>) -> Response {
    if state.config.auth.mode != AuthMode::FakeOAuth {
        return (StatusCode::NOT_FOUND, "fake OAuth mode is not enabled").into_response();
    }
    let issuer = state.config.auth.oauth.issuer.trim_end_matches('/');
    Json(json!({
        "issuer": issuer,
        "authorization_endpoint": format!("{issuer}/oauth/authorize"),
        "token_endpoint": format!("{issuer}/oauth/token"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["client_secret_post", "client_secret_basic"],
        "scopes_supported": state.config.auth.oauth.required_scopes,
    }))
    .into_response()
}

async fn oauth_authorize(
    State(state): State<AppState>,
    Query(query): Query<FakeOAuthAuthorizeQuery>,
) -> impl IntoResponse {
    if state.config.auth.mode != AuthMode::FakeOAuth {
        return (StatusCode::NOT_FOUND, "fake OAuth mode is not enabled").into_response();
    }
    if query.response_type.as_deref() != Some("code") {
        return (StatusCode::BAD_REQUEST, "unsupported response_type").into_response();
    }
    if query.code_challenge_method.as_deref() != Some("S256") {
        return (StatusCode::BAD_REQUEST, "unsupported code_challenge_method").into_response();
    }
    let Some(code_challenge) = query
        .code_challenge
        .filter(|challenge| !challenge.is_empty())
    else {
        return (StatusCode::BAD_REQUEST, "missing code_challenge").into_response();
    };
    if query.client_id.as_deref() != Some(&state.config.auth.fake_oauth.client_id) {
        return (StatusCode::BAD_REQUEST, "invalid client_id").into_response();
    }
    let Some(redirect_uri) = query.redirect_uri else {
        return (StatusCode::BAD_REQUEST, "missing redirect_uri").into_response();
    };
    if !is_allowed_chatgpt_redirect_uri(&state.config, &redirect_uri) {
        return (StatusCode::BAD_REQUEST, "invalid redirect_uri").into_response();
    }

    let code = state
        .fake_oauth_codes
        .issue(redirect_uri.clone(), code_challenge, query.scope);
    let mut target = redirect_uri;
    append_query_param(&mut target, "code", &code);
    if let Some(state_param) = query.state {
        append_query_param(&mut target, "state", &state_param);
    }
    Redirect::to(&target).into_response()
}

async fn finish_login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<FakeOAuthTokenForm>,
) -> impl IntoResponse {
    if state.config.auth.mode != AuthMode::FakeOAuth {
        return (StatusCode::NOT_FOUND, "fake OAuth mode is not enabled").into_response();
    }
    if let Err(response) = validate_fake_oauth_client(&headers, &form, &state.config) {
        return *response;
    }
    if form.grant_type != "authorization_code" {
        return oauth_error(StatusCode::BAD_REQUEST, "unsupported_grant_type", None);
    }
    let Some(code) = form.code.as_deref() else {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            Some("missing code"),
        );
    };
    let Some(redirect_uri) = form.redirect_uri.as_deref() else {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            Some("missing redirect_uri"),
        );
    };
    let scope =
        match state
            .fake_oauth_codes
            .consume(code, redirect_uri, form.code_verifier.as_deref())
        {
            Ok(scope) => scope,
            Err(FakeOAuthTokenError::InvalidGrant) => {
                return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant", None);
            }
            Err(FakeOAuthTokenError::InvalidRequest(description)) => {
                return oauth_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request",
                    Some(description),
                );
            }
        };
    let access_token = match state.auth.static_bearer_token() {
        Ok(token) => token,
        Err(_) => return oauth_error(StatusCode::INTERNAL_SERVER_ERROR, "server_error", None),
    };
    Json(json!({
        "access_token": access_token,
        "token_type": "Bearer",
        "expires_in": 31536000,
        "scope": scope.unwrap_or_else(|| state.config.auth.oauth.required_scopes.join(" ")),
    }))
    .into_response()
}

fn oauth_error(
    status: StatusCode,
    error: &'static str,
    description: Option<&'static str>,
) -> axum::response::Response {
    let mut body = json!({"error": error});
    if let Some(description) = description {
        body["error_description"] = Value::String(description.to_string());
    }
    (status, Json(body)).into_response()
}

fn validate_fake_oauth_client(
    headers: &HeaderMap,
    form: &FakeOAuthTokenForm,
    config: &Config,
) -> Result<(), Box<axum::response::Response>> {
    let Some(expected_credential) = fake_oauth_client_credential(config) else {
        return Err(Box::new(oauth_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            Some("fake OAuth client secret is not configured"),
        )));
    };
    let credentials = basic_client_credentials(headers).or_else(|| {
        Some((
            form.client_id.as_deref()?.to_string(),
            form.client_credential.as_deref()?.to_string(),
        ))
    });
    let Some((presented_client_id, presented_credential)) = credentials else {
        return Err(Box::new(oauth_error(
            StatusCode::UNAUTHORIZED,
            "invalid_client",
            Some("missing client authentication"),
        )));
    };
    if !constant_time_eq(
        presented_client_id.as_bytes(),
        config.auth.fake_oauth.client_id.as_bytes(),
    ) || !constant_time_eq(
        presented_credential.as_bytes(),
        expected_credential.as_bytes(),
    ) {
        return Err(Box::new(oauth_error(
            StatusCode::UNAUTHORIZED,
            "invalid_client",
            Some("invalid client authentication"),
        )));
    };
    Ok(())
}

fn fake_oauth_client_credential(config: &Config) -> Option<String> {
    match env::var(&config.auth.fake_oauth.client_credential_env) {
        Ok(secret) if !secret.is_empty() => Some(secret),
        _ => config
            .auth
            .fake_oauth
            .client_credential
            .clone()
            .filter(|secret| !secret.is_empty()),
    }
}

fn basic_client_credentials(headers: &HeaderMap) -> Option<(String, String)> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let encoded = value.strip_prefix("Basic ")?;
    let decoded = STANDARD.decode(encoded).ok()?;
    let text = String::from_utf8(decoded).ok()?;
    let (client_id, credential) = text.split_once(':')?;
    Some((client_id.to_string(), credential.to_string()))
}

fn is_allowed_chatgpt_redirect_uri(config: &Config, uri: &str) -> bool {
    config
        .auth
        .fake_oauth
        .allowed_redirect_uri_prefixes
        .iter()
        .any(|prefix| uri.starts_with(prefix))
        || config
            .auth
            .fake_oauth
            .allowed_redirect_uris
            .iter()
            .any(|allowed| uri == allowed)
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let max = a.len().max(b.len());
    let mut diff = a.len() ^ b.len();
    for i in 0..max {
        let aa = a.get(i).copied().unwrap_or(0);
        let bb = b.get(i).copied().unwrap_or(0);
        diff |= (aa ^ bb) as usize;
    }
    diff == 0
}

fn verify_pkce_s256(verifier: &str, expected_challenge: &str) -> bool {
    let digest = Sha256::digest(verifier.as_bytes());
    let challenge = URL_SAFE_NO_PAD.encode(digest);
    challenge == expected_challenge
}

fn append_query_param(url: &mut String, key: &str, value: &str) {
    let separator = if url.contains('?') { '&' } else { '?' };
    url.push(separator);
    url.push_str(&percent_encode(key));
    url.push('=');
    url.push_str(&percent_encode(value));
}

fn percent_encode(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push('%');
            encoded.push(HEX[(byte >> 4) as usize] as char);
            encoded.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    encoded
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
        ),
        tool(
            prefix,
            "write_stdin",
            "Write input to, or poll output from, a real-access persistent machine session.",
            write_input_schema(),
            Some(exec_output_schema()),
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
        ),
        tool(
            prefix,
            "bootstrap",
            "Return information about the persistent machine with real access.",
            obj_schema(vec![]),
            Some(bootstrap_output_schema()),
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
            ),
            tool(
                prefix,
                "load_skill",
                "Load skill instructions from the persistent machine.",
                obj_schema(vec![("skill", "string", true)]),
                Some(load_skill_output_schema()),
            ),
        ]);
    }

    tools
}

fn tool(
    prefix: &str,
    name: &str,
    description: &str,
    input_schema: Value,
    output_schema: Option<Value>,
) -> Value {
    let mut value = json!({
        "name": format!("{prefix}{name}"),
        "description": description,
        "inputSchema": input_schema,
        "annotations": {
            "readOnlyHint": true,
            "destructiveHint": false,
            "openWorldHint": false
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

    #[test]
    fn fake_oauth_codes_are_single_use_and_validate_redirect_and_pkce() {
        let codes = FakeOAuthCodes::default();
        let verifier = "correct horse battery staple";
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let code = codes.issue(
            "https://chatgpt.com/connector/oauth/callback".to_string(),
            challenge,
            Some("agentbox:exec".to_string()),
        );

        assert_eq!(
            codes.consume(
                &code,
                "https://chatgpt.com/connector/oauth/callback",
                Some(verifier),
            ),
            Ok(Some("agentbox:exec".to_string()))
        );
        assert_eq!(
            codes.consume(
                &code,
                "https://chatgpt.com/connector/oauth/callback",
                Some(verifier),
            ),
            Err(FakeOAuthTokenError::InvalidGrant)
        );
    }

    #[test]
    fn fake_oauth_codes_require_redirect_and_pkce_verifier() {
        let codes = FakeOAuthCodes::default();
        let verifier = "correct horse battery staple";
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));

        let missing_verifier_code = codes.issue(
            "https://chatgpt.com/connector/oauth/callback".to_string(),
            challenge.clone(),
            None,
        );
        assert_eq!(
            codes.consume(
                &missing_verifier_code,
                "https://chatgpt.com/connector/oauth/callback",
                None,
            ),
            Err(FakeOAuthTokenError::InvalidRequest("missing code_verifier"))
        );

        let wrong_redirect_code = codes.issue(
            "https://chatgpt.com/connector/oauth/callback".to_string(),
            challenge,
            None,
        );
        assert_eq!(
            codes.consume(
                &wrong_redirect_code,
                "https://chatgpt.com/connector/oauth/other",
                Some(verifier),
            ),
            Err(FakeOAuthTokenError::InvalidGrant)
        );
    }

    #[test]
    fn fake_oauth_redirects_are_restricted_to_chatgpt() {
        assert!(is_allowed_chatgpt_redirect_uri(
            &Config::default(),
            "https://chatgpt.com/connector/oauth/VXRLIj9YMlc"
        ));
        assert!(!is_allowed_chatgpt_redirect_uri(
            &Config::default(),
            "https://evil.example/callback"
        ));
    }

    #[test]
    fn query_params_are_percent_encoded() {
        let mut url = "https://chatgpt.com/connector/oauth/callback".to_string();
        append_query_param(&mut url, "state", "a b&c");
        assert_eq!(
            url,
            "https://chatgpt.com/connector/oauth/callback?state=a%20b%26c"
        );
    }
}
