# Dedicated GNOME computer-use device

This documents the working Computer Use setup on the dedicated Agentbox Linux device and, importantly, the extra changes required beyond simply installing a desktop MCP.

Validated on 2026-08-08 with:

- Debian 13 (trixie)
- Linux 6.12
- GNOME Shell 48.7
- GNOME Wayland
- `computer-use-linux` exposed through Agentbox as the `computer_*` tool family
- `ydotoold` for deterministic keyboard/input fallback

The target is a **dedicated automation device**, not a shared workstation. Several steps below deliberately grant the automation account persistent desktop-control permissions.

## Result

The final `computer-use-linux doctor` readiness state is:

```text
can_register_mcp_tools         true
can_build_accessibility_tree  true
can_query_windows             true
can_focus_apps                true
can_focus_windows             true
can_send_development_input    true
blockers                      []
```

The verified end-to-end path is:

```text
ChatGPT
  -> authenticated Agentbox HTTPS MCP
  -> local stdio computer-use-linux MCP
  -> GNOME session DBus / AT-SPI / XDG portal / ydotool
  -> real Wayland desktop
```

## What was already working

Before the final fixes, the machine already had:

- a live GNOME Wayland graphical session;
- an AT-SPI accessibility bus visible to the automation account;
- `ydotool` and `ydotoold`;
- read/write access to `/dev/uinput` through a dedicated `uinput` group;
- an Agentbox user service;
- a per-user `ydotoold` service and socket at `/run/user/$UID/.ydotool_socket`;
- passwordless/non-interactive `sudo` for the dedicated account;
- GDM automatic login configured for the automation account;
- user lingering enabled so Agentbox and ydotoold survive graphical logout.

The two blockers were:

1. recent GNOME denies `org.gnome.Shell.Introspect.GetWindows` to ordinary clients, so exact window listing/focus was unavailable;
2. screenshots through the portal required interactive authorization and the stored unsandboxed-host permission was `no`.

## Install computer-use-linux

The npm package is one supported installation path:

```bash
npm install -g @agent-sh/computer-use-linux
computer-use-linux doctor | jq .readiness
```

For an Agentbox service, use the absolute path returned by:

```bash
command -v computer-use-linux
```

Then register it as a local MCP as described in [Local MCP proxy](local-mcp-proxy.md):

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

The four session variables matter when Agentbox is launched as a persistent user service rather than from a terminal inside GNOME.

## Persistent input with ydotool

`ydotoold` needs read/write access to `/dev/uinput`. On this device a dedicated group and udev rule are used:

```bash
sudo groupadd -f uinput
sudo usermod -aG uinput "$USER"
printf '%s\n' 'KERNEL=="uinput", GROUP="uinput", MODE="0660", OPTIONS+="static_node=uinput"' \
  | sudo tee /etc/udev/rules.d/99-uinput.rules
printf '%s\n' uinput | sudo tee /etc/modules-load.d/uinput.conf
sudo modprobe uinput
sudo udevadm control --reload-rules
sudo udevadm trigger --name-match=uinput
```

Re-login after changing group membership. Verify:

```bash
id
ls -l /dev/uinput
```

The device should be writable by the automation account's group.

Run `ydotoold` as a **user** service, not a world-accessible system daemon:

```ini
# ~/.config/systemd/user/ydotoold.service
[Unit]
Description=Starts ydotoold Daemon

[Service]
Type=simple
Restart=always
RestartSec=3
ExecStartPre=/bin/sleep 2
ExecStart=/home/agent/.local/bin/ydotoold --socket-path=/run/user/%U/.ydotool_socket
ExecReload=/usr/bin/kill -HUP $MAINPID
KillMode=process
TimeoutSec=180

[Install]
WantedBy=basic.target
```

Enable it:

```bash
systemctl --user daemon-reload
systemctl --user enable --now ydotoold.service
systemctl --user status ydotoold.service --no-pager
```

