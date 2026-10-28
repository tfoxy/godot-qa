use crate::results::FailureDetail;
use serde_json::json;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use yaml_serde::Value as YamlValue;

#[derive(Debug, Clone)]
pub struct ProjectConfig {
    pub project_root: PathBuf,
    pub repo_root: PathBuf,
    pub verbose: bool,
    pub godot_binary: Option<String>,
    pub qa_dir: PathBuf,
    pub artifacts_dir: PathBuf,
    pub scenarios_dir: PathBuf,
    pub addon_dir: PathBuf,
    pub repo_addon_dir: PathBuf,
    /// Shared by invocation clones so parallel bundle children hash installed files once.
    pub addon_identity_cache: Arc<OnceLock<crate::addon_identity::AddonIdentityComparison>>,
    pub godot_launch_prefix: Vec<String>,
    pub godot_windowed_launch_prefix: Vec<String>,
    pub godot_args: Vec<String>,
    pub godot_project_args: Vec<String>,
    pub godot_launch_mode: GodotLaunchMode,
    pub godot_audio_mode: GodotAudioMode,
    pub scenario_warm_default: bool,
    pub scenario_warm_strict_default: bool,
    pub scenario_jobs_default: Option<i64>,
    /// `scenario.schedule` from `qa/config.yaml`: the bundle dispatch order. `None` (the
    /// default) means the runner picks its own default order.
    pub scenario_schedule_default: Option<ScenarioSchedule>,
    /// `logs.ignore` from `qa/config.yaml`: substrings that silence engine `ERROR:`/
    /// `WARNING:` log lines in every log scan. Empty (the default) silences nothing.
    pub log_ignore_patterns: Vec<String>,
    /// `artifacts.keep_runs` from `qa/config.yaml`: how many run directories to keep per
    /// scenario and per bundle under the artifacts root. `None` (the default) means
    /// unlimited — nothing is ever pruned, which is the historical behaviour.
    pub artifacts_keep_runs: Option<usize>,
    pub project_config_failure: Option<FailureDetail>,
    /// Warnings about `qa/` config files in unsupported spellings (json/yml) that exist on
    /// disk but are silently ignored. Never affects exit codes or verdicts; surfaced so a
    /// consumer who ships e.g. `qa/config.json` learns it is being skipped.
    pub config_warnings: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GodotLaunchMode {
    #[default]
    Auto,
    Headless,
    Windowed,
}

/// Audio driver axis, independent of the display (`GodotLaunchMode`) axis:
/// `On` forces the real audio driver, `Off` forces `--audio-driver Dummy`, and
/// `Auto` defers to the scenario (audio only when it captures audio evidence).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GodotAudioMode {
    #[default]
    Auto,
    On,
    Off,
}

/// Bundle dispatch order (`scenario.schedule`). `Manifest` keeps the bundle expansion
/// order; `LongestFirst` dispatches the scenarios with the longest known durations first
/// so the parallel tail is short.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScenarioSchedule {
    Manifest,
    LongestFirst,
}

impl ScenarioSchedule {
    pub fn as_str(self) -> &'static str {
        match self {
            ScenarioSchedule::Manifest => "manifest",
            ScenarioSchedule::LongestFirst => "longest_first",
        }
    }
}

impl std::str::FromStr for ScenarioSchedule {
    type Err = String;

    /// Both spellings of the hyphenated CLI form are accepted so `--schedule longest-first`
    /// and `schedule: longest_first` in YAML name the same policy.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "manifest" => Ok(ScenarioSchedule::Manifest),
            "longest-first" | "longest_first" => Ok(ScenarioSchedule::LongestFirst),
            other => Err(format!(
                "unknown scenario schedule: {other} (expected manifest or longest-first)"
            )),
        }
    }
}

impl ProjectConfig {
    pub fn installed_addon_identity(&self) -> &crate::addon_identity::AddonIdentityComparison {
        self.addon_identity_cache
            .get_or_init(|| crate::addon_identity::compare_installed(&self.addon_dir))
    }

    pub fn project_config_failure_for_command(&self, command: &str) -> Option<FailureDetail> {
        self.project_config_failure.clone().map(|mut failure| {
            failure.command = Some(command.to_string());
            failure
        })
    }

    pub fn launch_mode_headless_override(&self) -> Option<bool> {
        match self.godot_launch_mode {
            GodotLaunchMode::Auto => None,
            GodotLaunchMode::Headless => Some(true),
            GodotLaunchMode::Windowed => Some(false),
        }
    }

    pub fn audio_mode_override(&self) -> Option<bool> {
        match self.godot_audio_mode {
            GodotAudioMode::Auto => None,
            GodotAudioMode::On => Some(true),
            GodotAudioMode::Off => Some(false),
        }
    }
}

pub fn repo_root() -> PathBuf {
    distribution_root(&env::current_exe().unwrap_or_else(|_| PathBuf::from("godot-qa")))
}

/// Locate optional distribution resources without retaining the builder's checkout path.
/// Development binaries live under `target/`; installed binaries may have an addon
/// beside them or no source resources at all (for example, `cargo install`).
fn distribution_root(executable: &Path) -> PathBuf {
    let directory = executable.parent().unwrap_or_else(|| Path::new("."));
    for candidate in directory.ancestors() {
        if candidate.join("addons/godot_qa/plugin.cfg").is_file() {
            return candidate.to_path_buf();
        }
    }
    directory.to_path_buf()
}

pub fn absolute_path(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    if path.is_absolute() {
        path.components().collect()
    } else {
        env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
            .components()
            .collect()
    }
}

pub fn find_project_root(start: Option<&Path>) -> PathBuf {
    let mut current = absolute_path(start.unwrap_or_else(|| Path::new(".")));
    if !current.exists() {
        return current;
    }
    if current.is_file() {
        current = current.parent().map(Path::to_path_buf).unwrap_or(current);
    }

    for candidate in current.ancestors() {
        if candidate.join("Cargo.toml").exists() || candidate.join("project.godot").exists() {
            return candidate.to_path_buf();
        }
    }
    current
}

