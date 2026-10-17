use serde_json::{Value, json};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct RunArtifacts {
    pub run_id: String,
    pub root_dir: PathBuf,
    pub meta_path: PathBuf,
    pub request_path: PathBuf,
    pub result_path: PathBuf,
    pub logs_path: PathBuf,
    pub progress_path: PathBuf,
    pub slow_summary_path: PathBuf,
    pub snapshots_dir: PathBuf,
    pub temporal_dir: PathBuf,
    pub audio_dir: PathBuf,
}

impl RunArtifacts {
    pub fn to_json(&self) -> Value {
        json!({
            "run_root": self.root_dir,
            "meta_path": self.meta_path,
            "request_path": self.request_path,
            "result_path": self.result_path,
            "logs_path": self.logs_path,
            "progress": self.progress_path,
            "progress_path": self.progress_path,
            "slow_summary": self.slow_summary_path,
            "snapshots_dir": self.snapshots_dir,
            "temporal_dir": self.temporal_dir,
            "audio_dir": self.audio_dir,
        })
    }
}

#[derive(Debug, Clone)]
pub struct RecordArtifacts {
    pub run_id: String,
    pub root_dir: PathBuf,
    pub meta_path: PathBuf,
    pub request_path: PathBuf,
    pub result_path: PathBuf,
    pub logs_path: PathBuf,
    pub recording_path: PathBuf,
    pub candidate_path: PathBuf,
    pub snapshots_dir: PathBuf,
}

impl RecordArtifacts {
    pub fn to_json(&self) -> Value {
        json!({
            "root": self.root_dir,
            "meta": self.meta_path,
            "request": self.request_path,
            "result": self.result_path,
            "logs": self.logs_path,
            "recording": self.recording_path,
            "candidate": self.candidate_path,
            "snapshots_dir": self.snapshots_dir,
        })
    }
}

pub fn prepare_run_artifacts(
    artifacts_root: &Path,
    scenario: &Value,
    project_root: &Path,
    scenario_path: &Path,
    command: &str,
    godot_binary: Option<&str>,
    godot_launch_prefix: &[String],
    godot_windowed_launch_prefix: &[String],
    run_id: Option<&str>,
    started_at: Option<&str>,
) -> io::Result<RunArtifacts> {
    let scenario_name = scenario
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("scenario");
    let (resolved_run_id, root_dir) = allocate_run_root(artifacts_root, scenario_name, run_id)?;
    let resolved_started_at = started_at
        .map(str::to_owned)
        .unwrap_or_else(default_started_at);
    let mut nonce_bytes = [0_u8; 16];
    getrandom::fill(&mut nonce_bytes).map_err(|error| io::Error::other(error.to_string()))?;
    let run_nonce: String = nonce_bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();

    let artifacts = RunArtifacts {
        run_id: resolved_run_id.clone(),
        root_dir: root_dir.clone(),
        meta_path: root_dir.join("meta.json"),
        request_path: root_dir.join("request.json"),
        result_path: root_dir.join("result.json"),
        logs_path: root_dir.join("logs.txt"),
        progress_path: root_dir.join("progress.jsonl"),
        slow_summary_path: root_dir.join("slow-summary.json"),
        snapshots_dir: root_dir.join("snapshots"),
        temporal_dir: root_dir.join("temporal"),
        audio_dir: root_dir.join("audio"),
    };
    fs::create_dir_all(&artifacts.snapshots_dir)?;
    fs::create_dir_all(&artifacts.temporal_dir)?;
    fs::create_dir_all(&artifacts.audio_dir)?;

    let mut meta = json!({
            "tool": "godot-qa",
            "tool_version": env!("CARGO_PKG_VERSION"),
            "project_root": project_root,
            "scenario": scenario_name,
            "scene": scenario.get("scene").cloned().unwrap_or(Value::Null),
            "viewport": viewport_value(scenario),
            "godot_binary": godot_binary,
            "godot_launch_prefix": godot_launch_prefix,
            "godot_windowed_launch_prefix": godot_windowed_launch_prefix,
            "godot_args": godot_args_value(scenario),
            "project_args": crate::run::redact_project_args_for_metadata(&project_args_value(scenario)),
            "started_at": resolved_started_at,
            "run_id": resolved_run_id,
            "run_nonce": run_nonce,
            "command": command,
            "scenario_path": scenario_path,
    });
    if let Some(time_control) = scenario.get("time_control") {
        if let Some(meta) = meta.as_object_mut() {
            meta.insert("time_control".to_string(), time_control.clone());
        }
    }
    write_json(&artifacts.meta_path, &meta)?;

    let mut run = json!({
        "id": resolved_run_id,
        "started_at": resolved_started_at,
        "nonce": run_nonce,
        "command": command,
        "godot_binary": godot_binary,
        "godot_launch_prefix": godot_launch_prefix,
        "godot_windowed_launch_prefix": godot_windowed_launch_prefix,
        "godot_args": godot_args_value(scenario),
        "project_args": crate::run::redact_project_args_for_metadata(&project_args_value(scenario)),
    });
    if let Some(time_control) = scenario.get("time_control") {
        if let Some(run) = run.as_object_mut() {
            run.insert("time_control".to_string(), time_control.clone());
        }
    }
    if let Some(frame_budget) = crate::run::scenario_frame_budget_json(scenario, project_root) {
        if let Some(run) = run.as_object_mut() {
            run.insert("frame_budget".to_string(), frame_budget);
        }
    }
    write_json(
        &artifacts.request_path,
        &json!({
            "scenario": scenario,
            "artifacts": artifacts.to_json(),
            "run": run
        }),
    )?;

    Ok(artifacts)
}

