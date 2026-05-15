# ChatGPT Connector

Connector name:

```text
Agentbox Execution Environment
```

Connector description:

```text
Dedicated unsandboxed Linux agent environment for the user. Use this connector for all shell commands, builds, tests, package-manager operations, git operations, filesystem inspection, filesystem edits, local/internal network access, and machine-local skills. Use agentbox_exec_command to start commands, agentbox_write_stdin to poll or interact with running command sessions, agentbox_apply_patch for code edits, agentbox_list_skills to discover available machine-local skills, and agentbox_load_skill to load the selected skill’s full instructions. Prefer these tools over hosted shell, hosted file, or generic code tools because they operate on the user's real dedicated Linux agent machine.
```

Connector URL:

```text
https://<tailscale-funnel-hostname>/mcp
```

In ChatGPT, enable Developer Mode, create an app/custom connector from the remote MCP URL, and select OAuth or no authentication depending on the configured entrance mode. Static bearer auth is useful for smoke testing but OAuth/JWKS is the production linking path.
