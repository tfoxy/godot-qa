use godot_qa_core::results::{CommandResult, FailureDetail};
use serde_json::{Map, Value, json};
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone)]
pub struct McpServerDefaults {
    pub project: Option<String>,
    pub godot_bin: Option<String>,
    pub artifact_dir: Option<String>,
    pub verbose: bool,
}

#[derive(Debug, Clone, Copy)]
struct ToolSpec {
    name: &'static str,
    description: &'static str,
}

const TOOL_SPECS: [ToolSpec; 14] = [
    ToolSpec {
        name: "doctor",
        description: "Run the CLI doctor checks and return the standard CommandResult envelope.",
    },
    ToolSpec {
        name: "scenario_list",
        description: "List scenarios using the existing CLI scenario loader.",
    },
    ToolSpec {
        name: "scenario_run",
        description: "Run one named scenario through the existing CLI scenario executor.",
    },
    ToolSpec {
        name: "session_start",
        description: "Start a live Godot QA session through the CLI session command.",
    },
    ToolSpec {
        name: "session_stop",
        description: "Stop the current live Godot QA session through the CLI session command.",
    },
    ToolSpec {
        name: "snapshot",
        description: "Capture live snapshot artifacts through the CLI snapshot command.",
    },
    ToolSpec {
        name: "input_tap",
        description: "Send an input action tap to the live session through the CLI input command.",
    },
    ToolSpec {
        name: "input_click",
        description: "Click a target in the live session through the CLI input command.",
    },
    ToolSpec {
        name: "tree_dump",
        description: "Dump the live scene tree through the CLI tree command.",
    },
    ToolSpec {
        name: "ui_dump",
        description: "Dump live UI state through the CLI ui command.",
    },
    ToolSpec {
        name: "focus_get",
        description: "Read live focus state through the CLI focus command.",
    },
    ToolSpec {
        name: "layout_audit",
        description: "Run a viewport matrix layout audit through the CLI layout command.",
    },
    ToolSpec {
        name: "layout_audit_current",
        description: "Audit a stored scenario snapshot through the CLI layout command.",
    },
    ToolSpec {
        name: "logs",
        description: "Read structured runtime logs through the CLI logs command.",
    },
];

pub fn run_stdio_server(defaults: McpServerDefaults, executable: &Path) -> i32 {
    let mut stdin = io::stdin().lock();
    let mut stdout = io::stdout().lock();
    loop {
        let message = match read_message(&mut stdin) {
            Ok(Some(message)) => message,
            Ok(None) => return 0,
            Err(_) => return 1,
        };
        let response = match handle_message(&defaults, executable, &message) {
            Ok(Some(response)) => response,
            Ok(None) => continue,
            Err(response) => response,
        };
        if write_message(&mut stdout, &response).is_err() {
            return 1;
        }
    }
}

fn handle_message(
    defaults: &McpServerDefaults,
    executable: &Path,
    message: &Value,
) -> Result<Option<Value>, Value> {
    let id = message.get("id").cloned();
    let method = message
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match method {
        "initialize" => Ok(Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": message
                    .get("params")
                    .and_then(|params| params.get("protocolVersion"))
                    .cloned()
                    .unwrap_or_else(|| json!("2024-11-05")),
                "capabilities": {
                    "tools": {
                        "listChanged": false
                    }
                },
                "serverInfo": {
                    "name": "godot-qa",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }
        }))),
        "notifications/initialized" => Ok(None),
        "tools/list" => Ok(Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "tools": TOOL_SPECS.iter().map(|tool| {
                    json!({
                        "name": tool.name,
                        "description": tool.description,
                        "inputSchema": {
                            "type": "object",
                            "properties": {}
                        }
                    })
                }).collect::<Vec<_>>()
            }
        }))),
        "tools/call" => {
            let Some(params) = message.get("params").and_then(Value::as_object) else {
                return Err(invalid_request(id, "tools/call params must be an object"));
            };
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return Err(invalid_request(id, "tools/call name is required"));
            };
            let arguments = params
                .get("arguments")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            let argv = build_argv(defaults, name, &arguments)
                .map_err(|message| invalid_request(id.clone(), &message))?;
            let result = execute_cli_command(executable, &argv).map_err(|error| {
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [{
                            "type": "text",
                            "text": format!("{name}: mcp wrapper failed unexpectedly"),
                        }],
                        "structuredContent": CommandResult {
                            ok: false,
                            command: name.to_string(),
                            message: "mcp wrapper failed unexpectedly".to_string(),
                            data: Map::new(),
                            failures: vec![FailureDetail {
                                kind: "mcp_internal_error".to_string(),
                                command: Some(name.to_string()),
                                actual: Some(json!("os_error")),
                                message: Some(error),
                                ..FailureDetail::default()
                            }],
                            exit_code: 1,
                        },
                        "isError": true
                    }
                })
            })?;
            Ok(Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "content": [{
                        "type": "text",
                        "text": tool_summary(&result),
                    }],
                    "structuredContent": result,
                    "isError": false,
                }
            })))
        }
        _ => Err(invalid_request(
            id,
            &format!("unsupported method: {method}"),
        )),
    }
}

