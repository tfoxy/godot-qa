use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};
use godot_qa_scenario::review as scenario_review;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "godot-qa", about = "Runtime QA CLI for Godot", version)]
pub(crate) struct Cli {
    #[arg(long, global = true, action = ArgAction::SetTrue)]
    pub(crate) json: bool,
    #[arg(long, global = true)]
    pub(crate) project: Option<String>,
    #[arg(long = "godot-bin", global = true)]
    pub(crate) godot_bin: Option<String>,
    #[arg(long = "artifact-dir", global = true)]
    pub(crate) artifact_dir: Option<String>,
    #[arg(long, global = true, action = ArgAction::SetTrue)]
    pub(crate) verbose: bool,
    #[command(subcommand)]
    pub(crate) command: Commands,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Commands {
    Doctor(DoctorArgs),
    #[command(name = "install-skills")]
    InstallSkills,
    Import,
    Check(CheckArgs),
    Diagnostics(DiagnosticsArgs),
    Profile(ProfileArgs),
    Scenario(ScenarioArgs),
    Review(ReviewArgs),
    #[command(name = "live-gate")]
    LiveGate(LiveGateArgs),
    #[command(name = "qa-state")]
    QaState(QaStateArgs),
    Artifact(ArtifactArgs),
    Asset(AssetArgs),
    Session(SessionArgs),
    Input(InputArgs),
    Snapshot(SnapshotArgs),
    Tree(TreeArgs),
    Ui,
    Focus(FocusArgs),
    Node(NodeArgs),
    Physics(PhysicsArgs),
    Surface(SurfaceArgs),
    Trace(TraceArgs),
    Event(EventArgs),
    Telemetry(TelemetryArgs),
    Audio(AudioArgs),
    Errors,
    Layout(LayoutArgs),
    Assert(AssertArgs),
    Screenshot(ScreenshotArgs),
    Visual(VisualArgs),
    #[command(name = "edit-live")]
    EditLive(EditArgs),
    #[command(name = "apply-patch")]
    ApplyPatch(ApplyPatchArgs),
    Mcp,
    Logs(LogsArgs),
    /// Internal: run the persistent debug-host sidecar that backs `--hot-reload`.
    /// Hidden from `--help`, so it never appears in the doctor command surface.
    #[command(name = "__debug-host", hide = true)]
    DebugHost(DebugHostArgs),
    #[command(name = "__presented-helper", hide = true)]
    PresentedHelper(PresentedHelperArgs),
}

#[derive(Debug, Args)]
pub(crate) struct AssetArgs {
    #[command(subcommand)]
    pub(crate) command: AssetCommands,
}

#[derive(Debug, Subcommand)]
pub(crate) enum AssetCommands {
    /// Import candidates and compare original/candidate runtime evidence.
    Try { manifest: String },
    /// Inspect a trial, or list the project's journals and active reservation.
    Status { trial_id: Option<String> },
    /// Accept a verified trial without approving screenshot baselines.
    Keep { trial_id: String },
    /// Restore the original scene, including interrupted trial recovery.
    Discard { trial_id: String },
}

#[derive(Debug, Args)]
pub(crate) struct PresentedHelperArgs {
    #[arg(long)]
    pub(crate) owner: PathBuf,
    #[arg(long)]
    pub(crate) result: PathBuf,
    #[arg(long)]
    pub(crate) nonce: String,
    #[arg(long)]
    pub(crate) scene: String,
    #[arg(long)]
    pub(crate) viewport: String,
    #[arg(long)]
    pub(crate) backend: String,
    #[arg(long)]
    pub(crate) baseline: Option<PathBuf>,
    #[arg(long)]
    pub(crate) out: PathBuf,
    #[arg(long)]
    pub(crate) threshold: f64,
    #[arg(long, default_value_t = 0)]
    pub(crate) channel_tolerance: u8,
}

#[derive(Debug, Args)]
pub(crate) struct DebugHostArgs {
    /// Path to write the debug-host record JSON (`.qa/debug_host.json`).
    #[arg(long = "record")]
    pub(crate) record: PathBuf,
    /// Optional sidecar log file.
    #[arg(long = "log")]
    pub(crate) log: Option<PathBuf>,
    /// Project root, used to resolve `res://` scene paths for structural (`.tscn`) diffs.
    #[arg(long = "project-root")]
    pub(crate) project_root: Option<PathBuf>,
    /// The session's launch scene (`res://…`), baseline-snapshotted for structural diffs.
    #[arg(long = "scene")]
    pub(crate) scene: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct DoctorArgs {
    #[arg(long = "strict-binary-surface", action = ArgAction::SetTrue)]
    pub(crate) strict_binary_surface: bool,
    #[arg(long = "strict-addon-version", action = ArgAction::SetTrue)]
    pub(crate) strict_addon_version: bool,
}

#[derive(Debug, Args)]
pub(crate) struct CheckArgs {
    #[arg(long, conflicts_with_all = ["scene", "tests", "script", "all_scripts"])]
    pub(crate) all_scenes: bool,
    #[arg(long, conflicts_with_all = ["all_scenes", "tests", "script", "all_scripts"])]
    pub(crate) scene: Option<String>,
    #[arg(long, conflicts_with_all = ["all_scenes", "scene", "script", "all_scripts"])]
    pub(crate) tests: bool,
    #[arg(long, conflicts_with_all = ["all_scenes", "scene", "tests", "all_scripts"])]
    pub(crate) script: Option<String>,
    #[arg(long, conflicts_with_all = ["all_scenes", "scene", "tests", "script"])]
    pub(crate) all_scripts: bool,
    #[arg(long, requires = "tests", conflicts_with_all = ["all_scenes", "scene"])]
    pub(crate) batch: bool,
    #[arg(
        long = "test",
        action = ArgAction::Append,
        requires = "tests",
        conflicts_with_all = ["all_scenes", "scene", "tag"]
    )]
    pub(crate) test: Vec<String>,
    #[arg(
        long = "tag",
        action = ArgAction::Append,
        requires = "tests",
        conflicts_with_all = ["all_scenes", "scene", "test"]
    )]
    pub(crate) tag: Vec<String>,
    #[arg(
        long = "profile",
        requires = "tests",
        conflicts_with_all = ["all_scenes", "scene"],
        action = ArgAction::Append,
        num_args = 0..=1,
        default_missing_value = ""
    )]
    pub(crate) profile: Vec<String>,
    #[arg(long = "timeout-seconds")]
    pub(crate) timeout_seconds: Option<f64>,
}

#[derive(Debug, Args)]
pub(crate) struct DiagnosticsArgs {
    #[command(subcommand)]
    pub(crate) command: DiagnosticsCommands,
}

#[derive(Debug, Subcommand)]
pub(crate) enum DiagnosticsCommands {
    Editor(EditorDiagnosticsArgs),
}