## GNOME accessibility

Run:

```bash
computer-use-linux setup
computer-use-linux doctor | jq .accessibility
```

`computer-use-linux setup` configures the GNOME accessibility bridge when it is missing. Restart already-running GUI applications if their accessibility trees are not visible afterward.

On the validated device, the AT-SPI bus and accessibility tree were already functional before the final window/screenshot fixes.

## Fix exact window listing and focus on GNOME Wayland

On this GNOME release the stock introspection call fails with:

```text
org.freedesktop.DBus.Error.AccessDenied: GetWindows is not allowed
```

The fix was to install the `computer-use-linux` GNOME Shell extension:

```bash
computer-use-linux setup-window-targeting
```

This installs/enables:

```text
~/.local/share/gnome-shell/extensions/computer-use-linux@avifenesh.dev
```

The extension exposes a user-session DBus service:

```text
dev.avifenesh.ComputerUseLinux.WindowControl
```

It provides exact window enumeration and activation from inside GNOME Shell rather than relying on the denied public introspection API.

### GNOME must load the extension

Enabling the extension is not enough for an already-running Wayland shell. A graphical logout/login is required. On a dedicated unattended device, restarting GDM is a practical way to guarantee a clean shell reload:

```bash
sudo systemctl restart gdm.service
```

This closes all GUI applications. Do it only after automatic login and persistent user services are configured.

After the new session starts:

```bash
gnome-extensions info computer-use-linux@avifenesh.dev
gdbus call --session \
  --dest org.freedesktop.DBus \
  --object-path /org/freedesktop/DBus \
  --method org.freedesktop.DBus.NameHasOwner \
  dev.avifenesh.ComputerUseLinux.WindowControl
```

Expected state:

```text
Enabled: Yes
State: ACTIVE
(true,)
```

## Make screenshots non-interactive on a dedicated device

The direct GNOME Shell screenshot API returned `AccessDenied`, and the XDG Desktop Portal fallback initially had a stored host permission of `no`.

For an ordinary workstation, use the normal portal prompt. For this **dedicated automation account**, we intentionally changed the XDG permission store so unsandboxed host applications may take screenshots without an interactive dialog.

Inspect the current permission:

```bash
gdbus call --session \
  --dest org.freedesktop.impl.portal.PermissionStore \
  --object-path /org/freedesktop/impl/portal/PermissionStore \
  --method org.freedesktop.impl.portal.PermissionStore.GetPermission \
  screenshot screenshot ''
```

Set the host permission to `yes`:

```bash
gdbus call --session \
  --dest org.freedesktop.impl.portal.PermissionStore \
  --object-path /org/freedesktop/impl/portal/PermissionStore \
  --method org.freedesktop.impl.portal.PermissionStore.SetPermission \
  screenshot true screenshot '' "['yes']"
```

Verify:

```bash
gdbus call --session \
  --dest org.freedesktop.impl.portal.PermissionStore \
  --object-path /org/freedesktop/impl/portal/PermissionStore \
  --method org.freedesktop.impl.portal.PermissionStore.GetPermission \
  screenshot screenshot ''
```

Expected result:

```text
(['yes'],)
```

This permission persisted across the GNOME logout/login on the validated device. The computer-use backend can therefore fall back to `xdg-desktop-portal` and capture without a prompt even though GNOME's direct screenshot DBus API remains restricted.

To undo the dedicated-device override, remove the unsandboxed-host permission:

```bash
gdbus call --session \
  --dest org.freedesktop.impl.portal.PermissionStore \
  --object-path /org/freedesktop/impl/portal/PermissionStore \
  --method org.freedesktop.impl.portal.PermissionStore.DeletePermission \
  screenshot screenshot ''
```

## Keep Agentbox alive across desktop restarts

Enable lingering for the automation account:

```bash
sudo loginctl enable-linger "$USER"
loginctl show-user "$USER" -p Linger
```

Expected:

