# Testing

Required local checks:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
./scripts/closed-loop.sh
```

`scripts/closed-loop.sh` builds the binary, creates a failing Rust fixture repo, creates fixture skills, starts a local authenticated server, then runs `scripts/smoke_client.py`.

The smoke client covers MCP initialize, tools/list, tool annotations, bearer auth rejection/acceptance, fast commands, nonzero exits, stdout/stderr capture, paths with spaces, heredocs, Unicode, invalid UTF-8, long-running polling, TTY stdin, Ctrl-C, unknown sessions, non-TTY stdin rejection, concurrent sessions, output truncation, apply_patch, skill listing/loading, and bootstrap.