#[derive(Debug, Args)]
pub(crate) struct EditorDiagnosticsArgs {
    #[arg(long = "strict-warnings", action = ArgAction::SetTrue)]
    pub(crate) strict_warnings: bool,
    #[arg(long, value_enum, default_value_t = EditorDiagnosticsSource::Combined)]
    pub(crate) source: EditorDiagnosticsSource,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EditorDiagnosticsSource {
    Import,
    Editor,
    Combined,
}

#[derive(Debug, Args)]
pub(crate) struct ProfileArgs {
    #[command(subcommand)]
    pub(crate) command: ProfileCommands,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ProfileCommands {
    Editor(EditorProfileArgs),
}

#[derive(Debug, Args)]
pub(crate) struct EditorProfileArgs {
    #[command(subcommand)]
    pub(crate) command: EditorProfileCommands,
}

#[derive(Debug, Subcommand)]
pub(crate) enum EditorProfileCommands {
    Export(EditorProfileExportArgs),
}

#[derive(Debug, Args)]
pub(crate) struct EditorProfileExportArgs {}

#[derive(Debug, Args)]
pub(crate) struct QaStateArgs {
    #[command(subcommand)]
    pub(crate) command: QaStateCommands,
}

#[derive(Debug, Subcommand)]
pub(crate) enum QaStateCommands {
    Validate(QaStateValidateArgs),
}

#[derive(Debug, Args)]
pub(crate) struct QaStateValidateArgs {
    #[arg(long)]
    pub(crate) schema: PathBuf,
    #[arg(long, conflicts_with_all = ["trace", "state_path", "target", "method", "provider"])]
    pub(crate) input: Option<PathBuf>,
    #[arg(long, requires = "state_path", conflicts_with_all = ["input", "target", "method", "provider"])]
    pub(crate) trace: Option<PathBuf>,
    #[arg(long = "state-path", requires = "trace", conflicts_with_all = ["input", "target", "method", "provider"])]
    pub(crate) state_path: Option<String>,
    #[arg(long, requires = "method", conflicts_with_all = ["input", "trace", "state_path", "provider"])]
    pub(crate) target: Option<String>,
    #[arg(long, requires = "target", conflicts_with_all = ["input", "trace", "state_path", "provider"])]
    pub(crate) method: Option<String>,
    #[arg(long, conflicts_with_all = ["input", "trace", "state_path", "target", "method"])]
    pub(crate) provider: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct ScenarioArgs {
    #[command(subcommand)]
    pub(crate) command: ScenarioCommands,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ScenarioCommands {
    List,
    Bundle(ScenarioBundleArgs),
    Lint(ScenarioLintArgs),
    Run(ScenarioRunArgs),
    Sweep(ScenarioSweepArgs),
    Profile(ScenarioProfileArgs),
    Init(ScenarioInitArgs),
    Record(ScenarioRecordArgs),
}

#[derive(Debug, Args)]
pub(crate) struct ScenarioBundleArgs {
    #[command(subcommand)]
    pub(crate) command: ScenarioBundleCommands,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ScenarioBundleCommands {
    List,
    Run(ScenarioBundleRunArgs),
}

#[derive(Debug, Args)]
pub(crate) struct ScenarioBundleRunArgs {
    /// One or more bundles to run. Naming several runs them under one envelope and,
    /// with --warm, over ONE shared warm session pool.
    #[arg(required = true, num_args = 1..)]
    pub(crate) names: Vec<String>,
    /// Bundle parallelism. Omitted -> qa/config.yaml scenario.jobs -> 0
    /// (available parallelism).
    #[arg(long, allow_hyphen_values = true)]
    pub(crate) jobs: Option<i64>,
    #[arg(long, action = ArgAction::SetTrue)]
    pub(crate) strict: bool,
    #[arg(long, conflicts_with = "windowed", action = ArgAction::SetTrue)]
    pub(crate) headless: bool,
    #[arg(long, action = ArgAction::SetTrue)]
    pub(crate) windowed: bool,
    /// Launch Godot with the real audio driver (sound plays / is captured).
    #[arg(long, conflicts_with = "no_audio", action = ArgAction::SetTrue)]
    pub(crate) audio: bool,
    /// Launch Godot with `--audio-driver Dummy` (no sound out loud); the default
    /// unless the scenario captures audio.
    #[arg(long = "no-audio", action = ArgAction::SetTrue)]
    pub(crate) no_audio: bool,
    /// Maximum attempts for retryable infrastructure startup failures before assertions begin.
    #[arg(
        long = "retry-startup",
        alias = "retry-infrastructure-startup",
        default_value_t = 1
    )]
    pub(crate) retry_infrastructure_startup: usize,
    /// Delay between retryable infrastructure startup attempts.
    #[arg(
        long = "retry-startup-backoff-ms",
        alias = "retry-backoff-ms",
        default_value_t = 0
    )]
    pub(crate) retry_backoff_ms: u64,
    /// Reuse ONE warm Godot process across scenarios (fast dev loop). Scenarios
    /// needing process isolation fall back to the cold path automatically.
    #[arg(long = "warm", alias = "reuse-process", action = ArgAction::SetTrue)]
    pub(crate) warm: bool,
    /// With --warm, also run every scenario cold and assert the warm/cold
    /// logical verdicts match (cold stays the authoritative gate).
    #[arg(long = "equivalence-check", action = ArgAction::SetTrue)]
    pub(crate) equivalence_check: bool,
    /// With --warm, fail the bundle when a scenario silently falls back to
    /// cold for a reason it did not declare via `execution.isolation: cold`.
    #[arg(long = "warm-strict", action = ArgAction::SetTrue)]
    pub(crate) warm_strict: bool,
    /// Force cold execution for this run, overriding a `scenario.warm: true`
    /// project config default.
    #[arg(long = "cold", conflicts_with = "warm", action = ArgAction::SetTrue)]
    pub(crate) cold: bool,
    /// Disable --warm-strict for this run, overriding a
    /// `scenario.warm_strict: true` project config default.
    #[arg(long = "no-warm-strict", conflicts_with = "warm_strict", action = ArgAction::SetTrue)]
    pub(crate) no_warm_strict: bool,
    /// Run only this scenario of the bundle (repeatable). An unknown name fails the
    /// invocation rather than quietly running nothing.
    #[arg(long = "only", value_name = "SCENARIO", action = ArgAction::Append)]
    pub(crate) only: Vec<String>,
    /// Run only the scenarios that did not pass in this bundle's most recent run.
    #[arg(long = "failed-only", action = ArgAction::SetTrue)]
    pub(crate) failed_only: bool,
    /// Dispatch order for parallel runs. Omitted -> qa/config.yaml
    /// scenario.schedule -> longest-first.
    #[arg(long = "schedule", value_enum)]
    pub(crate) schedule: Option<BundleScheduleArg>,
}

/// `--schedule` values. Mirrors `godot_qa_core::config::ScenarioSchedule` so the flag and
/// the `scenario.schedule` project default name the same two policies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum BundleScheduleArg {
    Manifest,
    LongestFirst,
}

#[derive(Debug, Args)]
pub(crate) struct ScenarioLintArgs {
    #[arg(long)]
    pub(crate) scenario: Option<String>,
    #[arg(long, action = ArgAction::SetTrue)]
    pub(crate) strict: bool,
}

#[derive(Debug, Args)]
pub(crate) struct ScenarioRunArgs {
    #[arg(required_unless_present = "file", conflicts_with = "file")]
    pub(crate) name: Option<String>,
    #[arg(long)]
    pub(crate) file: Option<String>,
    /// Execution target. Desktop launches a local Godot process; android deploys a
    /// one-shot scenario-mode session to a device.
    #[arg(long, value_enum, default_value_t = SessionTargetArg::Desktop)]
    pub(crate) target: SessionTargetArg,
    /// Android export preset. Required when `--target android`.
    #[arg(long)]
    pub(crate) preset: Option<String>,
    /// ADB device serial. May be omitted when exactly one authorized device is present.
    #[arg(long)]
    pub(crate) device: Option<String>,
    /// Android application id override for custom Gradle configurations.
    #[arg(long = "package")]
    pub(crate) package_id: Option<String>,
    /// Android only: skip debug-APK cache reuse and force a fresh export.
    #[arg(long = "no-apk-cache", action = ArgAction::SetTrue)]
    pub(crate) no_apk_cache: bool,
    #[arg(long, conflicts_with = "windowed", action = ArgAction::SetTrue)]
    pub(crate) headless: bool,
    #[arg(long, action = ArgAction::SetTrue)]
    pub(crate) windowed: bool,
    /// Launch Godot with the real audio driver (sound plays / is captured).
    #[arg(long, conflicts_with = "no_audio", action = ArgAction::SetTrue)]
    pub(crate) audio: bool,
    /// Launch Godot with `--audio-driver Dummy` (no sound out loud); the default
    /// unless the scenario captures audio.
    #[arg(long = "no-audio", action = ArgAction::SetTrue)]
    pub(crate) no_audio: bool,
    #[arg(long = "visual-risk", action = ArgAction::SetTrue)]
    pub(crate) visual_risk: bool,
    #[arg(long = "visual-risk-strict", action = ArgAction::SetTrue)]
    pub(crate) visual_risk_strict: bool,
}