pub fn load_project_config(
    start: Option<&Path>,
    godot_bin: Option<&str>,
    artifact_dir: Option<&str>,
    verbose: bool,
    resolved_godot_binary: Option<String>,
) -> ProjectConfig {
    let project_root = find_project_root(start);
    let repo_root = repo_root();
    let qa_dir = project_root.join("qa");
    let generated_qa_dir = project_root.join(".qa");
    let project_config = load_godot_qa_config(&project_root);
    let artifacts_dir = match artifact_dir {
        Some(raw) => {
            let raw_path = PathBuf::from(raw);
            if raw_path.is_absolute() {
                raw_path.components().collect()
            } else {
                project_root.join(raw_path).components().collect()
            }
        }
        None => generated_qa_dir.join("artifacts"),
    };

    ProjectConfig {
        project_root: project_root.clone(),
        repo_root: repo_root.clone(),
        verbose,
        godot_binary: godot_bin.map(str::to_owned).or(resolved_godot_binary),
        qa_dir: qa_dir.clone(),
        artifacts_dir,
        scenarios_dir: qa_dir.join("scenarios"),
        addon_dir: project_root.join("addons").join("godot_qa"),
        repo_addon_dir: repo_root.join("addons").join("godot_qa"),
        addon_identity_cache: Default::default(),
        godot_launch_prefix: project_config
            .as_ref()
            .map(|config| config.godot_launch_prefix.clone())
            .unwrap_or_default(),
        godot_windowed_launch_prefix: project_config
            .as_ref()
            .map(|config| config.godot_windowed_launch_prefix.clone())
            .unwrap_or_default(),
        godot_args: project_config
            .as_ref()
            .map(|config| config.godot_args.clone())
            .unwrap_or_default(),
        godot_project_args: project_config
            .as_ref()
            .map(|config| config.godot_project_args.clone())
            .unwrap_or_default(),
        godot_launch_mode: project_config
            .as_ref()
            .map(|config| config.godot_launch_mode)
            .unwrap_or_default(),
        godot_audio_mode: project_config
            .as_ref()
            .map(|config| config.godot_audio_mode)
            .unwrap_or_default(),
        scenario_warm_default: project_config
            .as_ref()
            .map(|config| config.scenario_warm_default)
            .unwrap_or_default(),
        scenario_warm_strict_default: project_config
            .as_ref()
            .map(|config| config.scenario_warm_strict_default)
            .unwrap_or_default(),
        scenario_jobs_default: project_config
            .as_ref()
            .map(|config| config.scenario_jobs_default)
            .unwrap_or_default(),
        scenario_schedule_default: project_config
            .as_ref()
            .map(|config| config.scenario_schedule_default)
            .unwrap_or_default(),
        log_ignore_patterns: project_config
            .as_ref()
            .map(|config| config.log_ignore_patterns.clone())
            .unwrap_or_default(),
        artifacts_keep_runs: project_config
            .as_ref()
            .map(|config| config.artifacts_keep_runs)
            .unwrap_or_default(),
        config_warnings: project_config
            .as_ref()
            .map(|config| config.config_warnings.clone())
            .unwrap_or_default(),
        project_config_failure: project_config.err(),
    }
}

#[derive(Debug, Clone, Default)]
struct GodotQaConfig {
    godot_launch_prefix: Vec<String>,
    godot_windowed_launch_prefix: Vec<String>,
    godot_args: Vec<String>,
    godot_project_args: Vec<String>,
    godot_launch_mode: GodotLaunchMode,
    godot_audio_mode: GodotAudioMode,
    scenario_warm_default: bool,
    scenario_warm_strict_default: bool,
    scenario_jobs_default: Option<i64>,
    scenario_schedule_default: Option<ScenarioSchedule>,
    log_ignore_patterns: Vec<String>,
    artifacts_keep_runs: Option<usize>,
    config_warnings: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Default)]
struct GodotQaConfigOverlay {
    godot_launch_prefix: Option<Vec<String>>,
    godot_windowed_launch_prefix: Option<Vec<String>>,
    godot_args: Option<Vec<String>>,
    godot_project_args: Option<Vec<String>>,
    godot_launch_mode: Option<GodotLaunchMode>,
    godot_audio_mode: Option<GodotAudioMode>,
    scenario_warm_default: Option<bool>,
    scenario_warm_strict_default: Option<bool>,
    scenario_jobs_default: Option<i64>,
    scenario_schedule_default: Option<ScenarioSchedule>,
    log_ignore_patterns: Option<Vec<String>>,
    artifacts_keep_runs: Option<usize>,
    /// `unknown_config_key` warnings collected while parsing this one file. The overlay is
    /// the only thing `load_godot_qa_config_file` returns, so warnings ride along on it and
    /// `apply` appends them to the merged config.
    warnings: Vec<serde_json::Value>,
}

impl GodotQaConfig {
    fn apply(&mut self, overlay: GodotQaConfigOverlay) {
        if let Some(godot_launch_prefix) = overlay.godot_launch_prefix {
            self.godot_launch_prefix = godot_launch_prefix;
        }
        if let Some(godot_windowed_launch_prefix) = overlay.godot_windowed_launch_prefix {
            self.godot_windowed_launch_prefix = godot_windowed_launch_prefix;
        }
        if let Some(godot_args) = overlay.godot_args {
            self.godot_args = godot_args;
        }
        if let Some(godot_project_args) = overlay.godot_project_args {
            self.godot_project_args = godot_project_args;
        }
        if let Some(godot_launch_mode) = overlay.godot_launch_mode {
            self.godot_launch_mode = godot_launch_mode;
        }
        if let Some(godot_audio_mode) = overlay.godot_audio_mode {
            self.godot_audio_mode = godot_audio_mode;
        }
        if let Some(scenario_warm_default) = overlay.scenario_warm_default {
            self.scenario_warm_default = scenario_warm_default;
        }
        if let Some(scenario_warm_strict_default) = overlay.scenario_warm_strict_default {
            self.scenario_warm_strict_default = scenario_warm_strict_default;
        }
        if let Some(scenario_jobs_default) = overlay.scenario_jobs_default {
            self.scenario_jobs_default = Some(scenario_jobs_default);
        }
        if let Some(scenario_schedule_default) = overlay.scenario_schedule_default {
            self.scenario_schedule_default = Some(scenario_schedule_default);
        }
        // Array overlay semantics match `godot.args`: a present array replaces the base
        // one outright, and an empty local array clears the project default.
        if let Some(log_ignore_patterns) = overlay.log_ignore_patterns {
            self.log_ignore_patterns = log_ignore_patterns;
        }
        if let Some(artifacts_keep_runs) = overlay.artifacts_keep_runs {
            self.artifacts_keep_runs = Some(artifacts_keep_runs);
        }
        self.config_warnings.extend(overlay.warnings);
    }
}

#[allow(clippy::result_large_err)]
fn load_godot_qa_config(project_root: &Path) -> Result<GodotQaConfig, FailureDetail> {
    let mut config = GodotQaConfig::default();
    for file_name in ["config.yaml", "config.local.yaml"] {
        let config_path = project_root.join("qa").join(file_name);
        if !config_path.exists() {
            continue;
        }
        config.apply(load_godot_qa_config_file(
            &config_path,
            &format!("qa/{file_name}"),
        )?);
    }
    // Only config.yaml/config.local.yaml are read. A consumer who ships an unsupported
    // spelling (json/yml) would otherwise have it silently ignored, so surface a warning
    // for each such file that exists on disk. This never fails or changes exit codes.
    for file_name in [
        "config.json",
        "config.local.json",
        "config.yml",
        "config.local.yml",
    ] {
        let config_path = project_root.join("qa").join(file_name);
        if config_path.exists() {
            config
                .config_warnings
                .push(unsupported_config_file_warning(&format!("qa/{file_name}")));
        }
    }
    resolve_launch_prefix_paths(project_root, &mut config.godot_launch_prefix);
    resolve_launch_prefix_paths(project_root, &mut config.godot_windowed_launch_prefix);
    Ok(config)
}

/// Build the warning value for a `qa/` config file in an unsupported spelling. Field naming
/// mirrors `addon_version_mismatch_warning` (`type`/`severity`/`remediation`), kept minimal.
fn unsupported_config_file_warning(display_path: &str) -> serde_json::Value {
    json!({
        "type": "unsupported_config_file",
        "severity": "warning",
        "path": display_path,
        "expected": "qa/config.yaml (or qa/config.local.yaml)",
        "remediation": "rename the file to qa/config.yaml — only config.yaml/config.local.yaml are read; this file is being ignored",
    })
}

