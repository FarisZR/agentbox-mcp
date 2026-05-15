# Tailscale Funnel

## Prerequisites

- Tailscale v1.38.3 or newer.
- Device logged in with `tailscale up`.
- MagicDNS and HTTPS certificates enabled in the tailnet.
- Funnel node attribute enabled in the tailnet policy.
- `agentbox-mcp` listening on `127.0.0.1:8787`.

Current local CLI help was checked with Tailscale `1.96.4`; the supported command form is:

```bash
tailscale funnel --bg --https=443 --yes 127.0.0.1:8787
```

Funnel can listen on public HTTPS ports `443`, `8443`, or `10000`.

## Setup

```bash
export agentbox_MCP_TOKEN='<long random token>'
cargo run -- --config config.example.toml
./scripts/setup-tailscale-funnel.sh
```

If Tailscale requires admin/web approval, approve Funnel in the admin console and rerun the script.

## Verify

```bash
curl http://127.0.0.1:8787/healthz
tailscale funnel status
curl -H "Authorization: Bearer $agentbox_MCP_TOKEN" https://<hostname>/mcp
```

Use this ChatGPT connector URL:

```text
https://<hostname>/mcp
```

## Disable

```bash
tailscale funnel reset
```
