# Security Model

This server is intentionally powerful. After a request is authenticated, tools run unsandboxed on the real Linux machine. There are no command allowlists, denylists, path jails, Docker restrictions, sudo restrictions, or artificial network limits.

Security controls are placed at the MCP HTTP entrance:

- `none`: localhost-only development. The server logs a warning if used with a non-loopback bind.
- `static_bearer`: requires `Authorization: Bearer <token>` where the token comes from `agentbox_MCP_TOKEN` by default.
- `oauth_jwks`: validates JWT signature using JWKS, issuer, audience, expiry/nbf, and required scopes.

Do not expose this server publicly without strong authentication and a dedicated machine boundary.
