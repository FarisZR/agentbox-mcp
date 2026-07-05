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

if command -v openssl >/dev/null; then
  OAUTH_CLIENT_GATE="$(openssl rand -hex 32)"
elif command -v python3 >/dev/null; then
  OAUTH_CLIENT_GATE="$(python3 - <<'PY'
import secrets
print(secrets.token_hex(32))
PY
)"
else
  echo "Need openssl or python3 to generate an OAuth client gate." >&2
  exit 1
fi

sed \
  -e "s#https://your-tailnet-or-funnel-hostname#${HOST}#g" \
  -e "s#REPLACE_WITH_64_HEX_BEARER_TOKEN#${TOKEN}#g" \
  -e "s#REPLACE_WITH_64_HEX_OAUTH_CLIENT_CREDENTIAL#${OAUTH_CLIENT_GATE}#g" \
  config.chatgpt-fake-oauth.example.toml > "${OUT}"

cat <<MSG
Wrote ${OUT}

Run:
  cargo run -- --config ${OUT}

Expose with Tailscale Funnel:
  ./scripts/setup-tailscale-funnel.sh

ChatGPT connector URL:
  ${HOST}/mcp

ChatGPT authentication:
  Type: OAuth
  Registration method: User-Defined OAuth Client
  OAuth Client ID: chatgpt-agentbox
  OAuth Client Secret: ${OAUTH_CLIENT_GATE}
  Token endpoint auth method: client_secret_post or client_secret_basic
  Auth URL: ${HOST}/oauth/authorize
  Token URL: ${HOST}/oauth/token
  Authorization server base: ${HOST}
  Resource: ${HOST}
  OIDC: off
  Default scopes: agentbox:exec
  Base scopes: empty, or agentbox:exec

Keep ${OUT} private. The OAuth client gate protects the public token endpoint; the bearer token is returned to ChatGPT only after that gate is validated.
MSG
