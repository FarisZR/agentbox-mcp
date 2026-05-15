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

In ChatGPT, enable Developer Mode, create an app/custom connector from the remote MCP URL, and select OAuth or no authentication depending on the configured entrance mode. Static bearer auth is useful for smoke testing but OAuth/JWKS is the production linking path.

## Quick OAuth Setup

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