/// Recognised keys per `qa/config.yaml` mapping level. Anything else is ignored by the
/// parser, so it is reported as an `unknown_config_key` warning instead of silently
/// disabling the feature the author thought they were enabling.
const ROOT_CONFIG_KEYS: &[&str] = &["godot", "scenario", "artifacts", "logs"];
const GODOT_CONFIG_KEYS: &[&str] = &[
    "launch_prefix",
    "windowed_launch_prefix",
    "args",
    "project_args",
    "launch_mode",
    "audio_mode",
];
const SCENARIO_CONFIG_KEYS: &[&str] = &["warm", "warm_strict", "jobs", "schedule"];
const ARTIFACTS_CONFIG_KEYS: &[&str] = &["keep_runs"];
const LOGS_CONFIG_KEYS: &[&str] = &["ignore"];

/// Push one `unknown_config_key` warning per unrecognised key in `mapping`. `prefix` is the
/// dotted parent path (`None` at the config root). Non-string keys cannot name a config key
/// and are left alone. Field naming mirrors `unsupported_config_file_warning`.
fn collect_unknown_config_keys(
    mapping: &yaml_serde::Mapping,
    prefix: Option<&str>,
    known: &[&str],
    display_path: &str,
    warnings: &mut Vec<serde_json::Value>,
) {
    for key in mapping.keys() {
        let Some(name) = key.as_str() else {
            continue;
        };
        if known.contains(&name) {
            continue;
        }
        let dotted = match prefix {
            Some(prefix) => format!("{prefix}.{name}"),
            None => name.to_string(),
        };
        warnings.push(json!({
            "type": "unknown_config_key",
            "severity": "warning",
            "path": display_path,
            "key": dotted,
            "expected": known,
            "remediation": "remove or rename the key; it is being ignored",
        }));
    }
}

#[allow(clippy::result_large_err)]
fn load_godot_qa_config_file(
    config_path: &Path,
    display_path: &str,
) -> Result<GodotQaConfigOverlay, FailureDetail> {
    let raw = fs::read_to_string(config_path).map_err(|error| {
        project_config_failure(
            config_path,
            &format!("readable {display_path}"),
            "OSError",
            error.to_string(),
        )
    })?;
    let value: YamlValue = yaml_serde::from_str(&raw).map_err(|error| {
        project_config_failure(
            config_path,
            &format!("valid YAML object in {display_path}"),
            "invalid_yaml",
            error.to_string(),
        )
    })?;
    if matches!(value, YamlValue::Null) {
        return Ok(GodotQaConfigOverlay::default());
    }
    let Some(root) = value.as_mapping() else {
        return Err(project_config_failure(
            config_path,
            &format!("YAML object at {display_path} root"),
            value_type(&value),
            format!("{display_path} root must be an object"),
        ));
    };
    let mut warnings = Vec::new();
    collect_unknown_config_keys(root, None, ROOT_CONFIG_KEYS, display_path, &mut warnings);
    let (
        godot_launch_prefix,
        godot_windowed_launch_prefix,
        godot_args,
        godot_project_args,
        godot_launch_mode,
        godot_audio_mode,
    ) = match root.get("godot") {
        None => (None, None, None, None, None, None),
        Some(godot) => {
            let Some(godot) = godot.as_mapping() else {
                return Err(project_config_failure(
                    config_path,
                    "godot object",
                    value_type(godot),
                    format!("{display_path} godot must be an object"),
                ));
            };
            collect_unknown_config_keys(
                godot,
                Some("godot"),
                GODOT_CONFIG_KEYS,
                display_path,
                &mut warnings,
            );
            let godot_launch_prefix = parse_optional_string_array(
                godot.get("launch_prefix"),
                "godot.launch_prefix",
                config_path,
                display_path,
            )?;
            let godot_windowed_launch_prefix = parse_optional_string_array(
                godot.get("windowed_launch_prefix"),
                "godot.windowed_launch_prefix",
                config_path,
                display_path,
            )?;
            let godot_args = parse_optional_string_array(
                godot.get("args"),
                "godot.args",
                config_path,
                display_path,
            )?;
            let godot_project_args = parse_optional_string_array(
                godot.get("project_args"),
                "godot.project_args",
                config_path,
                display_path,
            )?;
            let godot_launch_mode =
                parse_optional_launch_mode(godot.get("launch_mode"), config_path)?;
            let godot_audio_mode = parse_optional_audio_mode(godot.get("audio_mode"), config_path)?;
            (
                godot_launch_prefix,
                godot_windowed_launch_prefix,
                godot_args,
                godot_project_args,
                godot_launch_mode,
                godot_audio_mode,
            )
        }
    };

    let (
        scenario_warm_default,
        scenario_warm_strict_default,
        scenario_jobs_default,
        scenario_schedule_default,
    ) = match root.get("scenario") {
        None => (None, None, None, None),
        Some(scenario) => {
            let Some(scenario) = scenario.as_mapping() else {
                return Err(project_config_failure(
                    config_path,
                    "scenario object",
                    value_type(scenario),
                    format!("{display_path} scenario must be an object"),
                ));
            };
            collect_unknown_config_keys(
                scenario,
                Some("scenario"),
                SCENARIO_CONFIG_KEYS,
                display_path,
                &mut warnings,
            );
            let warm = parse_optional_bool(
                scenario.get("warm"),
                "scenario.warm",
                config_path,
                display_path,
            )?;
            let warm_strict = parse_optional_bool(
                scenario.get("warm_strict"),
                "scenario.warm_strict",
                config_path,
                display_path,
            )?;
            let jobs = parse_optional_integer(
                scenario.get("jobs"),
                "scenario.jobs",
                config_path,
                display_path,
            )?;
            let schedule = parse_optional_schedule(scenario.get("schedule"), config_path)?;
            (warm, warm_strict, jobs, schedule)
        }
    };

    let log_ignore_patterns = match root.get("logs") {
        None => None,
        Some(logs) => {
            let Some(logs) = logs.as_mapping() else {
                return Err(project_config_failure(
                    config_path,
                    "logs object",
                    value_type(logs),
                    format!("{display_path} logs must be an object"),
                ));
            };
            collect_unknown_config_keys(
                logs,
                Some("logs"),
                LOGS_CONFIG_KEYS,
                display_path,
                &mut warnings,
            );
            parse_optional_string_array(
                logs.get("ignore"),
                "logs.ignore",
                config_path,
                display_path,
            )?
        }
    };

    let artifacts_keep_runs = match root.get("artifacts") {
        None => None,
        Some(artifacts) => {
            let Some(artifacts) = artifacts.as_mapping() else {
                return Err(project_config_failure(
                    config_path,
                    "artifacts object",
                    value_type(artifacts),
                    format!("{display_path} artifacts must be an object"),
                ));
            };
            collect_unknown_config_keys(
                artifacts,
                Some("artifacts"),
                ARTIFACTS_CONFIG_KEYS,
                display_path,
                &mut warnings,
            );
            parse_optional_keep_runs(artifacts.get("keep_runs"), config_path, display_path)?
        }
    };

    Ok(GodotQaConfigOverlay {
        godot_launch_prefix,
        godot_windowed_launch_prefix,
        godot_args,
        godot_project_args,
        godot_launch_mode,
        godot_audio_mode,
        scenario_warm_default,
        scenario_warm_strict_default,
        scenario_jobs_default,
        scenario_schedule_default,
        log_ignore_patterns,
        artifacts_keep_runs,
        warnings,
    })
}

