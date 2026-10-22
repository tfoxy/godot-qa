//! Durable, provider-neutral asset trials. The engine owns binding facts and
//! scene serialization; this module owns validation, evidence and file custody.
mod evidence;
mod journal;
mod manifest;
#[cfg(test)]
mod tests;

use godot_qa_core::config::ProjectConfig;
use godot_qa_core::results::{CommandResult, FailureDetail};
use journal::*;
use manifest::{Binding, Manifest};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

fn fail(kind: &str, target: &str, message: &str) -> FailureDetail {
    FailureDetail {
        kind: kind.into(),
        command: Some("asset try".into()),
        target: Some(target.into()),
        message: Some(message.into()),
        expected: Some(json!(
            "verified, unchanged trial inputs and durable recovery evidence"
        )),
        actual: Some(json!(message)),
        ..Default::default()
    }
}
fn io_failure(path: &Path, error: std::io::Error) -> FailureDetail {
    let mut f = fail(
        "asset_trial_io_failed",
        &path.display().to_string(),
        &error.to_string(),
    );
    f.details
        .insert("io_kind".into(), json!(format!("{:?}", error.kind())));
    f.path = Some(path.display().to_string());
    f
}
fn sidecar(path: &Path, extension: &str) -> PathBuf {
    PathBuf::from(format!("{}.{extension}", path.display()))
}
fn random_id() -> Result<String, FailureDetail> {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b)
        .map_err(|e| fail("asset_trial_random_failed", "trial ID", &e.to_string()))?;
    Ok(b.iter().map(|v| format!("{v:02x}")).collect())
}
fn canonical_config(config: &ProjectConfig) -> Result<ProjectConfig, FailureDetail> {
    let mut c = config.clone();
    c.project_root =
        fs::canonicalize(&config.project_root).map_err(|e| io_failure(&config.project_root, e))?;
    Ok(c)
}
fn error_result(command: &str, mut f: FailureDetail) -> CommandResult {
    f.command = Some(command.into());
    CommandResult {
        ok: false,
        command: command.into(),
        message: format!("{command} failed"),
        data: Default::default(),
        failures: vec![f],
        exit_code: 1,
    }
}
fn result(command: &str, j: &Journal) -> CommandResult {
    let mut r = CommandResult::success(command, format!("asset trial {}: {}", j.id, j.state));
    r.data=json!({"trial_id":j.id,"state":j.state,"phase":j.phase,"intent":j.intent,"technical_status":if j.phase=="evaluated" && j.failures.is_empty(){"pass"}else if j.failures.is_empty(){"incomplete"}else{"fail"},"review_status":"unreviewed","journal":j.dir().join("journal.json"),"backup":j.dir().join("original.tscn"),"evidence_root":j.evidence_root,"review_index":j.dir().join("review.json"),"baseline":j.baseline,"candidate":j.candidate,"warnings":j.warnings,"scope":"The edited shared scene affects every instance; runtime evidence verifies only the explicitly selected instance.","recovery":format!("Keep the journal and backup. Recover with godot-qa asset discard {} --project {} --json",j.id,j.project.display())}).as_object().unwrap().clone();
    r.failures = j.failures.clone();
    if !j.failures.is_empty() && command != "asset status" {
        r.ok = false;
        r.exit_code = 1;
    }
    r
}
fn finish_error(command: &str, j: &mut Journal, mut f: FailureDetail) -> CommandResult {
    f.command = Some(command.into());
    f.artifacts=Some(json!({"journal":j.dir().join("journal.json").display().to_string(),"backup":j.dir().join("original.tscn").display().to_string(),"evidence_root":j.evidence_root.display().to_string()}).as_object().unwrap().clone());
    f.details.insert("phase".into(), json!(j.phase));
    f.details.insert(
        "recovery".into(),
        json!(format!(
            "godot-qa asset discard {} --project {} --json",
            j.id,
            j.project.display()
        )),
    );
    j.state = if f.kind == "asset_trial_conflict" {
        "conflict"
    } else {
        "recovery_required"
    }
    .into();
    j.failures.push(f);
    if let Err(f) = j.save() {
        j.failures.push(f);
    }
    result(command, j)
}
fn with_exclusive(
    config: &ProjectConfig,
    command: &str,
    operation: impl FnOnce(&ProjectConfig) -> CommandResult,
) -> CommandResult {
    let config = match canonical_config(config) {
        Ok(c) => c,
        Err(f) => return error_result(command, f),
    };
    let _lease =
        match godot_qa_process::project_access::ProjectLease::exclusive(&config.project_root) {
            Ok(v) => v,
            Err(f) => return error_result(command, f),
        };
    let _scope = godot_qa_process::project_access::CoordinatorScope::enter(&config.project_root);
    if let Err(f) = check_legacy_sessions(&config) {
        return error_result(command, f);
    }
    operation(&config)
}