#[derive(Debug, Args)]
pub(crate) struct ScenarioSweepArgs {
    #[arg(required_unless_present = "file", conflicts_with = "file")]
    pub(crate) name: Option<String>,
    #[arg(long)]
    pub(crate) file: Option<String>,
    /// Explicit comma-separated seed list, e.g. --seeds 1,2,3.
    #[arg(
        long = "seeds",
        value_delimiter = ',',
        allow_hyphen_values = true,
        conflicts_with_all = ["seed_start", "seed_count"]
    )]
    pub(crate) seeds: Vec<i64>,
    /// First seed of a contiguous range (pairs with --seed-count).
    #[arg(
        long = "seed-start",
        requires = "seed_count",
        allow_hyphen_values = true
    )]
    pub(crate) seed_start: Option<i64>,
    /// Number of seeds in the contiguous range (pairs with --seed-start).
    #[arg(long = "seed-count", requires = "seed_start")]
    pub(crate) seed_count: Option<u32>,
    /// Run every selected seed K times back to back (same seed repeated).
    #[arg(long, default_value_t = 1)]
    pub(crate) repeat: u32,
    /// Exit non-zero when any seed run fails (default: only infrastructure
    /// errors fail the sweep; the aggregate collects the distribution).
    #[arg(long = "require-all-pass", action = ArgAction::SetTrue)]
    pub(crate) require_all_pass: bool,
    #[arg(long, conflicts_with = "windowed", action = ArgAction::SetTrue)]
    pub(crate) headless: bool,
    #[arg(long, action = ArgAction::SetTrue)]
    pub(crate) windowed: bool,
    /// Launch Godot with the real audio driver (sound plays / is captured).
    #[arg(long, conflicts_with = "no_audio", action = ArgAction::SetTrue)]
    pub(crate) audio: bool,
    /// Launch Godot with `--audio-driver Dummy` (no sound out loud); the default
    /// unless the scenario captures audio.
    #[arg(long = "no-audio", action = ArgAction::SetTrue)]
    pub(crate) no_audio: bool,
    /// Extra project arg appended to every seed run (repeatable).
    #[arg(long = "project-arg", action = ArgAction::Append, allow_hyphen_values = true)]
    pub(crate) project_args: Vec<String>,
}

#[derive(Debug, Args)]
pub(crate) struct ScenarioProfileArgs {
    pub(crate) name: String,
    #[arg(long)]
    pub(crate) soak: Option<String>,
    #[arg(long, conflicts_with = "windowed", action = ArgAction::SetTrue)]
    pub(crate) headless: bool,
    #[arg(long, action = ArgAction::SetTrue)]
    pub(crate) windowed: bool,
    /// Launch Godot with the real audio driver (sound plays / is captured).
    #[arg(long, conflicts_with = "no_audio", action = ArgAction::SetTrue)]
    pub(crate) audio: bool,
    /// Launch Godot with `--audio-driver Dummy` (no sound out loud); the default
    /// unless the scenario captures audio.
    #[arg(long = "no-audio", action = ArgAction::SetTrue)]
    pub(crate) no_audio: bool,
}