#[allow(clippy::result_large_err)]
fn parse_optional_schedule(
    value: Option<&YamlValue>,
    config_path: &Path,
) -> Result<Option<ScenarioSchedule>, FailureDetail> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Some(raw) = value.as_str() else {
        return Err(project_config_failure(
            config_path,
            "scenario.schedule string: manifest or longest-first",
            value_type(value),
            "scenario.schedule must be a string".to_string(),
        ));
    };
    raw.parse::<ScenarioSchedule>().map(Some).map_err(|_| {
        project_config_failure(
            config_path,
            "scenario.schedule string: manifest or longest-first",
            raw,
            "scenario.schedule must be one of: manifest, longest-first".to_string(),
        )
    })
}

/// `artifacts.keep_runs` must be a positive integer. Zero would mean "delete everything the
/// run just wrote", so it is rejected rather than silently treated as unlimited.
#[allow(clippy::result_large_err)]
fn parse_optional_keep_runs(
    value: Option<&YamlValue>,
    config_path: &Path,
    display_path: &str,
) -> Result<Option<usize>, FailureDetail> {
    let Some(raw) =
        parse_optional_integer(value, "artifacts.keep_runs", config_path, display_path)?
    else {
        return Ok(None);
    };
    if raw < 1 {
        return Err(project_config_failure(
            config_path,
            "artifacts.keep_runs positive integer",
            &raw.to_string(),
            format!("{display_path} artifacts.keep_runs must be a positive integer"),
        ));
    }
    Ok(Some(raw as usize))
}

#[allow(clippy::result_large_err)]
fn parse_optional_string_array(
    value: Option<&YamlValue>,
    field: &str,
    config_path: &Path,
    display_path: &str,
) -> Result<Option<Vec<String>>, FailureDetail> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Some(items) = value.as_sequence() else {
        return Err(project_config_failure(
            config_path,
            &format!("{field} array of non-empty strings"),
            value_type(value),
            format!("{display_path} {field} must be an array"),
        ));
    };
    let mut parsed = Vec::with_capacity(items.len());
    for (index, entry) in items.iter().enumerate() {
        let Some(raw) = entry.as_str() else {
            return Err(project_config_failure(
                config_path,
                &format!("{field} array of non-empty strings"),
                value_type(entry),
                format!("{field}[{index}] must be a string"),
            ));
        };
        if raw.is_empty() {
            return Err(project_config_failure(
                config_path,
                &format!("{field} array of non-empty strings"),
                "empty_string",
                format!("{field}[{index}] must not be empty"),
            ));
        }
        parsed.push(raw.to_string());
    }
    Ok(Some(parsed))
}

#[allow(clippy::result_large_err)]
fn parse_optional_launch_mode(
    value: Option<&YamlValue>,
    config_path: &Path,
) -> Result<Option<GodotLaunchMode>, FailureDetail> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Some(raw) = value.as_str() else {
        return Err(project_config_failure(
            config_path,
            "godot.launch_mode string: auto, headless, or windowed",
            value_type(value),
            "godot.launch_mode must be a string".to_string(),
        ));
    };
    match raw {
        "auto" => Ok(Some(GodotLaunchMode::Auto)),
        "headless" => Ok(Some(GodotLaunchMode::Headless)),
        "windowed" => Ok(Some(GodotLaunchMode::Windowed)),
        _ => Err(project_config_failure(
            config_path,
            "godot.launch_mode string: auto, headless, or windowed",
            raw,
            "godot.launch_mode must be one of: auto, headless, windowed".to_string(),
        )),
    }
}

#[allow(clippy::result_large_err)]
fn parse_optional_audio_mode(
    value: Option<&YamlValue>,
    config_path: &Path,
) -> Result<Option<GodotAudioMode>, FailureDetail> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Some(raw) = value.as_str() else {
        return Err(project_config_failure(
            config_path,
            "godot.audio_mode string: auto, on, or off",
            value_type(value),
            "godot.audio_mode must be a string".to_string(),
        ));
    };
    match raw {
        "auto" => Ok(Some(GodotAudioMode::Auto)),
        "on" => Ok(Some(GodotAudioMode::On)),
        "off" => Ok(Some(GodotAudioMode::Off)),
        _ => Err(project_config_failure(
            config_path,
            "godot.audio_mode string: auto, on, or off",
            raw,
            "godot.audio_mode must be one of: auto, on, off".to_string(),
        )),
    }
}

#[allow(clippy::result_large_err)]
fn parse_optional_bool(
    value: Option<&YamlValue>,
    field: &str,
    config_path: &Path,
    display_path: &str,
) -> Result<Option<bool>, FailureDetail> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Some(raw) = value.as_bool() else {
        return Err(project_config_failure(
            config_path,
            &format!("{field} boolean"),
            value_type(value),
            format!("{display_path} {field} must be a boolean"),
        ));
    };
    Ok(Some(raw))
}

#[allow(clippy::result_large_err)]
fn parse_optional_integer(
    value: Option<&YamlValue>,
    field: &str,
    config_path: &Path,
    display_path: &str,
) -> Result<Option<i64>, FailureDetail> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Some(raw) = value.as_i64() else {
        return Err(project_config_failure(
            config_path,
            &format!("{field} integer"),
            value_type(value),
            format!("{display_path} {field} must be an integer"),
        ));
    };
    Ok(Some(raw))
}

fn resolve_launch_prefix_paths(project_root: &Path, launch_prefix: &mut [String]) {
    for entry in launch_prefix {
        if !has_path_separator(entry) {
            continue;
        }
        let path = PathBuf::from(&*entry);
        if path.is_relative() {
            *entry = project_root.join(path).to_string_lossy().into_owned();
        }
    }
}

fn has_path_separator(value: &str) -> bool {
    value.contains('/') || value.contains('\\')
}

fn project_config_failure(
    path: &Path,
    expected: &str,
    actual: &str,
    message: String,
) -> FailureDetail {
    FailureDetail {
        kind: "invalid_project_config".to_string(),
        command: Some("project config".to_string()),
        path: Some(path.to_string_lossy().into_owned()),
        message: Some(message),
        expected: Some(json!(expected)),
        actual: Some(json!(actual)),
        ..FailureDetail::default()
    }
}

fn value_type(value: &YamlValue) -> &'static str {
    match value {
        YamlValue::Null => "null",
        YamlValue::Bool(_) => "bool",
        YamlValue::Number(_) => "number",
        YamlValue::String(_) => "string",
        YamlValue::Sequence(_) => "array",
        YamlValue::Mapping(_) => "object",
        YamlValue::Tagged(_) => "tagged",
    }
}

