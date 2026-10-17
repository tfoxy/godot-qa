#![allow(clippy::result_large_err)]

use godot_qa_core::audio::{
    AudioAnalysisOptions, analyze_wav_file_with_options, write_audio_analysis_artifact,
};
use godot_qa_core::config::ProjectConfig;
use godot_qa_core::profile;
use godot_qa_core::project_file::read_project_godot_cached;
use godot_qa_core::results::{CommandResult, FailureDetail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub mod android_artifacts;
pub mod event_trace;
pub mod random;

pub const PROTOCOL_VERSION: &str = "0.1";

pub const SESSION_STATUS: &str = "session.status";
pub const SESSION_PAUSE: &str = "session.pause";
pub const SESSION_RESUME: &str = "session.resume";
pub const SESSION_STEP: &str = "session.step";
pub const SESSION_WAIT_FRAMES: &str = "session.wait_frames";
pub const SESSION_SET_TIME_SCALE: &str = "session.set_time_scale";
pub const SESSION_RELOAD_SCENE: &str = "session.reload_scene";
pub const SESSION_CHANGE_SCENE: &str = "session.change_scene";
pub const SESSION_SET_OPTIONS: &str = "session.set_options";
pub const SCENARIO_RUN: &str = "scenario.run";
pub const SESSION_STOP: &str = "session.stop";
pub const SESSION_PROFILE: &str = "session.profile";
pub const SESSION_SOAK_START: &str = "session.profile.soak_start";
pub const SESSION_SOAK_STATUS: &str = "session.profile.soak_status";
pub const SESSION_SOAK_STOP: &str = "session.profile.soak_stop";
pub const SNAPSHOT_CAPTURE: &str = "snapshot.capture";
pub const SCREENSHOT_CAPTURE: &str = "screenshot.capture";
pub const RECORD_EXPORT: &str = "record.export";
pub const AUDIO_CAPTURE: &str = "audio.capture";
pub const AUDIO_EXPORT: &str = "audio.export";
pub const AUDIO_RENDER: &str = "audio.render";
pub const LOGS_READ: &str = "logs.read";
pub const TREE_DUMP: &str = "tree.dump";
pub const UI_DUMP: &str = "ui.dump";
pub const FOCUS_GET: &str = "focus.get";
pub const NODE_INSPECT: &str = "node.inspect";
pub const PHYSICS_INSPECT: &str = "physics.inspect";
pub const PHYSICS_OVERLAY_COLLECT: &str = "physics.overlay.collect";
pub const SURFACE_AUDIT_PROBE: &str = "surface.audit_probe";
pub const VISUAL_AUDIT_TARGETS: &str = "visual.audit_targets";
pub const TRACE_RECORD: &str = "trace.record";
pub const EVENT_TRACE: &str = "event.trace";
pub const TELEMETRY_RECORD: &str = "telemetry.record";
pub const FOCUS_GRAPH: &str = "focus.graph";
pub const ERRORS_READ: &str = "errors.read";
pub const INPUT_TAP_ACTION: &str = "input.tap_action";
pub const INPUT_HOLD_ACTION: &str = "input.hold_action";
pub const INPUT_RELEASE_ACTION: &str = "input.release_action";
pub const INPUT_CLICK_TARGET: &str = "input.click_target";
pub const INPUT_MOUSE_CLICK: &str = "input.mouse_click";
pub const INPUT_KEY: &str = "input.key";
pub const INPUT_TEXT: &str = "input.text";
pub const INPUT_JOY_BUTTON: &str = "input.joy_button";
pub const INPUT_JOY_MOTION: &str = "input.joy_motion";
pub const INPUT_JOY_STATE: &str = "input.joy_state";
pub const INPUT_MOUSE_WHEEL: &str = "input.mouse_wheel";
pub const INPUT_TOUCH_DOWN: &str = "input.touch_down";
pub const INPUT_TOUCH_MOVE: &str = "input.touch_move";
pub const INPUT_TOUCH_UP: &str = "input.touch_up";
pub const INPUT_TOUCH_TAP: &str = "input.touch_tap";
pub const INPUT_DRAG: &str = "input.drag";
pub const INPUT_RESET: &str = "input.reset";
pub const SETUP_JOY_CONNECT: &str = "setup.joy_connect";
pub const SETUP_JOY_DISCONNECT: &str = "setup.joy_disconnect";
pub const SETUP_CALL_METHOD: &str = "setup.call_method";
pub const FIXTURE_CALL: &str = "fixture.call";
pub const EDIT_SET_PROPERTY: &str = "edit.set_property";
pub const EDIT_CALL_METHOD: &str = "edit.call_method";
pub const EDIT_DIFF: &str = "edit.diff";
pub const EDIT_EXPORT_PATCH: &str = "edit.export_patch";
pub const ASSERT_EXISTS: &str = "assert.exists";
pub const ASSERT_NOT_EXISTS: &str = "assert.not_exists";
pub const ASSERT_VISIBLE: &str = "assert.visible";
pub const ASSERT_VISIBLE_PIXELS: &str = "assert.visible_pixels";
pub const ASSERT_NOT_OCCLUDED: &str = "assert.not_occluded";
pub const ASSERT_RENDERS_ABOVE: &str = "assert.renders_above";
pub const ASSERT_RENDERS_BELOW: &str = "assert.renders_below";
pub const ASSERT_HIDDEN: &str = "assert.hidden";
pub const ASSERT_ENABLED: &str = "assert.enabled";
pub const ASSERT_DISABLED: &str = "assert.disabled";
pub const ASSERT_FOCUSED: &str = "assert.focused";
pub const ASSERT_TEXT_EQUALS: &str = "assert.text_equals";
pub const ASSERT_TEXT_CONTAINS: &str = "assert.text_contains";
pub const ASSERT_INSIDE_VIEWPORT: &str = "assert.inside_viewport";
pub const ASSERT_NO_OVERLAP: &str = "assert.no_overlap";
pub const ASSERT_NO_ERRORS: &str = "assert.no_errors";
pub const ASSERT_DISTANCE: &str = "assert.distance";
pub const ASSERT_STATE_MATCHES: &str = "assert.state_matches";
pub const QA_STATE_READ: &str = "qa_state.read";
pub const STATE_RECORD: &str = "state.record";
pub const ASSERT_SURFACE_PROBE: &str = "assert.surface_probe";
pub const ASSERT_SURFACE_MATCHES: &str = "assert.surface_matches";
pub const ASSERT_TEMPORAL_MAX_DELTA_POSITION: &str = "assert.temporal.max_delta_position";
pub const ASSERT_TEMPORAL_FOLLOWS: &str = "assert.temporal.follows";
pub const ASSERT_TEMPORAL_NEVER: &str = "assert.temporal.never";
pub const ASSERT_TEMPORAL_WITHIN_LIMITS: &str = "assert.temporal.within_limits";
pub const ASSERT_TEMPORAL_DISTANCE_CHANGED: &str = "assert.temporal.distance_changed";

const LIVE_SESSION_REQUEST_TIMEOUT_SECONDS: f64 = 5.0;
const LIVE_AUDIO_REQUEST_GRACE_SECONDS: f64 = 5.0;
const LIVE_SESSION_WAIT_TIMEOUT_FPS: f64 = 30.0;
const LIVE_SESSION_WAIT_TIMEOUT_GRACE_SECONDS: f64 = 5.0;
const LIVE_SESSION_WAIT_TIMEOUT_MAX_SECONDS: f64 = 120.0;
const LIVE_TRACE_RECORD_TIMEOUT_FPS: f64 = 5.0;
const LIVE_TRACE_RECORD_TIMEOUT_GRACE_SECONDS: f64 = 20.0;
const LIVE_EVENT_TRACE_TIMEOUT_FPS: f64 = 10.0;
const LIVE_EVENT_TRACE_TIMEOUT_GRACE_SECONDS: f64 = 20.0;
const LIVE_PROFILE_TIMEOUT_FPS: f64 = 10.0;
const LIVE_PROFILE_TIMEOUT_GRACE_SECONDS: f64 = 20.0;
const LIVE_SESSION_STARTUP_TIMEOUT_SECONDS: f64 = 10.0;
const LIVE_SCENARIO_RUN_DEFAULT_TIMEOUT_SECONDS: f64 = 120.0;
const LIVE_SCENARIO_RUN_GRACE_SECONDS: f64 = 15.0;
const LIVE_SCENARIO_RUN_TIMEOUT_MAX_SECONDS: f64 = 600.0;
// When the engine enforces a frame budget (params carry timeout_frames), the
// transport wall clock is only a hang backstop: slack absorbs host load and
// the cap bounds true hangs without failing correct slow-wall runs.
const LIVE_SCENARIO_RUN_FRAME_BUDGET_SLACK: f64 = 3.0;
const LIVE_SCENARIO_RUN_FRAME_BUDGET_TIMEOUT_MAX_SECONDS: f64 = 1800.0;
const PROCESS_STOP_WAIT_TIMEOUT_SECONDS: f64 = 3.0;
const LIVE_COMMAND_RETRY_BACKOFF_MILLIS: &[u64] = &[25, 50];
const LIVE_COMMAND_BUSY_FPS: f64 = 30.0;
const LIVE_COMMAND_BUSY_GRACE_MILLIS: u64 = 750;
const LIVE_COMMAND_BUSY_MAX_BACKOFF_MILLIS: u64 = 120_000;
const LIVE_COMMAND_RETRYABLE_FAILURE_KINDS: &[&str] = &[
    "session_bridge_busy",
    "resource_unavailable",
    "resource_busy",
    "bridge_busy",
    "session_connect_failed",
];
const LIVE_PROFILE_RETRY_BACKOFF_MILLIS: &[u64] = &[25, 50];
const LIVE_PROFILE_RETRYABLE_FAILURE_KINDS: &[&str] =
    &["session_bridge_busy", "session_connect_failed"];
const DEFAULT_FIXED_FPS: i64 = 60;
const MAX_SAFE_FIXED_FPS: i64 = 100_000;
const PROJECT_MAX_FPS_KEY: &str = "application/run/max_fps";
const PROJECT_PHYSICS_TICKS_KEY: &str = "physics/common/physics_ticks_per_second";

#[derive(Debug, Clone)]
pub struct ProtocolError {
    pub failure: FailureDetail,
}

#[derive(Debug, Clone)]
pub struct LiveAudioCaptureRequest {
    pub seconds: f64,
    pub max_seconds: f64,
    pub bus: Option<String>,
    pub name: String,
    pub out: Option<PathBuf>,
    pub analyze: bool,
    pub strict_warnings: bool,
    pub stop_after_state: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct LiveAudioExportRequest {
    pub target: String,
    pub method: String,
    pub args: Value,
    pub name: String,
    pub out: Option<PathBuf>,
    pub analyze: bool,
    pub strict_warnings: bool,
}

#[derive(Debug, Clone)]
pub struct LiveAudioRenderRequest {
    pub target: String,
    pub method: String,
    pub args: Value,
    pub seconds: f64,
    pub max_seconds: f64,
    pub mix_rate: u32,
    pub name: String,
    pub out: Option<PathBuf>,
    pub analyze: bool,
    pub strict_warnings: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    /// Windows process creation time, used to reject reused PIDs during cleanup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_identity: Option<u64>,
    pub protocol_version: String,
    pub session_id: String,
    pub pid: i64,
    pub host: String,
    pub port: u16,
    pub scene: String,
    pub viewport: Vec<i64>,
    pub artifact_dir: String,
    pub request_path: String,
    pub token_file: String,
    pub started_at: String,
    #[serde(default)]
    pub headless: bool,
    #[serde(default)]
    pub project_args: Vec<String>,
    #[serde(default)]
    pub frame_pacing: Option<SessionFramePacing>,
    /// Execution target. Omitted records from pre-Android releases deserialize as desktop.
    #[serde(default, skip_serializing_if = "SessionTarget::is_desktop")]
    pub target: SessionTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub android: Option<AndroidSessionMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AndroidSessionMetadata {
    pub device_serial: String,
    pub package: String,
    pub preset: String,
    /// PID in the Android device process namespace. Never pass this to host process APIs.
    pub device_pid: i64,
    pub device_port: u16,
    pub forwarded_host_port: u16,
    pub adb_path: String,
    /// Controlled private device artifact root. Returned paths must stay below it.
    pub device_artifact_root: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logcat_pid: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logcat_process_identity: Option<u64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub deployment_log_path: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub logcat_log_path: String,
}

impl SessionRecord {
    pub fn to_json_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or_else(|_| json!({}))
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionTarget {
    #[default]
    Desktop,
    Android,
}

impl SessionTarget {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::Android => "android",
        }
    }

    pub fn is_desktop(&self) -> bool {
        *self == Self::Desktop
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionLiveness {
    Running,
    Stopped,
    /// The target cannot currently be queried. Metadata must remain recoverable.
    Unavailable,
}

#[derive(Debug, Clone)]
pub struct SessionStartRequest<'a> {
    pub target: SessionTarget,
    pub scene: Option<&'a str>,
    /// None means the target default (1920x1080 on desktop, device viewport on Android).
    pub viewport: Option<&'a str>,
    pub headless: bool,
    pub audio: bool,
    /// Retained separately so Android can reject an explicit `--no-audio`.
    pub no_audio: bool,
    pub project_args: &'a [String],
    pub frame_pacing_request: Option<Option<i64>>,
    pub fastest: bool,
    pub hot_reload: bool,
    pub preset: Option<&'a str>,
    pub device: Option<&'a str>,
    pub package: Option<&'a str>,
    /// Android only: skip debug-APK cache reuse and force a fresh export. The fresh APK
    /// still refreshes the cache entry (docker-style bypass of the read only).
    pub no_apk_cache: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionFramePacing {
    pub fixed_fps: i64,
    pub source: String,
    pub project_setting: Option<String>,
}

impl SessionFramePacing {
    pub fn generated_godot_args(&self) -> Vec<String> {
        vec![
            "--fixed-fps".to_string(),
            self.fixed_fps.to_string(),
            "--disable-vsync".to_string(),
        ]
    }

    pub fn to_json(&self) -> Value {
        json!({
            "mode": "fastest",
            "fixed_fps": self.fixed_fps,
            "source": self.source,
            "project_setting": self.project_setting,
            "generated_godot_args": self.generated_godot_args(),
        })
    }
}

include!("protocol.rs");
include!("session.rs");
include!("live_commands.rs");
include!("pool_session.rs");
include!("events_live.rs");
include!("audio_live.rs");
include!("profile_live.rs");
include!("transport.rs");
