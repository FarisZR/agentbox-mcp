# Implementation Log

## Upstream References Checked

- MCP Streamable HTTP specification: https://modelcontextprotocol.io/specification/2025-06-18/basic/transports. Current protocol version observed: `2025-06-18`. Notes: a single MCP endpoint supports POST and optionally GET SSE; GET may return 405 when SSE is not offered.
- MCP overview/schema: https://modelcontextprotocol.io/specification/2025-06-18/basic/index and https://modelcontextprotocol.io/specification/2025-06-18/schema.
- rmcp Rust SDK: https://github.com/modelcontextprotocol/rust-sdk at commit `cc66e3091e1584f48ee1e0058a2a1201a1d35c81`. Checked `crates/rmcp/tests/test_with_js.rs` and `crates/rmcp/src/transport/streamable_http_server/tower.rs` for Streamable HTTP behavior and JSON response mode.
- OpenAI ChatGPT developer mode docs: https://developers.openai.com/api/docs/guides/developer-mode. Lines checked noted support for SSE and streaming HTTP and OAuth/no/mixed authentication.
- OpenAI remote MCP docs: https://developers.openai.com/api/docs/guides/tools-connectors-mcp. Notes: remote MCP servers on public internet are listed through MCP tools/list and called through MCP tool calls.
- OpenAI Codex source: https://github.com/openai/codex at commit `3dc278b68ea476e03d54a605df8fe52d4a0cef88`. Checked `codex-rs/core/src/tools/handlers/shell_spec.rs`, `codex-rs/core/src/unified_exec/process_manager.rs`, and `codex-rs/core/src/unified_exec/errors.rs`.
- Tailscale Funnel docs: https://tailscale.com/docs/reference/tailscale-cli/funnel, last validated Jan 26, 2026, and https://tailscale.com/docs/features/tailscale-funnel, last validated Jan 20, 2026.
- Local Tailscale CLI: `tailscale version` reported `1.96.4`, commit `8cf541dfd1e0a97096c01cb775d5e26336f3bc6c`. `tailscale funnel --help` and `tailscale serve --help` were run; help showed `tailscale funnel <target>`, `--bg`, `--https`, `--set-path`, and `--yes`.

## Decisions

- Implemented a compact Axum JSON-RPC Streamable HTTP server instead of binding directly to rmcp. The rmcp implementation was used as the behavior reference. This keeps dynamic prefixed tools and custom auth straightforward.
- Authentication is enforced before tool dispatch.
- Command strings are never split in Rust. They are passed as one shell argument after `-lc` or `-c`.
- Non-TTY stdin is closed and `write_stdin` rejects non-empty input.
- TTY sessions use `portable-pty`.
- Output is lossy UTF-8 and truncated with a head/tail marker using approximate chars/4 token counting.

## Deviations From Codex

- No sandbox, approvals, or permission escalation fields.
- GET SSE is not implemented and returns 405.
- The patcher is smaller than Codex upstream apply_patch but supports the required closed-loop patch workflow.

## Verification

Run:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
./scripts/closed-loop.sh
```

## Known Limitations

- OAuth/JWKS validation is implemented, but the closed-loop uses static bearer auth.
- No resumable SSE/event stream support.
- PTY terminal size is fixed at 120x24.
