use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "godot-qa-export-{label}-{stamp}-{}",
        std::process::id()
    ));
    fs::create_dir(&path).unwrap();
    path
}

fn cli(selection: &Path, out: &Path) -> (bool, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_godot-qa"))
        .args(["--json", "artifact", "export"])
        .arg(selection)
        .arg("--out")
        .arg(out)
        .output()
        .unwrap();
    let body = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&output.stdout)));
    (output.status.success(), body)
}

fn write_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

fn fixture(root: &Path, id: &str, ok: bool) -> Value {
    let run = root.join(id);
    fs::create_dir(&run).unwrap();
    fs::create_dir(run.join("snapshots")).unwrap();
    let image = image::RgbImage::from_pixel(2, 2, image::Rgb([4, 9, 16]));
    image.save(run.join("snapshots/scene.png")).unwrap();
    write_json(
        &run.join("result.json"),
        &json!({
            "ok":ok,
            "command":"scenario run",
            "message":"password=private-do-not-export",
            "data":{"run_identity":{"run_id":id}, "artifacts":{"secret":"/home/private/token"}},
            "failures": if ok {vec![]} else {vec![json!({"type":"assertion_failed","message":"private failure detail","actual":{"token":"SECRET"}})]}
        }),
    );
    write_json(
        &run.join("profile.json"),
        &json!({"data":{"frame":{"p95_ms":13.5},"private_token":"SECRET"}}),
    );
    json!({
        "id":id,"run_root":id,"result":"result.json","run_id":id,
        "revision":{"tool":"0.1.0","project":"abc123","build":"debug"},
        "environment":{"renderer":"vulkan","display_backend":"gamescope","resolution":[1920,1080],"instrumented":true},
        "images":[{"id":"scene","path":"snapshots/scene.png","role":"original","snapshot":"scene"}],
        "metrics":[{"id":"frame","path":"profile.json","fields":{"p95_ms":"/data/frame/p95_ms"}}]
    })
}

fn wav_fixture(root: &Path, id: &str, ok: bool) -> Value {
    let mut source = fixture(root, id, ok);
    let samples = [0_i16, 1200, -2400, 600, 0, -600, 2400, -1200];
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36_u32 + samples.len() as u32 * 2).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&8000_u32.to_le_bytes());
    wav.extend_from_slice(&16000_u32.to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(samples.len() as u32 * 2).to_le_bytes());
    for sample in samples {
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    fs::write(root.join(id).join("candidate.wav"), wav).unwrap();
    source["audio"] = json!([{"id":"candidate","path":"candidate.wav","role":"original"}]);
    source
}

