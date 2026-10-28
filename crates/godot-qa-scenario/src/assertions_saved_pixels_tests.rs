use super::*;
use image::{Rgba, RgbaImage};
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn cached_saved_pixel_assertions_preserve_target_threshold_and_failure_results() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("godot-qa-saved-pixels-{unique}"));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("screenshot.png");
    RgbaImage::from_fn(128, 128, |x, y| {
        if x < 32 && y < 64 {
            Rgba([255, 64, 32, 255])
        } else {
            Rgba([8, 8, 8, 255])
        }
    })
    .save(&path)
    .unwrap();
    let artifacts = Map::from_iter([
        ("run_root".to_string(), json!(root)),
        ("screenshot".to_string(), json!(path)),
    ]);
    let context = json!({
        "tree": {"root": {
            "path": "/root/Main", "name": "Main", "children": [
                {"path": "/root/Main/foreground", "qa_id": "foreground", "visual_evidence": {
                    "coordinate_space": "viewport_2d", "rect": [0, 0, 32, 64]
                }},
                {"path": "/root/Main/background", "qa_id": "background", "visual_evidence": {
                    "coordinate_space": "viewport_2d", "rect": [32, 0, 32, 64]
                }},
                {"path": "/root/Main/unsupported", "qa_id": "unsupported"}
            ]
        }},
        "ui": {"controls": []}
    });
    let indexed = SavedSnapshotPixelContext::new(&context);
    let mut cache = screenshots::ScreenshotAnalysisCache::default();
    let mut failure_kinds = Vec::new();
    for target in [
        "foreground",
        "background",
        "unsupported",
        "missing",
        "foreground",
    ] {
        let assertion = json!({"target": format!("qa:{target}"), "min_count": 1, "min_ratio": 0.5});
        let assertion = assertion.as_object().unwrap();
        let mut standalone = assertion.clone();
        standalone.insert("name".to_string(), json!("visible_pixels"));
        let expected = evaluate_saved_visual_assertion(&standalone, &context, &artifacts);
        let cached = evaluate_saved_snapshot_pixels(assertion, &indexed, &artifacts, &mut cache);
        match (expected, cached) {
            (Ok(expected), Ok(cached)) => {
                assert_eq!(expected.data, cached.data);
                assert_eq!(
                    serde_json::to_value(expected.failure).unwrap(),
                    serde_json::to_value(&cached.failure).unwrap()
                );
                if let Some(failure) = cached.failure {
                    failure_kinds.push(failure.kind);
                }
            }
            (Err(expected), Err(cached)) => {
                assert_eq!(
                    serde_json::to_value(expected).unwrap(),
                    serde_json::to_value(&cached).unwrap()
                );
                failure_kinds.push(cached.kind);
            }
            _ => panic!("cached and standalone outcome shapes differ"),
        }
    }
    assert_eq!(
        failure_kinds,
        [
            "visible_pixels_threshold_mismatch",
            "unsupported_target_geometry",
            "target_not_found"
        ]
    );
    std::fs::remove_dir_all(root).unwrap();
}
