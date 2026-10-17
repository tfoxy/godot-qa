# MCP Wrapper

The optional MCP stdio server wraps the same CLI/library behavior used by scripts and humans.

## Implemented shape

```bash
godot-qa mcp --project /path/to/project
```

The wrapper:

- reuses the existing CLI parser/handlers through `execute_argv()`
- keeps `CommandResult` as the structured MCP payload
- uses stdio transport only
- keeps MCP behavior on the same shipped Rust CLI surface
- serializes tool execution inside the MCP process to avoid concurrent live-session mutations

## Stable MCP tools

```text
doctor
scenario_list
scenario_run
session_start
session_stop
snapshot
input_tap
input_click
ui_dump
tree_dump
focus_get
layout_audit
layout_audit_current
logs
```

## Result strategy

MCP tool results should include:

- the existing `CommandResult` object as structured content
- a short text summary derived from `command` plus `message`
- machine-readable failures and artifact paths exactly as the CLI already returns

Validation failures and runtime assertion failures stay inside the result payload with `ok: false`; they are not translated into MCP transport errors.

## Client configuration examples

Generic stdio launch:

```json
{
  "command": "godot-qa",
  "args": ["mcp", "--project", "/path/to/project"]
}
```

Claude Desktop-style example:

```json
{
  "mcpServers": {
    "godot-qa": {
      "command": "godot-qa",
      "args": ["mcp", "--project", "/path/to/project"]
    }
  }
}
```

Inspector/debug flow:

```bash
npx -y @modelcontextprotocol/inspector
```

Then configure the inspector's stdio command as:

```text
godot-qa mcp --project /path/to/project
```

## Non-goals

- No SSE or HTTP transport.
- No duplicate automation layer separate from the CLI.
- No unsafe eval or live-edit exposure through MCP.
- No screenshot baseline approval flows through MCP.