pub fn run_asset_try(config: &ProjectConfig, manifest_path: &str) -> CommandResult {
    with_exclusive(config, "asset try", |config| {
        let root = config.project_root.join(".qa/asset-trials");
        let active = root.join("active.json");
        if active.exists() {
            return error_result(
                "asset try",
                fail(
                    "asset_trial_active",
                    &active.display().to_string(),
                    "discard or keep the active trial before starting another",
                ),
            );
        }
        let prepared = match manifest::prepare(config, manifest_path) {
            Ok(p) => p,
            Err(f) => return error_result("asset try", f),
        };
        let id = match random_id() {
            Ok(id) => id,
            Err(f) => return error_result("asset try", f),
        };
        let dir = root.join(&id);
        if let Err(e) = fs::create_dir_all(&dir) {
            return error_result("asset try", io_failure(&dir, e));
        }
        let evidence_root = config.artifacts_dir.join("asset-trials").join(&id);
        let setup = (|| {
            manifest::no_links(&config.project_root, &dir)?;
            fs::create_dir_all(&evidence_root).map_err(|e| io_failure(&evidence_root, e))?;
            write_new(
                &evidence_root.join(".asset-trial-pin"),
                b"Durable trial evidence: never prune automatically.\n",
            )?;
            let scene = manifest::project_path(&config.project_root, &prepared.manifest.scene)?;
            let original = manifest::bounded_read(&scene, 32 * 1024 * 1024)?;
            write_new(&dir.join("original.tscn"), &original)?;
            // Exercise same-filesystem no-clobber linking before source mutation.
            let feasibility = scene
                .parent()
                .unwrap()
                .join(format!(".asset-trial-{id}.probe"));
            fs::hard_link(dir.join("original.tscn"), &feasibility)
                .map_err(|e| io_failure(&feasibility, e))?;
            fs::remove_file(&feasibility).map_err(|e| io_failure(&feasibility, e))?;
            for (index, bytes) in prepared.sources.iter().enumerate() {
                write_new(&dir.join(format!("candidate-{index}.bin")), bytes)?;
            }
            let mut frozen = snapshot_tree(
                &config.project_root,
                &[evidence_root.clone(), config.artifacts_dir.clone()],
            )?;
            frozen.remove(prepared.manifest.scene.trim_start_matches("res://"));
            let j = Journal {
                format: "godot-qa.asset-trial-journal.v1".into(),
                id: id.clone(),
                project: config.project_root.clone(),
                manifest: prepared.manifest.clone(),
                scenario: prepared.scenario.clone(),
                execution_config: execution_config(config),
                source_paths: prepared
                    .manifest
                    .bindings
                    .iter()
                    .map(|b| config.project_root.join(&b.source))
                    .collect(),
                candidate_hashes: prepared
                    .sources
                    .iter()
                    .map(|b| godot_qa_core::hash::sha256_hex(b))
                    .collect(),
                state: "prepared".into(),
                phase: "prepared".into(),
                intent: None,
                original_hash: godot_qa_core::hash::sha256_hex(&original),
                output_hash: None,
                scene_uid: Some(evidence::header_uid(&String::from_utf8_lossy(&original))),
                owned: Default::default(),
                media_captures: Default::default(),
                frozen,
                evidence_root,
                evidence_hashes: Default::default(),
                baseline: None,
                candidate: None,
                import_probe: None,
                failures: vec![],
                warnings: vec![],
            };
            j.save_journal()?;
            // create_new prevents reservation replacement even if a foreign actor
            // ignores the advisory project lock.
            write_new(
                &active,
                &serde_json::to_vec(
                    &json!({"trial_id":id,"state":"prepared","coordinator_pid":std::process::id()}),
                )
                .unwrap(),
            )?;
            Ok::<_, FailureDetail>(j)
        })();
        let mut j = match setup {
            Ok(j) => j,
            Err(f) => return error_result("asset try", f),
        };
        match execute_trial(config, &mut j, &prepared) {
            Ok(()) => result("asset try", &j),
            Err(f) => {
                // Before any media publication it is safe to release reservation.
                // The journal remains readable, including baseline failure artifacts.
                if j.owned.is_empty() && j.intent.is_none() && j.output_hash.is_none() {
                    j.failures.push(f);
                    j.state = "discarded".into();
                    if let Err(f) = j.save().and_then(|_| j.clear_active()) {
                        j.failures.push(f);
                    }
                    result("asset try", &j)
                } else {
                    finish_error("asset try", &mut j, f)
                }
            }
        }
    })
}

