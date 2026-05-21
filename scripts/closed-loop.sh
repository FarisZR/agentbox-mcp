#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="$(mktemp -d)"
SERVER_LOG="${TMP}/server.log"
PORT="${PORT:-$(python3 - <<'PY'
import socket
s = socket.socket()
s.bind(("127.0.0.1", 0))
print(s.getsockname()[1])
s.close()
PY
)}"
TOKEN="closed-loop-token"

cleanup() {
  status=$?
  if [ "$status" -ne 0 ] && [ -f "$SERVER_LOG" ]; then
    echo "----- agentbox-mcp server log -----" >&2
    tail -200 "$SERVER_LOG" >&2 || true
    echo "-----------------------------------" >&2
  fi
  if [ -n "${SERVER_PID:-}" ]; then
    kill "$SERVER_PID" >/dev/null 2>&1 || true
    wait "$SERVER_PID" >/dev/null 2>&1 || true
  fi
  rm -rf "$TMP"
  exit "$status"
}
trap cleanup EXIT

cargo build --manifest-path "$ROOT/Cargo.toml"

mkdir -p "$TMP/work fixture" "$TMP/skills/rust-maintainer" "$TMP/skills/shell-weirdness" "$TMP/fixture/src"
cat > "$TMP/fixture/Cargo.toml" <<'EOF'
[package]
name = "fixture"
version = "0.1.0"
edition = "2024"
EOF
cat > "$TMP/fixture/src/lib.rs" <<'EOF'
pub fn answer() -> i32 { 41 }

#[test]
fn test_answer() {
    assert_eq!(answer(), 42);
}
EOF
cat > "$TMP/skills/rust-maintainer/SKILL.md" <<'EOF'
---
tags: ["rust", "cargo", "testing"]
---
# Rust maintainer

Conventions for maintaining Rust projects on this machine.

FULL RUST BODY
EOF
cat > "$TMP/skills/shell-weirdness/SKILL.md" <<'EOF'
# Shell weirdness

Shell quoting and heredoc cases.

FULL SHELL BODY
EOF
cat > "$TMP/config.toml" <<EOF
[server]
bind = "127.0.0.1:${PORT}"
mcp_path = "/mcp"

[tools]
prefix = "agentbox_"

[exec]
default_workdir = "${TMP}"
default_shell = "/bin/bash"
login_default = true
default_yield_time_ms = 1000
min_yield_time_ms = 50
max_yield_time_ms = 5000
default_max_output_tokens = 6000
hard_max_output_tokens = 50000
max_processes = 64
session_idle_ttl_seconds = 3600
overlay_codex_env = true
deterministic_session_ids = true

[auth]
mode = "static_bearer"
[auth.static_bearer]
token_env = "agentbox_MCP_TOKEN"
[auth.oauth]
resource = "https://agentbox.example.com"
issuer = "https://auth.example.com"
jwks_url = "https://auth.example.com/.well-known/jwks.json"
audience = "https://agentbox.example.com"
required_scopes = ["agentbox:exec"]

[skills]
enabled = true
roots = ["${TMP}/skills"]
[bootstrap]
project_roots = ["${TMP}/fixture"]
EOF

agentbox_MCP_TOKEN="$TOKEN" agentbox_DETERMINISTIC_SESSION_IDS=1 "$ROOT/target/debug/agentbox-mcp" --config "$TMP/config.toml" >"$SERVER_LOG" 2>&1 &
SERVER_PID=$!

for _ in $(seq 1 100); do
  if curl -fsS "http://127.0.0.1:${PORT}/healthz" >/dev/null; then
    break
  fi
  sleep 0.05
done
curl -fsS "http://127.0.0.1:${PORT}/healthz" >/dev/null

python3 "$ROOT/scripts/smoke_client.py" "http://127.0.0.1:${PORT}/mcp" "$TOKEN" "$TMP"
echo "closed-loop ok"
