use serde_json::{Value, json};
use std::{fs, path::PathBuf, process::Command, time::SystemTime};

struct SavedEvidence(PathBuf);
impl SavedEvidence {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "godot-qa-saved-assert-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join(".qa/artifacts/demo/run/snapshots/final")).unwrap();
        fs::write(root.join("project.godot"), "config_version=5\n").unwrap();
        let fixture = Self(root);
        fixture.write("result.json", json!({"snapshots": ["final"]}));
        fixture.snapshot(
            "tree",
            json!({"root": {"path": "/root/Main", "metadata": {}, "children": [
                {"path": "/root/Main/Button", "qa_id": "button", "visible": true, "metadata": {}},
                {"path": "/root/Main/Other", "qa_id": "other", "visible": false, "metadata": {}}
            ]}}),
        );
        fixture.snapshot("ui", json!({"viewport": [100, 100], "controls": [
            {"path": "/root/Main/Button", "text": "Wrong", "disabled": true, "global_rect": [-1, 0, 20, 20]},
            {"path": "/root/Main/Other", "disabled": false, "global_rect": [0, 0, 20, 20]}
        ]}));
        fixture.snapshot("focus", json!({"focus_owner_path": "/root/Main/Other"}));
        fixture.snapshot(
            "errors",
            json!({"count": 1, "errors": [{"message": "runtime failure"}]}),
        );
        fixture
    }
    fn write(&self, path: &str, value: Value) {
        fs::write(
            self.0.join(".qa/artifacts/demo/run").join(path),
            value.to_string(),
        )
        .unwrap();
    }
    fn snapshot(&self, name: &str, value: Value) {
        self.write(&format!("snapshots/final/{name}.json"), value);
    }
    fn aliases(&self, aliases: Value) {
        self.snapshot(
            "tree",
            json!({"root": {
                "path": "/root/Main/Button", "qa_id": "button", "metadata": {"qa_aliases": aliases}
            }}),
        );
    }
    fn run(&self, args: &str, code: Option<&str>) {
        let output = Command::new(env!("CARGO_BIN_EXE_godot-qa"))
            .args(["--project"])
            .arg(&self.0)
            .args(["assert"])
            .args(args.split_whitespace())
            .args(["--scenario", "demo", "--run-id", "run", "--json"])
            .output()
            .unwrap();
        let body: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(output.status.success(), code.is_none(), "{args}: {body}");
        assert_eq!(body["ok"], code.is_none(), "{args}: {body}");
        if let Some(code) = code {
            assert_eq!(body["failures"][0]["type"], code, "{args}: {body}");
            assert_eq!(body["failures"][0]["command"], "assert");
            assert!(body["failures"][0]["artifacts"]["tree"].is_string());
        }
    }
}
impl Drop for SavedEvidence {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
macro_rules! saved_failures {
    ($($name:ident: $args:literal => $code:literal;)*) => {$ (
        #[test]
        fn $name() { SavedEvidence::new().run($args, Some($code)); }
    )*};
}
saved_failures! {
    exists: "exists qa:missing" => "target_not_found";
    not_exists: "not_exists qa:button" => "not_exists_mismatch";
    visible: "visible qa:other" => "visible_mismatch";
    hidden: "hidden qa:button" => "hidden_mismatch";
    enabled: "enabled qa:button" => "enabled_mismatch";
    disabled: "disabled qa:other" => "disabled_mismatch";
    focused: "focused qa:button" => "focused_mismatch";
    text_equals: "text_equals qa:button Expected" => "text_equals_mismatch";
    text_contains: "text_contains qa:button Expected" => "text_contains_mismatch";
    inside_viewport: "inside_viewport qa:button" => "inside_viewport_mismatch";
    no_overlap: "no_overlap qa:button qa:other" => "no_overlap_mismatch";
    no_errors: "no_errors" => "runtime_errors_present";
    snapshot_exists: "snapshot_exists qa:missing" => "snapshot_target_missing";
    snapshot_visible: "snapshot_visible qa:other" => "snapshot_visible_mismatch";
    snapshot_contains_type: "snapshot_contains_type MissingType" => "snapshot_type_missing";
    visible_pixels: "visible_pixels qa:button --min-count 1" => "missing_screenshot_image";
    not_occluded: "not_occluded qa:button --occluder qa:other" => "unsupported_saved_assertion";
    renders_above: "renders_above qa:button qa:other" => "unsupported_saved_assertion";
    renders_below: "renders_below qa:button qa:other" => "unsupported_saved_assertion";
    max_delta_position: "max_delta_position qa:button --max 1 --window-frames 2" => "unsupported_saved_assertion";
    follows: "follows qa:button --leader qa:other --tolerance 1 --window-frames 2" => "unsupported_saved_assertion";
    never: "never qa:button state --forbidden bad --window-frames 2" => "unsupported_saved_assertion";
    within_limits: "within_limits qa:button --min 0 --max 1 --window-frames 2" => "unsupported_saved_assertion";
    distance: "distance qa:button qa:other --max 1" => "unsupported_saved_assertion";
    distance_changed: "distance_changed qa:button qa:other --min 1 --window-frames 2" => "unsupported_saved_assertion";
    surface_probe: "surface_probe qa:button --provider qa:other --point 0,0 --surface ground" => "unsupported_saved_assertion";
    surface_matches: "surface_matches qa:button" => "unsupported_saved_assertion";
}

#[test]
fn supported_saved_assertions_pass_with_matching_evidence() {
    let fixture = SavedEvidence::new();
    for args in [
        "exists qa:button",
        "not_exists qa:missing",
        "visible qa:button",
        "hidden qa:other",
        "enabled qa:other",
        "disabled qa:button",
        "focused qa:other",
        "text_equals qa:button Wrong",
        "text_contains qa:button ron",
    ] {
        fixture.run(args, None);
    }
    fixture.snapshot(
        "ui",
        json!({"viewport": [100, 100], "controls": [
        {"path": "/root/Main/Button", "global_rect": [0, 0, 10, 10]},
        {"path": "/root/Main/Other", "global_rect": [10, 0, 10, 10]}]}),
    );
    fixture.run("inside_viewport qa:button", None);
    fixture.run("no_overlap qa:button qa:other", None);
    fixture.snapshot("errors", json!({"count": 0, "errors": []}));
    fixture.run("no_errors", None);
}

#[test]
fn missing_saved_fields_cannot_pass() {
    let fixture = SavedEvidence::new();
    fixture.snapshot("ui", json!({"controls": []}));
    fixture.snapshot("focus", json!({}));
    fixture.snapshot("errors", json!({}));
    for args in [
        "enabled qa:button",
        "disabled qa:button",
        "focused qa:button",
        "text_equals qa:button Wrong",
        "text_contains qa:button Wrong",
        "inside_viewport qa:button",
        "no_overlap qa:button qa:other",
        "no_errors",
    ] {
        fixture.run(args, Some("unsupported_saved_assertion"));
    }
}

#[test]
fn saved_qa_alias_not_exists_fails() {
    let fixture = SavedEvidence::new();
    for alias in ["submit", "qa:submit"] {
        fixture.aliases(json!([alias]));
        fixture.run("not_exists qa:submit", Some("not_exists_mismatch"));
    }
}

#[test]
fn saved_qa_alias_exists_passes() {
    let fixture = SavedEvidence::new();
    for alias in ["submit", "qa:submit"] {
        fixture.aliases(json!([alias]));
        fixture.run("exists qa:submit", None);
    }
}

#[test]
fn saved_qa_alias_deduplicates_same_node() {
    let fixture = SavedEvidence::new();
    fixture.aliases(json!(["button", "qa:button", "button", null, 42]));
    fixture.run("exists qa:button", None);
    fixture.run("not_exists qa:button", Some("not_exists_mismatch"));
}

#[test]
fn saved_qa_alias_reports_ambiguous_nodes() {
    let fixture = SavedEvidence::new();
    fixture.snapshot(
        "tree",
        json!({"root": {
            "path": "/root/Main", "metadata": {}, "children": [
                {"path": "/root/Main/Button", "metadata": {"qa_aliases": ["submit"]}},
                {"path": "/root/Main/Other", "metadata": {"qa_aliases": ["qa:submit"]}}
            ]
        }}),
    );
    fixture.run("exists qa:submit", Some("ambiguous_target"));
    fixture.run("not_exists qa:submit", Some("ambiguous_target"));
}

#[test]
fn saved_qa_absence_requires_metadata() {
    let fixture = SavedEvidence::new();
    for tree in [
        json!({"root": {"path": "/root/Main"}}),
        json!({"root": {"path": "/root/Main", "metadata": {}, "children": [
            {"path": "/root/Main/Button", "qa_id": "button"}
        ]}}),
    ] {
        fixture.snapshot("tree", tree);
        fixture.run("not_exists qa:missing", Some("unsupported_saved_assertion"));
        fixture.run("not_exists path:/root/Main/Missing", None);
        fixture.run("not_exists name:Missing", None);
    }
}
