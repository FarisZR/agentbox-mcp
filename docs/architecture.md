# Architecture

`agentbox-mcp` is split into small modules:

- `mcp`: Axum Streamable HTTP endpoint for JSON-RPC initialize, tools/list, and tools/call.
- `mcp_proxy`: persistent downstream stdio MCP clients, tool discovery, first-class tool re-export, and stable dispatcher routing.
- `auth`: entrance authentication for none, static bearer, and OAuth/JWKS.
- `exec`: unsandboxed process/session manager.
- `apply_patch`: Codex-style filesystem patching.
- `skills`: optional skill discovery and loading.
- `bootstrap`: machine profile and login-shell tool detection.
- `config`: TOML config and env overrides.

Execution never tokenizes `cmd`. The server always invokes the selected shell as either:

```text
shell -lc "<cmd>"
shell -c "<cmd>"
```

Non-TTY sessions use `tokio::process::Command` with stdin closed and stdout/stderr captured into one output buffer. TTY sessions use `portable-pty` and keep a PTY writer for `write_stdin`.

Bootstrap resolves common tools through the configured login shell, so user-level package managers and runtimes installed through shell profile setup, such as `nvm`, Cargo, or Bun, appear in the machine profile.

Set `[skills].enabled = false` to omit `agentbox_list_skills` and `agentbox_load_skill` from `tools/list`.

## Local MCP aggregation

When `[mcp_proxy].enabled = true`, `McpProxyRegistry` is built before the public listener starts. Each configured local stdio MCP is spawned and initialized with `rmcp`, and its initial `tools/list` result is retained for the lifetime of the Agentbox process.

First-class proxy tools are flattened into Agentbox `tools/list` as `<alias>_<downstream-name>`. Calls are routed internally back to the original server/tool pair. Agentbox also keeps stable `list_local_mcp_tools` and `call_local_mcp_tool` tools for discovery/dispatch independent of first-class names.

Downstream MCP processes inherit Agentbox's environment by default, with per-server overrides. Desktop MCPs typically need explicit session-bus/display variables when Agentbox runs as a persistent systemd user service. See [local-mcp-proxy.md](local-mcp-proxy.md).
