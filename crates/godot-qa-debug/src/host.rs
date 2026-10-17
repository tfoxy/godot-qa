//! The persistent debugger-host listener plus its localhost control channel.
//!
//! Godot's game connects OUT to us (`--remote-debug tcp://127.0.0.1:<dbg_port>`), so
//! this host must be listening before the game launches and must stay alive for the
//! whole session. It:
//!   * accepts the game's debugger connection and holds it open,
//!   * immediately disables error-break/breakpoint blocking so a script error never
//!     freezes the game, and auto-continues if the game ever breaks,
//!   * continuously drains the game's unsolicited output/perf/error stream,
//!   * serves a small newline-delimited-JSON control channel so transient CLI
//!     processes (`session reload`, `session watch`) can request reloads, which are
//!     translated into debugger frames and written to the game.
//!
//! The reload ack means "frame written to the game"; the engine applies the reload on
//! its next idle poll (fire-and-forget over the debugger protocol).

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use godot_qa_scene::{SceneDiff, SceneEditOp, SceneState};

use crate::variant;

const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

/// On-disk record describing a running debug host, written to `.qa/debug_host.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebugHostRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_identity: Option<u64>,
    pub pid: i64,
    pub dbg_port: u16,
    pub control_port: u16,
    #[serde(default)]
    pub bound: bool,
}

/// The kind of reload a control request asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReloadKind {
    Scripts,
    AllScripts,
    Resources,
}

/// A request sent by a transient CLI process over the control channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlRequest {
    pub op: String,
    #[serde(default)]
    pub paths: Vec<String>,
}

impl ControlRequest {
    pub fn reload_scripts(paths: Vec<String>) -> Self {
        Self {
            op: "reload_scripts".to_string(),
            paths,
        }
    }

    pub fn reload_resources(paths: Vec<String>) -> Self {
        Self {
            op: "reload_resources".to_string(),
            paths,
        }
    }

    /// Faithful structural live-edit of the given `res://` scene paths: diff each
    /// against the sidecar's baseline snapshot and apply the `scene:live_*` delta.
    pub fn reload_structure(paths: Vec<String>) -> Self {
        Self {
            op: "reload_structure".to_string(),
            paths,
        }
    }

    pub fn status() -> Self {
        Self {
            op: "status".to_string(),
            paths: Vec::new(),
        }
    }
}

/// The host's reply to a control request.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ControlResponse {
    pub ok: bool,
    #[serde(default)]
    pub connected: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub applied: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// For `reload_structure`: reasons a scene could not be applied as live edits and
    /// must be full-scene-reloaded instead. Empty when everything applied.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fell_back: Vec<String>,
    /// For `reload_structure`: the number of `scene:live_*` ops sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ops_applied: Option<usize>,
}

impl ControlResponse {
    fn ok_connected() -> Self {
        Self {
            ok: true,
            connected: true,
            ..Self::default()
        }
    }

    fn error(message: impl Into<String>, connected: bool) -> Self {
        Self {
            ok: false,
            connected,
            error: Some(message.into()),
            ..Self::default()
        }
    }
}

/// Configuration for a debug host process.
pub struct DebugHostConfig {
    /// Where to write the `DebugHostRecord` JSON once both sockets are bound.
    pub record_path: PathBuf,
    /// Optional sidecar log file.
    pub log_path: Option<PathBuf>,
    /// How long to wait for the game to dial in before giving up.
    pub game_accept_timeout: Duration,
    /// Project root, used to resolve `res://` scene paths to files for structural diffs.
    pub project_root: Option<PathBuf>,
    /// The session's initial `res://` scene, snapshotted at bind time so the first
    /// structural edit diffs against what the game actually launched with.
    pub initial_scene: Option<String>,
}