fn execute_trial(
    config: &ProjectConfig,
    j: &mut Journal,
    p: &manifest::Prepared,
) -> Result<(), FailureDetail> {
    let scene = manifest::project_path(&j.project, &j.manifest.scene)?;
    verify_frozen(j)?;
    check_hash(&scene, &j.original_hash)?;
    let baseline = run_phase(config, j, p, "before");
    j.baseline = Some(Value::Object(baseline.data.clone()));
    j.save()?;
    if !baseline.ok {
        j.failures.extend(baseline.failures);
        return Err(fail(
            "asset_trial_baseline_failed",
            &j.id,
            "baseline gameplay/captures failed before source staging",
        ));
    }
    evidence::baseline(j, j.baseline.as_ref().unwrap())?;
    verify_frozen(j)?;
    check_hash(&scene, &j.original_hash)?;
    j.step("baseline_complete")?;
    for (i, binding) in j.manifest.bindings.clone().iter().enumerate() {
        stage_binding(j, binding, i, &mut |_| Ok(()))?;
    }
    j.step("staged")?;
    let before_import = snapshot_tree(&j.project, std::slice::from_ref(&config.artifacts_dir))?;
    for binding in &j.manifest.bindings {
        for suffix in ["import", "uid"] {
            let resource = format!("{}.{suffix}", binding.destination);
            let path = manifest::project_path(&j.project, &resource)?;
            if fs::symlink_metadata(&path).is_ok() {
                return Err(fail(
                    "asset_trial_conflict",
                    &resource,
                    "destination sidecar appeared after preflight; preserve the foreign file and discard the staged trial",
                ));
            }
        }
    }
    j.intent("import")?;
    let import = godot_qa_process::launch_godot_import_with_args(
        config.godot_binary.as_deref().unwrap_or_default(),
        &config.godot_launch_prefix,
        &config.godot_args,
        &j.project,
        &j.dir().join("import.log"),
    );
    // Inventory sidecars even on import error so recovery knows exact ownership.
    for binding in &j.manifest.bindings {
        for suffix in ["import", "uid"] {
            let resource = format!("{}.{suffix}", binding.destination);
            let path = manifest::project_path(&j.project, &resource)?;
            if path.is_file() {
                j.owned.insert(resource, file_hash(&path)?);
            }
        }
    }
    j.intent = None;
    j.save()?;
    let after_import = snapshot_tree(&j.project, std::slice::from_ref(&config.artifacts_dir))?;
    for (path, hash) in &after_import {
        if before_import.get(path) != Some(hash) && !j.owned.contains_key(&format!("res://{path}"))
        {
            j.warnings.push(format!(
                "Import changed nonowned source file {path}; it will never be undone by discard."
            ));
        }
    }
    for path in before_import.keys() {
        if !after_import.contains_key(path) && !j.owned.contains_key(&format!("res://{path}")) {
            j.warnings.push(format!(
                "Import removed nonowned source file {path}; discard never restores it."
            ));
        }
    }
    j.save()?;
    let import = import?;
    if import.returncode != 0 {
        return Err(fail(
            "asset_trial_import_failed",
            &j.dir().join("import.log").display().to_string(),
            "Godot import failed; consult retained import logs",
        ));
    }
    verify_frozen(j)?;
    check_hash(&scene, &j.original_hash)?;
    verify_owned(j)?;
    let probe = offline(
        config,
        j,
        p,
        "probe",
        json!({"resources":j.manifest.bindings.iter().map(|b|&b.destination).collect::<Vec<_>>()}),
    )?;
    validate_import(&probe)?;
    j.import_probe = Some(probe);
    j.save()?;
    let output = j.dir().join("output.tscn");
    let writer = offline(
        config,
        j,
        p,
        "write",
        json!({"scene":j.manifest.scene,"output_path":output,"bindings":j.manifest.bindings}),
    )?;
    let original =
        fs::read_to_string(j.dir().join("original.tscn")).map_err(|e| io_failure(&scene, e))?;
    let candidate = fs::read_to_string(&output).map_err(|e| io_failure(&output, e))?;
    evidence::scene_gate(&original, &candidate, &j.manifest, &writer)?;
    fs::File::open(&output)
        .and_then(|f| f.sync_all())
        .map_err(|e| io_failure(&output, e))?;
    j.output_hash = Some(file_hash(&output)?);
    j.save()?;
    verify_frozen(j)?;
    verify_owned(j)?;
    check_hash(&scene, &j.original_hash)?;
    let original_hash = j.original_hash.clone();
    replace_no_clobber(j, &scene, &output, &original_hash, "apply", &mut |_| Ok(()))?;
    fs::remove_file(&output).map_err(|e| io_failure(&output, e))?;
    sync_dir(&j.dir())?;
    j.step("applied")?;
    uid_probe(config, j, p)?;
    verify_frozen(j)?;
    verify_owned(j)?;
    check_hash(&scene, j.output_hash.as_deref().unwrap())?;
    let candidate = run_phase(config, j, p, "after");
    j.candidate = Some(Value::Object(candidate.data.clone()));
    j.save()?;
    if !candidate.ok {
        j.failures.extend(candidate.failures);
        return Err(fail(
            "asset_trial_candidate_failed",
            &j.id,
            "candidate gameplay/captures failed; retain evidence and discard to restore",
        ));
    }
    verify_frozen(j)?;
    verify_owned(j)?;
    check_hash(&scene, j.output_hash.as_deref().unwrap())?;
    evidence::candidate(j, j.candidate.as_ref().unwrap())?;
    let mut comparisons = Vec::new();
    for binding in &j.manifest.bindings {
        if binding.evidence.snapshot.is_some() {
            let before = evidence::capture_artifact(j.baseline.as_ref().unwrap(), binding)?;
            let after = evidence::capture_artifact(j.candidate.as_ref().unwrap(), binding)?;
            let diff = j.evidence_root.join(format!("{}-diff.png", binding.id));
            let metrics = j.evidence_root.join(format!("{}-metrics.json", binding.id));
            let comparison = crate::screenshots::compare_screenshot_files(
                "asset try",
                &after,
                &before,
                1.0,
                Some(&diff),
                Some(&metrics),
            );
            if !comparison.ok {
                return Err(fail(
                    "asset_trial_comparison_failed",
                    &binding.id,
                    "unable to create technical image comparison",
                ));
            }
            let contact_dir = j.evidence_root.join(format!("{}-review", binding.id));
            fs::create_dir(&contact_dir).map_err(|e| io_failure(&contact_dir, e))?;
            atomic_json(
                &contact_dir.join("manifest.json"),
                &json!({"command":"screenshot.sequence","captured_count":2,"frames":[{"index":0,"path":before},{"index":1,"path":after}]}),
            )?;
            let contact = crate::contact_sheet::build_contact_sheet(
                config,
                crate::contact_sheet::ScreenshotContactSheetOptions {
                    sequence_dir: contact_dir.display().to_string(),
                    columns: 2,
                    thumb_width: 480,
                    out: contact_dir.join("contact-sheet.png").display().to_string(),
                },
            );
            if !contact.ok {
                return Err(fail(
                    "asset_trial_comparison_failed",
                    &binding.id,
                    "contact sheet generation failed",
                ));
            }
            comparisons.push(json!({"binding":binding.id,"image_diff":comparison,"contact_sheet":contact,"ordering":["before","after"]}));
        } else {
            let mut audio_review = Vec::new();
            for (phase, data) in [
                ("before", j.baseline.as_ref().unwrap()),
                ("after", j.candidate.as_ref().unwrap()),
            ] {
                let wav = evidence::capture_artifact(data, binding)?;
                let spectrogram = j
                    .evidence_root
                    .join(format!("{}-{phase}-spectrogram.png", binding.id));
                crate::audio::write_spectrogram_artifact(&wav, &spectrogram).map_err(|e| {
                    fail("asset_trial_comparison_failed", &binding.id, &e.to_string())
                })?;
                let analysis = crate::audio::analyze_wav_file(&wav).map_err(|e| {
                    fail("asset_trial_comparison_failed", &binding.id, &e.to_string())
                })?;
                audio_review.push(json!({"phase":phase,"wav":wav,"spectrogram":spectrogram,"analysis":analysis,"attribution":"Playback overlap on a recorded bus does not prove isolated attribution or aesthetic quality."}));
            }
            comparisons.push(json!({"binding":binding.id,"audio":audio_review}));
        }
    }
    j.evidence_hashes = snapshot_tree(&j.evidence_root, &[])?;
    atomic_json(
        &j.dir().join("review.json"),
        &json!({"format":"godot-qa.asset-trial-review.v1","technical_status":"pass","review_status":"unreviewed","before":j.baseline,"after":j.candidate,"import_probe":j.import_probe,"bindings":j.manifest.bindings,"comparisons":comparisons,"journal":j.dir().join("journal.json")}),
    )?;
    j.step("evaluated")
}
fn run_phase(
    config: &ProjectConfig,
    j: &Journal,
    p: &manifest::Prepared,
    phase: &str,
) -> CommandResult {
    let mut config = config.clone();
    config.artifacts_dir = j.evidence_root.join(phase);
    config.artifacts_keep_runs = None;
    let mut scenario = p.scenario.clone();
    scenario["_asset_trial"] = json!({"version":1,"bindings":j.manifest.bindings});
    crate::run::run_scenario_once(
        &config,
        &p.scenario_path,
        &scenario,
        "godot-qa asset try",
        None,
        None,
        Default::default(),
    )
}
fn offline(
    config: &ProjectConfig,
    j: &Journal,
    p: &manifest::Prepared,
    operation: &str,
    extra: Value,
) -> Result<Value, FailureDetail> {
    let request_path = j.dir().join(format!("{operation}-request.json"));
    let result_path = j.dir().join(format!("{operation}-result.json"));
    let logs = j.dir().join(format!("{operation}.log"));
    let mut request = extra;
    request["version"] = json!(1);
    request["operation"] = json!(if operation == "keep-probe" {
        "probe"
    } else {
        operation
    });
    request["result_path"] = json!(result_path);
    atomic_json(&request_path, &request)?;
    if result_path.exists() {
        fs::remove_file(&result_path).map_err(|e| io_failure(&result_path, e))?;
    }
    let headless = j
        .baseline
        .as_ref()
        .and_then(|v| v["headless"].as_bool())
        .unwrap_or(false);
    let audio = j
        .baseline
        .as_ref()
        .and_then(|v| v["audio"].as_bool())
        .unwrap_or(true);
    let args: Vec<String> = p.scenario["godot_args"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    let run =
        godot_qa_process::launch_godot_asset_trial_runner(godot_qa_process::GodotOneShotLaunch {
            binary: config.godot_binary.as_deref().unwrap_or_default(),
            launch_prefix: &config.godot_launch_prefix,
            windowed_launch_prefix: &config.godot_windowed_launch_prefix,
            default_godot_args: &config.godot_args,
            extra_godot_args: &args,
            default_project_args: &[],
            extra_project_args: &[],
            project_root: &j.project,
            request_path: &request_path,
            logs_path: &logs,
            headless,
            audio,
            viewport: None,
            timeout: Some(std::time::Duration::from_secs(120)),
            fixed_fps: None,
        })?;
    let response = read_json(&result_path)?;
    if run.returncode != 0 || response["ok"] != true {
        let mut failure = fail(
            response["error"]["type"]
                .as_str()
                .unwrap_or("asset_trial_runtime_failed"),
            &logs.display().to_string(),
            response["error"]["message"]
                .as_str()
                .unwrap_or("offline Godot trial operation failed"),
        );
        failure.actual = Some(response);
        return Err(failure);
    }
    Ok(response)
}
fn validate_import(probe: &Value) -> Result<(), FailureDetail> {
    let resources = probe["data"]["resources"].as_array().ok_or_else(|| {
        fail(
            "asset_fingerprint_unavailable",
            "probe",
            "no imported resource facts",
        )
    })?;
    for resource in resources {
        if resource["fingerprint"].is_null() {
            return Err(fail(
                "asset_fingerprint_unavailable",
                resource["path"].as_str().unwrap_or_default(),
                "loaded content readback unavailable",
            ));
        }
        if resource["fingerprint"]["kind"] == "rgba8"
            && resource["import"]["params"]["compress/mode"].as_i64() != Some(0)
        {
            return Err(fail(
                "asset_trial_import_unsupported",
                resource["path"].as_str().unwrap_or_default(),
                "PNG requires lossless compress/mode=0",
            ));
        }
    }
    Ok(())
}
fn uid_probe(
    config: &ProjectConfig,
    j: &Journal,
    p: &manifest::Prepared,
) -> Result<(), FailureDetail> {
    let result = offline(
        config,
        j,
        p,
        "uid",
        json!({"scene":j.manifest.scene,"expected_uid":j.scene_uid.as_deref().unwrap_or_default()}),
    )?;
    if result["data"]["scene_uid"].as_str() != j.scene_uid.as_deref()
        || (j.scene_uid.as_deref() != Some("") && result["data"]["mapped_path"] != j.manifest.scene)
    {
        return Err(fail(
            "asset_trial_scene_uid_mapping_changed",
            &j.manifest.scene,
            "original scene UID mapping changed; preserve backup and recover before further launches",
        ));
    }
    Ok(())
}
fn verify_owned(j: &Journal) -> Result<(), FailureDetail> {
    for (resource, hash) in &j.owned {
        check_hash(&manifest::project_path(&j.project, resource)?, hash)?;
    }
    Ok(())
}

pub fn run_asset_status(config: &ProjectConfig, id: Option<&str>) -> CommandResult {
    let config = match canonical_config(config) {
        Ok(c) => c,
        Err(f) => return error_result("asset status", f),
    };
    if let Some(id) = id {
        return match load(&config, id) {
            Ok(j) => result("asset status", &j),
            Err(f) => error_result("asset status", f),
        };
    }
    let root = config.project_root.join(".qa/asset-trials");
    let mut trials = vec![];
    if let Ok(entries) = fs::read_dir(&root) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.len() == 32 {
                trials.push(match load(&config, &name) {
                    Ok(j) => json!(result("asset status", &j)),
                    Err(f) => json!({"trial_id":name,"failure":f}),
                });
            }
        }
    }
    let mut r = CommandResult::success("asset status", "asset trial journals");
    r.data.insert("trials".into(), json!(trials));
    r.data.insert(
        "active".into(),
        read_json(&root.join("active.json")).unwrap_or(Value::Null),
    );
    r
}
pub fn run_asset_keep(config: &ProjectConfig, id: &str) -> CommandResult {
    with_exclusive(config, "asset keep", |config| {
        let mut j = match load(config, id) {
            Ok(j) => j,
            Err(f) => return error_result("asset keep", f),
        };
        if j.state == "kept" {
            if let Err(f) = clear_terminal_reservation(&j) {
                return error_result("asset keep", f);
            }
            return result("asset keep", &j);
        }
        if let Err(f) = require_reservation(&j) {
            return error_result("asset keep", f);
        }
        if j.state != "evaluated" || !j.failures.is_empty() {
            return error_result(
                "asset keep",
                fail(
                    "asset_trial_keep_unverified",
                    id,
                    "keep requires an evaluated trial with all technical checks passing",
                ),
            );
        }
        let verify = (|| {
            if execution_config(config) != j.execution_config {
                return Err(fail(
                    "asset_trial_noncomparable",
                    id,
                    "effective Godot configuration differs from evaluated trial; use the original launch settings",
                ));
            }
            verify_frozen(&j)?;
            verify_owned(&j)?;
            check_hash(
                &manifest::project_path(&j.project, &j.manifest.scene)?,
                j.output_hash.as_deref().unwrap_or_default(),
            )?;
            for (relative, hash) in &j.evidence_hashes {
                check_hash(&j.evidence_root.join(relative), hash)?;
            }
            evidence::candidate(
                &j,
                j.candidate.as_ref().ok_or_else(|| {
                    fail(
                        "asset_trial_keep_unverified",
                        id,
                        "candidate evidence absent",
                    )
                })?,
            )?;
            let p = recovery_prepared(config, &j)?;
            let fresh = offline(
                config,
                &j,
                &p,
                "keep-probe",
                json!({"resources":j.manifest.bindings.iter().map(|b|&b.destination).collect::<Vec<_>>()}),
            )?;
            validate_import(&fresh)?;
            let original_probe = j
                .import_probe
                .as_ref()
                .ok_or_else(|| fail("asset_trial_keep_unverified", id, "import probe missing"))?;
            if fresh["backend"] != original_probe["backend"] {
                return Err(fail(
                    "asset_trial_noncomparable",
                    id,
                    "fresh import backend changed",
                ));
            }
            for resource in fresh["data"]["resources"].as_array().into_iter().flatten() {
                let old = original_probe["data"]["resources"]
                    .as_array()
                    .and_then(|rows| rows.iter().find(|row| row["path"] == resource["path"]));
                if old.is_none_or(|old| {
                    old["fingerprint"] != resource["fingerprint"]
                        || old["import"] != resource["import"]
                        || old["source_sha256"] != resource["source_sha256"]
                }) {
                    return Err(fail(
                        "asset_binding_unconfirmed",
                        id,
                        "fresh imported content changed since candidate validation",
                    ));
                }
            }
            uid_probe(config, &j, &p)?;
            verify_frozen(&j)?;
            verify_owned(&j)?;
            j.state = "kept".into();
            j.intent = None;
            j.save()?;
            j.clear_active()
        })();
        match verify {
            Ok(()) => result("asset keep", &j),
            Err(f) => finish_error("asset keep", &mut j, f),
        }
    })
}
pub fn run_asset_discard(config: &ProjectConfig, id: &str) -> CommandResult {
    with_exclusive(config, "asset discard", |config| {
        let mut j = match load(config, id) {
            Ok(j) => j,
            Err(f) => return error_result("asset discard", f),
        };
        if j.state == "discarded" {
            if let Err(f) = clear_terminal_reservation(&j) {
                return error_result("asset discard", f);
            }
            let mut r = result("asset discard", &j);
            r.ok = true;
            r.exit_code = 0;
            return r;
        }
        if j.state == "kept" {
            return error_result(
                "asset discard",
                fail(
                    "asset_trial_terminal_conflict",
                    id,
                    "kept trial cannot be discarded",
                ),
            );
        }
        let active = j.project.join(".qa/asset-trials/active.json");
        if !active.exists()
            && j.phase == "prepared"
            && j.intent.is_none()
            && j.owned.is_empty()
            && j.output_hash.is_none()
        {
            j.state = "discarded".into();
            return match j.save_journal() {
                Ok(()) => result("asset discard", &j),
                Err(f) => error_result("asset discard", f),
            };
        }
        if let Err(f) = require_reservation(&j) {
            return error_result("asset discard", f);
        }
        match discard(config, &mut j) {
            Ok(()) => {
                let mut r = result("asset discard", &j);
                r.ok = true;
                r.exit_code = 0;
                r
            }
            Err(f) => finish_error("asset discard", &mut j, f),
        }
    })
}
fn restore_scene_files(j: &mut Journal) -> Result<(), FailureDetail> {
    let scene = manifest::project_path(&j.project, &j.manifest.scene)?;
    // Recover every capture/publication gap from actual bytes, not receipt presence.
    if !scene.exists() {
        if !matches!(
            j.intent.as_deref(),
            Some(
                "apply_capture_intent"
                    | "apply_captured"
                    | "apply_publish_intent"
                    | "restore_capture_intent"
                    | "restore_captured"
                    | "restore_publish_intent"
            )
        ) {
            return Err(fail(
                "asset_trial_conflict",
                &scene.display().to_string(),
                "scene is missing outside a journaled capture/publication gap; preserve the external deletion",
            ));
        }
        let apply = j.dir().join("apply-captured.tscn");
        let restore = j.dir().join("restore-captured.tscn");
        if !apply.exists() && !restore.exists() {
            return Err(fail(
                "asset_trial_recovery_required",
                &scene.display().to_string(),
                "scene missing without a captured operation; refuse automatic writes",
            ));
        }
        if apply.exists() {
            check_hash(&apply, &j.original_hash)?;
        }
        if restore.exists() {
            check_hash(&restore, j.output_hash.as_deref().unwrap_or_default())?;
        }
        let publish = j
            .dir()
            .join(format!("restore-output-{}.tscn", random_id()?));
        write_new(
            &publish,
            &manifest::bounded_read(&j.dir().join("original.tscn"), 32 * 1024 * 1024)?,
        )?;
        j.intent("restore_publish_intent")?;
        fs::hard_link(&publish, &scene).map_err(|e| io_failure(&scene, e))?;
        fs::remove_file(&publish).map_err(|e| io_failure(&publish, e))?;
        sync_dir(scene.parent().unwrap())?;
        j.receipt("restore_published")?;
    } else {
        let hash = file_hash(&scene)?;
        if hash != j.original_hash {
            if Some(hash.as_str()) != j.output_hash.as_deref() {
                return Err(fail(
                    "asset_trial_conflict",
                    &scene.display().to_string(),
                    "scene contains unknown edits; original backup and all current bytes are preserved",
                ));
            }
            let restore_output = j
                .dir()
                .join(format!("restore-output-{}.tscn", random_id()?));
            write_new(
                &restore_output,
                &manifest::bounded_read(&j.dir().join("original.tscn"), 32 * 1024 * 1024)?,
            )?;
            let expected = j.output_hash.clone().unwrap();
            replace_no_clobber(
                j,
                &scene,
                &restore_output,
                &expected,
                "restore",
                &mut |_| Ok(()),
            )?;
            fs::remove_file(&restore_output).map_err(|e| io_failure(&restore_output, e))?;
        }
    }
    check_hash(&scene, &j.original_hash)?;
    // Hidden output must not survive into any subsequent Godot process.
    let output = j.dir().join("output.tscn");
    if output.exists() {
        if let Some(hash) = &j.output_hash {
            check_hash(&output, hash)?;
            fs::remove_file(&output).map_err(|e| io_failure(&output, e))?;
        } else {
            // A crash/rejected save may leave unverified output. Preserve its
            // bytes under a non-resource name before any engine restart.
            let captured = j
                .dir()
                .join(format!("unverified-output-{}.bin", random_id()?));
            fs::rename(&output, &captured).map_err(|e| io_failure(&output, e))?;
            j.warnings.push(format!(
                "Unverified writer output retained at {}",
                captured.display()
            ));
        }
    }
    Ok(())
}
fn discard(config: &ProjectConfig, j: &mut Journal) -> Result<(), FailureDetail> {
    retain_unconfirmed_stage(j)?;
    restore_scene_files(j)?;
    let p = recovery_prepared(config, j)?;
    uid_probe(config, j, &p)?;
    // A source scan cannot prove the absence of dynamic loads. Retain such files
    // explicitly; literal references in any other scene/script also prevent removal.
    let mut refs = source_references(config, j)?;
    let engine_refs = offline(
        config,
        j,
        &p,
        "references",
        json!({"resources":j.manifest.bindings.iter().map(|b|&b.destination).collect::<Vec<_>>()}),
    )?;
    for binding in &j.manifest.bindings {
        let entry = engine_refs["data"]["resources"]
            .as_array()
            .and_then(|a| a.iter().find(|v| v["path"] == binding.destination));
        if engine_refs["data"]["complete"] != true || entry.is_none_or(|v| v["referenced"] != false)
        {
            refs.push(binding.destination.clone());
        }
    }
    for (resource, hash) in j.owned.clone() {
        let path = manifest::project_path(&j.project, &resource)?;
        if !path.exists() && !j.media_captures.contains_key(&resource) {
            continue;
        }
        if refs.iter().any(|reference| {
            resource == *reference || resource.starts_with(&format!("{reference}."))
        }) {
            if let Some(name) = j.media_captures.get(&resource) {
                let captured = j.dir().join(name);
                if captured.exists() {
                    check_hash(&captured, &hash)?;
                    if !path.exists() {
                        write_new(&path, &manifest::bounded_read(&captured, 32 * 1024 * 1024)?)?;
                    }
                }
            }
            j.warnings.push(format!("Retained {resource}: source references or dynamic resource loading make deletion uncertain."));
            continue;
        }
        remove_owned_media(j, &resource, &hash, &mut |_| Ok(()))?;
    }
    j.state = "discarded".into();
    j.intent = None;
    j.save()?;
    j.clear_active()
}
fn recovery_prepared(
    config: &ProjectConfig,
    j: &Journal,
) -> Result<manifest::Prepared, FailureDetail> {
    let scenario_path = match (
        &j.manifest.validation.scenario,
        &j.manifest.validation.scenario_file,
    ) {
        (Some(name), None) => {
            crate::scenario_doc::find_scenario_by_name(&config.scenarios_dir, name)
                .unwrap_or_else(|| config.scenarios_dir.join(name))
        }
        (_, Some(path)) => j.project.join(path),
        _ => {
            return Err(fail(
                "asset_trial_journal_invalid",
                &j.id,
                "validation locator missing",
            ));
        }
    };
    // Recovery needs only launch settings. Keep the originally recorded effective
    // backend through baseline; do not require the scenario to remain parseable.
    Ok(manifest::Prepared {
        manifest: j.manifest.clone(),
        scenario_path,
        scenario: j.scenario.clone(),
        sources: vec![],
    })
}
fn source_references(config: &ProjectConfig, j: &Journal) -> Result<Vec<String>, FailureDetail> {
    let tree = snapshot_tree(&j.project, std::slice::from_ref(&config.artifacts_dir))?;
    let mut refs = vec![];
    for path in tree.keys() {
        if matches!(
            Path::new(path).extension().and_then(|e| e.to_str()),
            Some("gdextension" | "dll" | "so" | "dylib" | "exe")
        ) {
            refs.extend(j.manifest.bindings.iter().map(|b| b.destination.clone()));
            continue;
        }
        if !matches!(
            Path::new(path).extension().and_then(|e| e.to_str()),
            Some("gd" | "cs" | "tscn" | "tres" | "godot")
        ) {
            continue;
        }
        if path.starts_with("addons/godot_qa/") {
            continue;
        }
        let text = fs::read_to_string(j.project.join(path))
            .map_err(|e| io_failure(&j.project.join(path), e))?;
        for b in &j.manifest.bindings {
            if text.contains(&b.destination) || has_dynamic_load(&text) {
                refs.push(b.destination.clone());
            }
        }
    }
    Ok(refs)
}