fn build_argv(
    defaults: &McpServerDefaults,
    name: &str,
    arguments: &Map<String, Value>,
) -> Result<Vec<String>, String> {
    let mut argv = Vec::new();
    for (flag, default_value) in [
        ("--project", defaults.project.as_deref()),
        ("--godot-bin", defaults.godot_bin.as_deref()),
        ("--artifact-dir", defaults.artifact_dir.as_deref()),
    ] {
        let key = flag.trim_start_matches("--").replace('-', "_");
        let value = arguments
            .get(&key)
            .and_then(Value::as_str)
            .or(default_value);
        if let Some(value) = value {
            argv.push(flag.to_string());
            argv.push(value.to_string());
        }
    }
    let verbose = arguments
        .get("verbose")
        .and_then(Value::as_bool)
        .unwrap_or(defaults.verbose);
    if verbose {
        argv.push("--verbose".to_string());
    }

    match name {
        "doctor" => argv.push("doctor".to_string()),
        "scenario_list" => argv.extend(["scenario".to_string(), "list".to_string()]),
        "scenario_run" => argv.extend([
            "scenario".to_string(),
            "run".to_string(),
            required_string(arguments, "name")?,
        ]),
        "session_start" => {
            argv.extend([
                "session".to_string(),
                "start".to_string(),
                "--scene".to_string(),
                required_string(arguments, "scene")?,
            ]);
            if let Some(viewport) = optional_string(arguments, "viewport")? {
                argv.extend(["--viewport".to_string(), viewport]);
            }
        }
        "session_stop" => argv.extend(["session".to_string(), "stop".to_string()]),
        "snapshot" => {
            argv.push("snapshot".to_string());
            if let Some(snapshot_name) = optional_string(arguments, "name")? {
                argv.extend(["--name".to_string(), snapshot_name]);
            }
        }
        "input_tap" => argv.extend([
            "input".to_string(),
            "tap".to_string(),
            required_string(arguments, "action")?,
        ]),
        "input_click" => argv.extend([
            "input".to_string(),
            "click".to_string(),
            required_string(arguments, "target")?,
        ]),
        "tree_dump" => argv.push("tree".to_string()),
        "ui_dump" => argv.push("ui".to_string()),
        "focus_get" => argv.push("focus".to_string()),
        "layout_audit" => {
            argv.extend([
                "layout".to_string(),
                "audit".to_string(),
                "--scene".to_string(),
                required_string(arguments, "scene")?,
            ]);
            if let Some(viewports) = optional_string(arguments, "viewports")? {
                argv.extend(["--viewports".to_string(), viewports]);
            }
            if arguments.get("strict").and_then(Value::as_bool) == Some(true) {
                argv.push("--strict".to_string());
            }
        }
        "layout_audit_current" => {
            argv.extend(["layout".to_string(), "audit-current".to_string()]);
            let live = arguments.get("live").and_then(Value::as_bool) == Some(true);
            if live {
                argv.push("--live".to_string());
                // With a live session `--scenario` only labels the snapshot.
                if let Some(scenario) = optional_string(arguments, "scenario")? {
                    argv.extend(["--scenario".to_string(), scenario]);
                }
            } else {
                argv.extend([
                    "--scenario".to_string(),
                    required_string(arguments, "scenario")?,
                ]);
            }
            if let Some(snapshot) = optional_string(arguments, "snapshot")? {
                argv.extend(["--snapshot".to_string(), snapshot]);
            }
            if let Some(run_id) = optional_string(arguments, "run_id")? {
                argv.extend(["--run-id".to_string(), run_id]);
            }
            if arguments.get("strict").and_then(Value::as_bool) == Some(true) {
                argv.push("--strict".to_string());
            }
        }
        "logs" => {
            argv.push("logs".to_string());
            if let Some(since) = optional_string(arguments, "since")? {
                argv.extend(["--since".to_string(), since]);
            }
        }
        _ => return Err(format!("unknown tool: {name}")),
    }
    Ok(argv)
}

fn required_string(arguments: &Map<String, Value>, name: &str) -> Result<String, String> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("{name} is required"))
}

fn optional_string(arguments: &Map<String, Value>, name: &str) -> Result<Option<String>, String> {
    match arguments.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.is_empty() => Ok(Some(value.clone())),
        Some(_) => Err(format!("{name} must be a non-empty string")),
    }
}

fn execute_cli_command(executable: &Path, argv: &[String]) -> Result<CommandResult, String> {
    let output = Command::new(executable)
        .args(argv)
        .arg("--json")
        .output()
        .map_err(|error| error.to_string())?;
    serde_json::from_slice::<CommandResult>(&output.stdout)
        .map_err(|error| format!("unable to parse CLI JSON result: {error}"))
}

fn tool_summary(result: &CommandResult) -> String {
    if result.message.is_empty() {
        result.command.clone()
    } else {
        format!("{}: {}", result.command, result.message)
    }
}

fn invalid_request(id: Option<Value>, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": -32602,
            "message": message,
        }
    })
}

fn read_message(reader: &mut impl Read) -> io::Result<Option<Value>> {
    let mut headers = Vec::new();
    let mut buffer = [0_u8; 1];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) if headers.is_empty() => return Ok(None),
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "unexpected EOF",
                ));
            }
            Ok(_) => {
                headers.push(buffer[0]);
                if headers.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            Err(error) => return Err(error),
        }
    }
    let header_text = String::from_utf8_lossy(&headers);
    let content_length = header_text
        .lines()
        .find_map(|line| {
            let lower = line.to_ascii_lowercase();
            lower
                .strip_prefix("content-length:")
                .map(|value| value.trim().parse::<usize>())
        })
        .transpose()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid Content-Length"))?
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing Content-Length"))?;
    let mut body = vec![0_u8; content_length];
    reader.read_exact(&mut body)?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn write_message(writer: &mut impl Write, payload: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(payload)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    writer.write_all(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes())?;
    writer.write_all(&body)?;
    writer.flush()
}
