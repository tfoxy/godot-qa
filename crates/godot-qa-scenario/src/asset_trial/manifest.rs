use super::*;
use std::collections::BTreeSet;
use std::io::Read;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub format: String,
    pub scene: String,
    pub validation: Validation,
    pub bindings: Vec<Binding>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Validation {
    pub scenario: Option<String>,
    pub scenario_file: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub id: String,
    pub source: String,
    pub destination: String,
    pub edit_path: String,
    pub runtime_target: String,
    pub property: String,
    pub evidence: Evidence,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub snapshot: Option<String>,
    pub audio: Option<String>,
    pub state_before: String,
    pub state_after: String,
}
impl Evidence {
    pub fn name(&self) -> &str {
        self.snapshot
            .as_deref()
            .or(self.audio.as_deref())
            .unwrap_or_default()
    }
}

pub(super) struct Prepared {
    pub manifest: Manifest,
    pub scenario_path: PathBuf,
    pub scenario: Value,
    pub sources: Vec<Vec<u8>>,
}

pub(super) fn project_path(root: &Path, resource: &str) -> Result<PathBuf, FailureDetail> {
    let relative = resource.strip_prefix("res://").ok_or_else(|| {
        fail(
            "asset_trial_path_invalid",
            resource,
            "project-local res:// path required",
        )
    })?;
    if relative.is_empty()
        || relative.contains('\\')
        || Path::new(relative)
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err(fail(
            "asset_trial_path_invalid",
            resource,
            "path traversal, absolute paths, and alternate separators are forbidden",
        ));
    }
    let path = root.join(relative);
    no_links(root, &path)?;
    Ok(path)
}
pub(super) fn no_links(root: &Path, path: &Path) -> Result<(), FailureDetail> {
    let relative = path.strip_prefix(root).map_err(|_| {
        fail(
            "asset_trial_path_invalid",
            &path.display().to_string(),
            "path leaves selected project",
        )
    })?;
    let mut current = root.to_path_buf();
    for part in relative.components() {
        if !matches!(part, std::path::Component::Normal(_)) {
            return Err(fail(
                "asset_trial_path_invalid",
                &path.display().to_string(),
                "non-normal path component",
            ));
        }
        current.push(part);
        if let Ok(meta) = fs::symlink_metadata(&current) {
            if godot_qa_process::directory_link::is_link(&meta) {
                return Err(fail(
                    "asset_trial_path_invalid",
                    &current.display().to_string(),
                    "symbolic links and directory reparse points are unsupported",
                ));
            }
        }
    }
    Ok(())
}
pub(super) fn bounded_read(path: &Path, max: u64) -> Result<Vec<u8>, FailureDetail> {
    let file = fs::File::open(path).map_err(|e| io_failure(path, e))?;
    let mut bytes = Vec::new();
    file.take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| io_failure(path, e))?;
    if bytes.len() as u64 > max {
        return Err(fail(
            "asset_trial_asset_invalid",
            &path.display().to_string(),
            "input exceeds size limit",
        ));
    }
    Ok(bytes)
}
pub(super) fn media(bytes: &[u8], extension: &str) -> Result<(), FailureDetail> {
    let invalid = || {
        fail(
            "asset_trial_asset_invalid",
            extension,
            "expected PNG <=4096x4096 or PCM16 mono/stereo WAV of positive duration <=30s, at most 32 MiB",
        )
    };
    if bytes.len() > 32 * 1024 * 1024 {
        return Err(invalid());
    }
    match extension {
        "png" => {
            if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" {
                return Err(invalid());
            }
            let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
            let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
            if width == 0 || height == 0 || width > 4096 || height > 4096 {
                return Err(invalid());
            }
            image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
                .map_err(|_| invalid())?;
        }
        "wav" => {
            if bytes.len() < 12
                || &bytes[..4] != b"RIFF"
                || &bytes[8..12] != b"WAVE"
                || u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize + 8 != bytes.len()
            {
                return Err(invalid());
            }
            let mut offset = 12;
            let mut format = None;
            let mut data = None;
            while offset + 8 <= bytes.len() {
                let size =
                    u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize;
                let end = offset
                    .checked_add(8)
                    .and_then(|o| o.checked_add(size))
                    .filter(|e| *e <= bytes.len())
                    .ok_or_else(invalid)?;
                if &bytes[offset..offset + 4] == b"fmt " {
                    if size < 16 || format.is_some() {
                        return Err(invalid());
                    }
                    let b = &bytes[offset + 8..end];
                    format = Some((
                        u16::from_le_bytes(b[0..2].try_into().unwrap()),
                        u16::from_le_bytes(b[2..4].try_into().unwrap()),
                        u32::from_le_bytes(b[4..8].try_into().unwrap()),
                        u16::from_le_bytes(b[12..14].try_into().unwrap()),
                        u16::from_le_bytes(b[14..16].try_into().unwrap()),
                    ));
                }
                if &bytes[offset..offset + 4] == b"data" {
                    if data.is_some() {
                        return Err(invalid());
                    }
                    data = Some(size);
                }
                offset = end + (size % 2);
            }
            let (encoding, channels, rate, align, bits) = format.ok_or_else(invalid)?;
            let size = data.ok_or_else(invalid)?;
            if encoding != 1
                || !(1..=2).contains(&channels)
                || rate == 0
                || align != channels * 2
                || bits != 16
                || size == 0
                || size % usize::from(align) != 0
                || size as f64 / f64::from(rate) / f64::from(align) > 30.0
            {
                return Err(invalid());
            }
        }
        _ => return Err(invalid()),
    }
    Ok(())
}

