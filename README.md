# agentbox-mcp

`agentbox-mcp` is a Rust Streamable HTTP MCP server for a dedicated Linux agent machine. It exposes Codex-style execution tools to ChatGPT so the model can run commands, interact with long-lived TTY sessions, apply patches, inspect local skills, and bootstrap itself into the real machine context.

The execution tools are intentionally unsandboxed after the MCP entrance check. Security belongs at the MCP entrance: use OAuth/JWKS if you have it, static bearer for clients that can send headers, or the documented secret URL path for the barebones ChatGPT setup.

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

`mode = "static_bearer"` reads the token from `agentbox_MCP_TOKEN` by default and requires `Authorization: Bearer <token>`. This is best for clients that can send custom headers.

ChatGPT web Developer Mode currently supports OAuth or no authentication for custom MCP connectors. If you do not have OAuth, use `config.chatgpt-simple.example.toml` or generate a private config:

```bash
./scripts/create-chatgpt-simple-config.sh agentbox-mcp.chatgpt.toml https://<tailscale-hostname>
cargo run -- --config agentbox-mcp.chatgpt.toml
```

Then add the printed secret URL in ChatGPT and choose `No authentication`.

`mode = "oauth_jwks"` remains available if you later add a real IdP. It fetches the configured JWKS and validates JWT issuer, audience, expiry/nbf through `jsonwebtoken`, and required scopes.

See [docs/chatgpt-connector.md](docs/chatgpt-connector.md) and [docs/security-model.md](docs/security-model.md).

## Tailscale Funnel

Start the server on `127.0.0.1:8787`, then run:

```bash
./scripts/setup-tailscale-funnel.sh
```

The script checks Tailscale login state, verifies `/healthz`, prints the Funnel command, and configures:

```bash
tailscale funnel --bg --https=443 --yes 127.0.0.1:8787
```

Use the full MCP URL from your config as the ChatGPT custom MCP connector URL. For the simple setup this looks like `https://<tailscale-hostname>/mcp/<64-hex-secret>`. See [docs/tailscale-funnel.md](docs/tailscale-funnel.md).

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
