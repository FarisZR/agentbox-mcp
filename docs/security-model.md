# Security Model

This server is intentionally powerful. After a request is authenticated, tools run unsandboxed on the real Linux machine. There are no command allowlists, denylists, path jails, Docker restrictions, sudo restrictions, or artificial network limits.

Security controls are placed at the MCP HTTP entrance:

- `none`: localhost-only development, or ChatGPT web with a high-entropy secret in `server.mcp_path`. The server logs a warning if used with a non-loopback bind.
- `static_bearer`: requires `Authorization: Bearer <token>`. The token can come from `agentbox_MCP_TOKEN` or `[auth.static_bearer].token`.
- `fake_oauth`: exposes a minimal OAuth authorization-code facade for ChatGPT. The token endpoint requires the configured OAuth client gate before returning the static bearer token as the OAuth access token. MCP requests are still checked like `static_bearer`.
- `oauth_jwks`: validates JWT signature using JWKS, issuer, audience, expiry/nbf, and required scopes.

## Recommended ChatGPT Security

For ChatGPT accounts that only expose OAuth connector auth, use fake OAuth with `auth.mode = "fake_oauth"`:

```toml
[server]
bind = "127.0.0.1:8787"
public_base_url = "https://<tailscale-funnel-hostname>"
mcp_path = "/mcp"

[auth]
mode = "fake_oauth"

[auth.static_bearer]
token = "<64-hex-random-token>"
token_env = "agentbox_MCP_TOKEN"

[auth.oauth]
resource = "https://<tailscale-funnel-hostname>"
issuer = "https://<tailscale-funnel-hostname>"
audience = "https://<tailscale-funnel-hostname>"
required_scopes = ["agentbox:exec"]
```

Fake OAuth is a compatibility shim. It does not authenticate different users. It protects the OAuth handoff with ChatGPT redirect URI validation, short-lived single-use authorization codes, PKCE validation, and a required OAuth client gate at the token endpoint. The final credential remains one bearer token.

Use API key / bearer token authentication in ChatGPT with `auth.mode = "static_bearer"` when that option is available:

```toml
[server]
bind = "127.0.0.1:8787"
mcp_path = "/mcp"

[auth]
mode = "static_bearer"

[auth.static_bearer]
token = "<64-hex-random-token>"
token_env = "agentbox_MCP_TOKEN"
```

Generate the token with `openssl rand -hex 32`. Treat the token like a password.

Recommended controls:

- Keep `bind = "127.0.0.1:8787"`.
- Expose only through Tailscale Funnel HTTPS.
- Do not publish logs/screenshots containing the token.
- Rotate the token immediately if it leaks.
- Prefer OAuth/JWKS if you later have a real IdP.