#[cfg(test)]
mod tests {
    use super::{GodotAudioMode, GodotLaunchMode, ScenarioSchedule, load_project_config};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn distribution_resources_follow_the_executable_not_the_build_checkout() {
        let root = unique_temp_dir();
        let distribution = root.join("release ü");
        fs::create_dir_all(distribution.join("addons/godot_qa")).unwrap();
        fs::write(
            distribution.join("addons/godot_qa/plugin.cfg"),
            "[plugin]\n",
        )
        .unwrap();
        assert_eq!(
            super::distribution_root(&distribution.join("godot-qa.exe")),
            distribution
        );
        let standalone = root.join("standalone");
        assert_eq!(
            super::distribution_root(&standalone.join("godot-qa.exe")),
            standalone
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn project_config_absent_has_no_launch_prefix() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        assert!(config.godot_launch_prefix.is_empty());
        assert!(config.godot_windowed_launch_prefix.is_empty());
        assert!(config.godot_args.is_empty());
        assert!(config.godot_project_args.is_empty());
        assert_eq!(config.godot_launch_mode, GodotLaunchMode::Auto);
        assert_eq!(config.godot_audio_mode, GodotAudioMode::Auto);
        assert_eq!(config.audio_mode_override(), None);
        assert_eq!(config.qa_dir, temp_dir.join("qa"));
        assert_eq!(config.artifacts_dir, temp_dir.join(".qa/artifacts"));
        assert!(config.project_config_failure.is_none());
    }

    #[test]
    fn project_config_loads_launch_defaults() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(
            temp_dir.join("qa/config.yaml"),
            "godot:\n  args:\n    - --rendering-driver\n    - opengl3\n  project_args:\n    - --no-countdown\n  launch_mode: headless\n",
        )
        .expect("write config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        assert_eq!(config.godot_args, vec!["--rendering-driver", "opengl3"]);
        assert_eq!(config.godot_project_args, vec!["--no-countdown"]);
        assert_eq!(config.godot_launch_mode, GodotLaunchMode::Headless);
        assert_eq!(config.launch_mode_headless_override(), Some(true));
        assert!(config.project_config_failure.is_none());
    }

    #[test]
    fn project_config_local_overlay_replaces_launch_defaults() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(
            temp_dir.join("qa/config.yaml"),
            "godot:\n  args:\n    - --base-godot\n  project_args:\n    - --base-project\n  launch_mode: headless\n",
        )
        .expect("write base config");
        fs::write(
            temp_dir.join("qa/config.local.yaml"),
            "godot:\n  args: []\n  project_args:\n    - --local-project\n  launch_mode: windowed\n",
        )
        .expect("write local config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        assert!(config.godot_args.is_empty());
        assert_eq!(config.godot_project_args, vec!["--local-project"]);
        assert_eq!(config.godot_launch_mode, GodotLaunchMode::Windowed);
        assert_eq!(config.launch_mode_headless_override(), Some(false));
        assert!(config.project_config_failure.is_none());
    }

    #[test]
    fn project_config_loads_scenario_warm_defaults() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(
            temp_dir.join("qa/config.yaml"),
            "scenario:\n  warm: true\n  warm_strict: true\n",
        )
        .expect("write config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        assert!(config.scenario_warm_default);
        assert!(config.scenario_warm_strict_default);
        assert!(config.project_config_failure.is_none());
    }

    #[test]
    fn project_config_absent_scenario_defaults_to_false() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        assert!(!config.scenario_warm_default);
        assert!(!config.scenario_warm_strict_default);
        assert_eq!(config.scenario_jobs_default, None);
    }

    #[test]
    fn project_config_loads_scenario_jobs_default() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(temp_dir.join("qa/config.yaml"), "scenario:\n  jobs: 1\n").expect("write config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        assert_eq!(config.scenario_jobs_default, Some(1));
        assert!(config.project_config_failure.is_none());
    }

    #[test]
    fn project_config_scenario_jobs_accepts_reserved_negative_values() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(temp_dir.join("qa/config.yaml"), "scenario:\n  jobs: -2\n")
            .expect("write config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        // Negative values mean "reserve N cores", exactly like --jobs -N.
        assert_eq!(config.scenario_jobs_default, Some(-2));
        assert!(config.project_config_failure.is_none());
    }

    #[test]
    fn project_config_rejects_non_integer_scenario_jobs() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(temp_dir.join("qa/config.yaml"), "scenario:\n  jobs: many\n")
            .expect("write config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        assert_eq!(config.scenario_jobs_default, None);
        assert_eq!(
            config
                .project_config_failure
                .as_ref()
                .map(|failure| failure.kind.as_str()),
            Some("invalid_project_config")
        );
    }

    #[test]
    fn project_config_rejects_invalid_scenario_warm_shape() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(
            temp_dir.join("qa/config.yaml"),
            "scenario:\n  warm: yes-please\n",
        )
        .expect("write config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        assert!(!config.scenario_warm_default);
        assert_eq!(
            config
                .project_config_failure
                .as_ref()
                .map(|failure| failure.kind.as_str()),
            Some("invalid_project_config")
        );
    }

    #[test]
    fn project_config_rejects_invalid_args_shape() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(
            temp_dir.join("qa/config.yaml"),
            "godot:\n  args: --headless\n",
        )
        .expect("write config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        assert!(config.godot_args.is_empty());
        assert_eq!(
            config
                .project_config_failure
                .as_ref()
                .map(|failure| failure.kind.as_str()),
            Some("invalid_project_config")
        );
    }

    #[test]
    fn project_config_loads_audio_mode() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(
            temp_dir.join("qa/config.yaml"),
            "godot:\n  audio_mode: on\n",
        )
        .expect("write config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        assert_eq!(config.godot_audio_mode, GodotAudioMode::On);
        assert_eq!(config.audio_mode_override(), Some(true));
        assert!(config.project_config_failure.is_none());
    }

    #[test]
    fn project_config_local_overlay_replaces_audio_mode() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(
            temp_dir.join("qa/config.yaml"),
            "godot:\n  audio_mode: on\n",
        )
        .expect("write base config");
        fs::write(
            temp_dir.join("qa/config.local.yaml"),
            "godot:\n  audio_mode: off\n",
        )
        .expect("write local config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        assert_eq!(config.godot_audio_mode, GodotAudioMode::Off);
        assert_eq!(config.audio_mode_override(), Some(false));
        assert!(config.project_config_failure.is_none());
    }

    #[test]
    fn project_config_rejects_invalid_audio_mode() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(
            temp_dir.join("qa/config.yaml"),
            "godot:\n  audio_mode: loud\n",
        )
        .expect("write config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        assert_eq!(config.godot_audio_mode, GodotAudioMode::Auto);
        assert_eq!(
            config
                .project_config_failure
                .as_ref()
                .map(|failure| failure.kind.as_str()),
            Some("invalid_project_config")
        );
    }

    #[test]
    fn project_config_rejects_invalid_launch_mode() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(
            temp_dir.join("qa/config.yaml"),
            "godot:\n  launch_mode: fullscreen\n",
        )
        .expect("write config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        assert_eq!(config.godot_launch_mode, GodotLaunchMode::Auto);
        assert_eq!(
            config
                .project_config_failure
                .as_ref()
                .map(|failure| failure.kind.as_str()),
            Some("invalid_project_config")
        );
    }

    #[test]
    fn explicit_artifact_dir_still_resolves_from_project_root() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");

