use serde_json::Value;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::sync::Mutex;

fn temp_project() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let project = env::temp_dir().join(format!("godot-qa-asset-trial-{nanos}"));
    fs::create_dir_all(&project).unwrap();
    let sample = source_root().join("examples/sample_project");
    fs::copy(sample.join("project.godot"), project.join("project.godot")).unwrap();
    for relative in [
        "assets/asset_trial",
        "qa/scenarios/examples/asset_coin_pickup.yaml",
        "ui/fixtures/asset_coin.gd",
        "ui/fixtures/asset_coin.gd.uid",
        "ui/fixtures/asset_coin.tscn",
        "ui/fixtures/asset_coin_decoration.tscn",
    ] {
        let source = sample.join(relative);
        let destination = project.join(relative);
        if source.is_dir() {
            copy_dir(&source, &destination);
        } else {
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(source, destination).unwrap();
        }
    }
    copy_dir(
        &source_root().join("addons/godot_qa"),
        &project.join("addons/godot_qa"),
    );
    let artifacts_ignore = project.join(".qa/artifacts/.gdignore");
    fs::create_dir_all(artifacts_ignore.parent().unwrap()).unwrap();
    fs::write(artifacts_ignore, "\n").unwrap();
    fs::create_dir_all(project.join("qa/asset-trials")).unwrap();
    project
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .components()
        .collect()
}

fn source_root() -> PathBuf {
    env::var_os("GODOT_QA_ASSET_TRIAL_SOURCE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(repo_root)
}

fn cli_binary() -> PathBuf {
    env::var_os("GODOT_QA_CLI_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_godot-qa")))
}

fn copy_dir(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if source_path.is_dir() {
            copy_dir(&source_path, &destination_path);
        } else {
            fs::copy(source_path, destination_path).unwrap();
        }
    }
}

fn run_try(project: &Path, manifest: &str) -> Output {
    fs::write(project.join("qa/asset-trials/trial.yaml"), manifest).unwrap();
    Command::new(cli_binary())
        .arg("--json")
        .arg("--project")
        .arg(project)
        .arg("asset")
        .arg("try")
        .arg("qa/asset-trials/trial.yaml")
        .output()
        .unwrap()
}

fn parse_json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "asset try returned invalid JSON: {error}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn malformed_asset_trial_manifest_returns_stable_json_failure() {
    let project = temp_project();
    let output = run_try(
        &project,
        "format: godot-qa.asset-trial.v1\nscene: res://ui/fixtures/asset_coin.tscn\nvalidation: {}\nbindings: []\n",
    );
    assert!(!output.status.success());
    let body = parse_json(&output);
    assert_eq!(body["ok"], false);
    assert_eq!(body["command"], "asset try");
    assert_eq!(body["failures"][0]["type"], "asset_trial_manifest_invalid");
}

#[test]
fn duplicate_asset_trial_edit_slots_return_stable_json_failure() {
    let project = temp_project();
    let output = run_try(
        &project,
        concat!(
            "format: godot-qa.asset-trial.v1\n",
            "scene: res://ui/fixtures/asset_coin.tscn\n",
            "validation:\n  scenario: asset_coin_pickup\n",
            "bindings:\n",
            "  - id: first\n    source: assets/asset_trial/coin_trial.png\n    destination: res://assets/asset_trial/a.png\n",
            "    edit_path: CoinSprite\n    runtime_target: qa:asset_coin_sprite\n    property: texture\n",
            "    evidence: {snapshot: shot, state_before: before, state_after: after}\n",
            "  - id: second\n    source: assets/asset_trial/coin_trial.png\n    destination: res://assets/asset_trial/b.png\n",
            "    edit_path: CoinSprite\n    runtime_target: qa:asset_coin_sprite\n    property: texture\n",
            "    evidence: {snapshot: shot2, state_before: before2, state_after: after2}\n",
        ),
    );
    assert!(!output.status.success());
    let body = parse_json(&output);
    assert_eq!(body["ok"], false);
    assert_eq!(body["failures"][0]["type"], "asset_trial_manifest_invalid");
}

