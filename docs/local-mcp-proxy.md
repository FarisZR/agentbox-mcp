# Local MCP proxy

`agentbox-mcp` can spawn trusted local **stdio MCP servers** and expose their tools through the same authenticated Agentbox endpoint used by ChatGPT. Agentbox is the remote MCP server from ChatGPT's point of view and an MCP client to each local child process.

This is useful for machine-local capabilities such as desktop control, browser automation, hardware access, or private developer tools that should not expose their own network listener.

## How it works

At Agentbox startup, the proxy:

1. reads `[mcp_proxy.servers.*]` entries;
2. starts each configured command as a child process;
3. performs MCP initialization over stdio;
4. calls `tools/list` on the child;
5. stores the downstream schemas and routing information;
6. optionally re-exports each downstream tool as a first-class Agentbox tool;
7. keeps the child MCP session alive for later `tools/call` requests.

The proxy uses the official Rust MCP SDK (`rmcp`) for the downstream client transport. It does not open another TCP port.

## Add an MCP

Add a server to the Agentbox TOML:

```toml
[mcp_proxy]
enabled = true
discovery_timeout_ms = 10000
call_timeout_ms = 120000

[mcp_proxy.servers.example]
alias = "example"
command = "/absolute/path/to/example-mcp"
args = ["mcp"]
inherit_env = true
expose_tools = true

[mcp_proxy.servers.example.env]
EXAMPLE_SETTING = "value"
```

Then restart Agentbox:

```bash
systemctl --user restart agentbox-mcp.service
systemctl --user status agentbox-mcp.service --no-pager
```

If Agentbox is not managed by systemd, restart the `agentbox-mcp --config ...` process instead.

### Configuration fields

- `enabled`: enable this downstream server entry. Defaults to `true`.
- `alias`: prefix used for first-class tools. If omitted or empty, the TOML server key is used.
- `command`: executable to spawn. Prefer an absolute path for services.
- `args`: arguments passed to the MCP command.
- `inherit_env`: inherit Agentbox's process environment before applying `env`. Defaults to `true`.
- `env`: environment variables added or overridden for this MCP child.
- `expose_tools`: when `true`, downstream tools become first-class tools in Agentbox `tools/list`. When `false`, they remain callable through the stable dispatcher only.
- `discovery_timeout_ms`: global timeout for child initialization and initial tool discovery.
- `call_timeout_ms`: global timeout for downstream tool calls.

## Tool names and schemas

A downstream tool is exported as:

```text
<alias>_<downstream-tool-name>
```

For example, with `alias = "computer"`:

```text
screenshot       -> computer_screenshot
list_windows     -> computer_list_windows
press_key        -> computer_press_key
```

Agentbox preserves the downstream tool's description, `inputSchema`, optional `outputSchema`, and annotations. Tool call results are forwarded as MCP tool results, including structured content and image content.

Using an alias prevents collisions when two local MCP servers expose common names such as `search`, `open`, or `screenshot`.

## Stable dispatcher tools

Agentbox also exposes two native tools when `[mcp_proxy].enabled = true`:

- `agentbox_list_local_mcp_tools`: inspect connected local MCPs and their original schemas.
- `agentbox_call_local_mcp_tool`: call a downstream tool using `{server, tool, arguments}`.

The dispatcher is useful when `expose_tools = false`, while debugging a server, or when a local MCP may gain tools without needing every tool to be permanently first-class in ChatGPT.

A failed downstream MCP does not bring down Agentbox. It appears as `connected = false` with an error in `agentbox_list_local_mcp_tools`.

## Desktop MCP example

Desktop MCPs need access to the graphical user's session bus and display. The dedicated GNOME machine uses:

```toml
[mcp_proxy.servers.computer]
alias = "computer"
command = "/home/agent/.local/bin/computer-use-linux"
args = ["mcp"]
inherit_env = true
expose_tools = true

[mcp_proxy.servers.computer.env]
PATH = "/home/agent/.local/bin:/usr/local/bin:/usr/bin:/bin"
XDG_RUNTIME_DIR = "/run/user/<uid>"
DBUS_SESSION_BUS_ADDRESS = "unix:path=/run/user/<uid>/bus"
WAYLAND_DISPLAY = "wayland-0"
DISPLAY = ":0"
```

Replace `<uid>` with `id -u`. Confirm the actual display variables from the graphical login rather than assuming them:

```bash
computer-use-linux doctor | jq .platform
```

See [Dedicated GNOME computer-use device](dedicated-gnome-computer-use.md) for the complete setup used on the Agentbox device.

## Verify a new MCP

After restarting Agentbox, check its logs and discovery state:

```bash
journalctl --user -u agentbox-mcp.service -n 100 --no-pager
```

From ChatGPT, call `agentbox_list_local_mcp_tools`. A healthy entry should report `connected = true`, and first-class tools should appear as `<alias>_<tool>` when `expose_tools = true`.

For a desktop MCP, also run its own readiness command directly on the device. For `computer-use-linux`:

```bash
computer-use-linux doctor | jq .readiness
```

## Updating a local MCP

Agentbox discovers downstream tools at process startup. After adding, removing, upgrading, or changing the configuration of a local MCP, restart Agentbox so the downstream tool registry is rebuilt.

ChatGPT can keep a cached/frozen connector tool definition. After first-class tool names or schemas change, refresh the Agentbox connector/tool metadata in ChatGPT. The stable dispatcher can still be useful for diagnostics, but do not assume a running ChatGPT conversation will automatically acquire new first-class schemas.

## Security boundary

Treat every configured local MCP as trusted code. It runs as the Agentbox OS user and can receive calls from any client that successfully authenticates to the Agentbox MCP entrance.

For powerful MCPs such as computer use:

- keep Agentbox bound to loopback and expose only the authenticated HTTPS connector;
- use absolute executable paths in service configuration;
- keep secrets out of committed TOML files;
- make device/input sockets user-scoped rather than world-accessible;
- only configure local MCP servers you intend ChatGPT to be able to call.