        let config = load_project_config(Some(&temp_dir), None, Some("qa/artifacts"), false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        assert_eq!(config.artifacts_dir, temp_dir.join("qa/artifacts"));
    }

    #[test]
    fn project_config_loads_launch_prefix_array() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(
            temp_dir.join("qa/config.yaml"),
            "godot:\n  launch_prefix:\n    - xvfb-run\n    - -a\n",
        )
        .expect("write config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        assert_eq!(config.godot_launch_prefix, vec!["xvfb-run", "-a"]);
        assert!(config.godot_windowed_launch_prefix.is_empty());
        assert!(config.project_config_failure.is_none());
    }

    #[test]
    fn project_config_loads_windowed_launch_prefix_array() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(
            temp_dir.join("qa/config.yaml"),
            "godot:\n  launch_prefix:\n    - global-wrap\n  windowed_launch_prefix:\n    - xvfb-run\n    - -a\n",
        )
        .expect("write config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        assert_eq!(config.godot_launch_prefix, vec!["global-wrap"]);
        assert_eq!(config.godot_windowed_launch_prefix, vec!["xvfb-run", "-a"]);
        assert!(config.project_config_failure.is_none());
    }

    #[test]
    fn project_config_local_overlay_replaces_windowed_launch_prefix() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(
            temp_dir.join("qa/config.yaml"),
            "godot:\n  windowed_launch_prefix:\n    - xvfb-run\n    - -a\n",
        )
        .expect("write base config");
        fs::write(
            temp_dir.join("qa/config.local.yaml"),
            "godot:\n  windowed_launch_prefix:\n    - custom-display\n",
        )
        .expect("write local config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        assert_eq!(config.godot_windowed_launch_prefix, vec!["custom-display"]);
        assert!(config.project_config_failure.is_none());
    }

    #[test]
    fn project_config_local_empty_windowed_launch_prefix_clears_base() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(
            temp_dir.join("qa/config.yaml"),
            "godot:\n  windowed_launch_prefix:\n    - xvfb-run\n    - -a\n",
        )
        .expect("write base config");
        fs::write(
            temp_dir.join("qa/config.local.yaml"),
            "godot:\n  windowed_launch_prefix: []\n",
        )
        .expect("write local config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        assert!(config.godot_windowed_launch_prefix.is_empty());
        assert!(config.project_config_failure.is_none());
    }