/// A bound-but-not-yet-serving debug host.
pub struct DebugHost {
    dbg_listener: TcpListener,
    control_listener: TcpListener,
    log_path: Option<PathBuf>,
    dbg_port: u16,
    control_port: u16,
    game_accept_timeout: Duration,
    project_root: Option<PathBuf>,
    /// Baseline parsed scenes keyed by `res://` path. The authoritative "before" for
    /// structural diffs; updated after each successful `reload_structure`.
    snapshots: Mutex<HashMap<String, godot_qa_scene::SceneState>>,
}

impl DebugHost {
    /// Bind both the debugger listener and the control listener on ephemeral localhost
    /// ports and write the `DebugHostRecord`. Binding here (rather than the parent
    /// picking ports) removes the pick/bind race entirely.
    pub fn bind(cfg: &DebugHostConfig) -> std::io::Result<DebugHost> {
        let dbg_listener = TcpListener::bind(("127.0.0.1", 0))?;
        let control_listener = TcpListener::bind(("127.0.0.1", 0))?;
        let dbg_port = dbg_listener.local_addr()?.port();
        let control_port = control_listener.local_addr()?.port();

        let record = DebugHostRecord {
            process_identity: None,
            pid: std::process::id() as i64,
            dbg_port,
            control_port,
            bound: true,
        };
        write_record_atomically(&cfg.record_path, &record)?;

        // Snapshot the initial scene now (before any edits) so structural diffs have a
        // correct baseline matching what the game loaded at launch.
        let mut snapshots = HashMap::new();
        if let (Some(project_root), Some(scene)) = (&cfg.project_root, &cfg.initial_scene)
            && let Some(state) = read_and_parse_scene(project_root, scene)
        {
            snapshots.insert(scene.clone(), state);
        }

        Ok(DebugHost {
            dbg_listener,
            control_listener,
            log_path: cfg.log_path.clone(),
            dbg_port,
            control_port,
            game_accept_timeout: cfg.game_accept_timeout,
            project_root: cfg.project_root.clone(),
            snapshots: Mutex::new(snapshots),
        })
    }

    pub fn dbg_port(&self) -> u16 {
        self.dbg_port
    }

    pub fn control_port(&self) -> u16 {
        self.control_port
    }

    /// Serve until `shutdown` is set. Blocks. Waits for the game to connect, sends the
    /// safety frames, drains the game stream on a reader thread, and serves control
    /// connections on this thread.
    pub fn run(self, shutdown: Arc<AtomicBool>) -> std::io::Result<()> {
        self.log(&format!(
            "debug host bound dbg_port={} control_port={}",
            self.dbg_port, self.control_port
        ));

        let game = match self.accept_game(&shutdown)? {
            Some(game) => game,
            None => {
                self.log("no game connected before shutdown/timeout; exiting");
                return Ok(());
            }
        };
        self.log("game debugger connection accepted");

        let writer = Arc::new(Mutex::new(game.try_clone()?));
        let connected = Arc::new(AtomicBool::new(true));

        // Safety frames first: never block the game on script errors or breakpoints.
        {
            // Recover from a poisoned lock rather than panicking: the guarded value is
            // just a cloned TcpStream write handle, so a peer thread that panicked mid
            // write leaves it usable and one panic must not brick the whole debug host.
            let mut guard = writer
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let _ = guard.write_all(&variant::message_set_ignore_error_breaks(true));
            let _ = guard.write_all(&variant::message_set_skip_breakpoints(true));
            let _ = guard.flush();
        }

        let reader_stream = game.try_clone()?;
        let reader_writer = Arc::clone(&writer);
        let reader_connected = Arc::clone(&connected);
        let reader_log = self.log_path.clone();
        let reader_handle = std::thread::spawn(move || {
            drain_game_stream(reader_stream, reader_writer, reader_connected, reader_log);
        });

        self.serve_control(&writer, &connected, &shutdown);

        // Unblock the reader thread and wait for it.
        let _ = game.shutdown(Shutdown::Both);
        let _ = reader_handle.join();
        self.log("debug host shutting down");
        Ok(())
    }