fn godot_args_value(scenario: &Value) -> Value {
    scenario
        .get("godot_args")
        .and_then(Value::as_array)
        .map(|args| Value::Array(args.clone()))
        .unwrap_or_else(|| json!([]))
}

fn project_args_value(scenario: &Value) -> Value {
    scenario
        .get("project_args")
        .and_then(Value::as_array)
        .map(|args| Value::Array(args.clone()))
        .unwrap_or_else(|| json!([]))
}

pub fn prepare_record_artifacts(
    artifacts_root: &Path,
    scenario_name: &str,
    project_root: &Path,
    scene: &str,
    output_path: &Path,
    command: &str,
    viewport: &[i64],
    headless: bool,
    run_id: Option<&str>,
    started_at: Option<&str>,
) -> io::Result<RecordArtifacts> {
    let (resolved_run_id, root_dir) =
        allocate_run_root(&artifacts_root.join("recordings"), scenario_name, run_id)?;
    let resolved_started_at = started_at
        .map(str::to_owned)
        .unwrap_or_else(default_started_at);

    let artifacts = RecordArtifacts {
        run_id: resolved_run_id.clone(),
        root_dir: root_dir.clone(),
        meta_path: root_dir.join("meta.json"),
        request_path: root_dir.join("request.json"),
        result_path: root_dir.join("result.json"),
        logs_path: root_dir.join("logs.txt"),
        recording_path: root_dir.join("recording.json"),
        candidate_path: root_dir.join("candidate.json"),
        snapshots_dir: root_dir.join("snapshots"),
    };
    fs::create_dir_all(&artifacts.snapshots_dir)?;

    write_json(
        &artifacts.meta_path,
        &json!({
            "tool": "godot-qa",
            "tool_version": env!("CARGO_PKG_VERSION"),
            "project_root": project_root,
            "scenario": scenario_name,
            "scene": scene,
            "viewport": viewport,
            "started_at": resolved_started_at,
            "command": command,
            "scenario_path": output_path,
            "headless": headless,
        }),
    )?;
    write_json(
        &artifacts.request_path,
        &json!({
            "recording": {
                "name": scenario_name,
                "scene": scene,
                "output_path": output_path,
                "viewport": viewport,
                "headless": headless,
            },
            "artifacts": artifacts.to_json(),
            "run": {
                "id": resolved_run_id,
                "started_at": resolved_started_at,
                "command": command,
            }
        }),
    )?;

    Ok(artifacts)
}

fn allocate_run_root(
    artifacts_root: &Path,
    scenario_name: &str,
    run_id: Option<&str>,
) -> io::Result<(String, PathBuf)> {
    let scenario_root = artifacts_root.join(scenario_name);
    if let Some(run_id) = run_id {
        fs::create_dir_all(&scenario_root)?;
        let root_dir = scenario_root.join(run_id);
        fs::create_dir(&root_dir)?;
        return Ok((run_id.to_string(), root_dir));
    }

    fs::create_dir_all(&scenario_root)?;
    let base_run_id = default_run_id();
    let mut candidate_run_id = base_run_id.clone();
    let mut suffix = 1;
    loop {
        let root_dir = scenario_root.join(&candidate_run_id);
        match fs::create_dir(&root_dir) {
            Ok(()) => return Ok((candidate_run_id, root_dir)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                candidate_run_id = format!("{base_run_id}-{suffix:02}");
                suffix += 1;
            }
            Err(error) => return Err(error),
        }
    }
}

fn default_run_id() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| format!("{}Z", duration.as_secs()))
        .unwrap_or_else(|_| "0Z".to_string())
}

fn default_started_at() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| format!("{}Z", duration.as_secs()))
        .unwrap_or_else(|_| "0Z".to_string())
}

pub fn write_json(path: &Path, payload: &Value) -> io::Result<()> {
    fs::write(
        path,
        serde_json::to_string_pretty(payload).unwrap_or_else(|_| "{}".to_string()),
    )
}

fn viewport_value(scenario: &Value) -> Value {
    let Some(viewport) = scenario.get("viewport").and_then(Value::as_object) else {
        return Value::Null;
    };
    let width = viewport.get("width").and_then(Value::as_i64);
    let height = viewport.get("height").and_then(Value::as_i64);
    match (width, height) {
        (Some(width), Some(height)) => json!([width, height]),
        _ => Value::Null,
    }
}