    #[test]
    fn project_config_resolves_relative_prefix_path_from_project_root() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(
            temp_dir.join("qa/config.yaml"),
            "godot:\n  launch_prefix:\n    - xvfb-run\n    - -a\n    - tools/wrap-godot\n    - --flag\n",
        )
        .expect("write config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        assert_eq!(
            config.godot_launch_prefix,
            vec![
                "xvfb-run".to_string(),
                "-a".to_string(),
                temp_dir
                    .join("tools/wrap-godot")
                    .to_string_lossy()
                    .into_owned(),
                "--flag".to_string(),
            ]
        );
    }

    #[test]
    fn project_config_resolves_relative_windowed_prefix_path_from_project_root() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(
            temp_dir.join("qa/config.yaml"),
            "godot:\n  windowed_launch_prefix:\n    - tools/display-wrap\n    - --flag\n",
        )
        .expect("write config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        assert_eq!(
            config.godot_windowed_launch_prefix,
            vec![
                temp_dir
                    .join("tools/display-wrap")
                    .to_string_lossy()
                    .into_owned(),
                "--flag".to_string(),
            ]
        );
    }

    #[test]
    fn project_config_rejects_invalid_windowed_launch_prefix_shape() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(
            temp_dir.join("qa/config.yaml"),
            "godot:\n  windowed_launch_prefix: xvfb-run\n",
        )
        .expect("write config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        assert!(config.godot_windowed_launch_prefix.is_empty());
        assert_eq!(
            config
                .project_config_failure
                .as_ref()
                .map(|failure| failure.kind.as_str()),
            Some("invalid_project_config")
        );
    }

    #[test]
    fn project_config_allows_yaml_comments() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(
            temp_dir.join("qa/config.yaml"),
            "# local display wrapper\n\ngodot:\n  # argv prefix, not shell syntax\n  launch_prefix:\n    - xvfb-run\n    - -a\n",
        )
        .expect("write config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        assert_eq!(config.godot_launch_prefix, vec!["xvfb-run", "-a"]);
        assert!(config.project_config_failure.is_none());
    }

    #[test]
    fn project_config_loads_local_only_config() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(
            temp_dir.join("qa/config.local.yaml"),
            "godot:\n  launch_prefix:\n    - xvfb-run\n    - -a\n",
        )
        .expect("write local config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        assert_eq!(config.godot_launch_prefix, vec!["xvfb-run", "-a"]);
        assert!(config.project_config_failure.is_none());
    }

    #[test]
    fn project_config_local_overlay_replaces_launch_prefix() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(
            temp_dir.join("qa/config.yaml"),
            "godot:\n  launch_prefix:\n    - base-wrapper\n",
        )
        .expect("write base config");
        fs::write(
            temp_dir.join("qa/config.local.yaml"),
            "godot:\n  launch_prefix:\n    - local-wrapper\n    - --local\n",
        )
        .expect("write local config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        assert_eq!(config.godot_launch_prefix, vec!["local-wrapper", "--local"]);
        assert!(config.project_config_failure.is_none());
    }

    #[test]
    fn project_config_local_empty_launch_prefix_clears_base() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(
            temp_dir.join("qa/config.yaml"),
            "godot:\n  launch_prefix:\n    - base-wrapper\n",
        )
        .expect("write base config");
        fs::write(
            temp_dir.join("qa/config.local.yaml"),
            "godot:\n  launch_prefix: []\n",
        )
        .expect("write local config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        assert!(config.godot_launch_prefix.is_empty());
        assert!(config.project_config_failure.is_none());
    }

    #[test]
    fn project_config_local_missing_key_inherits_base() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(
            temp_dir.join("qa/config.yaml"),
            "godot:\n  launch_prefix:\n    - base-wrapper\n",
        )
        .expect("write base config");
        fs::write(temp_dir.join("qa/config.local.yaml"), "# no overrides\n")
            .expect("write local config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        assert_eq!(config.godot_launch_prefix, vec!["base-wrapper"]);
        assert!(config.project_config_failure.is_none());
    }

    #[test]
    fn project_config_ignores_config_json() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(
            temp_dir.join("qa/config.json"),
            r#"{"godot":{"launch_prefix":["json-wrapper"]}}"#,
        )
        .expect("write json config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        assert!(config.godot_launch_prefix.is_empty());
        assert!(config.project_config_failure.is_none());
        // The ignored file is not silent anymore: it surfaces exactly one warning.
        assert_eq!(config.config_warnings.len(), 1);
        let warning = &config.config_warnings[0];
        assert_eq!(warning["type"], "unsupported_config_file");
        assert_eq!(warning["severity"], "warning");
        assert_eq!(warning["path"], "qa/config.json");
        assert_eq!(
            warning["expected"],
            "qa/config.yaml (or qa/config.local.yaml)"
        );
        assert!(warning["remediation"].is_string());
    }

    #[test]
    fn project_config_warns_for_config_yml_spelling() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(
            temp_dir.join("qa/config.yml"),
            "godot:\n  launch_prefix:\n    - yml-wrapper\n",
        )
        .expect("write yml config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        // The `.yml` spelling is ignored (not read) but surfaces the warning with its path.
        assert!(config.godot_launch_prefix.is_empty());
        assert!(config.project_config_failure.is_none());
        assert_eq!(config.config_warnings.len(), 1);
        assert_eq!(config.config_warnings[0]["type"], "unsupported_config_file");
        assert_eq!(config.config_warnings[0]["path"], "qa/config.yml");
    }

    #[test]
    fn project_config_supported_yaml_carries_no_config_warning() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(
            temp_dir.join("qa/config.yaml"),
            "godot:\n  launch_prefix:\n    - yaml-wrapper\n",
        )
        .expect("write yaml config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        assert_eq!(config.godot_launch_prefix, vec!["yaml-wrapper"]);
        assert!(config.project_config_failure.is_none());
        assert!(config.config_warnings.is_empty());
    }

    #[test]
    fn project_config_rejects_invalid_launch_prefix_shape() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(
            temp_dir.join("qa/config.yaml"),
            "godot:\n  launch_prefix: xvfb-run -a\n",
        )
        .expect("write config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        assert!(config.godot_launch_prefix.is_empty());
        assert_eq!(
            config
                .project_config_failure
                .as_ref()
                .map(|failure| failure.kind.as_str()),
            Some("invalid_project_config")
        );
    }

    #[test]
    fn project_config_rejects_empty_launch_prefix_entry() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(
            temp_dir.join("qa/config.yaml"),
            "godot:\n  launch_prefix:\n    - xvfb-run\n    - \"\"\n",
        )
        .expect("write config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        assert!(config.godot_launch_prefix.is_empty());
        assert_eq!(
            config
                .project_config_failure
                .as_ref()
                .map(|failure| failure.kind.as_str()),
            Some("invalid_project_config")
        );
    }

    #[test]
    fn project_config_rejects_malformed_local_yaml_with_local_path() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(
            temp_dir.join("qa/config.yaml"),
            "godot:\n  launch_prefix:\n    - base-wrapper\n",
        )
        .expect("write base config");
        fs::write(temp_dir.join("qa/config.local.yaml"), "godot: [\n").expect("write local config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);
        let failure = config.project_config_failure.clone();

        let _ = fs::remove_dir_all(&temp_dir);
        assert!(config.godot_launch_prefix.is_empty());
        assert_eq!(
            failure.as_ref().map(|failure| failure.kind.as_str()),
            Some("invalid_project_config")
        );
        assert!(
            failure
                .and_then(|failure| failure.path)
                .is_some_and(|path| std::path::Path::new(&path).ends_with("qa/config.local.yaml"))
        );
    }

    #[test]
    fn project_config_loads_artifacts_keep_runs() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(
            temp_dir.join("qa/config.yaml"),
            "artifacts:\n  keep_runs: 3\n",
        )
        .expect("write config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        assert_eq!(config.artifacts_keep_runs, Some(3));
        assert!(config.project_config_failure.is_none());
        assert!(config.config_warnings.is_empty());
    }

    #[test]
    fn project_config_absent_artifacts_keep_runs_is_unlimited() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(temp_dir.join("qa/config.yaml"), "scenario:\n  warm: true\n")
            .expect("write config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        // Unset means unlimited: nothing is ever pruned, which is the historical behaviour.
        assert_eq!(config.artifacts_keep_runs, None);
    }

    #[test]
    fn project_config_local_overlay_replaces_artifacts_keep_runs() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(
            temp_dir.join("qa/config.yaml"),
            "artifacts:\n  keep_runs: 10\n",
        )
        .expect("write base config");
        fs::write(
            temp_dir.join("qa/config.local.yaml"),
            "artifacts:\n  keep_runs: 2\n",
        )
        .expect("write local config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        assert_eq!(config.artifacts_keep_runs, Some(2));
        assert!(config.project_config_failure.is_none());
    }

    #[test]
    fn project_config_rejects_non_positive_artifacts_keep_runs() {
        for raw in ["0", "-1"] {
            let temp_dir = unique_temp_dir();
            fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
            fs::write(
                temp_dir.join("qa/config.yaml"),
                format!("artifacts:\n  keep_runs: {raw}\n"),
            )
            .expect("write config");

            let config = load_project_config(Some(&temp_dir), None, None, false, None);

            let _ = fs::remove_dir_all(&temp_dir);
            assert_eq!(config.artifacts_keep_runs, None, "keep_runs: {raw}");
            assert_eq!(
                config
                    .project_config_failure
                    .as_ref()
                    .map(|failure| failure.kind.as_str()),
                Some("invalid_project_config"),
                "keep_runs: {raw}"
            );
        }
    }

    #[test]
    fn project_config_rejects_non_integer_artifacts_keep_runs() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(
            temp_dir.join("qa/config.yaml"),
            "artifacts:\n  keep_runs: plenty\n",
        )
        .expect("write config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        assert_eq!(config.artifacts_keep_runs, None);
        assert_eq!(
            config
                .project_config_failure
                .as_ref()
                .map(|failure| failure.kind.as_str()),
            Some("invalid_project_config")
        );
    }

    #[test]
    fn project_config_rejects_non_object_artifacts_section() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(temp_dir.join("qa/config.yaml"), "artifacts: keep_runs\n").expect("write config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        assert_eq!(config.artifacts_keep_runs, None);
        assert_eq!(
            config
                .project_config_failure
                .as_ref()
                .map(|failure| failure.kind.as_str()),
            Some("invalid_project_config")
        );
    }

    #[test]
    fn project_config_warns_for_unknown_keys_at_every_level() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(
            temp_dir.join("qa/config.yaml"),
            concat!(
                "warmStrict: true\n",
                "godot:\n  launch_mode: headless\n  audioMode: on\n",
                "scenario:\n  warm: true\n  parallelism: 8\n",
                "artifacts:\n  keep_runs: 2\n  keepRuns: 4\n",
            ),
        )
        .expect("write config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        // A typo must never fail the command, and the recognised siblings still apply.
        assert!(config.project_config_failure.is_none());
        assert_eq!(config.godot_launch_mode, GodotLaunchMode::Headless);
        assert!(config.scenario_warm_default);
        assert_eq!(config.artifacts_keep_runs, Some(2));

        let keys: Vec<&str> = config
            .config_warnings
            .iter()
            .map(|warning| warning["key"].as_str().expect("key string"))
            .collect();
        assert_eq!(
            keys,
            vec![
                "warmStrict",
                "godot.audioMode",
                "scenario.parallelism",
                "artifacts.keepRuns"
            ]
        );
        for warning in &config.config_warnings {
            assert_eq!(warning["type"], "unknown_config_key");
            assert_eq!(warning["severity"], "warning");
            assert_eq!(warning["path"], "qa/config.yaml");
            assert!(warning["expected"].is_array());
            assert!(warning["remediation"].is_string());
        }
        assert_eq!(
            config.config_warnings[3]["expected"],
            serde_json::json!(["keep_runs"])
        );
    }

    #[test]
    fn project_config_unknown_key_warning_reports_the_local_overlay_path() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(temp_dir.join("qa/config.yaml"), "scenario:\n  warm: true\n")
            .expect("write base config");
        fs::write(
            temp_dir.join("qa/config.local.yaml"),
            "scenario:\n  warm_strict_mode: true\n",
        )
        .expect("write local config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        assert_eq!(config.config_warnings.len(), 1);
        assert_eq!(config.config_warnings[0]["type"], "unknown_config_key");
        assert_eq!(
            config.config_warnings[0]["key"],
            "scenario.warm_strict_mode"
        );
        assert_eq!(config.config_warnings[0]["path"], "qa/config.local.yaml");
    }

    #[test]
    fn project_config_loads_scenario_schedule() {
        for (raw, expected) in [
            ("manifest", ScenarioSchedule::Manifest),
            ("longest-first", ScenarioSchedule::LongestFirst),
            ("longest_first", ScenarioSchedule::LongestFirst),
        ] {
            let temp_dir = unique_temp_dir();
            fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
            fs::write(
                temp_dir.join("qa/config.yaml"),
                format!("scenario:\n  schedule: {raw}\n"),
            )
            .expect("write config");

            let config = load_project_config(Some(&temp_dir), None, None, false, None);

            let _ = fs::remove_dir_all(&temp_dir);
            assert_eq!(
                config.scenario_schedule_default,
                Some(expected),
                "schedule: {raw}"
            );
            assert!(config.project_config_failure.is_none(), "schedule: {raw}");
            assert!(config.config_warnings.is_empty(), "schedule: {raw}");
        }
    }

    #[test]
    fn project_config_absent_scenario_schedule_is_unset() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(temp_dir.join("qa/config.yaml"), "scenario:\n  warm: true\n")
            .expect("write config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        // Unset leaves the choice to the runner default rather than pinning an order here.
        assert_eq!(config.scenario_schedule_default, None);
    }

    #[test]
    fn project_config_local_overlay_replaces_scenario_schedule() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(
            temp_dir.join("qa/config.yaml"),
            "scenario:\n  schedule: longest-first\n",
        )
        .expect("write base config");
        fs::write(
            temp_dir.join("qa/config.local.yaml"),
            "scenario:\n  schedule: manifest\n",
        )
        .expect("write local config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        assert_eq!(
            config.scenario_schedule_default,
            Some(ScenarioSchedule::Manifest)
        );
        assert!(config.project_config_failure.is_none());
    }

    #[test]
    fn project_config_rejects_invalid_scenario_schedule() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(
            temp_dir.join("qa/config.yaml"),
            "scenario:\n  schedule: shortest-first\n",
        )
        .expect("write config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        assert_eq!(config.scenario_schedule_default, None);
        assert_eq!(
            config
                .project_config_failure
                .as_ref()
                .map(|failure| failure.kind.as_str()),
            Some("invalid_project_config")
        );
    }

    #[test]
    fn project_config_rejects_non_string_scenario_schedule() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(
            temp_dir.join("qa/config.yaml"),
            "scenario:\n  schedule: 3\n",
        )
        .expect("write config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        assert_eq!(config.scenario_schedule_default, None);
        assert_eq!(
            config
                .project_config_failure
                .as_ref()
                .map(|failure| failure.kind.as_str()),
            Some("invalid_project_config")
        );
    }

    #[test]
    fn project_config_loads_log_ignore_patterns() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(
            temp_dir.join("qa/config.yaml"),
            "logs:\n  ignore:\n    - \"ObjectDB instances leaked\"\n    - vulkan\n",
        )
        .expect("write config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        assert_eq!(
            config.log_ignore_patterns,
            vec!["ObjectDB instances leaked", "vulkan"]
        );
        assert!(config.project_config_failure.is_none());
        assert!(config.config_warnings.is_empty());
    }

    #[test]
    fn project_config_absent_log_ignore_patterns_is_empty() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        assert!(config.log_ignore_patterns.is_empty());
    }

    #[test]
    fn project_config_local_overlay_replaces_log_ignore_patterns() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(
            temp_dir.join("qa/config.yaml"),
            "logs:\n  ignore:\n    - base-noise\n",
        )
        .expect("write base config");
        fs::write(
            temp_dir.join("qa/config.local.yaml"),
            "logs:\n  ignore:\n    - local-noise\n",
        )
        .expect("write local config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        // Array overlay semantics match `godot.args`: the local array replaces the base one.
        assert_eq!(config.log_ignore_patterns, vec!["local-noise"]);
        assert!(config.project_config_failure.is_none());
    }

    #[test]
    fn project_config_local_empty_log_ignore_patterns_clears_base() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(
            temp_dir.join("qa/config.yaml"),
            "logs:\n  ignore:\n    - base-noise\n",
        )
        .expect("write base config");
        fs::write(
            temp_dir.join("qa/config.local.yaml"),
            "logs:\n  ignore: []\n",
        )
        .expect("write local config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        // An empty local array explicitly clears the project default, like `godot.args: []`.
        assert!(config.log_ignore_patterns.is_empty());
        assert!(config.project_config_failure.is_none());
    }

    #[test]
    fn project_config_rejects_non_string_log_ignore_entry() {
        for raw in ["    - 3\n", "    - \"\"\n"] {
            let temp_dir = unique_temp_dir();
            fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
            fs::write(
                temp_dir.join("qa/config.yaml"),
                format!("logs:\n  ignore:\n{raw}"),
            )
            .expect("write config");

            let config = load_project_config(Some(&temp_dir), None, None, false, None);

            let _ = fs::remove_dir_all(&temp_dir);
            assert!(config.log_ignore_patterns.is_empty(), "entry: {raw:?}");
            assert_eq!(
                config
                    .project_config_failure
                    .as_ref()
                    .map(|failure| failure.kind.as_str()),
                Some("invalid_project_config"),
                "entry: {raw:?}"
            );
        }
    }

    #[test]
    fn project_config_rejects_non_object_logs_section() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(temp_dir.join("qa/config.yaml"), "logs: ignore\n").expect("write config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        assert!(config.log_ignore_patterns.is_empty());
        assert_eq!(
            config
                .project_config_failure
                .as_ref()
                .map(|failure| failure.kind.as_str()),
            Some("invalid_project_config")
        );
    }

    #[test]
    fn project_config_warns_for_unknown_logs_key() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(temp_dir.join("qa")).expect("create qa dir");
        fs::write(
            temp_dir.join("qa/config.yaml"),
            "logs:\n  ignore:\n    - real-noise\n  bogus: true\n",
        )
        .expect("write config");

        let config = load_project_config(Some(&temp_dir), None, None, false, None);

        let _ = fs::remove_dir_all(&temp_dir);
        // A typo under `logs` warns exactly like `scenario.bogus`, and the sibling applies.
        assert!(config.project_config_failure.is_none());
        assert_eq!(config.log_ignore_patterns, vec!["real-noise"]);
        assert_eq!(config.config_warnings.len(), 1);
        assert_eq!(config.config_warnings[0]["type"], "unknown_config_key");
        assert_eq!(config.config_warnings[0]["key"], "logs.bogus");
        assert_eq!(config.config_warnings[0]["path"], "qa/config.yaml");
        assert_eq!(
            config.config_warnings[0]["expected"],
            serde_json::json!(["ignore"])
        );
    }

    fn unique_temp_dir() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "godot-qa-core-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }
}
