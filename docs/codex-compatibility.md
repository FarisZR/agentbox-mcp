# Codex Compatibility

The primary tools mirror Codex unified exec:

- input fields: `cmd`, `workdir`, `shell`, `tty`, `login`, `yield_time_ms`, `max_output_tokens`
- running sessions return `session_id`
- `write_stdin` accepts `session_id`, `chars`, `yield_time_ms`, `max_output_tokens`
- output fields: `chunk_id`, `wall_time_seconds`, `exit_code`, `session_id`, `original_token_count`, `output`

Differences:

- Codex has approval/sandbox machinery. `agentbox-mcp` deliberately omits it because the machine is dedicated and unsandboxed after auth.
- Codex uses its own patch crate. This repo includes a smaller Rust patcher that covers add/update/delete/move patches and the closed-loop fixture.
- Codex can stream tool events internally. This MCP server returns recent output per request.
