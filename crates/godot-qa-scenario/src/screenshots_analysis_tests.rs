use super::*;
use std::hint::black_box;
use std::time::Instant;

fn temp_root(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("godot-qa-analysis-{label}-{unique}"));
    fs::create_dir_all(&root).unwrap();
    root
}

fn useful_image() -> RgbaImage {
    RgbaImage::from_fn(128, 128, |x, y| {
        if x < 32 && y < 64 {
            Rgba([255, 64, 32, 255])
        } else {
            Rgba([8, 8, 8, 255])
        }
    })
}

#[test]
fn target_analysis_preserves_quality_clipping_and_low_information_contracts() {
    let root = temp_root("metrics");
    let path = root.join("useful.png");
    useful_image().save(&path).unwrap();
    let analysis = ScreenshotAnalysis::load(&path).unwrap();
    let quality = analysis.quality();
    assert_eq!(quality["non_background_pixel_count"], 2048);
    assert_eq!(quality["non_background_ratio"], 0.125);
    assert_eq!(quality["low_information"], false);
    assert_eq!(quality["dominant_colors"][0]["rgba"], json!([8, 8, 8, 255]));
    assert_eq!(quality["dominant_colors"][1]["count"], 2048);

    // Fractional edges round outward and negative bounds clip to the image.
    let rect = [-0.5, -1.0, 32.25, 65.0];
    let measured = analysis.measure_target(rect);
    assert_eq!(measured["target_rect"], json!(rect));
    assert_eq!(measured["measured_pixel_count"], 2048);
    assert_eq!(measured["useful_pixel_count"], 2048);
    assert_eq!(measured["useful_pixel_ratio"], 1.0);
    assert_eq!(measured["screenshot_quality"], *quality);
    assert_eq!(
        analysis.measure_target([256.0, 256.0, 10.0, 10.0])["measured_pixel_count"],
        0
    );
    assert_eq!(
        analysis.measure_target([32.0, 0.0, 32.0, 64.0])["useful_pixel_count"],
        0
    );

    for (label, image) in [
        ("transparent", RgbaImage::new(128, 128)),
        (
            "small",
            RgbaImage::from_pixel(8, 8, Rgba([255, 64, 32, 255])),
        ),
    ] {
        let path = root.join(format!("{label}.png"));
        image.save(&path).unwrap();
        let evidence = measure_saved_target_pixels(&path, [0.0, 0.0, 10.0, 10.0]).unwrap();
        assert_eq!(evidence["low_information"], true);
        assert_eq!(evidence["useful_pixel_count"], 0);
        assert_eq!(evidence["useful_pixel_ratio"], 0.0);
        assert!(evidence.get("measured_pixel_count").is_none());
        assert_eq!(
            evidence["low_information_reasons"],
            evidence["screenshot_quality"]["low_information_reasons"]
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn analysis_cache_reuses_pixels_and_quality_but_is_scoped_and_bounded() {
    let root = temp_root("cache");
    let paths: Vec<_> = (0..4).map(|i| root.join(format!("{i}.png"))).collect();
    for path in &paths {
        useful_image().save(path).unwrap();
    }
    let mut cache = ScreenshotAnalysisCache::default();
    let original = cache
        .get(&paths[0])
        .unwrap()
        .measure_target([0.0, 0.0, 32.0, 64.0]);
    let pixels = cache.get(&paths[0]).unwrap().pixels.as_ptr();
    assert_eq!(pixels, cache.get(&paths[0]).unwrap().pixels.as_ptr());
    assert!(cache.get(&paths[0]).unwrap().quality.get().is_some());
    // Finalization artifacts are immutable. Prove a second target doesn't reread
    // PNG bytes, while a fresh finalization does see an unreadable artifact.
    fs::write(&paths[0], b"invalid PNG").unwrap();
    assert_eq!(
        cache
            .get(&paths[0])
            .unwrap()
            .measure_target([0.0, 0.0, 32.0, 64.0]),
        original
    );
    assert!(ScreenshotAnalysisCache::default().get(&paths[0]).is_err());
    useful_image().save(&paths[0]).unwrap();
    for path in &paths[1..] {
        cache.get(path).unwrap().quality();
        assert!(cache.entries.len() <= 2);
        assert!(cache.recent.len() <= 2);
    }
    assert!(!cache.entries.contains_key(&paths[0]));
    let reloaded = cache.get(&paths[0]).unwrap();
    // Its retained quality is already initialized; no whole-image rescan.
    assert!(reloaded.quality.get().is_some());
    assert_eq!(reloaded.measure_target([0.0, 0.0, 32.0, 64.0]), original);
    assert_eq!(cache.entries.len(), 2);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cached_comparisons_preserve_results_errors_and_exact_diff_pixels() {
    let root = temp_root("compare");
    let actual = root.join("actual.png");
    let baseline = root.join("baseline.png");
    let diff = root.join("diff.png");
    let metrics = root.join("metrics.json");
    let mut cache = ScreenshotAnalysisCache::default();
    fs::write(&actual, b"invalid PNG").unwrap();
    let missing = compare_screenshot_files_cached(
        "scenario run",
        &actual,
        &baseline,
        0.0,
        Some(&diff),
        Some(&metrics),
        &mut cache,
    );
    assert_eq!(missing.failures[0].kind, "baseline_not_found");
    assert!(cache.entries.is_empty());
    useful_image().save(&actual).unwrap();
    useful_image().save(&baseline).unwrap();
    let mut changed = useful_image();
    changed.put_pixel(5, 10, Rgba([16, 32, 64, 255]));
    changed.save(&actual).unwrap();
    for (size, threshold) in [((128, 128), 0.0), ((128, 128), 1.0), ((129, 128), 0.0)] {
        if size.0 != 128 {
            RgbaImage::from_pixel(size.0, size.1, Rgba([8, 8, 8, 255]))
                .save(&baseline)
                .unwrap();
        }
        let original = compare_screenshot_files(
            "scenario run",
            &actual,
            &baseline,
            threshold,
            Some(&diff),
            Some(&metrics),
        );
        let expected_diff = diff
            .is_file()
            .then(|| image::open(&diff).unwrap().into_rgba8());
        if diff.is_file() {
            fs::remove_file(&diff).unwrap();
        }
        let cached = compare_screenshot_files_cached(
            "scenario run",
            &actual,
            &baseline,
            threshold,
            Some(&diff),
            Some(&metrics),
            &mut cache,
        );
        assert_eq!(
            serde_json::to_value(cached).unwrap(),
            serde_json::to_value(original).unwrap()
        );
        assert_eq!(diff.is_file(), expected_diff.is_some());
        if let Some(expected) = expected_diff {
            let saved = image::open(&diff).unwrap().into_rgba8();
            assert_eq!(saved, expected);
            assert_eq!(saved.get_pixel(5, 10).0, [255, 0, 0, 255]);
            if size.0 == 128 {
                assert_eq!(saved.get_pixel(0, 0).0, [0, 0, 0, 0]);
            }
            fs::remove_file(&diff).unwrap();
        }
    }
    fs::write(&baseline, b"invalid PNG").unwrap();
    assert_eq!(
        compare_screenshot_files_cached(
            "scenario run",
            &actual,
            &baseline,
            0.0,
            None,
            None,
            &mut cache
        )
        .failures[0]
            .kind,
        "image_decode_failed"
    );
    useful_image().save(&baseline).unwrap();
    fs::write(&actual, b"invalid PNG").unwrap();
    let mut errors = ScreenshotAnalysisCache::default();
    let first = compare_screenshot_files_cached(
        "scenario run",
        &actual,
        &baseline,
        0.0,
        None,
        None,
        &mut errors,
    );
    assert_eq!(first.failures[0].kind, "image_decode_failed");
    assert!(errors.entries[&actual].is_err());
    let second = compare_screenshot_files_cached(
        "scenario run",
        &actual,
        &baseline,
        0.0,
        None,
        None,
        &mut errors,
    );
    assert_eq!(
        serde_json::to_value(first).unwrap(),
        serde_json::to_value(second).unwrap()
    );
    // Cached decode errors obey the same bounded eviction as decoded frames.
    let third = root.join("third.png");
    useful_image().save(&third).unwrap();
    errors.get(&baseline).unwrap();
    errors.get(&third).unwrap();
    assert_eq!(errors.entries.len(), 2);
    assert!(!errors.entries.contains_key(&actual));
    fs::remove_dir_all(root).unwrap();
}

/// Reproduce with:
/// cargo test -p godot-qa-scenario --release screenshot_analysis_repeated_targets_benchmark -- --ignored --nocapture
///
/// The uncached reference explicitly reproduces the old two-decode/one-quality
/// scan per target. It omits the old extra RGBA clones, making savings conservative.
/// Cached timing includes the initial decode and quality scan; disk writes are excluded.
#[test]
#[ignore = "manual wall-clock benchmark; run optimized and without other workloads"]
fn screenshot_analysis_repeated_targets_benchmark() {
    const TARGETS: usize = 16;
    let root = temp_root("benchmark");
    for (width, height) in [(1280, 720), (1920, 1080), (3840, 2160)] {
        let path = root.join(format!("{width}x{height}.png"));
        RgbaImage::from_fn(width, height, |x, y| {
            let color = ((x / 64 + y / 64) % 16) as u8;
            Rgba([color * 16, color * 8, 255 - color * 8, 255])
        })
        .save(&path)
        .unwrap();
        let rects: Vec<_> = (0..TARGETS)
            .map(|i| {
                [
                    f64::from(width) * (i % 4) as f64 / 4.0,
                    f64::from(height) * (i / 4) as f64 / 4.0,
                    f64::from(width) / 8.0,
                    f64::from(height) / 8.0,
                ]
            })
            .collect();
        let start = Instant::now();
        let reference: Vec<_> = rects
            .iter()
            .map(|rect| {
                let quality = analyze_screenshot_file(&path).unwrap();
                let analysis = ScreenshotAnalysis::load(&path).unwrap();
                analysis.quality.set(quality).unwrap();
                black_box(analysis.measure_target(*rect))
            })
            .collect();
        let uncached = start.elapsed();
        let start = Instant::now();
        let mut cache = ScreenshotAnalysisCache::default();
        let cached: Vec<_> = rects
            .iter()
            .map(|rect| black_box(cache.get(&path).unwrap().measure_target(*rect)))
            .collect();
        let elapsed = start.elapsed();
        assert_eq!(reference, cached);
        eprintln!(
            "{width}x{height} {TARGETS} targets: reference={:.3}ms cached={:.3}ms speedup={:.2}x",
            uncached.as_secs_f64() * 1000.0,
            elapsed.as_secs_f64() * 1000.0,
            uncached.as_secs_f64() / elapsed.as_secs_f64()
        );
    }
    fs::remove_dir_all(root).unwrap();
}