#[test]
fn asset_trial_destination_escape_returns_stable_json_failure() {
    let project = temp_project();
    let output = run_try(
        &project,
        concat!(
            "format: godot-qa.asset-trial.v1\n",
            "scene: res://ui/fixtures/asset_coin.tscn\n",
            "validation:\n  scenario: asset_coin_pickup\n",
            "bindings:\n",
            "  - id: texture\n    source: assets/asset_trial/coin_trial.png\n",
            "    destination: res://../outside.png\n",
            "    edit_path: CoinSprite\n    runtime_target: qa:asset_coin_sprite\n    property: texture\n",
            "    evidence: {snapshot: shot, state_before: before, state_after: after}\n",
        ),
    );
    assert!(!output.status.success());
    let body = parse_json(&output);
    assert_eq!(body["ok"], false);
    assert_eq!(body["failures"][0]["type"], "asset_trial_path_invalid");
}

#[cfg(unix)]
static REAL_ACCEPTANCE_LOCK: Mutex<()> = Mutex::new(());

#[cfg(unix)]
fn acceptance_sample(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let project = env::temp_dir().join(format!("godot-qa-asset-acceptance-{label}-{nanos}"));
    let sample = source_root().join("examples/sample_project");
    copy_sample_tree(&sample, &project, Path::new(""));
    copy_dir(
        &source_root().join("addons/godot_qa"),
        &project.join("addons/godot_qa"),
    );
    let artifacts_ignore = project.join(".qa/artifacts/.gdignore");
    fs::create_dir_all(artifacts_ignore.parent().unwrap()).unwrap();
    fs::write(artifacts_ignore, "\n").unwrap();

    let godot = godot_bin();
    let output = Command::new(&godot)
        .arg("--headless")
        .arg("--path")
        .arg(&project)
        .arg("--import")
        .output()
        .unwrap_or_else(|error| panic!("cannot import disposable sample with {godot:?}: {error}"));
    assert!(
        output.status.success(),
        "sample import failed in {}\nstdout: {}\nstderr: {}",
        project.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    project
}

#[cfg(unix)]
fn copy_sample_tree(source: &Path, destination: &Path, relative: &Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        let child_relative = relative.join(&name);
        let name_string = name.to_string_lossy();
        // `.qa` and qa/config.local.yaml are disposable developer state (often
        // containing native test tools, old evidence, or machine-local launch
        // wrappers); a fresh project gets its standard ignore marker recreated
        // by acceptance_sample and uses the caller's isolated display wrapper.
        if name_string == ".godot"
            || name_string == ".qa"
            || child_relative == Path::new("qa/config.local.yaml")
        {
            continue;
        }
        let source_path = entry.path();
        let destination_path = destination.join(&name);
        if source_path.is_dir() {
            copy_sample_tree(&source_path, &destination_path, &child_relative);
        } else {
            fs::copy(source_path, destination_path).unwrap();
        }
    }
}

#[cfg(unix)]
fn godot_bin() -> PathBuf {
    env::var_os("GODOT_BIN")
        .map(PathBuf::from)
        .or_else(|| env::var_os("GODOT_QA_GODOT_BIN").map(PathBuf::from))
        .unwrap_or_else(|| panic!("opt-in asset acceptance requires GODOT_BIN"))
}

