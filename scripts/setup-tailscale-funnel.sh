#!/usr/bin/env bash
set -euo pipefail

LOCAL="${LOCAL:-127.0.0.1:8787}"
HTTPS_PORT="${HTTPS_PORT:-443}"
TOKEN_SET="${agentbox_MCP_TOKEN:-}"

if ! command -v tailscale >/dev/null; then
  echo "tailscale is not installed. Install it from https://tailscale.com/download/linux, then run: tailscale up" >&2
  exit 1
fi

echo "Tailscale version:"
tailscale version

if ! tailscale status --json >/tmp/agentbox-tailscale-status.json 2>/dev/null; then
  echo "This device is not logged in to Tailscale. Run: sudo tailscale up" >&2
  exit 1
fi

if ! curl -fsS "http://${LOCAL}/healthz" >/dev/null; then
  echo "agentbox-mcp is not reachable at http://${LOCAL}/healthz."
  echo "Start it first, for example:"
  echo "  export agentbox_MCP_TOKEN='<long random token>'"
  echo "  cargo run -- --config config.example.toml"
  exit 1
fi

if [ -z "$TOKEN_SET" ]; then
  echo "Warning: agentbox_MCP_TOKEN is not set in this shell. Static bearer auth smoke checks will fail unless the server has it set."
fi

echo
echo "This will expose http://${LOCAL} on the public internet through Tailscale Funnel HTTPS port ${HTTPS_PORT}."
echo "Command:"
echo "  tailscale funnel --bg --https=${HTTPS_PORT} --yes ${LOCAL}"
echo
read -r -p "Proceed? [y/N] " reply
case "$reply" in
  y|Y|yes|YES) ;;
  *) echo "Aborted."; exit 1 ;;
esac

if ! tailscale funnel --bg "--https=${HTTPS_PORT}" --yes "${LOCAL}"; then
  echo "Funnel setup did not complete. Tailscale may require web/admin approval."
  echo "Next steps:"
  echo "  1. Ensure MagicDNS and HTTPS certificates are enabled in the Tailscale admin console."
  echo "  2. Ensure your tailnet policy has the Funnel node attribute."
  echo "  3. Run this script again, or manually run: tailscale funnel --bg --https=${HTTPS_PORT} --yes ${LOCAL}"
  exit 1
fi

echo
tailscale funnel status || true
HOST="$(tailscale status --json | python3 -c 'import json,sys; s=json.load(sys.stdin); print(s.get("Self",{}).get("DNSName","").rstrip("."))')"
if [ -n "$HOST" ]; then
  echo "Public base URL: https://${HOST}"
  echo "ChatGPT connector URL: https://${HOST}/mcp"
  echo "If using static bearer auth, configure ChatGPT with API key / bearer token authentication."
  echo "Verify:"
  echo "  curl -H \"Authorization: Bearer \$agentbox_MCP_TOKEN\" https://${HOST}/mcp"
fi
