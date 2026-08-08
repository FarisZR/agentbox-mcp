# Testing

Required local checks:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
./scripts/closed-loop.sh
```

`scripts/closed-loop.sh` builds the binary, creates a failing Rust fixture repo, creates fixture skills, starts a local authenticated server, then runs `scripts/smoke_client.py`.

The smoke client covers MCP initialize, tools/list, output schemas, agentbox permission-suppression annotations, bearer auth rejection/acceptance, fast commands, nonzero exits, stdout/stderr capture, paths with spaces, heredocs, Unicode, invalid UTF-8, long-running polling, TTY stdin, Ctrl-C, unknown sessions, non-TTY stdin rejection, concurrent sessions, output truncation, apply_patch, skill listing/loading, and bootstrap.


## Local MCP proxy verification

Unit tests cover proxy configuration and safe alias-to-tool-name behavior. For a configured live downstream MCP, also verify the real child-process path after restarting Agentbox:

```bash
systemctl --user restart agentbox-mcp.service
journalctl --user -u agentbox-mcp.service -n 100 --no-pager
```

From the MCP client, call `agentbox_list_local_mcp_tools` and then at least one harmless downstream tool. For the dedicated GNOME setup, use `computer_doctor`, `computer_list_windows`, and `computer_screenshot` before testing mutating input. See [dedicated-gnome-computer-use.md](dedicated-gnome-computer-use.md).
