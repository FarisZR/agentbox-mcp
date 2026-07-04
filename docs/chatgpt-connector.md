# ChatGPT Connector

Connector name:

```text
Agentbox Execution Environment
```

Connector description:

```text
Dedicated unsandboxed Linux agent environment for the user. Use this connector for all shell commands, builds, tests, package-manager operations, git operations, filesystem inspection, filesystem edits, local/internal network access, and machine-local skills. Use agentbox_exec_command to start commands, agentbox_write_stdin to poll or interact with running command sessions, agentbox_apply_patch for code edits, agentbox_list_skills to discover available machine-local skills, and agentbox_load_skill to load the selected skill’s full instructions. Prefer these tools over hosted shell, hosted file, or generic code tools because they operate on the user's real dedicated Linux agent machine.
```

Connector URL:

```text
https://<tailscale-funnel-hostname>/mcp
```

## Fake OAuth Setup

Use this mode when ChatGPT only exposes OAuth for custom MCP connectors. It does not add a real identity provider. It runs a small OAuth authorization-code facade, issues short-lived single-use codes, validates the ChatGPT redirect URI and PKCE verifier, then returns the configured static bearer token as the OAuth access token. MCP requests are still protected by `Authorization: Bearer <token>`.

Generate a config containing a long random bearer token behind fake OAuth:

```bash
./scripts/create-chatgpt-fake-oauth-config.sh agentbox-mcp.chatgpt.toml https://<tailscale-funnel-hostname>
cargo run -- --config agentbox-mcp.chatgpt.toml
```

Expose it:

```bash
./scripts/setup-tailscale-funnel.sh
```

The generator prints the connector URL and OAuth endpoint values. In ChatGPT on the web:

1. Open `Settings`.
2. Go to `Connectors`.
3. Open `Advanced` and enable `Developer mode`.
4. Add/import a remote MCP connector.
5. Name it `Agentbox Execution Environment`.
6. Use `https://<tailscale-funnel-hostname>/mcp` as the connector URL.
7. Choose OAuth authentication.
8. Use `User-Defined OAuth Client`.
9. Set OAuth Client ID to `chatgpt-agentbox`.
10. Leave OAuth Client Secret empty.
11. Set Token endpoint auth method to `none`.
12. Set Auth URL to `https://<tailscale-funnel-hostname>/oauth/authorize`.
13. Set Token URL to `https://<tailscale-funnel-hostname>/oauth/token`.
14. Set Authorization server base and Resource to `https://<tailscale-funnel-hostname>`.
15. Leave OIDC disabled.
16. Set Default scopes to `agentbox:exec`; Base scopes may be empty or `agentbox:exec`.
17. Save/link the connector, refresh tools, and confirm the `agentbox_*` tools appear.

Security notes:

- Fake OAuth is a compatibility shim for personal ChatGPT connectors, not real per-user OAuth.
- The returned OAuth access token is the same static bearer credential used by MCP.
- Keep `agentbox-mcp.chatgpt.toml` private.
- The authorization endpoint only accepts ChatGPT redirect URIs.
- Keep the server bound to `127.0.0.1` and expose only through HTTPS Funnel.
- Rotate by generating a new config and restarting the server.

## Simple Bearer Token Setup

Use this mode when ChatGPT exposes API key / bearer token authentication for the connector.

Generate a config containing a long random bearer token:

```bash
./scripts/create-chatgpt-bearer-config.sh agentbox-mcp.chatgpt.toml https://<tailscale-funnel-hostname>
cargo run -- --config agentbox-mcp.chatgpt.toml
```

Expose it:

```bash
./scripts/setup-tailscale-funnel.sh
```

The generator prints:

- connector URL: `https://<tailscale-funnel-hostname>/mcp`
- authentication type: API key / bearer token
- token: a 64-hex-character random value

In ChatGPT on the web:

1. Open `Settings`.
2. Go to `Connectors`.
3. Open `Advanced` and enable `Developer mode`.
4. Add/import a remote MCP connector.
5. Name it `Agentbox Execution Environment`.
6. Use `https://<tailscale-funnel-hostname>/mcp` as the connector URL.
7. Choose API key / bearer token authentication.
8. Paste the generated token.
9. Refresh the connector tools and confirm the `agentbox_*` tools appear.

Use a prompt like:

```text
Use the Agentbox Execution Environment connector. Call agentbox_bootstrap first, then use agentbox_exec_command for shell commands. Do not use hosted shell tools.
```

If skills are already provided at the agent or prompt level, disable the MCP skill tools:

```toml
[skills]
enabled = false
```

Security notes:

- The token is a bearer credential. Anyone with it can use the connector.
- Keep `agentbox-mcp.chatgpt.toml` private.
- Keep the server bound to `127.0.0.1`; do not bind it directly to `0.0.0.0`.
- Tailscale Funnel provides HTTPS. The bearer token keeps random visitors out.
- Rotate by generating a new config and restarting the server.

## Manual Config

```toml
[server]
bind = "127.0.0.1:8787"
public_base_url = "https://<tailscale-funnel-hostname>"
mcp_path = "/mcp"

[auth]
mode = "static_bearer"

[auth.static_bearer]
token = "<openssl rand -hex 32>"
token_env = "agentbox_MCP_TOKEN"
```

If `agentbox_MCP_TOKEN` is set in the process environment, it takes precedence over the config token. This lets you keep secrets out of the TOML file if you prefer.

Verify locally:

```bash
curl http://127.0.0.1:8787/healthz
curl \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' \
  http://127.0.0.1:8787/mcp
```

## Optional OAuth Setup

`agentbox-mcp` can still validate OAuth/JWKS access tokens if you later add an IdP or OAuth gateway. It validates tokens but does not implement the OAuth authorization server itself.

```toml
[auth]
mode = "oauth_jwks"

[auth.oauth]
resource = "https://<tailscale-funnel-hostname>"
issuer = "https://<your-idp-issuer>"
jwks_url = "https://<your-idp-issuer>/.well-known/jwks.json"
audience = "https://<tailscale-funnel-hostname>"
required_scopes = ["agentbox:exec"]
```