#[cfg(unix)]
fn trial_cli(project: &Path, args: &[&str]) -> Value {
    let godot = godot_bin();
    let output = Command::new(cli_binary())
        .arg("--json")
        .arg("--project")
        .arg(project)
        .arg("--godot-bin")
        .arg(&godot)
        .args(args)
        .output()
        .unwrap();
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "CLI returned invalid JSON ({error}), status {}\nstdout: {}\nstderr: {}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[cfg(unix)]
fn manifest_with_bindings(project: &Path, include_texture: bool, include_audio: bool) -> PathBuf {
    let source = fs::read_to_string(project.join("qa/asset-trials/coin.yaml")).unwrap();
    let (header, texture_and_audio) = source.split_once("  - id: coin_texture\n").unwrap();
    let (texture, audio) = texture_and_audio
        .split_once("  - id: pickup_sfx\n")
        .unwrap();
    let mut output = format!("{header}  - id: coin_texture\n");
    if include_texture {
        output.push_str(texture);
        output.push_str("  - id: pickup_sfx\n");
    }
    if include_audio {
        output.push_str(audio);
    } else if include_texture {
        // The audio block starts only after the pickup binding id and is omitted.
        output.truncate(output.find("  - id: pickup_sfx\n").unwrap());
    }
    if !include_texture {
        output = format!("{header}  - id: pickup_sfx\n{audio}");
    }
    let path = project.join("qa/asset-trials/acceptance.yaml");
    fs::write(&path, output).unwrap();
    path
}

#[cfg(unix)]
fn begin_trial(project: &Path, include_texture: bool, include_audio: bool) -> (Value, String) {
    let manifest = manifest_with_bindings(project, include_texture, include_audio);
    let relative = manifest
        .strip_prefix(project)
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/");
    let body = trial_cli(project, &["asset", "try", &relative]);
    assert_eq!(body["ok"], true, "asset try failed: {body:#}");
    assert_eq!(
        body["data"]["state"], "evaluated",
        "unexpected trial state: {body:#}"
    );
    assert_eq!(body["data"]["technical_status"], "pass");
    let id = body["data"]["trial_id"].as_str().unwrap().to_owned();
    assert_trial_captures(&body, include_texture, include_audio);
    (body, id)
}

#[cfg(unix)]
fn assert_trial_captures(body: &Value, include_texture: bool, include_audio: bool) {
    let candidate = &body["data"]["candidate"];
    if include_texture {
        let capture = &candidate["asset_trial"]["captures"]["asset_coin_visible"]["bindings"];
        let binding = capture
            .as_array()
            .unwrap()
            .iter()
            .find(|binding| binding["id"] == "coin_texture")
            .unwrap();
        assert_eq!(
            binding["resource"]["path"],
            "res://assets/asset_trial/coin_candidate.png"
        );
        assert!(binding["visible_pixel_count"].as_u64().unwrap_or(0) > 0);
    }
    if include_audio {
        let capture = &candidate["asset_trial"]["captures"]["asset_coin_sfx"]["bindings"];
        let binding = capture
            .as_array()
            .unwrap()
            .iter()
            .find(|binding| binding["id"] == "pickup_sfx")
            .unwrap();
        assert_eq!(
            binding["resource"]["path"],
            "res://assets/asset_trial/pickup_candidate.wav"
        );
        assert_eq!(binding["playback_overlap"], true);
        assert_eq!(binding["bus_matches"], true);
        assert_ne!(candidate["asset_trial"]["backend"]["audio_driver"], "Dummy");
        assert_eq!(
            candidate["audio_analysis"]["asset_coin_sfx"]["near_silence"],
            false
        );
        let wav = candidate["audio_artifacts"]["asset_coin_sfx"]["wav"]
            .as_str()
            .unwrap();
        assert!(Path::new(wav).is_file(), "missing captured WAV: {wav}");
    }
}

#[cfg(unix)]
fn discard_and_check(project: &Path, id: &str, original_scene: &[u8], destinations: &[&str]) {
    let result = trial_cli(project, &["asset", "discard", id]);
    assert_eq!(result["ok"], true, "asset discard failed: {result:#}");
    assert_eq!(result["data"]["state"], "discarded");
    assert_eq!(
        fs::read(project.join("ui/fixtures/asset_coin.tscn")).unwrap(),
        original_scene
    );
    for destination in destinations {
        assert_discard_disposition(&result, project, destination);
    }
}

#[cfg(unix)]
fn assert_discard_disposition(result: &Value, project: &Path, destination: &str) {
    let path = project.join(destination.strip_prefix("res://").unwrap());
    let sidecar = PathBuf::from(format!("{}.import", path.display()));
    if path.exists() {
        // The sample has code with dynamic resource loading, so cleanup may
        // conservatively retain trial-owned files. Verify those bytes remain
        // the exact candidate, and that the retained-file warning is visible.
        let source = if destination.ends_with("coin_candidate.png") {
            "coin_trial.png"
        } else if destination.ends_with("pickup_candidate.wav") {
            "pickup_trial.wav"
        } else {
            panic!("unexpected retained asset path: {destination}");
        };
        assert_eq!(
            fs::read(&path).unwrap(),
            fs::read(
                source_root()
                    .join("examples/sample_project/assets/asset_trial")
                    .join(source)
            )
            .unwrap(),
            "discard changed retained candidate bytes at {}",
            path.display()
        );
        assert!(
            sidecar.is_file(),
            "retained media is missing its import sidecar"
        );
        let warnings = result["data"]["warnings"].as_array().unwrap();
        assert!(
            warnings.iter().any(|warning| {
                warning
                    .as_str()
                    .is_some_and(|message| message.contains(&format!("Retained {destination}")))
            }),
            "retained file lacks an explanatory warning: {warnings:#?}"
        );
    } else {
        assert!(
            !sidecar.exists(),
            "discard removed media but left sidecar {}",
            sidecar.display()
        );
    }
}

#[cfg(unix)]
#[test]
fn real_asset_trial_lifecycle_acceptance() {
    if env::var("GODOT_QA_ASSET_TRIAL_ACCEPTANCE").as_deref() != Ok("1") {
        eprintln!("skipping real asset trial acceptance; set GODOT_QA_ASSET_TRIAL_ACCEPTANCE=1");
        return;
    }
    let _serial = REAL_ACCEPTANCE_LOCK.lock().unwrap();
    assert!(godot_bin().is_file(), "GODOT_BIN is not an executable file");
    let source_scene =
        fs::read(source_root().join("examples/sample_project/ui/fixtures/asset_coin.tscn"))
            .unwrap();

    for (name, texture, audio, destinations) in [
        (
            "png-only",
            true,
            false,
            vec!["res://assets/asset_trial/coin_candidate.png"],
        ),
        (
            "wav-only",
            false,
            true,
            vec!["res://assets/asset_trial/pickup_candidate.wav"],
        ),
        (
            "combined",
            true,
            true,
            vec![
                "res://assets/asset_trial/coin_candidate.png",
                "res://assets/asset_trial/pickup_candidate.wav",
            ],
        ),
    ] {
        let project = acceptance_sample(name);
        let (_, id) = begin_trial(&project, texture, audio);
        let status = trial_cli(&project, &["asset", "status", &id]);
        assert_eq!(status["ok"], true, "asset status failed: {status:#}");
        assert_eq!(status["data"]["state"], "evaluated");
        discard_and_check(&project, &id, &source_scene, &destinations);
        fs::remove_dir_all(&project).unwrap();
    }

    let project = acceptance_sample("keep-replay");
    let (_, id) = begin_trial(&project, true, true);
    let kept = trial_cli(&project, &["asset", "keep", &id]);
    assert_eq!(kept["ok"], true, "asset keep failed: {kept:#}");
    assert_eq!(kept["data"]["state"], "kept");
    let replay = trial_cli(&project, &["scenario", "run", "asset_coin_pickup"]);
    assert_eq!(
        replay["ok"], true,
        "fresh replay after keep failed: {replay:#}"
    );
    fs::remove_dir_all(&project).unwrap();

    let project = acceptance_sample("conflict-retry");
    let (_, id) = begin_trial(&project, true, true);
    let scene = project.join("ui/fixtures/asset_coin.tscn");
    let candidate_scene = fs::read(&scene).unwrap();
    let mut user_edit = candidate_scene.clone();
    user_edit.extend_from_slice(b"\n; acceptance user edit\n");
    fs::write(&scene, &user_edit).unwrap();
    let conflict = trial_cli(&project, &["asset", "discard", &id]);
    assert_eq!(conflict["ok"], false);
    assert_eq!(conflict["failures"][0]["type"], "asset_trial_conflict");
    assert_eq!(
        fs::read(&scene).unwrap(),
        user_edit,
        "conflict overwrote user's bytes"
    );
    assert_eq!(
        trial_cli(&project, &["asset", "status", &id])["data"]["state"],
        "conflict"
    );
    fs::write(&scene, &candidate_scene).unwrap();
    discard_and_check(
        &project,
        &id,
        &source_scene,
        &[
            "res://assets/asset_trial/coin_candidate.png",
            "res://assets/asset_trial/pickup_candidate.wav",
        ],
    );
    let repeated_discard = trial_cli(&project, &["asset", "discard", &id]);
    assert_eq!(
        repeated_discard["ok"], true,
        "idempotent discard recovery failed: {repeated_discard:#}"
    );
    assert_eq!(repeated_discard["data"]["state"], "discarded");
    assert_eq!(fs::read(&scene).unwrap(), source_scene);
    for destination in [
        "res://assets/asset_trial/coin_candidate.png",
        "res://assets/asset_trial/pickup_candidate.wav",
    ] {
        assert_discard_disposition(&repeated_discard, &project, destination);
    }
    fs::remove_dir_all(&project).unwrap();
}
