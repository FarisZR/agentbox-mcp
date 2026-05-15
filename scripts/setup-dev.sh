#!/usr/bin/env bash
set -euo pipefail

command -v cargo >/dev/null || {
  echo "Rust/Cargo is required. Install rustup from https://rustup.rs/ and rerun this script." >&2
  exit 1
}

cargo build
cat <<'MSG'
Development setup complete.

Run locally with:
  export agentbox_MCP_TOKEN='replace-with-a-long-random-token'
  cargo run -- --config config.example.toml

Health:
  curl http://127.0.0.1:8787/healthz
MSG