fn require_reservation(j: &Journal) -> Result<(), FailureDetail> {
    let marker = read_json(&j.project.join(".qa/asset-trials/active.json"))?;
    if marker["trial_id"] != j.id {
        return Err(fail(
            "asset_trial_reservation_conflict",
            &j.id,
            "active reservation belongs to another trial",
        ));
    }
    Ok(())
}
fn check_legacy_sessions(config: &ProjectConfig) -> Result<(), FailureDetail> {
    let live = config.artifacts_dir.join("live");
    if let Ok(entries) = fs::read_dir(live) {
        for entry in entries.flatten() {
            if let Ok(meta) = fs::symlink_metadata(entry.path()) {
                if godot_qa_process::directory_link::is_link(&meta) {
                    continue;
                }
            }
            let path = entry.path().join("session.json");
            if !path.exists() {
                continue;
            }
            let data = read_json(&path)?;
            if let Some(pid) = data["pid"]
                .as_i64()
                .or_else(|| data["process"]["pid"].as_i64())
            {
                if godot_qa_process::pid_is_running(pid) {
                    return Err(fail(
                        "asset_trial_project_busy",
                        &path.display().to_string(),
                        "stop the existing live/pooled session before changing trial sources",
                    ));
                }
            }
        }
    }
    Ok(())
}

