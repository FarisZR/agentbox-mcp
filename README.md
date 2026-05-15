# agentbox-mcp

`agentbox-mcp` is a Rust Streamable HTTP MCP server for a dedicated Linux agent machine. It exposes Codex-style execution tools to ChatGPT so the model can run commands, interact with long-lived TTY sessions, apply patches, inspect local skills, and bootstrap itself into the real machine context.

The execution tools are intentionally unsandboxed after the HTTP request is authenticated. Security belongs at the MCP entrance: use strong bearer or OAuth/JWKS authentication before exposing this server.

## Build

```bash
cargo build
```

## Run Locally

```bash
export agentbox_MCP_TOKEN="$(openssl rand -hex 32)"
cargo run -- --config config.example.toml
curl http://127.0.0.1:8787/healthz
```

Smoke initialize:

```bash
./scripts/mcp-smoke.sh
```

## Test

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
./scripts/closed-loop.sh
```

## Authentication

`mode = "none"` is only for localhost development. `mode = "static_bearer"` reads the token from `agentbox_MCP_TOKEN` by default and requires `Authorization: Bearer <token>`.

`mode = "oauth_jwks"` fetches the configured JWKS and validates JWT issuer, audience, expiry/nbf through `jsonwebtoken`, and required scopes. It also exposes `/.well-known/oauth-protected-resource` and returns a bearer challenge on missing/invalid auth. See [docs/security-model.md](docs/security-model.md).

For ChatGPT OAuth setup, see [docs/chatgpt-connector.md](docs/chatgpt-connector.md). `agentbox-mcp` is an OAuth protected resource, not an authorization server; use your IdP or OAuth gateway for the authorization and token endpoints, then configure this server to validate its JWTs.

## Tailscale Funnel

Start the server on `127.0.0.1:8787`, then run:

```bash
./scripts/setup-tailscale-funnel.sh
```

The script checks Tailscale login state, verifies `/healthz`, prints the Funnel command, and configures:

```bash
tailscale funnel --bg --https=443 --yes 127.0.0.1:8787
```

Use `https://<tailscale-hostname>/mcp` as the ChatGPT custom MCP connector URL. See [docs/tailscale-funnel.md](docs/tailscale-funnel.md).

## Tool Reference

Default names use the `agentbox_` prefix:

- `agentbox_exec_command`: run a shell command and return output or `session_id`.
- `agentbox_write_stdin`: poll or write to a running TTY session.
- `agentbox_apply_patch`: apply a Codex-style patch to the real filesystem.
- `agentbox_bootstrap`: return machine profile and usage instructions.
- `agentbox_list_skills`: compact skill catalog only.
- `agentbox_load_skill`: full selected skill instructions.

Set `[tools].prefix = ""` to expose Codex-style names like `exec_command` and `write_stdin`.

## Known Limitations

The MCP endpoint currently returns JSON responses for POST requests and `405` for GET SSE streams. This is allowed for simple Streamable HTTP request/response servers, but it does not implement server-to-client notifications or resumable SSE. The patch tool uses OpenAI Codex’s upstream `codex-apply-patch` crate pinned to the commit recorded in [docs/implementation-log.md](docs/implementation-log.md).
