use std::process::Command;

use godot_qa_scene::{Severity, diff, parse_scene};

const ROOT: &str = "[node name=\"Root\" type=\"Node\"]\nvalue = ";

#[test]
fn excessive_nesting_returns_error_without_aborting() {
    const CHILD: &str = "GODOT_QA_SCENE_NESTING_CHILD";
    if std::env::var_os(CHILD).is_some() {
        for (open, close) in [("[", "]"), ("{\"key\":", "}"), ("Foo(", ")")] {
            let source = format!("{ROOT}{}0{}", open.repeat(10_000), close.repeat(10_000));
            let scene = parse_scene(&source);
            assert!(scene.has_errors(), "expected fatal nesting diagnostic");
            assert!(scene.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "godot_parser_nesting_limit_exceeded"
                    && diagnostic.severity == Severity::Error
                    && diagnostic.line == Some(2)
            }));
            let out = diff(&parse_scene(&format!("{ROOT}0")), &scene);
            assert!(out.requires_full_reload());
            assert!(out.ops.is_empty());
        }
        return;
    }

    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "excessive_nesting_returns_error_without_aborting",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "nested parser subprocess failed: {}\n{}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn malformed_values_force_diff_fallback_without_ops() {
    let valid = parse_scene(&format!("{ROOT}true"));
    for value in [
        "",
        "@",
        "+",
        "1e",
        "[1, @]",
        "[1, 2",
        "[1 3]",
        "{\"a\" 1}",
        "{\"a\": 1",
        "{\"a\": 1 \"b\": 2}",
        "\"unfinished",
        "Foo(1",
        "Foo(1 2)",
        "&1",
        "1 @",
    ] {
        let malformed = parse_scene(&format!("{ROOT}{value}"));
        // Either baseline can be partial: neither direction is safe to edit live.
        for (old, new) in [(&valid, &malformed), (&malformed, &valid)] {
            let out = diff(old, new);
            assert!(out.requires_full_reload(), "value {value:?}: {out:?}");
            assert!(out.ops.is_empty(), "value {value:?}: {out:?}");
        }
        assert!(malformed.has_errors(), "value {value:?}");
        assert!(malformed.diagnostics.iter().all(|diagnostic| {
            diagnostic.severity == Severity::Error && diagnostic.line == Some(2)
        }));
    }
}

#[test]
fn missing_separators_report_error() {
    let mut missing = Vec::new();
    for value in ["[1 3]", "{\"a\": 1 \"b\": 2}", "Foo(1 2)"] {
        let scene = parse_scene(&format!("{ROOT}{value}"));
        if !scene.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "godot_parser_missing_separator"
                && diagnostic.severity == Severity::Error
                && diagnostic.line == Some(2)
        }) {
            missing.push(value);
        }
    }
    assert!(missing.is_empty(), "missing separator errors: {missing:?}");
}

#[test]
fn valid_container_separators_remain_supported() {
    for value in [
        "[1, ; comment\n2,]",
        "{\"a\": 1, ; comment\n\"b\": 2,}",
        "Foo(1, 2,)",
        "Array[NodePath]([])",
    ] {
        let scene = parse_scene(&format!("{ROOT}{value}"));
        assert!(
            scene.diagnostics.is_empty(),
            "value {value:?}: {:?}",
            scene.diagnostics
        );
    }
}

#[test]
fn moderate_nesting_remains_supported() {
    for (open, close) in [("[", "]"), ("{\"key\":", "}"), ("Foo(", ")")] {
        let source = format!("{ROOT}{}0{}", open.repeat(32), close.repeat(32));
        let scene = parse_scene(&source);
        assert!(scene.diagnostics.is_empty(), "{:?}", scene.diagnostics);
    }
}