fn clear_terminal_reservation(j: &Journal) -> Result<(), FailureDetail> {
    let path = j.project.join(".qa/asset-trials/active.json");
    if path.exists() && read_json(&path)?["trial_id"] == j.id {
        j.clear_active()?;
    }
    Ok(())
}

fn execution_config(config: &ProjectConfig) -> Value {
    json!({"godot_binary":config.godot_binary,"godot_binary_identity":binary_identity(config),"artifacts_dir":config.artifacts_dir,"godot_args":config.godot_args,"project_args":config.godot_project_args,"launch_prefix":config.godot_launch_prefix,"windowed_launch_prefix":config.godot_windowed_launch_prefix,"headless":config.launch_mode_headless_override(),"audio":config.audio_mode_override(),"log_ignore_patterns":config.log_ignore_patterns})
}
fn has_dynamic_load(text: &str) -> bool {
    let compact = text
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>();
    let lower = compact.to_ascii_lowercase();
    if [
        "load_threaded_request",
        "load_interactive",
        "load<",
        "loadbytes",
        "loadfrom",
        "loadwith",
    ]
    .iter()
    .any(|token| lower.contains(token))
    {
        return true;
    }
    for (index, _) in lower.match_indices("load(") {
        if lower[..index].ends_with("pre") {
            continue;
        }
        let args = &compact[index + 5..];
        let quote = match args.as_bytes().first() {
            Some(b'"') => '"',
            Some(b'\'') => '\'',
            _ => return true,
        };
        let Some(end) = args[1..].find(quote) else {
            return true;
        };
        if !args[end + 2..].starts_with(')') {
            return true;
        }
    }
    false
}