pub(super) fn prepare(
    config: &ProjectConfig,
    manifest_path: &str,
) -> Result<Prepared, FailureDetail> {
    if let Some(f) = config.project_config_failure_for_command("asset try") {
        return Err(f);
    }
    let root = &config.project_root;
    for name in [
        "qa_asset_trial_runner.gd",
        "qa_asset_trial_probe.gd",
        "qa_asset_trial_collector.gd",
        "qa_asset_trial_scene.gd",
    ] {
        let path = config.addon_dir.join(name);
        if !path.is_file() {
            return Err(fail(
                "addon_capability_missing",
                &path.display().to_string(),
                "refresh the installed addon; asset_trial_v1 requires writer, probe, and collector",
            ));
        }
    }
    let path = root.join(manifest_path);
    let bytes = bounded_read(&path, 1024 * 1024)?;
    let value = godot_qa_core::yaml::parse_yaml_document(&String::from_utf8_lossy(&bytes))
        .map_err(|e| {
            fail(
                "asset_trial_manifest_invalid",
                manifest_path,
                &e.to_string(),
            )
        })?;
    let manifest: Manifest = serde_json::from_value(value).map_err(|e| {
        fail(
            "asset_trial_manifest_invalid",
            manifest_path,
            &e.to_string(),
        )
    })?;
    if manifest.format != "godot-qa.asset-trial.v1"
        || manifest.bindings.is_empty()
        || manifest.bindings.len() > 64
    {
        return Err(fail(
            "asset_trial_manifest_invalid",
            manifest_path,
            "expected godot-qa.asset-trial.v1 and 1..64 bindings",
        ));
    }
    let scene = project_path(root, &manifest.scene)?;
    if scene.extension().and_then(|s| s.to_str()) != Some("tscn") || !scene.is_file() {
        return Err(fail(
            "asset_trial_scene_invalid",
            &manifest.scene,
            "edited scene must be an existing .tscn",
        ));
    }
    let parsed = godot_qa_scene::parse_scene(
        &fs::read_to_string(&scene).map_err(|e| io_failure(&scene, e))?,
    );
    if parsed.has_errors() || parsed.root().is_none_or(|node| node.is_instance()) {
        return Err(fail(
            "asset_trial_scene_unsupported",
            &manifest.scene,
            "inherited or unparseable edited scenes are unsupported",
        ));
    }
    for binding in &manifest.bindings {
        let node = parsed
            .nodes
            .iter()
            .find(|n| n.relative_path() == binding.edit_path)
            .ok_or_else(|| {
                fail(
                    "asset_trial_target_invalid",
                    &binding.edit_path,
                    "edit target is absent from the saved scene",
                )
            })?;
        let allowed = match binding.property.as_str() {
            "texture" => matches!(node.type_name.as_deref(), Some("Sprite2D" | "TextureRect")),
            "stream" => matches!(
                node.type_name.as_deref(),
                Some("AudioStreamPlayer" | "AudioStreamPlayer2D")
            ),
            _ => false,
        };
        if !allowed
            || parsed.nodes.iter().any(|ancestor| {
                ancestor.is_instance()
                    && (ancestor.relative_path() == binding.edit_path
                        || binding
                            .edit_path
                            .starts_with(&format!("{}/", ancestor.relative_path())))
            })
        {
            return Err(fail(
                "asset_trial_slot_unsupported",
                &binding.edit_path,
                "only direct scene-owned native Sprite2D/TextureRect texture or AudioStreamPlayer/2D stream slots are supported",
            ));
        }
    }
    let mut ids = BTreeSet::new();
    let mut slots = BTreeSet::new();
    let mut destinations = BTreeSet::new();
    let mut sources = Vec::new();
    for binding in &manifest.bindings {
        let dest = project_path(root, &binding.destination)?;
        if binding.id.is_empty()
            || binding.id.len() > 64
            || !binding
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            || !ids.insert(&binding.id)
            || !slots.insert((&binding.edit_path, &binding.property))
            || !destinations.insert(&binding.destination)
        {
            return Err(fail(
                "asset_trial_manifest_invalid",
                &binding.id,
                "duplicate/empty ID, destination, or edit slot",
            ));
        }
        if dest.exists()
            || sidecar(&dest, "import").exists()
            || sidecar(&dest, "uid").exists()
            || !dest.parent().is_some_and(Path::is_dir)
        {
            return Err(fail(
                "asset_trial_destination_exists",
                &binding.destination,
                "destination and sidecars must be absent; parent directory must already exist",
            ));
        }
        if binding.edit_path.is_empty()
            || binding.edit_path.starts_with('/')
            || binding.edit_path.contains(':')
            || binding.edit_path.split('/').any(|p| p == "..")
            || !(binding.runtime_target.starts_with("qa:")
                || binding.runtime_target.starts_with("path:"))
        {
            return Err(fail(
                "asset_trial_target_invalid",
                &binding.id,
                "use scene-root-relative edit_path and an explicit qa:/path: runtime locator",
            ));
        }
        let ext = dest
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default();
        if !matches!(
            (
                ext,
                binding.property.as_str(),
                binding.evidence.snapshot.is_some(),
                binding.evidence.audio.is_some()
            ),
            ("png", "texture", true, false) | ("wav", "stream", false, true)
        ) {
            return Err(fail(
                "asset_trial_slot_unsupported",
                &binding.id,
                "PNG texture needs snapshot evidence; WAV stream needs audio evidence",
            ));
        }
        let source = root.join(&binding.source);
        if source.extension().and_then(|e| e.to_str()) != Some(ext) {
            return Err(fail(
                "asset_trial_asset_invalid",
                &binding.source,
                "source and destination extensions must match",
            ));
        }
        let bytes = bounded_read(&source, 32 * 1024 * 1024)?;
        media(&bytes, ext)?;
        sources.push(bytes);
    }
    let scenario_path = match (
        &manifest.validation.scenario,
        &manifest.validation.scenario_file,
    ) {
        (Some(name), None) => {
            let paths = crate::scenario_doc::find_scenarios_by_name(&config.scenarios_dir, name);
            if paths.len() != 1 {
                return Err(fail(
                    "asset_trial_scenario_invalid",
                    name,
                    "scenario must resolve uniquely",
                ));
            }
            paths[0].clone()
        }
        (None, Some(path)) => root.join(path),
        _ => {
            return Err(fail(
                "asset_trial_manifest_invalid",
                manifest_path,
                "validation requires exactly one scenario or scenario_file",
            ));
        }
    };
    no_links(root, &scenario_path)?;
    let scenario = crate::scenario_doc::load_and_normalize_scenario_document(&scenario_path)
        .map_err(|e| e.failure)?;
    validate_evidence(&manifest, &scenario)?;
    // Evidence below the resource tree must already be excluded from import.
    if config.artifacts_dir.starts_with(root)
        && !config
            .artifacts_dir
            .ancestors()
            .take_while(|p| p.starts_with(root))
            .any(|p| p.join(".gdignore").is_file())
    {
        return Err(fail(
            "asset_trial_artifacts_scanned",
            &config.artifacts_dir.display().to_string(),
            "configure evidence outside the project or add a .gdignore before starting a trial",
        ));
    }
    Ok(Prepared {
        manifest,
        scenario_path,
        scenario,
        sources,
    })
}
pub(super) fn validate_evidence(
    manifest: &Manifest,
    scenario: &Value,
) -> Result<(), FailureDetail> {
    let invalid = |message: &str| fail("asset_trial_evidence_invalid", &manifest.scene, message);
    if scenario
        .get("viewports")
        .and_then(Value::as_array)
        .is_some_and(|v| v.len() > 1)
    {
        return Err(invalid("one viewport is required"));
    }
    if scenario
        .get("assert")
        .and_then(Value::as_array)
        .is_none_or(|a| {
            !a.iter().any(|assertion| {
                assertion.as_object().is_some_and(|fields| {
                    fields.keys().any(|key| {
                        !matches!(
                            key.as_str(),
                            "no_errors" | "no_warnings" | "screenshot_match" | "layout_audit"
                        )
                    })
                })
            })
        })
    {
        return Err(invalid("at least one gameplay assertion is required"));
    }
    for key in ["seed", "fixed_fps", "locale"] {
        if scenario.pointer(&format!("/determinism/{key}")).is_none() {
            return Err(invalid(&format!("declare determinism.{key}")));
        }
    }
    if scenario
        .pointer("/viewport/width")
        .and_then(Value::as_i64)
        .is_none_or(|v| v <= 0)
        || scenario
            .pointer("/viewport/height")
            .and_then(Value::as_i64)
            .is_none_or(|v| v <= 0)
    {
        return Err(invalid(
            "declare a positive deterministic viewport.width and viewport.height",
        ));
    }
    if !scenario["project_args"].is_array() {
        return Err(invalid(
            "declare project_args explicitly, including [] when unused",
        ));
    }
    let steps = scenario["steps"]
        .as_array()
        .ok_or_else(|| invalid("scenario steps required"))?;
    for binding in &manifest.bindings {
        let e = &binding.evidence;
        let key = if e.snapshot.is_some() {
            "snapshot"
        } else {
            "audio_capture"
        };
        let positions: Vec<_> = steps
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                s[key].as_str() == Some(e.name()) || s[key]["name"].as_str() == Some(e.name())
            })
            .map(|(i, _)| i)
            .collect();
        if positions.len() != 1 {
            return Err(invalid("evidence capture must exist exactly once"));
        }
        let i = positions[0];
        if i == 0
            || i + 1 >= steps.len()
            || steps[i - 1]["record_state"]["name"].as_str() != Some(&e.state_before)
            || steps[i + 1]["record_state"]["name"].as_str() != Some(&e.state_after)
        {
            return Err(invalid(
                "named record_state steps must immediately surround each selected capture",
            ));
        }
        if e.snapshot.is_some() {
            for assertion in scenario["assert"].as_array().into_iter().flatten() {
                if assertion["screenshot_match"]["snapshot"].as_str() == Some(e.name()) {
                    return Err(invalid(
                        "selected snapshot has screenshot_match; create a separate asset-review scenario",
                    ));
                }
            }
            if scenario.pointer("/snapshots/save_screenshot") == Some(&Value::Bool(false))
                || scenario.pointer("/artifacts/screenshots") == Some(&Value::Bool(false))
                || scenario.pointer("/artifacts/snapshots") == Some(&Value::Bool(false))
            {
                return Err(invalid("snapshot evidence must be enabled"));
            }
        }
    }
    Ok(())
}
