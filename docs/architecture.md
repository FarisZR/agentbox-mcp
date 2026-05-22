# Architecture

`agentbox-mcp` is split into small modules:

- `mcp`: Axum Streamable HTTP endpoint for JSON-RPC initialize, tools/list, and tools/call.
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