    fn accept_game(&self, shutdown: &Arc<AtomicBool>) -> std::io::Result<Option<TcpStream>> {
        self.dbg_listener.set_nonblocking(true)?;
        let deadline = Instant::now() + self.game_accept_timeout;
        loop {
            if shutdown.load(Ordering::SeqCst) || Instant::now() >= deadline {
                return Ok(None);
            }
            match self.dbg_listener.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(false)?;
                    return Ok(Some(stream));
                }
                Err(ref error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn serve_control(
        &self,
        writer: &Arc<Mutex<TcpStream>>,
        connected: &Arc<AtomicBool>,
        shutdown: &Arc<AtomicBool>,
    ) {
        if self.control_listener.set_nonblocking(true).is_err() {
            return;
        }
        while !shutdown.load(Ordering::SeqCst) {
            match self.control_listener.accept() {
                Ok((stream, _)) => {
                    if let Err(error) = self.handle_control_conn(stream, writer, connected) {
                        self.log(&format!("control connection error: {error}"));
                    }
                }
                Err(ref error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(error) => {
                    self.log(&format!("control accept error: {error}"));
                    break;
                }
            }
        }
    }

    fn handle_control_conn(
        &self,
        mut stream: TcpStream,
        writer: &Arc<Mutex<TcpStream>>,
        connected: &Arc<AtomicBool>,
    ) -> std::io::Result<()> {
        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let mut reader = BufReader::new(stream.try_clone()?);
        let mut line = String::new();
        reader.read_line(&mut line)?;
        let response = self.dispatch_control(line.trim(), writer, connected);
        let mut payload = serde_json::to_vec(&response).unwrap_or_default();
        payload.push(b'\n');
        stream.write_all(&payload)?;
        stream.flush()?;
        Ok(())
    }

    fn dispatch_control(
        &self,
        line: &str,
        writer: &Arc<Mutex<TcpStream>>,
        connected: &Arc<AtomicBool>,
    ) -> ControlResponse {
        let request: ControlRequest = match serde_json::from_str(line) {
            Ok(request) => request,
            Err(error) => {
                return ControlResponse::error(
                    format!("invalid_control_request: {error}"),
                    connected.load(Ordering::SeqCst),
                );
            }
        };
        let is_connected = connected.load(Ordering::SeqCst);
        if request.op == "status" || request.op == "ping" {
            return ControlResponse {
                ok: true,
                connected: is_connected,
                ..ControlResponse::default()
            };
        }
        if !is_connected {
            return ControlResponse::error("game_not_connected", false);
        }
        if request.op == "reload_structure" {
            return self.dispatch_structure(&request.paths, writer, connected);
        }
        let frame = match request.op.as_str() {
            "reload_scripts" => variant::message_reload_scripts(&request.paths),
            "reload_all_scripts" => variant::message_reload_all_scripts(),
            "reload_resources" => variant::message_reload_cached_files(&request.paths),
            other => {
                return ControlResponse::error(format!("unknown_op: {other}"), true);
            }
        };
        match write_frames(writer, std::slice::from_ref(&frame)) {
            Ok(()) => {
                self.log(&format!("applied {} paths={:?}", request.op, request.paths));
                ControlResponse {
                    applied: request.paths,
                    ..ControlResponse::ok_connected()
                }
            }
            Err(error) => {
                connected.store(false, Ordering::SeqCst);
                ControlResponse::error(format!("write_failed: {error}"), false)
            }
        }
    }

    /// Faithful structural live-edit: diff each scene against its baseline snapshot and
    /// send the `scene:live_*` delta. Scenes whose delta cannot be expressed live are
    /// reported in `fell_back` so the caller full-scene-reloads them instead.
    fn dispatch_structure(
        &self,
        paths: &[String],
        writer: &Arc<Mutex<TcpStream>>,
        connected: &Arc<AtomicBool>,
    ) -> ControlResponse {
        let Some(project_root) = self.project_root.clone() else {
            return ControlResponse::error("structural_reload_no_project_root", true);
        };

        // Recover from poisoning: the snapshot map is plain data, so a panicked
        // holder must not permanently disable structural reloads.
        let mut snapshots = self
            .snapshots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut applied = Vec::new();
        let mut fell_back = Vec::new();
        let mut ops_applied = 0usize;

        for scene_path in paths {
            let Some(new_state) = read_and_parse_scene(&project_root, scene_path) else {
                fell_back.push(format!("{scene_path}: could not read/parse scene file"));
                continue;
            };

            // No baseline yet (scene not the launch scene): capture and treat as a no-op
            // so the next edit diffs against a real "before".
            let Some(old_state) = snapshots.get(scene_path) else {
                snapshots.insert(scene_path.clone(), new_state);
                fell_back.push(format!(
                    "{scene_path}: baseline captured (no prior snapshot)"
                ));
                continue;
            };

            let scene_diff = godot_qa_scene::diff(old_state, &new_state);
            if scene_diff.requires_full_reload() {
                for reason in &scene_diff.fell_back {
                    fell_back.push(format!("{scene_path}: {reason}"));
                }
                continue;
            }

            let frames = structural_frames(scene_path, &scene_diff);
            if let Err(error) = write_frames(writer, &frames) {
                connected.store(false, Ordering::SeqCst);
                return ControlResponse::error(format!("write_failed: {error}"), false);
            }
            self.log(&format!(
                "applied reload_structure {scene_path} ops={}",
                scene_diff.ops.len()
            ));
            ops_applied += scene_diff.ops.len();
            snapshots.insert(scene_path.clone(), new_state);
            applied.push(scene_path.clone());
        }

        ControlResponse {
            ok: true,
            connected: true,
            applied,
            fell_back,
            ops_applied: Some(ops_applied),
            ..ControlResponse::default()
        }
    }

    fn log(&self, message: &str) {
        write_log(self.log_path.as_ref(), message);
    }
}

/// Write a batch of pre-encoded frames to the game under the writer lock.
fn write_frames(writer: &Arc<Mutex<TcpStream>>, frames: &[Vec<u8>]) -> std::io::Result<()> {
    // Recover a poisoned writer lock: it only guards a TcpStream write handle.
    let mut guard = writer
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    for frame in frames {
        guard.write_all(frame)?;
    }
    guard.flush()
}

/// Resolve a `res://` path to a filesystem path under `project_root`.
fn resolve_res_path(project_root: &Path, res_path: &str) -> Option<PathBuf> {
    let relative = res_path.strip_prefix("res://")?;
    Some(project_root.join(relative))
}

/// Read and parse a `res://` scene file, or `None` if it can't be read.
fn read_and_parse_scene(project_root: &Path, res_path: &str) -> Option<SceneState> {
    let file = resolve_res_path(project_root, res_path)?;
    let text = fs::read_to_string(file).ok()?;
    Some(godot_qa_scene::parse_scene(&text))
}

/// Intern a NodePath to a small int id, emitting a `live_node_path` frame the first time.
fn intern_path(
    frames: &mut Vec<Vec<u8>>,
    path_ids: &mut HashMap<String, i64>,
    next_id: &mut i64,
    path: &str,
) -> i64 {
    if let Some(id) = path_ids.get(path) {
        return *id;
    }
    let id = *next_id;
    *next_id += 1;
    path_ids.insert(path.to_string(), id);
    frames.push(variant::message_live_node_path(path, id));
    id
}

/// Encode a scene's structural diff into ordered `scene:live_*` frames: `live_set_root`
/// once, then one frame per op — interning each property target's NodePath via
/// `live_node_path` before the first `live_node_prop`/`live_node_prop_res` that uses it.
fn structural_frames(scene_path: &str, diff: &SceneDiff) -> Vec<Vec<u8>> {
    let mut frames = vec![variant::message_live_set_root("/root", scene_path)];
    let mut path_ids: HashMap<String, i64> = HashMap::new();
    let mut next_id: i64 = 1;
    for op in &diff.ops {
        match op {
            SceneEditOp::RemoveNode { path } => {
                frames.push(variant::message_live_remove_node(path));
            }
            SceneEditOp::ReparentNode {
                path,
                new_parent,
                new_name,
                pos,
            } => {
                frames.push(variant::message_live_reparent_node(
                    path, new_parent, new_name, *pos,
                ));
            }
            SceneEditOp::CreateNode {
                parent,
                type_name,
                name,
            } => {
                frames.push(variant::message_live_create_node(parent, type_name, name));
            }
            SceneEditOp::InstantiateNode {
                parent,
                scene_path,
                name,
            } => {
                frames.push(variant::message_live_instantiate_node(
                    parent, scene_path, name,
                ));
            }
            SceneEditOp::SetProp { path, prop, value } => {
                let id = intern_path(&mut frames, &mut path_ids, &mut next_id, path);
                frames.push(variant::message_live_node_prop(id, prop, value));
            }
            SceneEditOp::SetPropRes {
                path,
                prop,
                res_path,
            } => {
                let id = intern_path(&mut frames, &mut path_ids, &mut next_id, path);
                frames.push(variant::message_live_node_prop_res(id, prop, res_path));
            }
        }
    }
    frames
}

/// Read and discard everything the game sends, auto-continuing if it ever breaks.
fn drain_game_stream(
    mut stream: TcpStream,
    writer: Arc<Mutex<TcpStream>>,
    connected: Arc<AtomicBool>,
    log_path: Option<PathBuf>,
) {
    while let Ok(body) = read_frame(&mut stream) {
        if variant::peek_command(&body).as_deref() == Some("debug_enter") {
            write_log(
                log_path.as_ref(),
                "game broke into debugger; auto-continuing",
            );
            let mut guard = writer
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let _ = guard.write_all(&variant::message_continue());
            let _ = guard.flush();
        }
    }
    connected.store(false, Ordering::SeqCst);
    write_log(log_path.as_ref(), "game debugger connection closed");
}

fn read_frame(stream: &mut impl Read) -> std::io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > MAX_FRAME_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "debugger frame exceeds maximum size",
        ));
    }
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body)?;
    Ok(body)
}

