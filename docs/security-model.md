# Security Model

This server is intentionally powerful. After a request is authenticated, tools run unsandboxed on the real Linux machine. There are no command allowlists, denylists, path jails, Docker restrictions, sudo restrictions, or artificial network limits.

Security controls are placed at the MCP HTTP entrance:

- `none`: localhost-only development, or ChatGPT web with a high-entropy secret in `server.mcp_path`. The server logs a warning if used with a non-loopback bind.
- `static_bearer`: requires `Authorization: Bearer <token>`. The token can come from `agentbox_MCP_TOKEN` or `[auth.static_bearer].token`.
- `oauth_jwks`: validates JWT signature using JWKS, issuer, audience, expiry/nbf, and required scopes.

## Recommended ChatGPT Security

Use API key / bearer token authentication in ChatGPT with `auth.mode = "static_bearer"`:

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
- Prefer OAuth/JWKS if you later have an IdP.