fn binary_identity(config: &ProjectConfig) -> Value {
    let Some(binary) = config.godot_binary.as_deref() else {
        return Value::Null;
    };
    let candidate = if Path::new(binary).is_file() {
        Some(PathBuf::from(binary))
    } else {
        std::env::var_os("PATH").and_then(|paths| {
            std::env::split_paths(&paths)
                .map(|dir| dir.join(binary))
                .find(|p| p.is_file())
        })
    };
    match candidate {
        Some(path) => {
            json!({"path":fs::canonicalize(&path).unwrap_or_else(|_|path.clone()),"sha256":file_hash(&path).ok()})
        }
        None => Value::Null,
    }
}

fn stage_binding(
    j: &mut Journal,
    binding: &Binding,
    index: usize,
    before_create: &mut dyn FnMut(&Path) -> Result<(), FailureDetail>,
) -> Result<(), FailureDetail> {
    let destination = manifest::project_path(&j.project, &binding.destination)?;
    let snapshot = j.dir().join(format!("candidate-{index}.bin"));
    check_hash(&snapshot, &j.candidate_hashes[index])?;
    let bytes = manifest::bounded_read(&snapshot, 32 * 1024 * 1024)?;
    j.owned.insert(
        binding.destination.clone(),
        j.candidate_hashes[index].clone(),
    );
    j.intent(&format!("stage:{}", binding.destination))?;
    before_create(&destination)?;
    if let Err(f) = write_new(&destination, &bytes) {
        if f.details.get("io_kind") == Some(&json!("AlreadyExists")) {
            // Content equality is not ownership: create_new did not create it.
            j.owned.remove(&binding.destination);
            j.intent = None;
            j.save()?;
        }
        return Err(f);
    }
    j.intent = None;
    j.save()
}
fn remove_owned_media(
    j: &mut Journal,
    resource: &str,
    expected: &str,
    hook: &mut dyn FnMut(&str) -> Result<(), FailureDetail>,
) -> Result<(), FailureDetail> {
    let public = manifest::project_path(&j.project, resource)?;
    let previously_recorded = j.media_captures.contains_key(resource);
    let filename = match j.media_captures.get(resource) {
        Some(name) => name.clone(),
        None => {
            let name = format!("custody-{}.bin", random_id()?);
            j.media_captures.insert(resource.into(), name.clone());
            name
        }
    };
    let captured = j.dir().join(filename);
    if !captured.exists() {
        if previously_recorded
            && j.intent.as_deref() != Some(format!("remove_capture:{resource}").as_str())
        {
            if public.exists() {
                j.warnings.push(format!("Retained replacement at {resource}: prior custody deletion completed or cannot be disproved."));
            }
            return Ok(());
        }
        if !public.exists() {
            return Ok(());
        }
        j.intent(&format!("remove_capture:{resource}"))?;
        hook("capture_intent")?;
        fs::rename(&public, &captured).map_err(|e| io_failure(&public, e))?;
        sync_dir(public.parent().unwrap())?;
        sync_dir(&j.dir())?;
        hook("capture_moved")?;
        j.receipt(&format!("remove_captured:{resource}"))?;
        hook("captured")?;
    }
    if let Err(f) = check_hash(&captured, expected) {
        if fs::symlink_metadata(&public).is_err() {
            fs::hard_link(&captured, &public).map_err(|e| io_failure(&public, e))?;
            sync_dir(public.parent().unwrap())?;
        }
        // Both the captured foreign bytes and any recreated public pathname stay.
        return Err(f);
    }
    j.intent(&format!("remove_delete:{resource}"))?;
    hook("delete_intent")?;
    fs::remove_file(&captured).map_err(|e| io_failure(&captured, e))?;
    sync_dir(&j.dir())?;
    hook("deleted")?;
    j.receipt(&format!("removed:{resource}"))?;
    if public.exists() {
        j.warnings.push(format!("Retained foreign replacement at {resource}; only the captured owned bytes were deleted."));
    }
    Ok(())
}

fn retain_unconfirmed_stage(j: &mut Journal) -> Result<(), FailureDetail> {
    let Some(resource) = j
        .intent
        .as_deref()
        .and_then(|intent| intent.strip_prefix("stage:"))
        .map(str::to_owned)
    else {
        return Ok(());
    };
    // Intent proves an attempt, not create_new success. A crash can occur after
    // AlreadyExists (even for identical bytes) but before the ownership-removal
    // receipt. Preserve every public file whose successful creation is unproven.
    j.owned.remove(&resource);
    j.warnings.push(format!("Ownership was not confirmed for {resource}; any staged or foreign bytes at that path are retained."));
    j.intent = None;
    j.save()
}