#[test]
fn exported_wavs_are_exact_portable_unreviewed_and_preserve_failed_results() {
    let root = temp_root("wav-portable");
    let source = wav_fixture(&root, "failed", false);
    let selection = root.join("selection.json");
    write_json(
        &selection,
        &json!({"schema_version":1,"title":"Audio <review>","sources":[source]}),
    );
    let out = root.join("export");
    let (ok, body) = cli(&selection, &out);
    assert!(ok, "{body}");
    let manifest: Value =
        serde_json::from_slice(&fs::read(out.join("evidence.json")).unwrap()).unwrap();
    let source = &manifest["sources"][0];
    assert_eq!(source["automatic_check_status"], "failed");
    assert_eq!(source["failure_count"], 1);
    let audio = &source["audio"][0];
    assert_eq!(audio["source_sha256"], audio["export_sha256"]);
    assert_eq!(audio["audio_review"]["status"], "unreviewed");
    assert_eq!(audio["transformation"], "byte_exact_copy");
    assert_eq!(audio["metadata"]["sample_rate"], 8000);
    assert_eq!(audio["metadata"]["channels"], 1);
    let path = audio["path"].as_str().unwrap();
    assert!(!Path::new(path).is_absolute());
    assert_eq!(
        fs::read(out.join(path)).unwrap(),
        fs::read(root.join("failed/candidate.wav")).unwrap()
    );
    let moved = root.join("moved");
    fs::rename(&out, &moved).unwrap();
    let html = fs::read_to_string(moved.join("index.html")).unwrap();
    assert!(html.contains("<audio controls preload=\"none\""));
    assert!(html.contains(path));
    assert!(html.contains("Audio &lt;review&gt;"));
    assert!(moved.join(path).is_file());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalid_wav_selections_fail_before_destination_creation() {
    let root = temp_root("wav-invalid");
    let original = wav_fixture(&root, "run", true);
    for (label, change) in [
        (
            "role",
            json!({"id":"audio","path":"candidate.wav","role":"captured"}),
        ),
        (
            "traversal",
            json!({"id":"audio","path":"../run/candidate.wav","role":"original"}),
        ),
        (
            "extension",
            json!({"id":"audio","path":"result.json","role":"original"}),
        ),
        (
            "review",
            json!({"id":"audio","path":"candidate.wav","role":"original","review":{"status":"reviewed","reviewer":"human","method":"listening","scale":"native","scope":"whole","reviewed_at":"2026-10-28","sha256":"incorrect"}}),
        ),
    ] {
        let mut source = original.clone();
        source["audio"] = json!([change]);
        let selection = root.join(format!("{label}.json"));
        write_json(
            &selection,
            &json!({"schema_version":1,"title":"Audio","sources":[source]}),
        );
        let out = root.join(format!("out-{label}"));
        let (ok, body) = cli(&selection, &out);
        assert!(!ok, "{label}: {body}");
        assert!(!out.exists());
    }
    fs::write(root.join("run/candidate.wav"), b"not a WAV").unwrap();
    let selection = root.join("malformed.json");
    write_json(
        &selection,
        &json!({"schema_version":1,"title":"Audio","sources":[original]}),
    );
    let (ok, body) = cli(&selection, &root.join("out-malformed"));
    assert!(!ok, "{body}");
    assert_eq!(body["failures"][0]["type"], "export_audio_invalid");
    assert!(!root.join("out-malformed").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn wav_packet_limits_apply_across_sources() {
    let root = temp_root("wav-count");
    let mut first = wav_fixture(&root, "first", true);
    let second = wav_fixture(&root, "second", true);
    first["audio"] = Value::Array(
        (0..64)
            .map(|i| json!({"id":format!("audio{i}"),"path":"candidate.wav","role":"original"}))
            .collect(),
    );
    let selection = root.join("selection.json");
    write_json(
        &selection,
        &json!({"schema_version":1,"title":"Audio","sources":[first,second]}),
    );
    let out = root.join("export");
    let (ok, body) = cli(&selection, &out);
    assert!(!ok, "{body}");
    assert_eq!(body["failures"][0]["type"], "export_audio_resource_limit");
    assert!(!out.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn oversized_wav_is_rejected_before_export() {
    let root = temp_root("wav-oversize");
    let source = wav_fixture(&root, "run", true);
    fs::OpenOptions::new()
        .write(true)
        .open(root.join("run/candidate.wav"))
        .unwrap()
        .set_len(64 * 1024 * 1024 + 1)
        .unwrap();
    let selection = root.join("selection.json");
    write_json(
        &selection,
        &json!({"schema_version":1,"title":"Audio","sources":[source]}),
    );
    let out = root.join("export");
    let (ok, body) = cli(&selection, &out);
    assert!(!ok, "{body}");
    assert_eq!(body["failures"][0]["type"], "export_audio_resource_limit");
    assert!(!out.exists());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn wav_selection_rejects_symlink_escape() {
    let root = temp_root("wav-symlink");
    let mut source = wav_fixture(&root, "run", true);
    fs::copy(root.join("run/candidate.wav"), root.join("outside.wav")).unwrap();
    std::os::unix::fs::symlink(root.join("outside.wav"), root.join("run/escape.wav")).unwrap();
    source["audio"][0]["path"] = json!("escape.wav");
    let selection = root.join("selection.json");
    write_json(
        &selection,
        &json!({"schema_version":1,"title":"Audio","sources":[source]}),
    );
    let out = root.join("export");
    let (ok, body) = cli(&selection, &out);
    assert!(!ok, "{body}");
    assert_eq!(body["failures"][0]["type"], "export_path_rejected");
    assert!(!out.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn exported_images_are_exact_portable_and_json_is_sanitized() {
    let root = temp_root("portable");
    let source = fixture(&root, "passing", true);
    let selection = root.join("selection.json");
    write_json(
        &selection,
        &json!({"schema_version":1,"title":"Evidence <board>","sources":[source]}),
    );
    let out = root.join("export");
    let (ok, body) = cli(&selection, &out);
    assert!(ok, "{body}");
    let manifest: Value =
        serde_json::from_slice(&fs::read(out.join("evidence.json")).unwrap()).unwrap();
    let image = &manifest["sources"][0]["images"][0];
    let original = fs::read(root.join("passing/snapshots/scene.png")).unwrap();
    assert_eq!(
        fs::read(out.join(image["path"].as_str().unwrap())).unwrap(),
        original
    );
    assert_eq!(image["source_sha256"], image["export_sha256"]);
    assert_eq!(image["visual_review"]["status"], "unreviewed");
    let summary = &manifest["sources"][0]["result_summary"];
    assert_ne!(summary["source_sha256"], summary["export_sha256"]);
    assert_eq!(summary["transformation"], "allowlisted_result_v1");
    let metric = &manifest["sources"][0]["metrics"][0];
    assert_ne!(metric["source_sha256"], metric["export_sha256"]);
    assert_eq!(metric["transformation"], "scalar_projection_v1");
    let copied_metric: Value =
        serde_json::from_slice(&fs::read(out.join(metric["path"].as_str().unwrap())).unwrap())
            .unwrap();
    assert_eq!(copied_metric["fields"]["p95_ms"], 13.5);
    let exported = fs::read_to_string(out.join("evidence.json")).unwrap();
    assert!(
        !exported.contains("SECRET")
            && !exported.contains("private-do-not-export")
            && !exported.contains("/home/private")
    );
    let index = fs::read_to_string(out.join("index.html")).unwrap();
    assert!(index.contains("Evidence &lt;board&gt;"));
    let moved = root.join("moved");
    fs::rename(&out, &moved).unwrap();
    assert!(moved.join(image["path"].as_str().unwrap()).exists());
    assert!(moved.join("index.html").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn every_declared_failure_survives_even_with_passing_images() {
    let root = temp_root("failures");
    let passed = fixture(&root, "passed", true);
    let failed = fixture(&root, "failed", false);
    let selection = root.join("selection.json");
    write_json(
        &selection,
        &json!({"schema_version":1,"title":"Two runs","sources":[passed,failed]}),
    );
    let out = root.join("export");
    let (ok, body) = cli(&selection, &out);
    assert!(ok, "{body}");
    let manifest: Value =
        serde_json::from_slice(&fs::read(out.join("evidence.json")).unwrap()).unwrap();
    assert_eq!(manifest["sources"][0]["automatic_check_status"], "passed");
    assert_eq!(manifest["sources"][1]["automatic_check_status"], "failed");
    assert_eq!(
        manifest["sources"][1]["failures"][0]["type"],
        "assertion_failed"
    );
    assert_eq!(
        manifest["sources"][1]["images"][0]["capture_status"],
        "captured"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn explicit_review_must_match_exact_image_hash() {
    let root = temp_root("review");
    let mut source = fixture(&root, "reviewed", true);
    let selection = root.join("selection.json");
    let review = json!({"status":"reviewed","reviewer":"qa-team","method":"pixel-inspection","scale":"1:1","scope":"full-frame","reviewed_at":"2026-09-25T18:00:00Z","sha256":"bad"});
    source["images"][0]["review"] = review;
    write_json(
        &selection,
        &json!({"schema_version":1,"title":"Review","sources":[source]}),
    );
    let out = root.join("export");
    let (ok, body) = cli(&selection, &out);
    assert!(!ok);
    assert_eq!(body["failures"][0]["type"], "export_review_hash_mismatch");
    assert!(!out.exists());
    let bytes = fs::read(root.join("reviewed/snapshots/scene.png")).unwrap();
    let mut hasher = godot_qa_core::hash::Sha256::new();
    hasher.update(&bytes);
    let hash = godot_qa_core::hash::to_lower_hex(&hasher.finish());
    let mut source = fixture(&root, "reviewed2", true);
    fs::copy(
        root.join("reviewed/snapshots/scene.png"),
        root.join("reviewed2/snapshots/scene.png"),
    )
    .unwrap();
    source["images"][0]["review"] = json!({"status":"reviewed","reviewer":"qa-team","method":"pixel-inspection","scale":"1:1","scope":"full-frame","reviewed_at":"2026-09-25T18:00:00Z","sha256":hash});
    write_json(
        &selection,
        &json!({"schema_version":1,"title":"Review","sources":[source]}),
    );
    let (ok, body) = cli(&selection, &out);
    assert!(ok, "{body}");
    let manifest: Value =
        serde_json::from_slice(&fs::read(out.join("evidence.json")).unwrap()).unwrap();
    assert_eq!(
        manifest["sources"][0]["images"][0]["visual_review"]["sha256"],
        hash
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_symlink_escape_and_never_overwrites_existing_output() {
    let root = temp_root("boundaries");
    let source = fixture(&root, "run", true);
    let selection = root.join("selection.json");
    let out = root.join("export");
    fs::create_dir(&out).unwrap();
    fs::write(out.join("sentinel"), b"untouched").unwrap();
    write_json(
        &selection,
        &json!({"schema_version":1,"title":"Boundaries","sources":[source.clone()]}),
    );
    let (ok, body) = cli(&selection, &out);
    assert!(!ok);
    assert_eq!(body["failures"][0]["type"], "export_output_exists");
    assert_eq!(fs::read(out.join("sentinel")).unwrap(), b"untouched");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            root.join("selection.json"),
            root.join("run/snapshots/escaped.png"),
        )
        .unwrap();
        let mut source = source;
        source["images"][0]["path"] = json!("snapshots/escaped.png");
        write_json(
            &selection,
            &json!({"schema_version":1,"title":"Boundaries","sources":[source]}),
        );
        let (ok, body) = cli(&selection, &root.join("fresh"));
        assert!(!ok);
        assert_eq!(body["failures"][0]["type"], "export_path_rejected");
    }
    let mut source = fixture(&root, "log-check", true);
    fs::write(root.join("log-check/logs.json"), b"{\"count\":1}").unwrap();
    source["metrics"][0]["path"] = json!("logs.json");
    write_json(
        &selection,
        &json!({"schema_version":1,"title":"Boundaries","sources":[source]}),
    );
    let (ok, body) = cli(&selection, &root.join("fresh-log"));
    assert!(!ok);
    assert_eq!(body["failures"][0]["type"], "export_metric_invalid");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn missing_source_result_stays_visible_and_fails_command() {
    let root = temp_root("missing");
    let source = fixture(&root, "partial", true);
    fs::remove_file(root.join("partial/result.json")).unwrap();
    let selection = root.join("selection.json");
    write_json(
        &selection,
        &json!({"schema_version":1,"title":"Partial","sources":[source]}),
    );
    let out = root.join("export");
    let (ok, body) = cli(&selection, &out);
    assert!(!ok);
    assert_eq!(body["failures"][0]["type"], "export_source_incomplete");
    let manifest: Value =
        serde_json::from_slice(&fs::read(out.join("evidence.json")).unwrap()).unwrap();
    assert_eq!(
        manifest["sources"][0]["automatic_check_status"],
        "incomplete"
    );
    assert_eq!(
        manifest["sources"][0]["failures"][0]["type"],
        "source_result_missing_or_malformed"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn malformed_and_contradictory_result_claims_never_pass() {
    let root = temp_root("result-shape");
    let one = fixture(&root, "missing-failures", true);
    let two = fixture(&root, "contradiction", true);
    write_json(
        &root.join("missing-failures/result.json"),
        &json!({"ok":true,"command":"scenario run"}),
    );
    write_json(
        &root.join("contradiction/result.json"),
        &json!({"ok":true,"command":"scenario run","failures":[{"type":"assertion_failed","message":"SECRET"}]}),
    );
    let selection = root.join("selection.json");
    write_json(
        &selection,
        &json!({"schema_version":1,"title":"Result claims","sources":[one,two]}),
    );
    let out = root.join("export");
    let (ok, body) = cli(&selection, &out);
    assert!(!ok);
    assert_eq!(body["failures"][0]["type"], "export_source_incomplete");
    let manifest: Value =
        serde_json::from_slice(&fs::read(out.join("evidence.json")).unwrap()).unwrap();
    assert_eq!(
        manifest["sources"][0]["automatic_check_status"],
        "incomplete"
    );
    assert_eq!(
        manifest["sources"][0]["failures"][0]["type"],
        "source_result_malformed"
    );
    assert_eq!(manifest["sources"][1]["automatic_check_status"], "failed");
    assert_eq!(
        manifest["sources"][1]["failures"][0]["source_pointer"],
        "/failures/0"
    );
    assert_eq!(
        manifest["sources"][1]["failures"][1]["type"],
        "result_status_inconsistent"
    );
    assert!(
        !fs::read_to_string(out.join("evidence.json"))
            .unwrap()
            .contains("SECRET")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn contact_sheet_is_labeled_derived_and_sensitive_metric_pointer_rejected() {
    let root = temp_root("derived");
    let mut source = fixture(&root, "run", true);
    write_json(
        &root.join("run/result.json"),
        &json!({"ok":true,"command":"screenshot assert-startup-presented","failures":[]}),
    );
    source["images"][0]["role"] = json!("contact_sheet");
    let selection = root.join("selection.json");
    write_json(
        &selection,
        &json!({"schema_version":1,"title":"Derived","sources":[source.clone()]}),
    );
    let out = root.join("export");
    let (ok, body) = cli(&selection, &out);
    assert!(ok, "{body}");
    let manifest: Value =
        serde_json::from_slice(&fs::read(out.join("evidence.json")).unwrap()).unwrap();
    let image = &manifest["sources"][0]["images"][0];
    let summary: Value =
        serde_json::from_slice(&fs::read(out.join("sources/run/result-summary.json")).unwrap())
            .unwrap();
    assert_eq!(summary["command"], "screenshot assert-startup-presented");
    assert_eq!(image["capture_status"], "derived");
    assert_eq!(image["source_artifact_kind"], "derived_contact_sheet");
    assert_eq!(image["transformation"], "byte_exact_copy");
    assert_eq!(image["dimensions"], json!([2, 2]));
    source["metrics"][0]["fields"] = json!({"pin":"/data/connection/pin"});
    write_json(
        &selection,
        &json!({"schema_version":1,"title":"Derived","sources":[source]}),
    );
    let (ok, body) = cli(&selection, &root.join("unsafe"));
    assert!(!ok);
    assert_eq!(body["failures"][0]["type"], "export_selection_invalid");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_png_with_only_a_valid_signature() {
    let root = temp_root("corrupt-png");
    let source = fixture(&root, "run", true);
    fs::write(
        root.join("run/snapshots/scene.png"),
        b"\x89PNG\r\n\x1a\nnot an image",
    )
    .unwrap();
    let selection = root.join("selection.json");
    write_json(
        &selection,
        &json!({"schema_version":1,"title":"Corrupt","sources":[source]}),
    );
    let (ok, body) = cli(&selection, &root.join("export"));
    assert!(!ok);
    assert_eq!(body["failures"][0]["type"], "export_image_invalid");
    fs::remove_dir_all(root).unwrap();
}