```text
Linger=yes
```

The Agentbox service used on this device is a user service:

```ini
# ~/.config/systemd/user/agentbox-mcp.service
[Unit]
Description=Agentbox MCP server for ChatGPT connector
After=network-online.target tailscaled.service

[Service]
Type=simple
WorkingDirectory=/home/agent/agentbox-mcp
ExecStart=/home/agent/agentbox-mcp/target/release/agentbox-mcp --config /home/agent/agentbox-mcp/agentbox-mcp.chatgpt.toml
Restart=on-failure
RestartSec=5
Environment=RUST_LOG=info

[Install]
WantedBy=default.target
```

Enable both persistent services:

```bash
systemctl --user daemon-reload
systemctl --user enable --now agentbox-mcp.service ydotoold.service
```

## Automatic graphical login

For a physically dedicated device, GDM can automatically recreate the GNOME session after a reboot or display-manager restart.

On Debian, `/etc/gdm3/daemon.conf` contains:

```ini
[daemon]
AutomaticLoginEnable=true
AutomaticLogin=agent
WaylandEnable=true
```

Replace `agent` with the automation OS account. This reduces physical/local-login security and is not recommended on shared devices.

## Verification sequence

Run the local readiness check first:

```bash
computer-use-linux doctor | jq '{readiness,capabilities}'
```

Then verify the important layers individually:

```bash
systemctl --user is-active agentbox-mcp.service ydotoold.service
gnome-extensions info computer-use-linux@avifenesh.dev
computer-use-linux windows
computer-use-linux screenshot
```

From ChatGPT/Agentbox, verify the proxied production path rather than only the local CLI:

1. call `computer_doctor`;
2. call `computer_list_windows` and confirm the backend is `gnome-shell-extension`;
3. focus a known window by exact `window_id`;
4. take a screenshot and confirm it returns image content without a portal prompt;
5. send a harmless key such as `Escape` to that exact target window.

On 2026-08-08 the final live validation succeeded with exact Firefox window focus, a window-cropped PNG through `xdg-desktop-portal`, and targeted keyboard injection through `ydotool` after focus verification.

## Troubleshooting

### Extension says enabled but inactive

A Wayland GNOME Shell that was already running when the extension was enabled has not loaded it yet. Log out/in or restart GDM. Confirm `State: ACTIVE`, not merely `Enabled: Yes`.

### `GetWindows is not allowed`

This is expected on GNOME builds that restrict Shell introspection. The `computer-use-linux` extension is the intended fallback. Do not treat the stock introspection failure as a blocker when `gnome-shell-extension` is healthy.

### Screenshot still prompts or fails

Check the XDG permission store with `GetPermission` above and verify the Agentbox child has the correct `DBUS_SESSION_BUS_ADDRESS` and `XDG_RUNTIME_DIR`. Also inspect:

```bash
journalctl --user -u xdg-desktop-portal -u xdg-desktop-portal-gnome --since '10 min ago'
```

### Agentbox works from a shell but not from systemd

A persistent user service may not inherit the graphical shell's environment. Explicitly configure `XDG_RUNTIME_DIR`, `DBUS_SESSION_BUS_ADDRESS`, `WAYLAND_DISPLAY`, and `DISPLAY` under `[mcp_proxy.servers.computer.env]`.

### Input is unavailable

Check all three layers:

```bash
ls -l /dev/uinput
systemctl --user status ydotoold.service --no-pager
ls -l /run/user/$UID/.ydotool_socket
```

Then re-run `computer-use-linux doctor`.

## Security notes

This configuration intentionally makes the automation account equivalent to an operator sitting at the desktop: it can observe the screen, inspect accessibility content, focus windows, and synthesize keyboard/mouse input.

Keep that authority behind Agentbox's authenticated MCP entrance. Do not expose `ydotoold` over the network, do not make its socket world-readable, and do not remove Agentbox authentication merely because the physical machine is dedicated.
