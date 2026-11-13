use godot_qa_core::hash::sha256_hex;
use godot_qa_scenario::screenshots::compare_screenshot_files;
use image::{Rgba, RgbaImage};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("godot-qa-{name}-{unique}"));
    fs::create_dir_all(&root).unwrap();
    root
}

fn assert_output_alias_rejected(alias: impl Fn(&Path, &Path) -> PathBuf) {
    for input_is_actual in [false, true] {
        for output_is_diff in [false, true] {
            let root = temp_dir("compare-alias");
            let actual = root.join("actual.png");
            let baseline = root.join("baseline.png");
            RgbaImage::from_pixel(2, 2, Rgba([10, 20, 30, 255]))
                .save(&actual)
                .unwrap();
            RgbaImage::from_pixel(2, 2, Rgba([90, 80, 70, 255]))
                .save(&baseline)
                .unwrap();
            let before = [&actual, &baseline].map(|p| sha256_hex(&fs::read(p).unwrap()));
            let input = if input_is_actual { &actual } else { &baseline };
            let output = alias(&root, input);
            let safe = root.join("new-artifacts").join(if output_is_diff {
                "metrics.json"
            } else {
                "diff.png"
            });
            let (diff, metrics) = if output_is_diff {
                (&output, &safe)
            } else {
                (&safe, &output)
            };
            let result = compare_screenshot_files(
                "screenshot compare",
                &actual,
                &baseline,
                0.0,
                Some(diff),
                Some(metrics),
            );
            let after = [&actual, &baseline].map(|p| sha256_hex(&fs::read(p).unwrap()));
            assert_eq!(before, after, "comparison overwrote an input: {result:?}");
            assert!(
                !safe.exists(),
                "must validate all outputs before writing any artifact"
            );
            assert!(!safe.parent().unwrap().exists());
            assert!(!result.ok);
            assert_eq!(result.failures[0].kind, "screenshot_output_alias");
            assert_eq!(
                result.failures[0].command.as_deref(),
                Some("screenshot compare")
            );
            assert_eq!(result.failures[0].path.as_deref(), output.to_str());
            assert!(result.failures[0].expected.is_some());
            assert!(result.failures[0].actual.is_some());
            fs::remove_dir_all(root).unwrap();
        }
    }
}

#[test]
fn screenshot_compare_rejects_direct_input_outputs() {
    assert_output_alias_rejected(|_, input| input.to_path_buf());
}

#[test]
fn screenshot_compare_rejects_canonical_input_outputs() {
    assert_output_alias_rejected(|root, input| {
        fs::create_dir(root.join("subdir")).unwrap();
        root.join("subdir/..").join(input.file_name().unwrap())
    });
}

#[test]
fn screenshot_compare_rejects_hard_link_input_outputs() {
    assert_output_alias_rejected(|root, input| {
        let output = root.join("output.png");
        fs::hard_link(input, &output).unwrap();
        output
    });
}

#[cfg(unix)]
#[test]
fn screenshot_compare_rejects_symlink_input_outputs() {
    assert_output_alias_rejected(|root, input| {
        let output = root.join("output.png");
        std::os::unix::fs::symlink(input, &output).unwrap();
        output
    });
}

#[test]
fn screenshot_compare_rejects_oversized_union_canvas() {
    const CHILD_ROOT: &str = "GODOT_QA_COMPARE_LIMIT_CHILD_ROOT";
    if let Some(root) = std::env::var_os(CHILD_ROOT) {
        let root = PathBuf::from(root);
        let actual = root.join("actual.png");
        let baseline = root.join("baseline.png");
        let before = [&actual, &baseline].map(|p| sha256_hex(&fs::read(p).unwrap()));
        let diff = root.join("artifacts/diff.png");
        let metrics = root.join("artifacts/metrics.json");
        for write_outputs in [false, true] {
            let result = compare_screenshot_files(
                "screenshot compare",
                &actual,
                &baseline,
                1.0,
                write_outputs.then_some(diff.as_path()),
                write_outputs.then_some(metrics.as_path()),
            );
            assert!(!result.ok);
            assert_eq!(result.failures[0].kind, "screenshot_compare_resource_limit");
            assert_eq!(
                result.failures[0].command.as_deref(),
                Some("screenshot compare")
            );
            assert_eq!(result.failures[0].path.as_deref(), actual.to_str());
            assert_eq!(
                result.failures[0].expected,
                Some(serde_json::json!({"max_comparison_pixels": 64_000_000}))
            );
            assert_eq!(
                result.failures[0].actual,
                Some(serde_json::json!({
                    "actual_size": [100_000, 1],
                    "baseline_size": [1, 100_000],
                    "comparison_size": [100_000, 100_000],
                    "comparison_pixels": 10_000_000_000u64,
                }))
            );
            assert_eq!(result.data["actual"], serde_json::json!(actual));
            assert_eq!(result.data["baseline"], serde_json::json!(baseline));
            assert!(!root.join("artifacts").exists());
            assert_eq!(
                before,
                [&actual, &baseline].map(|p| sha256_hex(&fs::read(p).unwrap()))
            );
        }
        return;
    }

    let root = temp_dir("compare-oversized-canvas");
    for (name, width, height) in [("actual.png", 100_000, 1), ("baseline.png", 1, 100_000)] {
        RgbaImage::from_pixel(width, height, Rgba([10, 20, 30, 255]))
            .save(root.join(name))
            .unwrap();
    }
    // Bound the regression itself: the old comparator scans ten billion pixels
    // before attempting a 40 GB diff allocation.
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "screenshot_compare_rejects_oversized_union_canvas",
            "--nocapture",
        ])
        .env(CHILD_ROOT, &root)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let started = std::time::Instant::now();
    loop {
        if child.try_wait().unwrap().is_some() {
            let output = child.wait_with_output().unwrap();
            fs::remove_dir_all(root).unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            break;
        }
        if started.elapsed() > std::time::Duration::from_secs(5) {
            child.kill().unwrap();
            child.wait().unwrap();
            fs::remove_dir_all(root).unwrap();
            panic!("oversized screenshot comparison did not reject the canvas within five seconds");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}
