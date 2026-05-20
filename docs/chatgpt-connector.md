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
https://<tailscale-funnel-hostname>/mcp/<secret>
```

In ChatGPT, enable Developer Mode, create an app/custom connector from the remote MCP URL, and select `No authentication` for the simple setup below.

## Simple Setup Without OAuth

ChatGPT web currently supports OAuth or no authentication for custom MCP connectors. It does not give you a simple field for `Authorization: Bearer <static-token>`. If you do not have an OAuth server, use a random secret in the MCP URL path and keep that URL private.

This is the recommended barebones setup for a single-user dedicated agentbox:

```bash
./scripts/create-chatgpt-simple-config.sh agentbox-mcp.chatgpt.toml https://<tailscale-funnel-hostname>
cargo run -- --config agentbox-mcp.chatgpt.toml
```

Expose it:

```bash
./scripts/setup-tailscale-funnel.sh
```

The generated script prints a connector URL like:

```text
https://<tailscale-funnel-hostname>/mcp/7f3b...64_hex_chars...
```

In ChatGPT on the web:

1. Open `Settings`.
2. Go to `Connectors`.
3. Open `Advanced` and enable `Developer mode`.
4. Add/import a remote MCP connector.
5. Name it `Agentbox Execution Environment`.
6. Use the full secret URL printed by the script.
7. Choose `No authentication`.
8. Refresh the connector tools and confirm the `agentbox_*` tools appear.

Use a prompt like:

```text
Use the Agentbox Execution Environment connector. Call agentbox_bootstrap first, then use agentbox_exec_command for shell commands. Do not use hosted shell tools.
```

Security notes:

- The path secret is a bearer credential. Anyone with the full URL can use the connector.
- Keep the server bound to `127.0.0.1`; do not bind it directly to `0.0.0.0`.
- Tailscale Funnel provides HTTPS, but Funnel is public. The secret path is what keeps random visitors out.
- Rotate by generating a new config and restarting the server.

## Optional OAuth Setup

`agentbox-mcp` is an OAuth protected resource. It validates access tokens, but it does not implement the OAuth authorization server itself. Use an IdP or OAuth gateway such as Auth0, Okta, Zitadel, Keycloak, Dex, or your internal auth service.

1. Create an OAuth/OIDC application in your IdP for ChatGPT.
2. Configure the IdP to issue JWT access tokens with:
   - issuer matching `[auth.oauth].issuer`
   - audience matching `[auth.oauth].audience`
   - scope containing every value in `[auth.oauth].required_scopes`
   - signing keys published at `[auth.oauth].jwks_url`
3. Configure `agentbox-mcp`:

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

4. Start the server and expose it through Funnel:

```bash
cargo run -- --config agentbox-mcp.toml
./scripts/setup-tailscale-funnel.sh
```

5. Verify the protected-resource metadata:

```bash
curl https://<tailscale-funnel-hostname>/.well-known/oauth-protected-resource
```

6. In ChatGPT on the web:
   - Open `Settings`.
   - Go to `Connectors`.
   - Open `Advanced` and enable `Developer mode`.
   - Add/import a remote MCP connector.
   - Name it `Agentbox Execution Environment`.
   - Use `https://<tailscale-funnel-hostname>/mcp` as the connector URL.
   - Choose OAuth authentication.
   - Enter your IdP client details if ChatGPT asks for static client credentials, or choose the IdP metadata/DCR/CIMD option if your workspace and IdP support it.
   - Complete the OAuth consent flow.
   - Refresh the connector tools and confirm the `agentbox_*` tools appear.

7. In a new chat, select Developer Mode/tools and use a prompt like:

```text
Use the Agentbox Execution Environment connector. Call agentbox_bootstrap first, then use agentbox_exec_command for shell commands. Do not use hosted shell tools.
```

Troubleshooting:

- A `401` response should include `WWW-Authenticate` and point at `/.well-known/oauth-protected-resource`.
- If ChatGPT completes OAuth but tool calls fail, compare the JWT `iss`, `aud`, `exp`, `nbf`, and scopes against `agentbox-mcp.toml`.
- If tools do not appear, confirm the URL is exactly `/mcp`, Funnel is reachable from the public internet, and the server logs show a successful `initialize` and `tools/list`.
