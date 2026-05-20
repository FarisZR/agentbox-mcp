# Security Model

This server is intentionally powerful. After a request is authenticated, tools run unsandboxed on the real Linux machine. There are no command allowlists, denylists, path jails, Docker restrictions, sudo restrictions, or artificial network limits.

Security controls are placed at the MCP HTTP entrance:

- `none`: localhost-only development, or ChatGPT web with a high-entropy secret in `server.mcp_path`. The server logs a warning if used with a non-loopback bind.
- `static_bearer`: requires `Authorization: Bearer <token>` where the token comes from `agentbox_MCP_TOKEN` by default.
- `oauth_jwks`: validates JWT signature using JWKS, issuer, audience, expiry/nbf, and required scopes.

## Barebones ChatGPT Security

ChatGPT web Developer Mode supports OAuth or no authentication for imported MCP connectors. If you do not have an OAuth server, use a capability URL:

```toml
[server]
bind = "127.0.0.1:8787"
mcp_path = "/mcp/<64-hex-random-secret>"

[auth]
mode = "none"
```

This is not as strong as OAuth, but it avoids an endpoint that is open to everyone who only knows the hostname. The secret path must have at least 256 bits of entropy, must only be sent over HTTPS, and must be treated like a password.

Recommended controls:

- Keep `bind = "127.0.0.1:8787"`.
- Expose only through Tailscale Funnel HTTPS.
- Use `openssl rand -hex 32` for the path secret.
- Do not publish logs/screenshots containing the connector URL.
- Rotate the secret immediately if it leaks.
- Prefer OAuth/JWKS if you later have an IdP.
