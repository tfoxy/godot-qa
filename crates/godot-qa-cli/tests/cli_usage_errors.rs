use serde_json::{Value, json};
use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_godot-qa"))
        .args(args)
        .output()
        .unwrap()
}

fn assert_json_usage_error(command: &[&str], json_first: bool, diagnostic: &str) {
    let mut args = command.to_vec();
    if json_first {
        args.insert(0, "--json");
    } else {
        args.push("--json");
    }
    let output = run(&args);
    assert_eq!(output.status.code(), Some(2));
    let body: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid JSON: {error}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(body.as_object().unwrap().len(), 5);
    assert_eq!(body["ok"], false);
    assert_eq!(body["command"], "cli");
    let message = body["message"].as_str().unwrap();
    assert!(message.contains(diagnostic), "{body:#}");
    assert!(message.contains("--help"), "{body:#}");
    assert_eq!(body["data"], json!({}));
    assert_eq!(body["failures"].as_array().unwrap().len(), 1);
    let failure = &body["failures"][0];
    assert_eq!(failure["type"], "cli_usage_error");
    assert_eq!(failure["command"], "cli");
    assert_eq!(failure["message"], message);
    assert_eq!(failure["expected"], "valid CLI arguments (see --help)");
    assert_eq!(failure["actual"], json!(args));
}

#[test]
fn missing_positional_with_json_before_command() {
    assert_json_usage_error(&["input", "joy", "motion", "left_x"], true, "<VALUE>");
}

#[test]
fn missing_positional_with_json_after_command() {
    assert_json_usage_error(&["input", "joy", "motion", "left_x"], false, "<VALUE>");
}

#[test]
fn missing_option_value_with_json_before_command() {
    assert_json_usage_error(
        &["input", "joy", "motion", "left_x", "0.5", "--device"],
        true,
        "--device <DEVICE>",
    );
}

#[test]
fn missing_option_value_with_json_after_command() {
    assert_json_usage_error(
        &["input", "joy", "motion", "left_x", "0.5", "--device"],
        false,
        "--device <DEVICE>",
    );
}

#[test]
fn unknown_option_with_json_before_command() {
    assert_json_usage_error(
        &[
            "input",
            "joy",
            "motion",
            "left_x",
            "0.5",
            "--unknown-option",
        ],
        true,
        "--unknown-option",
    );
}

#[test]
fn unknown_option_with_json_after_command() {
    assert_json_usage_error(
        &[
            "input",
            "joy",
            "motion",
            "left_x",
            "0.5",
            "--unknown-option",
        ],
        false,
        "--unknown-option",
    );
}

#[test]
fn usage_errors_without_json_keep_plain_stderr() {
    for args in [
        vec!["input", "joy", "motion", "left_x"],
        vec!["input", "joy", "motion", "left_x", "0.5", "--device"],
        vec![
            "input",
            "joy",
            "motion",
            "left_x",
            "0.5",
            "--unknown-option",
        ],
    ] {
        let output = run(&args);
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty(), "{output:?}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.starts_with("error:"), "{stderr}");
        assert!(stderr.contains("--help"), "{stderr}");
    }
}

#[test]
fn help_and_version_with_json_keep_plain_stdout() {
    for args in [
        vec!["--help"],
        vec!["--version"],
        vec!["input", "joy", "motion", "--help"],
    ] {
        let plain = run(&args);
        assert_eq!(plain.status.code(), Some(0));
        assert!(plain.stderr.is_empty());
        assert!(!plain.stdout.is_empty());
        for json_first in [true, false] {
            let mut with_json = args.clone();
            if json_first {
                with_json.insert(0, "--json");
            } else {
                with_json.push("--json");
            }
            let output = run(&with_json);
            assert_eq!(output.status.code(), Some(0));
            assert!(output.stderr.is_empty());
            assert_eq!(output.stdout, plain.stdout);
        }
    }
}
