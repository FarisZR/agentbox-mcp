#!/usr/bin/env bash
set -euo pipefail

OUT="${1:-agentbox-mcp.chatgpt.toml}"
HOST="${2:-https://your-tailnet-or-funnel-hostname}"

if command -v openssl >/dev/null; then
  TOKEN="$(openssl rand -hex 32)"
elif command -v python3 >/dev/null; then
  TOKEN="$(python3 - <<'PY'
import secrets
print(secrets.token_hex(32))
PY
)"
else
  echo "Need openssl or python3 to generate a token." >&2
  exit 1
fi

sed \
  -e "s#https://your-tailnet-or-funnel-hostname#${HOST}#g" \
  -e "s#REPLACE_WITH_64_HEX_BEARER_TOKEN#${TOKEN}#g" \
  config.chatgpt-bearer.example.toml > "${OUT}"

cat <<MSG
Wrote ${OUT}

Run:
  cargo run -- --config ${OUT}

Expose with Tailscale Funnel:
  ./scripts/setup-tailscale-funnel.sh

ChatGPT connector URL:
  ${HOST}/mcp

ChatGPT authentication:
  Type: API key / bearer token
  Token: ${TOKEN}

Keep both ${OUT} and this token private.
MSG