#[derive(Debug, Args)]
pub(crate) struct ScenarioInitArgs {
    pub(crate) name: String,
    #[arg(long)]
    pub(crate) scene: String,
    #[arg(long, default_value = "1920x1080")]
    pub(crate) viewport: String,
    #[arg(long)]
    pub(crate) out: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct ScenarioRecordArgs {
    #[arg(long)]
    pub(crate) name: Option<String>,
    #[arg(value_name = "NAME")]
    pub(crate) positional_name: Option<String>,
    #[arg(long)]
    pub(crate) scene: String,
    #[arg(long, default_value = "1920x1080")]
    pub(crate) viewport: String,
    #[arg(long)]
    pub(crate) out: Option<String>,
    #[arg(long = "input-script")]
    pub(crate) input_script: Option<String>,
    #[arg(long = "duration-frames")]
    pub(crate) duration_frames: Option<i64>,
    #[arg(long, action = ArgAction::SetTrue)]
    pub(crate) headless: bool,
    /// Analog quantization bucket for recorded joypad motion (axis values are
    /// re-recorded when they cross a bucket boundary).
    #[arg(long = "joy-motion-step", default_value_t = 0.25)]
    pub(crate) joy_motion_step: f64,
    /// Deadzone below which recorded joypad motion snaps to 0.0.
    #[arg(long = "joy-motion-deadzone", default_value_t = 0.1)]
    pub(crate) joy_motion_deadzone: f64,
    /// Minimum frames between recorded motion values for one axis when the
    /// value drifts without crossing a bucket boundary.
    #[arg(long = "joy-motion-frame-gap", default_value_t = 6)]
    pub(crate) joy_motion_frame_gap: i64,
}

#[derive(Debug, Args)]
pub(crate) struct ReviewArgs {
    #[command(subcommand)]
    pub(crate) command: ReviewCommands,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ReviewCommands {
    #[command(name = "list-templates")]
    ListTemplates,
    Init(ReviewInitArgs),
}

#[derive(Debug, Args)]
pub(crate) struct ReviewInitArgs {
    pub(crate) template: String,
    #[arg(long)]
    pub(crate) name: String,
    #[arg(long)]
    pub(crate) scene: String,
    #[arg(long = "target", action = ArgAction::Append)]
    pub(crate) targets: Vec<String>,
    #[arg(long = "project-arg", action = ArgAction::Append, allow_hyphen_values = true)]
    pub(crate) project_args: Vec<String>,
    #[arg(long, default_value_t = scenario_review::DEFAULT_FRAMES)]
    pub(crate) frames: u32,
    #[arg(long = "screenshot-every", default_value_t = scenario_review::DEFAULT_SCREENSHOT_EVERY)]
    pub(crate) screenshot_every: u32,
    #[arg(long = "audio-seconds", default_value_t = scenario_review::DEFAULT_AUDIO_SECONDS)]
    pub(crate) audio_seconds: u32,
    #[arg(long = "trace-target")]
    pub(crate) trace_target: Option<String>,
    #[arg(long = "event-provider")]
    pub(crate) event_provider: Option<String>,
    #[arg(long)]
    pub(crate) out: PathBuf,
}

#[derive(Debug, Args)]
pub(crate) struct LiveGateArgs {
    #[command(subcommand)]
    pub(crate) command: LiveGateCommands,
}

#[derive(Debug, Subcommand)]
pub(crate) enum LiveGateCommands {
    List,
    Run(LiveGateRunArgs),
}

#[derive(Debug, Args)]
pub(crate) struct LiveGateRunArgs {
    pub(crate) name: String,
    #[arg(long, action = ArgAction::SetTrue)]
    pub(crate) strict: bool,
    #[arg(long, action = ArgAction::SetTrue)]
    pub(crate) attach: bool,
    #[arg(long)]
    pub(crate) scene: Option<String>,
    #[arg(long = "show-overlay", conflicts_with = "hide_overlay", action = ArgAction::SetTrue)]
    pub(crate) show_overlay: bool,
    #[arg(long = "hide-overlay", action = ArgAction::SetTrue)]
    pub(crate) hide_overlay: bool,
    #[arg(long, default_value = "1920x1080")]
    pub(crate) viewport: String,
    #[arg(long, action = ArgAction::SetTrue)]
    pub(crate) headless: bool,
}

impl ScenarioRecordArgs {
    pub(crate) fn resolved_name(&self) -> Result<&str, &'static str> {
        match (&self.name, &self.positional_name) {
            (Some(_), Some(_)) => Err("pass either --name or positional NAME, not both"),
            (Some(name), None) => Ok(name.as_str()),
            (None, Some(name)) => Ok(name.as_str()),
            (None, None) => Err("missing scenario name (--name or positional NAME)"),
        }
    }
}

#[derive(Debug, Args)]
pub(crate) struct ArtifactArgs {
    #[command(subcommand)]
    pub(crate) command: ArtifactCommands,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ArtifactCommands {
    Query(ArtifactQueryArgs),
    Summarize(ArtifactSummarizeArgs),
    Prune(ArtifactPruneArgs),
    BeginRun(ArtifactBeginRunArgs),
    ValidateResult(ArtifactValidateResultArgs),
    Export(ArtifactExportArgs),
}

#[derive(Debug, Args)]
pub(crate) struct ArtifactExportArgs {
    /// JSON selection manifest with explicit source files.
    pub(crate) selection: String,
    /// New portable evidence directory. Existing paths are never overwritten.
    #[arg(long)]
    pub(crate) out: String,
}

#[derive(Debug, Args)]
#[command(group(clap::ArgGroup::new("destination").required(true).args(["out", "run_root"])))]
pub(crate) struct ArtifactBeginRunArgs {
    /// New directory for a custom child run. It must not already exist.
    #[arg(long, value_name = "NEW_DIRECTORY")]
    pub(crate) out: Option<String>,
    /// Positional compatibility alias for --out.
    #[arg(value_name = "RUN_ROOT")]
    pub(crate) run_root: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct ArtifactValidateResultArgs {
    pub(crate) run_root: String,
    /// Exit code returned by the outer display wrapper.
    #[arg(long = "wrapper-exit", allow_hyphen_values = true)]
    pub(crate) wrapper_exit: i32,
    /// The orchestrator terminated the wrapper on its deadline.
    #[arg(long = "wrapper-timed-out", action = ArgAction::SetTrue)]
    pub(crate) wrapper_timed_out: bool,
}

#[derive(Debug, Args)]
pub(crate) struct ArtifactPruneArgs {
    /// Run directories to keep per scenario and per bundle. Overrides `artifacts.keep_runs`.
    #[arg(long = "keep-runs")]
    pub(crate) keep_runs: Option<usize>,
    /// Report what would be removed without deleting anything.
    #[arg(long = "dry-run", action = ArgAction::SetTrue)]
    pub(crate) dry_run: bool,
}

#[derive(Debug, Args)]
pub(crate) struct ArtifactQueryArgs {
    pub(crate) run_root: String,
    #[arg(long)]
    pub(crate) snapshot: String,
    #[arg(long, conflicts_with = "qa_id")]
    pub(crate) target: Option<String>,
    #[arg(long = "qa-id", conflicts_with = "target")]
    pub(crate) qa_id: Option<String>,
    #[arg(long = "type")]
    pub(crate) node_type: Option<String>,
    #[arg(long, action = ArgAction::SetTrue)]
    pub(crate) visible: bool,
}

#[derive(Debug, Args)]
pub(crate) struct ArtifactSummarizeArgs {
    pub(crate) run_root: String,
    #[arg(long = "type", action = ArgAction::Append)]
    pub(crate) artifact_types: Vec<String>,
    #[arg(long = "warnings-only", action = ArgAction::SetTrue)]
    pub(crate) warnings_only: bool,
    #[arg(long = "expect", action = ArgAction::Append)]
    pub(crate) expected_types: Vec<String>,
    #[arg(long = "markdown-out")]
    pub(crate) markdown_out: Option<String>,
    #[arg(long = "no-write", action = ArgAction::SetTrue)]
    pub(crate) no_write: bool,
}

#[derive(Debug, Args)]
pub(crate) struct SessionArgs {
    #[command(subcommand)]
    pub(crate) command: SessionCommands,
}

#[derive(Debug, Subcommand)]
pub(crate) enum SessionCommands {
    Start(SessionStartArgs),
    Status,
    List,
    #[command(name = "run-script")]
    RunScript(SessionRunScriptArgs),
    Wait(SessionFrameArgs),
    #[command(name = "time-scale")]
    TimeScale(SessionTimeScaleArgs),
    Reload(SessionReloadArgs),
    Watch(SessionWatchArgs),
    Stop,
    Pause,
    Resume,
    Step(SessionFrameArgs),
    Profile(SessionProfileArgs),
}

#[derive(Debug, Args)]
pub(crate) struct SessionStartArgs {
    /// Runtime target. Desktop preserves the existing local-process behavior.
    #[arg(long, value_enum, default_value_t = SessionTargetArg::Desktop)]
    pub(crate) target: SessionTargetArg,
    /// Android export preset. Required when `--target android`.
    #[arg(long)]
    pub(crate) preset: Option<String>,
    /// ADB device serial. May be omitted when exactly one authorized device is present.
    #[arg(long)]
    pub(crate) device: Option<String>,
    /// Android application id override for custom Gradle configurations.
    #[arg(long = "package")]
    pub(crate) package_id: Option<String>,
    #[arg(long)]
    pub(crate) scene: Option<String>,
    #[arg(long)]
    pub(crate) viewport: Option<String>,
    #[arg(long, action = ArgAction::SetTrue)]
    pub(crate) headless: bool,
    /// Launch Godot with the real audio driver (sound plays / is captured).
    #[arg(long, conflicts_with = "no_audio", action = ArgAction::SetTrue)]
    pub(crate) audio: bool,
    /// Launch Godot with `--audio-driver Dummy` (no sound out loud). By default a
    /// windowed session keeps real audio and a headless session is silent.
    #[arg(long = "no-audio", action = ArgAction::SetTrue)]
    pub(crate) no_audio: bool,
    #[arg(long, action = ArgAction::SetTrue)]
    pub(crate) fastest: bool,
    #[arg(long = "fixed-fps")]
    pub(crate) fixed_fps: Option<i64>,
    #[arg(long = "project-arg", action = ArgAction::Append, allow_hyphen_values = true)]
    pub(crate) project_args: Vec<String>,
    /// Enable engine-faithful hot reload: launch a debug-host sidecar and start Godot
    /// with `--remote-debug` so `session reload --script/--resource` and `session watch`
    /// can soft-reload live scripts/resources without restarting the session.
    #[arg(long = "hot-reload", action = ArgAction::SetTrue)]
    pub(crate) hot_reload: bool,
    /// Android only: skip debug-APK cache reuse and force a fresh export. The fresh APK
    /// still refreshes the cache entry.
    #[arg(long = "no-apk-cache", action = ArgAction::SetTrue)]
    pub(crate) no_apk_cache: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum SessionTargetArg {
    Desktop,
    Android,
}

#[derive(Debug, Args)]
pub(crate) struct SessionReloadArgs {
    /// Soft-reload a specific script (repeatable). Preserves live instance state.
    /// Requires a session started with `--hot-reload`.
    #[arg(long = "script", action = ArgAction::Append)]
    pub(crate) scripts: Vec<String>,
    /// Live-reload a specific resource (repeatable). Requires `--hot-reload`.
    /// Best-effort and unconfirmed: needs a debug or editor build; release-template
    /// exports silently ignore it.
    #[arg(long = "resource", action = ArgAction::Append)]
    pub(crate) resources: Vec<String>,
    /// Faithful structural live-edit of a `.tscn` scene (repeatable): diff it against the
    /// baseline and apply node add/remove/reparent + property changes without restarting.
    /// Requires `--hot-reload`; edits it can't express fall back to a full-scene reload.
    #[arg(long = "scene", action = ArgAction::Append)]
    pub(crate) scenes: Vec<String>,
}

#[derive(Debug, Args)]
pub(crate) struct SessionRunScriptArgs {
    pub(crate) path: String,
}

#[derive(Debug, Args)]
pub(crate) struct SessionFrameArgs {
    #[arg(long, default_value_t = 1)]
    pub(crate) frames: i64,
}

#[derive(Debug, Args)]
pub(crate) struct SessionTimeScaleArgs {
    pub(crate) time_scale: f64,
    #[arg(long = "physics-ticks-per-second")]
    pub(crate) physics_ticks_per_second: Option<i64>,
    #[arg(long = "max-physics-steps-per-frame")]
    pub(crate) max_physics_steps_per_frame: Option<i64>,
}

#[derive(Debug, Args)]
pub(crate) struct SessionProfileArgs {
    #[arg(long, conflicts_with_all = ["soak_status", "soak_stop", "frames", "seconds"])]
    pub(crate) soak: Option<String>,
    #[arg(long = "soak-status", conflicts_with_all = ["soak_stop", "frames", "seconds"])]
    pub(crate) soak_status: Option<String>,
    #[arg(long = "soak-stop", conflicts_with_all = ["frames", "seconds"])]
    pub(crate) soak_stop: Option<String>,
    #[arg(long, requires = "soak", action = ArgAction::SetTrue)]
    pub(crate) detach: bool,
    #[arg(long, conflicts_with = "seconds")]
    pub(crate) frames: Option<i64>,
    #[arg(long)]
    pub(crate) seconds: Option<f64>,
    #[arg(long = "max-frame-samples")]
    pub(crate) max_frame_samples: Option<i64>,
    #[arg(long, default_value_t = 10)]
    pub(crate) top: i64,
    #[arg(long = "setup-script")]
    pub(crate) setup_script: Option<String>,
    #[arg(long = "summary-json", action = ArgAction::SetTrue)]
    pub(crate) summary_json: bool,
}

#[derive(Debug, Args)]
pub(crate) struct SessionWatchArgs {
    #[arg(long = "debounce-ms", default_value_t = 300)]
    pub(crate) debounce_ms: u64,
}

#[derive(Debug, Args)]
pub(crate) struct InputArgs {
    #[command(subcommand)]
    pub(crate) command: InputCommands,
}

#[derive(Debug, Subcommand)]
pub(crate) enum InputCommands {
    Tap(InputActionArgs),
    Hold(InputHoldArgs),
    Release(InputActionArgs),
    Click(InputTargetArgs),
    Mouse(InputMouseArgs),
    Touch(InputTouchArgs),
    Drag(InputDragArgs),
    Key(InputKeyArgs),
    Type(InputTextArgs),
    Joy(InputJoyArgs),
}

#[derive(Debug, Args)]
pub(crate) struct InputActionArgs {
    pub(crate) action: String,
}

#[derive(Debug, Args)]
pub(crate) struct InputHoldArgs {
    pub(crate) action: String,
    #[arg(long, default_value_t = 1)]
    pub(crate) frames: i64,
}

#[derive(Debug, Args)]
pub(crate) struct InputTargetArgs {
    pub(crate) target: String,
}

#[derive(Debug, Args)]
pub(crate) struct InputMouseArgs {
    #[command(subcommand)]
    pub(crate) command: InputMouseCommands,
}

#[derive(Debug, Subcommand)]
pub(crate) enum InputMouseCommands {
    Click(InputMouseClickArgs),
    Wheel(InputMouseWheelArgs),
}

#[derive(Debug, Args)]
pub(crate) struct InputMouseClickArgs {
    #[arg(long)]
    pub(crate) x: i64,
    #[arg(long)]
    pub(crate) y: i64,
}

#[derive(Debug, Args)]
pub(crate) struct InputMouseWheelArgs {
    #[arg(long)]
    pub(crate) x: i64,
    #[arg(long)]
    pub(crate) y: i64,
    #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
    pub(crate) dy: f64,
    #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
    pub(crate) dx: f64,
}

#[derive(Debug, Args)]
pub(crate) struct InputTouchArgs {
    #[command(subcommand)]
    pub(crate) command: InputTouchCommands,
}

#[derive(Debug, Subcommand)]
pub(crate) enum InputTouchCommands {
    Down(InputTouchDownArgs),
    Move(InputTouchMoveArgs),
    Up(InputTouchUpArgs),
    Tap(InputTouchTapArgs),
}

#[derive(Debug, Args)]
pub(crate) struct InputTouchDownArgs {
    #[arg(long)]
    pub(crate) x: i64,
    #[arg(long)]
    pub(crate) y: i64,
    #[arg(long, default_value_t = 0)]
    pub(crate) index: i64,
}

#[derive(Debug, Args)]
pub(crate) struct InputTouchMoveArgs {
    #[arg(long)]
    pub(crate) x: i64,
    #[arg(long)]
    pub(crate) y: i64,
    #[arg(long, default_value_t = 0)]
    pub(crate) index: i64,
    #[arg(long, default_value_t = 1)]
    pub(crate) frames: i64,
}

#[derive(Debug, Args)]
pub(crate) struct InputTouchUpArgs {
    #[arg(long)]
    pub(crate) x: Option<i64>,
    #[arg(long)]
    pub(crate) y: Option<i64>,
    #[arg(long, default_value_t = 0)]
    pub(crate) index: i64,
}

#[derive(Debug, Args)]
pub(crate) struct InputTouchTapArgs {
    #[arg(long)]
    pub(crate) x: i64,
    #[arg(long)]
    pub(crate) y: i64,
    #[arg(long, default_value_t = 0)]
    pub(crate) index: i64,
}

/// Pointer device backing an `input drag` gesture. Serialized to the lowercase
/// protocol strings `"touch"`/`"mouse"` (see `runtime_router`).
#[derive(Clone, Debug, ValueEnum)]
pub(crate) enum PointerKind {
    Touch,
    Mouse,
}

impl std::fmt::Display for PointerKind {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        self.to_possible_value()
            .expect("no PointerKind values are skipped")
            .get_name()
            .fmt(f)
    }
}

#[derive(Debug, Args)]
pub(crate) struct InputDragArgs {
    #[arg(long)]
    pub(crate) from_x: i64,
    #[arg(long)]
    pub(crate) from_y: i64,
    #[arg(long)]
    pub(crate) to_x: i64,
    #[arg(long)]
    pub(crate) to_y: i64,
    #[arg(long, default_value_t = 8)]
    pub(crate) frames: i64,
    #[arg(long, value_enum, default_value_t = PointerKind::Touch)]
    pub(crate) pointer: PointerKind,
    #[arg(long, default_value_t = 0)]
    pub(crate) index: i64,
}

#[derive(Debug, Args)]
pub(crate) struct InputKeyArgs {
    pub(crate) key: String,
}

/// Freeform text entry: the addon injects a per-character `InputEventKey`
/// press/release for each codepoint of `text` with the matching `unicode` so
/// focused text controls receive the characters. Leading hyphens are allowed so
/// text such as `-3` or `--flag` is treated as the value, not CLI flags.
#[derive(Debug, Args)]
pub(crate) struct InputTextArgs {
    #[arg(allow_hyphen_values = true)]
    pub(crate) text: String,
}

#[derive(Debug, Args)]
pub(crate) struct InputJoyArgs {
    #[command(subcommand)]
    pub(crate) command: InputJoyCommands,
}

#[derive(Debug, Subcommand)]
pub(crate) enum InputJoyCommands {
    Button(InputJoyButtonArgs),
    Motion(InputJoyMotionArgs),
}

#[derive(Debug, Args)]
pub(crate) struct InputJoyButtonArgs {
    pub(crate) button: String,
    #[arg(long, default_value_t = 0)]
    pub(crate) device: i64,
    #[arg(long, default_value_t = 1)]
    pub(crate) frames: i64,
}

#[derive(Debug, Args)]
pub(crate) struct InputJoyMotionArgs {
    pub(crate) axis: String,
    pub(crate) value: f64,
    #[arg(long, default_value_t = 0)]
    pub(crate) device: i64,
    #[arg(long, default_value_t = 1)]
    pub(crate) frames: i64,
}

#[derive(Debug, Args)]
pub(crate) struct SnapshotArgs {
    #[arg(long, default_value = "current")]
    pub(crate) name: String,
    #[command(flatten)]
    pub(crate) filters: SnapshotFilterArgs,
}

#[derive(Debug, Args)]
pub(crate) struct TreeArgs {
    #[command(flatten)]
    pub(crate) filters: SnapshotFilterArgs,
}

#[derive(Debug, Args, Default)]
pub(crate) struct SnapshotFilterArgs {
    #[arg(long)]
    pub(crate) target: Option<String>,
    #[arg(long = "max-depth")]
    pub(crate) max_depth: Option<i64>,
    #[arg(long = "qa-only", action = ArgAction::SetTrue)]
    pub(crate) qa_only: bool,
    #[arg(long = "include-metadata", action = ArgAction::SetTrue)]
    pub(crate) include_metadata: bool,
    #[arg(long = "metadata-key", value_delimiter = ',')]
    pub(crate) metadata_key: Vec<String>,
    #[arg(long = "metadata-keys", value_delimiter = ',')]
    pub(crate) metadata_keys: Vec<String>,
}

#[derive(Debug, Args)]
pub(crate) struct FocusArgs {
    #[command(subcommand)]
    pub(crate) command: Option<FocusCommands>,
}

#[derive(Debug, Subcommand)]
pub(crate) enum FocusCommands {
    Graph,
}

#[derive(Debug, Args)]
pub(crate) struct NodeArgs {
    #[command(subcommand)]
    pub(crate) command: NodeCommands,
}

#[derive(Debug, Subcommand)]
pub(crate) enum NodeCommands {
    Inspect(NodeInspectArgs),
}

#[derive(Debug, Args)]
pub(crate) struct NodeInspectArgs {
    pub(crate) target: String,
}

#[derive(Debug, Args)]
pub(crate) struct PhysicsArgs {
    #[command(subcommand)]
    pub(crate) command: PhysicsCommands,
}

#[derive(Debug, Subcommand)]
pub(crate) enum PhysicsCommands {
    Inspect(PhysicsInspectArgs),
    Overlay(PhysicsOverlayArgs),
    #[command(name = "collision-report")]
    CollisionReport(PhysicsCollisionReportArgs),
}

#[derive(Debug, Args)]
pub(crate) struct PhysicsInspectArgs {
    pub(crate) target: String,
    #[arg(long, default_value_t = 256.0)]
    pub(crate) radius: f64,
    #[arg(long, default_value_t = 1)]
    pub(crate) frames: i64,
    #[arg(long = "state-provider", default_value = "scene-root")]
    pub(crate) state_provider: String,
    #[arg(long = "state-method", action = ArgAction::Append)]
    pub(crate) state_method: Vec<String>,
    #[arg(long = "no-project-state", action = ArgAction::SetTrue)]
    pub(crate) no_project_state: bool,
    #[arg(long)]
    pub(crate) snapshot: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct PhysicsOverlayArgs {
    pub(crate) target: Option<String>,
    #[arg(long, action = ArgAction::SetTrue)]
    pub(crate) viewport: bool,
    #[arg(long)]
    pub(crate) scenario: Option<String>,
    #[arg(long)]
    pub(crate) snapshot: Option<String>,
    #[arg(long, default_value_t = 256.0)]
    pub(crate) radius: f64,
    #[arg(long, default_value_t = 1)]
    pub(crate) frames: i64,
    #[arg(long)]
    pub(crate) out: PathBuf,
    #[arg(long = "include-aabb", action = ArgAction::SetTrue)]
    pub(crate) include_aabb: bool,
    #[arg(long = "include-contacts", action = ArgAction::SetTrue)]
    pub(crate) include_contacts: bool,
    #[arg(long = "state-provider", default_value = "scene-root")]
    pub(crate) state_provider: String,
    #[arg(long = "state-method", action = ArgAction::Append)]
    pub(crate) state_method: Vec<String>,
    #[arg(long = "no-project-state", action = ArgAction::SetTrue)]
    pub(crate) no_project_state: bool,
    #[arg(long = "max-shapes", default_value_t = 512)]
    pub(crate) max_shapes: i64,
    #[arg(long, action = ArgAction::SetTrue)]
    pub(crate) strict: bool,
}

#[derive(Debug, Args)]
pub(crate) struct PhysicsCollisionReportArgs {
    pub(crate) target: Option<String>,
    #[arg(long = "from-trace")]
    pub(crate) from_trace: Option<PathBuf>,
    #[arg(long = "setup-script")]
    pub(crate) setup_script: Option<String>,
    #[arg(long = "summary-json", action = ArgAction::SetTrue)]
    pub(crate) summary_json: bool,
    #[arg(long, default_value_t = 600)]
    pub(crate) frames: i64,
    #[arg(long = "sample-every-frames", default_value_t = 1)]
    pub(crate) sample_every_frames: i64,
    #[arg(long, action = ArgAction::SetTrue)]
    pub(crate) physics: bool,
    #[arg(long = "state-provider", default_value = "scene-root")]
    pub(crate) state_provider: String,
    #[arg(long = "state-method", action = ArgAction::Append)]
    pub(crate) state_method: Vec<String>,
    #[arg(long = "max-normal-retention")]
    pub(crate) max_normal_retention: Option<f64>,
    #[arg(long = "max-velocity-reversals")]
    pub(crate) max_velocity_reversals: Option<i64>,
    #[arg(long = "max-same-contact-frames")]
    pub(crate) max_same_contact_frames: Option<i64>,
    #[arg(long = "max-angular-velocity-spike")]
    pub(crate) max_angular_velocity_spike: Option<f64>,
    #[arg(long = "max-overlap-depth")]
    pub(crate) max_overlap_depth: Option<f64>,
    #[arg(long = "max-low-progress-contact-frames")]
    pub(crate) max_low_progress_contact_frames: Option<i64>,
    #[arg(long = "strict-warnings", action = ArgAction::SetTrue)]
    pub(crate) strict_warnings: bool,
}

#[derive(Debug, Args)]
pub(crate) struct SurfaceArgs {
    #[command(subcommand)]
    pub(crate) command: SurfaceCommands,
}

#[derive(Debug, Subcommand)]
pub(crate) enum SurfaceCommands {
    Audit(SurfaceAuditArgs),
}

#[derive(Debug, Args)]
pub(crate) struct SurfaceAuditArgs {
    pub(crate) provider: Option<String>,
    #[arg(long = "provider-id")]
    pub(crate) provider_id: Option<String>,
    #[arg(long = "setup-script")]
    pub(crate) setup_script: Option<String>,
    #[arg(long)]
    pub(crate) center: Option<String>,
    #[arg(long = "region-size")]
    pub(crate) region_size: Option<String>,
    #[arg(long)]
    pub(crate) route: Option<String>,
    #[arg(long = "left-region")]
    pub(crate) left_region: Option<String>,
    #[arg(long = "right-region")]
    pub(crate) right_region: Option<String>,
    #[arg(long = "mirror-axis")]
    pub(crate) mirror_axis: Option<String>,
    #[arg(long = "sample-step")]
    pub(crate) sample_step: f64,
    #[arg(long = "actor-radius")]
    pub(crate) actor_radius: f64,
    #[arg(long = "min-spawn-clearance")]
    pub(crate) min_spawn_clearance: Option<f64>,
    #[arg(long = "min-corridor-width")]
    pub(crate) min_corridor_width: Option<f64>,
    #[arg(long)]
    pub(crate) out: Option<PathBuf>,
    #[arg(long = "heatmap-out")]
    pub(crate) heatmap_out: Option<PathBuf>,
    #[arg(long = "provider-method", default_value = "qa_surface_probe")]
    pub(crate) provider_method: String,
    #[arg(long = "strict-warnings", action = ArgAction::SetTrue)]
    pub(crate) strict_warnings: bool,
}

#[derive(Debug, Args)]
pub(crate) struct TraceArgs {
    #[command(subcommand)]
    pub(crate) command: TraceCommands,
}

#[derive(Debug, Subcommand)]
pub(crate) enum TraceCommands {
    Record(TraceRecordArgs),
}

#[derive(Debug, Args)]
pub(crate) struct TraceRecordArgs {
    #[arg(long = "target", action = ArgAction::Append, required = true)]
    pub(crate) target: Vec<String>,
    #[arg(long = "setup-script")]
    pub(crate) setup_script: Option<String>,
    #[arg(long = "setup-timeout-seconds")]
    pub(crate) setup_timeout_seconds: Option<u64>,
    #[arg(long = "summary-json", action = ArgAction::SetTrue)]
    pub(crate) summary_json: bool,
    #[arg(long, default_value_t = 180)]
    pub(crate) frames: i64,
    #[arg(long = "sample-every-frames", default_value_t = 1)]
    pub(crate) sample_every_frames: i64,
    #[arg(long, action = ArgAction::SetTrue)]
    pub(crate) physics: bool,
    #[arg(long = "state-provider", default_value = "scene-root")]
    pub(crate) state_provider: String,
    #[arg(long = "state-method", action = ArgAction::Append)]
    pub(crate) state_method: Vec<String>,
    #[arg(long = "snapshot-start", action = ArgAction::SetTrue)]
    pub(crate) snapshot_start: bool,
    #[arg(long = "snapshot-end", action = ArgAction::SetTrue)]
    pub(crate) snapshot_end: bool,
    #[arg(long = "stop-after-state")]
    pub(crate) stop_after_state: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct EventArgs {
    #[command(subcommand)]
    pub(crate) command: EventCommands,
}

#[derive(Debug, Subcommand)]
pub(crate) enum EventCommands {
    Trace(EventTraceArgs),
    Summarize(EventSummarizeArgs),
}

#[derive(Debug, Args)]
pub(crate) struct EventTraceArgs {
    #[arg(long = "target", required = true)]
    pub(crate) target: String,
    #[arg(long = "provider", required = true)]
    pub(crate) provider: String,
    #[arg(long = "setup-script")]
    pub(crate) setup_script: Option<String>,
    #[arg(long = "summary-json", action = ArgAction::SetTrue)]
    pub(crate) summary_json: bool,
    #[arg(long, default_value_t = 180)]
    pub(crate) frames: i64,
    #[arg(long = "sample-every-frames", default_value_t = 1)]
    pub(crate) sample_every_frames: i64,
    #[arg(long = "pairing-config")]
    pub(crate) pairing_config: Option<PathBuf>,
    #[arg(long = "strict-warnings", action = ArgAction::SetTrue)]
    pub(crate) strict_warnings: bool,
}

#[derive(Debug, Args)]
pub(crate) struct TelemetryArgs {
    #[command(subcommand)]
    pub(crate) command: TelemetryCommands,
}

#[derive(Debug, Subcommand)]
pub(crate) enum TelemetryCommands {
    Record(TelemetryRecordArgs),
}

#[derive(Debug, Args)]
pub(crate) struct TelemetryRecordArgs {
    /// Registered read-only state provider id to sample.
    #[arg(long, conflicts_with_all = ["target", "method"])]
    pub(crate) provider: Option<String>,
    /// Target locator whose qa_ state method is sampled (pairs with --method).
    #[arg(long, requires = "method", conflicts_with = "provider")]
    pub(crate) target: Option<String>,
    /// Read-only qa_ state method sampled on --target.
    #[arg(long, requires = "target", conflicts_with = "provider")]
    pub(crate) method: Option<String>,
    #[arg(long, default_value_t = 600)]
    pub(crate) frames: i64,
    /// Sample every N frames within the window.
    #[arg(long = "every", default_value_t = 1)]
    pub(crate) every: i64,
    /// Copy of the telemetry JSONL written to this path in addition to the
    /// canonical session artifact.
    #[arg(long)]
    pub(crate) out: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub(crate) struct EventSummarizeArgs {
    pub(crate) events: PathBuf,
    #[arg(long = "pairing-config")]
    pub(crate) pairing_config: Option<PathBuf>,
    #[arg(long = "strict-warnings", action = ArgAction::SetTrue)]
    pub(crate) strict_warnings: bool,
}

#[derive(Debug, Args)]
pub(crate) struct AudioArgs {
    #[command(subcommand)]
    pub(crate) command: AudioCommands,
}

#[derive(Debug, Subcommand)]
pub(crate) enum AudioCommands {
    Capture(AudioCaptureArgs),
    Export(AudioExportArgs),
    Render(AudioRenderArgs),
    Analyze(AudioAnalyzeArgs),
}

#[derive(Debug, Args)]
pub(crate) struct AudioCaptureArgs {
    #[arg(long, action = ArgAction::SetTrue)]
    pub(crate) live: bool,
    #[arg(long = "setup-script")]
    pub(crate) setup_script: Option<String>,
    #[arg(long)]
    pub(crate) seconds: f64,
    #[arg(long = "max-seconds", default_value_t = 60.0)]
    pub(crate) max_seconds: f64,
    #[arg(long)]
    pub(crate) bus: Option<String>,
    #[arg(long, default_value = "capture")]
    pub(crate) name: String,
    #[arg(long)]
    pub(crate) out: Option<PathBuf>,
    #[arg(long, action = ArgAction::SetTrue)]
    pub(crate) analyze: bool,
    #[arg(long = "strict-warnings", action = ArgAction::SetTrue)]
    pub(crate) strict_warnings: bool,
    #[arg(long = "stop-after-state")]
    pub(crate) stop_after_state: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct AudioExportArgs {
    #[arg(long)]
    pub(crate) target: String,
    #[arg(long)]
    pub(crate) method: String,
    #[arg(long)]
    pub(crate) args: Option<String>,
    #[arg(long, default_value = "export")]
    pub(crate) name: String,
    #[arg(long)]
    pub(crate) out: Option<PathBuf>,
    #[arg(long, action = ArgAction::SetTrue)]
    pub(crate) analyze: bool,
    #[arg(long = "strict-warnings", action = ArgAction::SetTrue)]
    pub(crate) strict_warnings: bool,
}

#[derive(Debug, Args)]
pub(crate) struct AudioRenderArgs {
    /// Resolvable target node that exposes the qa_ voice-fill fixture method.
    #[arg(long)]
    pub(crate) target: String,
    /// qa_ method that fills `frame_count` mono samples, e.g. qa_fill_audio.
    #[arg(long)]
    pub(crate) method: String,
    /// Seconds of audio to render (the tool owns the bake loop, not the project).
    #[arg(long)]
    pub(crate) seconds: f64,
    #[arg(long = "max-seconds", default_value_t = 60.0)]
    pub(crate) max_seconds: f64,
    /// Output sample rate fed to the fixture and written to the WAV.
    #[arg(long = "mix-rate", default_value_t = 44_100)]
    pub(crate) mix_rate: u32,
    #[arg(long)]
    pub(crate) args: Option<String>,
    #[arg(long, default_value = "render")]
    pub(crate) name: String,
    #[arg(long)]
    pub(crate) out: Option<PathBuf>,
    #[arg(long, action = ArgAction::SetTrue)]
    pub(crate) analyze: bool,
    #[arg(long = "strict-warnings", action = ArgAction::SetTrue)]
    pub(crate) strict_warnings: bool,
}

#[derive(Debug, Args)]
pub(crate) struct AudioAnalyzeArgs {
    pub(crate) wav: PathBuf,
    #[arg(long = "window-seconds")]
    pub(crate) window_seconds: Option<f64>,
    #[arg(long = "hop-seconds")]
    pub(crate) hop_seconds: Option<f64>,
    /// Also render a waveform + spectrogram PNG to this path (must end in .png).
    #[arg(long = "spectrogram")]
    pub(crate) spectrogram: Option<PathBuf>,
    #[arg(long = "strict-warnings", action = ArgAction::SetTrue)]
    pub(crate) strict_warnings: bool,
}

#[derive(Debug, Args)]
pub(crate) struct LayoutArgs {
    #[command(subcommand)]
    pub(crate) command: LayoutCommands,
}

#[derive(Debug, Subcommand)]
pub(crate) enum LayoutCommands {
    AuditCurrent(LayoutAuditCurrentArgs),
    Audit(LayoutAuditArgs),
}

#[derive(Debug, Args)]
pub(crate) struct LayoutAuditCurrentArgs {
    /// Scenario whose saved snapshot to audit. Required unless `--live` is set,
    /// in which case the live session's freshly captured snapshot is used.
    #[arg(long)]
    pub(crate) scenario: Option<String>,
    #[arg(long, action = ArgAction::SetTrue)]
    pub(crate) live: bool,
    #[arg(long)]
    pub(crate) snapshot: Option<String>,
    #[arg(long = "run-id")]
    pub(crate) run_id: Option<String>,
    #[arg(long, action = ArgAction::SetTrue)]
    pub(crate) strict: bool,
}

#[derive(Debug, Args)]
pub(crate) struct LayoutAuditArgs {
    #[arg(long)]
    pub(crate) scene: String,
    #[arg(long, default_value = "1280x720,1920x1080")]
    pub(crate) viewports: String,
    #[arg(long, action = ArgAction::SetTrue)]
    pub(crate) strict: bool,
}

#[derive(Debug, Args)]
pub(crate) struct AssertArgs {
    pub(crate) assertion: String,
    pub(crate) target: Option<String>,
    pub(crate) expected: Option<String>,
    #[arg(long = "window-frames")]
    pub(crate) window_frames: Option<i64>,
    #[arg(long = "window-seconds")]
    pub(crate) window_seconds: Option<f64>,
    #[arg(long = "sample-every-frames")]
    pub(crate) sample_every_frames: Option<i64>,
    #[arg(long = "dwell-frames")]
    pub(crate) dwell_frames: Option<i64>,
    #[arg(long = "dwell-seconds")]
    pub(crate) dwell_seconds: Option<f64>,
    #[arg(long)]
    pub(crate) max: Option<f64>,
    #[arg(long)]
    pub(crate) min: Option<f64>,
    #[arg(long = "min-ratio")]
    pub(crate) min_ratio: Option<f64>,
    #[arg(long = "min-count")]
    pub(crate) min_count: Option<i64>,
    #[arg(long)]
    pub(crate) tolerance: Option<f64>,
    #[arg(long)]
    pub(crate) occluder: Option<String>,
    #[arg(long)]
    pub(crate) leader: Option<String>,
    #[arg(long)]
    pub(crate) property: Option<String>,
    #[arg(long)]
    pub(crate) forbidden: Option<String>,
    #[arg(long = "evidence-samples")]
    pub(crate) evidence_samples: Option<i64>,
    #[arg(long = "point")]
    pub(crate) point: Vec<String>,
    #[arg(long)]
    pub(crate) surface: Option<String>,
    #[arg(long)]
    pub(crate) provider: Option<String>,
    #[arg(long = "provider-id")]
    pub(crate) provider_id: Option<String>,
    #[arg(long = "provider-method")]
    pub(crate) provider_method: Option<String>,
    #[arg(long = "coordinate-space")]
    pub(crate) coordinate_space: Option<String>,
    #[arg(long)]
    pub(crate) radius: Option<f64>,
    #[arg(long = "max-points")]
    pub(crate) max_points: Option<i64>,
    #[arg(long)]
    pub(crate) scenario: Option<String>,
    #[arg(long)]
    pub(crate) snapshot: Option<String>,
    #[arg(long = "run-id")]
    pub(crate) run_id: Option<String>,
    #[arg(long, action = ArgAction::SetTrue)]
    pub(crate) live: bool,
}

#[derive(Debug, Args)]
pub(crate) struct ScreenshotArgs {
    #[command(subcommand)]
    pub(crate) command: ScreenshotCommands,
}

#[derive(Debug, Args)]
pub(crate) struct VisualArgs {
    #[command(subcommand)]
    pub(crate) command: VisualCommands,
}

#[derive(Debug, Subcommand)]
pub(crate) enum VisualCommands {
    Preflight(VisualPreflightArgs),
    Audit(VisualAuditArgs),
    Isolate(VisualIsolateArgs),
}

#[derive(Debug, Args)]
pub(crate) struct VisualPreflightArgs {
    #[arg(long, conflicts_with = "windowed", action = ArgAction::SetTrue)]
    pub(crate) headless: bool,
    #[arg(long, action = ArgAction::SetTrue)]
    pub(crate) windowed: bool,
}

#[derive(Debug, Args)]
pub(crate) struct VisualAuditArgs {
    #[arg(long, action = ArgAction::SetTrue)]
    pub(crate) live: bool,
    #[arg(long)]
    pub(crate) screenshot: Option<String>,
    #[arg(long)]
    pub(crate) sequence: Option<String>,
    #[arg(long)]
    pub(crate) scenario: Option<String>,
    #[arg(long = "run-id")]
    pub(crate) run_id: Option<String>,
    #[arg(long)]
    pub(crate) targets: Option<String>,
    #[arg(long = "target", action = ArgAction::Append)]
    pub(crate) target: Vec<String>,
    #[arg(long = "min-pixel-area")]
    pub(crate) min_pixel_area: Option<u64>,
    #[arg(long = "min-width")]
    pub(crate) min_width: Option<u32>,
    #[arg(long = "min-height")]
    pub(crate) min_height: Option<u32>,
    #[arg(long = "min-contrast-ratio")]
    pub(crate) min_contrast_ratio: Option<f64>,
    #[arg(long = "min-color-distance")]
    pub(crate) min_color_distance: Option<f64>,
    #[arg(long = "annotate-out")]
    pub(crate) annotate_out: Option<String>,
    #[arg(long = "strict-warnings", action = ArgAction::SetTrue)]
    pub(crate) strict_warnings: bool,
}

#[derive(Debug, Args)]
pub(crate) struct VisualIsolateArgs {
    #[arg(long)]
    pub(crate) out: String,
    #[arg(long = "setup-script")]
    pub(crate) setup_script: Option<String>,
    #[arg(long, default_value = "race_visual_layers")]
    pub(crate) provider: String,
    #[arg(long, default_value = "race_review")]
    pub(crate) driver: String,
    #[arg(long, default_value = "set_visual_layer_mode")]
    pub(crate) action: String,
    #[arg(long = "layer", action = ArgAction::Append)]
    pub(crate) layers: Vec<String>,
    #[arg(long = "columns", default_value_t = 4)]
    pub(crate) columns: i64,
    #[arg(long = "thumb-width", default_value_t = 360)]
    pub(crate) thumb_width: i64,
    #[arg(long = "contact-sheet-out")]
    pub(crate) contact_sheet_out: Option<String>,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ScreenshotCommands {
    Capture(ScreenshotCaptureArgs),
    #[command(name = "assert-startup-presented")]
    AssertStartupPresented(PresentedStartupArgs),
    Compare(ScreenshotCompareArgs),
    Sequence(ScreenshotSequenceArgs),
    #[command(name = "contact-sheet")]
    ContactSheet(ScreenshotContactSheetArgs),
}

#[derive(Debug, Args)]
pub(crate) struct PresentedStartupArgs {
    #[arg(long)]
    pub(crate) scene: String,
    #[arg(long, default_value = "1280x720")]
    pub(crate) viewport: String,
    #[arg(long = "display-backend", value_parser = ["wayland", "x11"], default_value = "wayland")]
    pub(crate) display_backend: String,
    #[arg(long)]
    pub(crate) baseline: Option<String>,
    #[arg(long)]
    pub(crate) out: Option<String>,
    #[arg(long, default_value_t = 0.01)]
    pub(crate) threshold: f64,
    /// Maximum absolute difference allowed in each RGBA channel of a pixel.
    #[arg(long, default_value_t = 0)]
    pub(crate) channel_tolerance: u8,
}

#[derive(Debug, Args)]
pub(crate) struct ScreenshotCaptureArgs {
    #[arg(long)]
    pub(crate) out: String,
    #[arg(long = "setup-script")]
    pub(crate) setup_script: Option<String>,
    #[arg(long = "require-visible", action = ArgAction::Append)]
    pub(crate) require_visible: Vec<String>,
    #[arg(long = "require-hidden", action = ArgAction::Append)]
    pub(crate) require_hidden: Vec<String>,
    #[arg(long = "require-state")]
    pub(crate) require_state: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct ScreenshotCompareArgs {
    #[arg(long)]
    pub(crate) actual: Option<String>,
    #[arg(long)]
    pub(crate) baseline: Option<String>,
    #[arg(long)]
    pub(crate) diff: Option<String>,
    #[arg(long, default_value_t = 0.01)]
    pub(crate) threshold: f64,
    #[arg(long, action = ArgAction::SetTrue)]
    pub(crate) live: bool,
}

#[derive(Debug, Args)]
pub(crate) struct ScreenshotSequenceArgs {
    #[arg(long)]
    pub(crate) frames: Option<i64>,
    #[arg(long)]
    pub(crate) every: Option<i64>,
    #[arg(long)]
    pub(crate) seconds: Option<f64>,
    #[arg(long)]
    pub(crate) fps: Option<f64>,
    #[arg(long)]
    pub(crate) out: String,
    #[arg(long = "setup-script")]
    pub(crate) setup_script: Option<String>,
    #[arg(long = "require-visible", action = ArgAction::Append)]
    pub(crate) require_visible: Vec<String>,
    #[arg(long = "require-hidden", action = ArgAction::Append)]
    pub(crate) require_hidden: Vec<String>,
    #[arg(long = "require-state")]
    pub(crate) require_state: Option<String>,
    /// Stop the sequence early once this state_matches predicate holds. Unlike the post-hoc
    /// `--require-state` guard, this ends capture as soon as the predicate matches.
    #[arg(long = "stop-after-state")]
    pub(crate) stop_after_state: Option<String>,
    #[arg(long)]
    pub(crate) scenario: Option<String>,
    #[arg(long = "allow-low-information", action = ArgAction::SetTrue)]
    pub(crate) allow_low_information: bool,
    #[arg(long = "visual-risk", action = ArgAction::SetTrue)]
    pub(crate) visual_risk: bool,
    #[arg(long = "visual-risk-strict", action = ArgAction::SetTrue)]
    pub(crate) visual_risk_strict: bool,
    #[arg(long = "contact-sheet", action = ArgAction::SetTrue)]
    pub(crate) contact_sheet: bool,
    #[arg(long = "contact-sheet-columns")]
    pub(crate) contact_sheet_columns: Option<i64>,
    #[arg(long = "contact-sheet-thumb-width")]
    pub(crate) contact_sheet_thumb_width: Option<i64>,
    #[arg(long = "contact-sheet-out")]
    pub(crate) contact_sheet_out: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct ScreenshotContactSheetArgs {
    pub(crate) sequence_dir: String,
    #[arg(long)]
    pub(crate) columns: i64,
    #[arg(long = "thumb-width")]
    pub(crate) thumb_width: i64,
    #[arg(long)]
    pub(crate) out: String,
}

#[derive(Debug, Args)]
pub(crate) struct EditArgs {
    #[command(subcommand)]
    pub(crate) command: EditCommands,
}

#[derive(Debug, Subcommand)]
pub(crate) enum EditCommands {
    Set(EditSetArgs),
    Call(EditCallArgs),
    Diff,
    ExportPatch(EditExportPatchArgs),
}

#[derive(Debug, Args)]
pub(crate) struct EditSetArgs {
    pub(crate) target: String,
    pub(crate) property: String,
    pub(crate) value: String,
}

#[derive(Debug, Args)]
pub(crate) struct EditCallArgs {
    pub(crate) target: String,
    pub(crate) method: String,
}

#[derive(Debug, Args)]
pub(crate) struct EditExportPatchArgs {
    #[arg(long)]
    pub(crate) out: String,
}

#[derive(Debug, Args)]
pub(crate) struct ApplyPatchArgs {
    pub(crate) patch: String,
}

#[derive(Debug, Args)]
pub(crate) struct LogsArgs {
    #[arg(long)]
    pub(crate) since: Option<String>,
}
