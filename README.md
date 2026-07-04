# agentbox-mcp

`agentbox-mcp` is a Rust Streamable HTTP MCP server for a dedicated Linux agent machine. It exposes Codex-style execution tools to ChatGPT so the model can run commands, interact with long-lived TTY sessions, apply patches, inspect local skills, and bootstrap itself into the real machine context.

The execution tools are intentionally unsandboxed after the MCP entrance check. Security belongs at the MCP entrance: use fake OAuth for ChatGPT accounts that only expose OAuth connector auth, use a strong static bearer token where API-key connector auth is available, or use OAuth/JWKS if you later add an identity provider.

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

`mode = "fake_oauth"` exposes a minimal OAuth authorization-code facade for ChatGPT. Linking redirects immediately back to ChatGPT with a short-lived single-use code, the token endpoint returns the configured static bearer token as the OAuth access token, and MCP requests are checked with the same static bearer logic.

For ChatGPT consumer accounts that do not expose raw API-key connector auth, generate a fake OAuth config:

```bash
./scripts/create-chatgpt-fake-oauth-config.sh agentbox-mcp.chatgpt.toml https://<tailscale-hostname>
cargo run -- --config agentbox-mcp.chatgpt.toml
```

Then add `https://<tailscale-hostname>/mcp` in ChatGPT, choose OAuth, use a user-defined OAuth client, leave the client secret empty, set token endpoint auth to `none`, and use `https://<tailscale-hostname>/oauth/authorize` and `https://<tailscale-hostname>/oauth/token` for the OAuth endpoints.

`mode = "static_bearer"` requires `Authorization: Bearer <token>`. The token can come from `agentbox_MCP_TOKEN` or from `[auth.static_bearer].token` in the config.

For the simplest ChatGPT setup, generate a config with a hardcoded random bearer token:

```bash
./scripts/create-chatgpt-bearer-config.sh agentbox-mcp.chatgpt.toml https://<tailscale-hostname>
cargo run -- --config agentbox-mcp.chatgpt.toml
```

Then add `https://<tailscale-hostname>/mcp` in ChatGPT and choose API key / bearer token authentication with the printed token.

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

Use `https://<tailscale-hostname>/mcp` as the ChatGPT custom MCP connector URL and configure fake OAuth or API key / bearer token authentication depending on what your ChatGPT account exposes. See [docs/chatgpt-connector.md](docs/chatgpt-connector.md) and [docs/tailscale-funnel.md](docs/tailscale-funnel.md).

## Tool Reference

Default names use the `agentbox_` prefix:

- `agentbox_exec_command`: run a shell command on the persistent machine with real access.
- `agentbox_write_stdin`: poll or write to a real-access persistent machine session.
- `agentbox_apply_patch`: apply a patch to files on the persistent machine with real access.
- `agentbox_bootstrap`: return information about the persistent machine with real access.
- `agentbox_list_skills`: compact skill catalog only.
- `agentbox_load_skill`: full selected skill instructions.

Set `[tools].prefix = ""` to expose Codex-style names like `exec_command` and `write_stdin`.
Set `[skills].enabled = false` to omit the skill tools when skills are provided outside MCP.

## Known Limitations

The MCP endpoint currently returns JSON responses for POST requests and `405` for GET SSE streams. This is allowed for simple Streamable HTTP request/response servers, but it does not implement server-to-client notifications or resumable SSE. The patch tool uses OpenAI Codex’s upstream `codex-apply-patch` crate pinned to the commit recorded in [docs/implementation-log.md](docs/implementation-log.md).