fn write_record_atomically(path: &PathBuf, record: &DebugHostRecord) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let payload = serde_json::to_vec_pretty(record).unwrap_or_default();
    fs::write(&tmp, payload)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

fn write_log(log_path: Option<&PathBuf>, message: &str) {
    if let Some(path) = log_path
        && let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path)
    {
        let _ = writeln!(file, "{message}");
    }
}

/// Connect to a running host's control channel, send one request, and read its reply.
pub fn send_control_request(
    control_port: u16,
    request: &ControlRequest,
) -> std::io::Result<ControlResponse> {
    let mut stream = TcpStream::connect(("127.0.0.1", control_port))?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    let mut payload = serde_json::to_vec(request).unwrap_or_default();
    payload.push(b'\n');
    stream.write_all(&payload)?;
    stream.flush()?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    serde_json::from_str(line.trim())
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_request_serialization_round_trips() {
        let request = ControlRequest::reload_scripts(vec!["res://a.gd".to_string()]);
        let json = serde_json::to_string(&request).unwrap();
        let parsed: ControlRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.op, "reload_scripts");
        assert_eq!(parsed.paths, vec!["res://a.gd".to_string()]);
    }

    #[test]
    fn status_request_needs_no_paths() {
        let parsed: ControlRequest = serde_json::from_str("{\"op\":\"status\"}").unwrap();
        assert_eq!(parsed.op, "status");
        assert!(parsed.paths.is_empty());
    }

    /// Read one length-prefixed frame body from a socket (test helper mirroring `read_frame`).
    fn read_frame_body(stream: &mut TcpStream) -> Vec<u8> {
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).expect("frame length");
        let len = u32::from_le_bytes(len_buf) as usize;
        let mut body = vec![0u8; len];
        stream.read_exact(&mut body).expect("frame body");
        body
    }

    // Risk #1 regression: a script error under `--hot-reload` must never freeze the game.
    // The host proactively disables error-break/breakpoint blocking on connect and
    // auto-continues if the game ever breaks, so `debug()` returns early and never blocks.
    #[test]
    fn host_sends_safety_frames_and_auto_continues_on_break() {
        let dir = std::env::temp_dir().join(format!(
            "gqa-debug-host-safety-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::create_dir_all(&dir);
        let record_path = dir.join("debug_host.json");
        let host = DebugHost::bind(&DebugHostConfig {
            record_path,
            log_path: None,
            game_accept_timeout: Duration::from_secs(5),
            project_root: None,
            initial_scene: None,
        })
        .unwrap();
        let dbg_port = host.dbg_port();

        let shutdown = Arc::new(AtomicBool::new(false));
        let run_shutdown = Arc::clone(&shutdown);
        let handle = std::thread::spawn(move || host.run(run_shutdown));

        // Play the game: dial in as the debugger client and hold the connection.
        let mut game = TcpStream::connect(("127.0.0.1", dbg_port)).expect("connect as game");
        game.set_read_timeout(Some(Duration::from_secs(5))).unwrap();

        // On connect the host must immediately send the two non-blocking safety frames.
        assert_eq!(
            variant::peek_command(&read_frame_body(&mut game)).as_deref(),
            Some("set_ignore_error_breaks")
        );
        assert_eq!(
            variant::peek_command(&read_frame_body(&mut game)).as_deref(),
            Some("set_skip_breakpoints")
        );

        // Simulate a script error: the game breaks into the debugger. The host must
        // auto-continue rather than leaving the game blocked.
        game.write_all(&variant::encode_message(
            "debug_enter",
            variant::MAIN_THREAD_ID,
            &[],
        ))
        .unwrap();
        assert_eq!(
            variant::peek_command(&read_frame_body(&mut game)).as_deref(),
            Some("continue")
        );

        shutdown.store(true, Ordering::SeqCst);
        let _ = game.shutdown(Shutdown::Both);
        handle.join().unwrap().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn bind_writes_record_with_ports() {
        let dir = std::env::temp_dir().join(format!("gqa-debug-host-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let record_path = dir.join("debug_host.json");
        let host = DebugHost::bind(&DebugHostConfig {
            record_path: record_path.clone(),
            log_path: None,
            game_accept_timeout: Duration::from_millis(10),
            project_root: None,
            initial_scene: None,
        })
        .unwrap();
        let contents = fs::read_to_string(&record_path).unwrap();
        let record: DebugHostRecord = serde_json::from_str(&contents).unwrap();
        assert!(record.bound);
        assert_eq!(record.dbg_port, host.dbg_port());
        assert_eq!(record.control_port, host.control_port());
        assert_ne!(record.dbg_port, record.control_port);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn reload_structure_diffs_edited_scene_and_sends_live_frames() {
        // End-to-end (no real Godot): baseline snapshot at bind, edit the .tscn on disk,
        // then a reload_structure control request must parse+diff+send scene:live_* frames
        // to the connected fake game.
        let dir = std::env::temp_dir().join(format!(
            "gqa-structure-e2e-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::create_dir_all(&dir);
        let scene_file = dir.join("main.tscn");
        fs::write(
            &scene_file,
            "[gd_scene format=3]\n[node name=\"Main\" type=\"Node2D\"]\n",
        )
        .unwrap();

        let host = DebugHost::bind(&DebugHostConfig {
            record_path: dir.join("debug_host.json"),
            log_path: None,
            game_accept_timeout: Duration::from_secs(5),
            project_root: Some(dir.clone()),
            initial_scene: Some("res://main.tscn".to_string()),
        })
        .unwrap();
        let dbg_port = host.dbg_port();
        let control_port = host.control_port();

        let shutdown = Arc::new(AtomicBool::new(false));
        let run_shutdown = Arc::clone(&shutdown);
        let handle = std::thread::spawn(move || host.run(run_shutdown));

        let mut game = TcpStream::connect(("127.0.0.1", dbg_port)).expect("connect as game");
        game.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        // Drain the two safety frames sent on connect.
        assert_eq!(
            variant::peek_command(&read_frame_body(&mut game)).as_deref(),
            Some("set_ignore_error_breaks")
        );
        assert_eq!(
            variant::peek_command(&read_frame_body(&mut game)).as_deref(),
            Some("set_skip_breakpoints")
        );

        // Edit the scene: add a Label with a text property under the root.
        fs::write(
            &scene_file,
            "[gd_scene format=3]\n\
             [node name=\"Main\" type=\"Node2D\"]\n\
             [node name=\"HUD\" type=\"Label\" parent=\".\"]\n\
             text = \"hi\"\n",
        )
        .unwrap();

        let response = send_control_request(
            control_port,
            &ControlRequest::reload_structure(vec!["res://main.tscn".to_string()]),
        )
        .expect("control request");
        assert!(response.ok, "response: {response:?}");
        assert!(
            response.fell_back.is_empty(),
            "fell_back: {:?}",
            response.fell_back
        );
        assert_eq!(response.applied, vec!["res://main.tscn".to_string()]);
        assert_eq!(response.ops_applied, Some(2)); // CreateNode + SetProp

        // The game must receive set_root, create_node, node_path (intern), node_prop.
        let commands: Vec<String> = (0..4)
            .map(|_| variant::peek_command(&read_frame_body(&mut game)).unwrap())
            .collect();
        assert_eq!(
            commands,
            vec![
                "scene:live_set_root",
                "scene:live_create_node",
                "scene:live_node_path",
                "scene:live_node_prop",
            ]
        );

        shutdown.store(true, Ordering::SeqCst);
        let _ = game.shutdown(Shutdown::Both);
        handle.join().unwrap().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn structural_frames_emit_set_root_then_ops_with_path_interning() {
        use godot_qa_scene::GodotValue;

        let diff = SceneDiff {
            ops: vec![
                SceneEditOp::CreateNode {
                    parent: ".".to_string(),
                    type_name: "Label".to_string(),
                    name: "HUD".to_string(),
                },
                SceneEditOp::SetProp {
                    path: "HUD".to_string(),
                    prop: "text".to_string(),
                    value: GodotValue::Str("hi".to_string()),
                },
                SceneEditOp::SetProp {
                    path: "HUD".to_string(),
                    prop: "visible".to_string(),
                    value: GodotValue::Bool(true),
                },
                SceneEditOp::RemoveNode {
                    path: "Old".to_string(),
                },
            ],
            fell_back: Vec::new(),
        };
        let frames = structural_frames("res://main.tscn", &diff);
        let commands: Vec<String> = frames
            .iter()
            .map(|frame| variant::peek_command(&frame[4..]).unwrap())
            .collect();
        assert_eq!(
            commands,
            vec![
                "scene:live_set_root",
                "scene:live_create_node",
                "scene:live_node_path", // "HUD" interned once...
                "scene:live_node_prop",
                "scene:live_node_prop", // ...reused, no second live_node_path
                "scene:live_remove_node",
            ]
        );
    }

    // A panicking holder must not brick the writer: `write_frames` recovers the
    // poisoned lock and keeps serving instead of propagating the poison panic.
    #[test]
    fn write_frames_recovers_from_a_poisoned_writer_lock() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");
        let mut peer = TcpStream::connect(addr).expect("connect loopback");
        let (server, _) = listener.accept().expect("accept loopback");
        let writer = Arc::new(Mutex::new(server));

        // Poison the mutex: panic while holding the guard.
        let poison_writer = Arc::clone(&writer);
        let _ = std::thread::spawn(move || {
            let _guard = poison_writer.lock().unwrap();
            panic!("poison the writer lock");
        })
        .join();
        assert!(writer.is_poisoned());

        let frame = vec![1u8, 2, 3, 4];
        write_frames(&writer, std::slice::from_ref(&frame)).expect("write after poison");

        let mut received = vec![0u8; frame.len()];
        peer.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        peer.read_exact(&mut received).expect("read frame bytes");
        assert_eq!(received, frame);
    }
}
