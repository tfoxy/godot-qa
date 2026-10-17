use godot_qa_scenario::audio::{
    AudioAnalysisOptions, analyze_wav_file, analyze_wav_file_with_options,
    write_audio_analysis_artifact,
};
use serde_json::Value;
use std::f64::consts::PI;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn audio_analysis_reports_non_silent_sine_metrics() {
    let path = write_fixture(
        "sine",
        wav_pcm16(44_100, 1, &sine_samples(440.0, 0.25, 0.5)),
    );

    let analysis = analyze_wav_file(&path).unwrap();

    assert_eq!(analysis["sample_rate"], 44_100);
    assert_eq!(analysis["channels"], 1);
    assert!(number(&analysis, "duration_seconds") > 0.24);
    assert!(number(&analysis, "peak") > 0.45);
    assert!(number(&analysis, "rms") > 0.30);
    assert!(number(&analysis, "silence_ratio") < 0.02);
    assert_eq!(analysis["near_silence"], false);
    assert!(number(&analysis, "near_silence_ratio") < 0.02);
    assert_eq!(analysis["clipping_sample_count"], 0);
    assert_eq!(analysis["clipping_region_count"], 0);
    assert!(number(&analysis, "zero_crossing_rate") > 0.0);
    assert!(analysis["transient_count"].as_u64().unwrap() < 10);
    assert!(number(&analysis, "transients_per_second") >= 0.0);
    assert!(analysis["spectral_bands"]["mid"].as_f64().unwrap() > 0.80);
    assert_eq!(number(&analysis, "window_seconds"), 1.0);
    assert_eq!(number(&analysis, "hop_seconds"), 0.5);
    assert!(analysis.get("samples").is_none());
}

#[test]
fn audio_analysis_reports_silence_and_near_silence() {
    let path = write_fixture("silence", wav_pcm16(8_000, 2, &vec![0.0; 800]));

    let analysis = analyze_wav_file(&path).unwrap();

    assert_eq!(analysis["channels"], 2);
    assert_eq!(analysis["near_silence"], true);
    assert_eq!(number(&analysis, "near_silence_ratio"), 1.0);
    assert_eq!(number(&analysis, "peak"), 0.0);
    assert_eq!(number(&analysis, "rms"), 0.0);
    assert_eq!(number(&analysis, "silence_ratio"), 1.0);
    assert_eq!(analysis["warning_ids"][0], "audio_analysis.near_silent");
    assert_eq!(analysis["spectral_bands"]["low"].as_f64().unwrap(), 0.0);
    assert_eq!(analysis["spectral_bands"]["mid"].as_f64().unwrap(), 0.0);
    assert_eq!(analysis["spectral_bands"]["high"].as_f64().unwrap(), 0.0);
}

#[test]
fn audio_analysis_counts_clipped_samples() {
    let samples = vec![1.0, -1.0, 0.4, -0.4, 1.0, 0.0, -1.0, 0.2];
    let path = write_fixture("clipped", wav_pcm16(8_000, 1, &samples));

    let analysis = analyze_wav_file(&path).unwrap();

    assert!(analysis["clipping_sample_count"].as_u64().unwrap() >= 4);
    assert!(analysis["clipping_region_count"].as_u64().unwrap() >= 3);
    assert!(analysis["clipping_regions"].as_array().unwrap().len() >= 3);
    assert!(warning_ids(&analysis).contains(&"audio_analysis.clipping_risk".to_string()));
    assert!(number(&analysis, "peak") >= 0.999);
}

#[test]
fn audio_analysis_counts_transients() {
    let mut samples = Vec::new();
    for idx in 0..200 {
        samples.push(if idx % 2 == 0 { 0.8 } else { -0.8 });
    }
    let path = write_fixture("transient", wav_pcm16(8_000, 1, &samples));

    let analysis = analyze_wav_file(&path).unwrap();

    assert!(analysis["transient_count"].as_u64().unwrap() > 150);
    assert!(number(&analysis, "transients_per_second") > 1000.0);
    assert!(
        !analysis["suspected_event_spam_regions"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(warning_ids(&analysis).contains(&"audio_analysis.event_spam_risk".to_string()));
    assert!(number(&analysis, "zero_crossing_rate") > 0.9);
}

#[test]
fn audio_analysis_reports_loop_boundary_discontinuity() {
    let mut samples = sine_samples(220.0, 0.1, 0.4);
    samples[0] = -0.9;
    let last = samples.len() - 1;
    samples[last] = 0.9;
    let path = write_fixture("loop_boundary", wav_pcm16(8_000, 1, &samples));

    let analysis = analyze_wav_file(&path).unwrap();

    assert!(number(&analysis, "loop_boundary_delta") > 1.7);
    assert!(warning_ids(&analysis).contains(&"audio_analysis.loop_discontinuity_high".to_string()));
}

#[test]
fn audio_analysis_writes_compact_json_artifact() {
    let wav_path = write_fixture(
        "artifact_source",
        wav_pcm16(8_000, 1, &sine_samples(440.0, 0.05, 0.5)),
    );
    let analysis = analyze_wav_file(&wav_path).unwrap();
    let artifact_path = temp_path("audio_analysis_artifact").with_extension("json");

    write_audio_analysis_artifact(&artifact_path, &analysis).unwrap();

    let written: Value = serde_json::from_str(&fs::read_to_string(artifact_path).unwrap()).unwrap();
    assert_eq!(written["sample_rate"], 8_000);
    assert!(written["spectral_bands"].as_object().unwrap().len() == 3);
    assert!(written.get("samples").is_none());
    assert!(written.get("raw_samples").is_none());
    assert!(written["peak_by_window"].is_array());
    assert!(written["rms_by_window"].is_array());
    assert!(written["transients_by_window"].is_array());
}

#[test]
fn audio_analysis_reports_windowed_peak_rms_and_transients() {
    let sample_rate = 100;
    let mut samples = vec![0.0; 200];
    for sample in samples.iter_mut().take(100).skip(50) {
        *sample = 0.5;
    }
    for (idx, sample) in samples.iter_mut().enumerate().take(150).skip(100) {
        *sample = if idx % 2 == 0 { 0.8 } else { -0.8 };
    }
    let path = write_fixture("windows", wav_pcm16(sample_rate, 1, &samples));

    let analysis = analyze_wav_file_with_options(
        &path,
        AudioAnalysisOptions {
            window_seconds: 0.5,
            hop_seconds: 0.5,
        },
    )
    .unwrap();

    assert_eq!(analysis["peak_by_window"].as_array().unwrap().len(), 4);
    assert_eq!(analysis["rms_by_window"].as_array().unwrap().len(), 4);
    assert_eq!(
        analysis["transients_by_window"].as_array().unwrap().len(),
        4
    );
    assert_eq!(analysis["peak_by_window"][0]["value"], 0.0);
    assert!(analysis["peak_by_window"][1]["value"].as_f64().unwrap() > 0.49);
    assert!(
        analysis["transients_by_window"][2]["count"]
            .as_u64()
            .unwrap()
            > 40
    );
}

#[test]
fn audio_analysis_reports_silence_regions() {
    let mut samples = vec![0.0; 50];
    samples.extend(vec![0.25; 50]);
    samples.extend(vec![0.0; 50]);
    let path = write_fixture("silence_regions", wav_pcm16(100, 1, &samples));

    let analysis = analyze_wav_file_with_options(
        &path,
        AudioAnalysisOptions {
            window_seconds: 0.5,
            hop_seconds: 0.5,
        },
    )
    .unwrap();

    let regions = analysis["silence_regions"].as_array().unwrap();
    assert_eq!(regions.len(), 2);
    assert_eq!(regions[0]["start_seconds"], 0.0);
    assert!(regions[0]["duration_seconds"].as_f64().unwrap() >= 0.5);
}

#[test]
fn audio_analysis_rejects_unbounded_window_configuration() {
    let path = write_fixture("too_many_windows", wav_pcm16(100, 1, &vec![0.0; 500]));

    let error = analyze_wav_file_with_options(
        &path,
        AudioAnalysisOptions {
            window_seconds: 0.5,
            hop_seconds: 0.0001,
        },
    )
    .unwrap_err();

    assert_eq!(error.code, "audio_analysis_invalid_options");
}

#[test]
fn audio_analysis_rejects_invalid_riff() {
    let path = write_fixture("invalid_riff", b"not a wave".to_vec());

    let error = analyze_wav_file(&path).unwrap_err();

    assert_eq!(error.code, "audio_invalid_riff");
}

#[test]
fn audio_analysis_rejects_missing_data_chunk() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&28u32.to_le_bytes());
    bytes.extend_from_slice(b"WAVE");
    bytes.extend_from_slice(b"fmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&8_000u32.to_le_bytes());
    bytes.extend_from_slice(&16_000u32.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    let path = write_fixture("missing_data", bytes);

    let error = analyze_wav_file(&path).unwrap_err();

    assert_eq!(error.code, "audio_missing_data_chunk");
}

#[test]
fn audio_analysis_rejects_unsupported_format() {
    let mut bytes = wav_pcm16(8_000, 1, &[0.0, 0.1]);
    bytes[20..22].copy_from_slice(&6u16.to_le_bytes());
    let path = write_fixture("unsupported", bytes);

    let error = analyze_wav_file(&path).unwrap_err();

    assert_eq!(error.code, "audio_unsupported_format");
}

#[test]
fn audio_analysis_reports_pure_tone_timbre_metrics() {
    let path = write_fixture(
        "pure_tone_metrics",
        wav_pcm16(44_100, 1, &sine_samples(440.0, 0.5, 0.5)),
    );

    let analysis = analyze_wav_file(&path).unwrap();

    // A pure tone is maximally tonal: near-zero flatness, centroid at the fundamental,
    // a single strong spectral peak, and a sine's peak-to-RMS crest of ~sqrt(2).
    assert!(
        number(&analysis, "spectral_flatness") < 0.01,
        "flatness {}",
        analysis["spectral_flatness"]
    );
    let centroid = number(&analysis, "spectral_centroid_hz");
    assert!((400.0..500.0).contains(&centroid), "centroid {centroid}");
    let crest = number(&analysis, "crest_factor");
    assert!((1.3..1.55).contains(&crest), "crest {crest}");
    assert_eq!(analysis["strong_peak_count"].as_u64().unwrap(), 1);
}

#[test]
fn audio_analysis_distinguishes_tonal_from_noisy_timbre() {
    let tone = write_fixture(
        "timbre_tone",
        wav_pcm16(44_100, 1, &sine_samples(440.0, 0.5, 0.5)),
    );
    let noise = write_fixture(
        "timbre_noise",
        wav_pcm16(44_100, 1, &pseudo_noise(22_050, 0.5)),
    );

    let tone_analysis = analyze_wav_file(&tone).unwrap();
    let noise_analysis = analyze_wav_file(&noise).unwrap();

    // Broadband noise is spectrally flat (~1); a pure tone is not.
    assert!(number(&noise_analysis, "spectral_flatness") > 0.5);
    assert!(
        number(&noise_analysis, "spectral_flatness") > number(&tone_analysis, "spectral_flatness")
    );
    assert!(
        number(&noise_analysis, "spectral_centroid_hz")
            > number(&tone_analysis, "spectral_centroid_hz")
    );
}

#[test]
fn audio_analysis_flags_sustained_tonal_drone_but_not_short_beep() {
    let drone = write_fixture(
        "tonal_drone",
        wav_pcm16(44_100, 1, &sine_samples(220.0, 4.0, 0.5)),
    );
    let beep = write_fixture(
        "tonal_beep",
        wav_pcm16(44_100, 1, &sine_samples(220.0, 0.5, 0.5)),
    );

    let drone_warnings = warning_ids(&analyze_wav_file(&drone).unwrap());
    let beep_warnings = warning_ids(&analyze_wav_file(&beep).unwrap());

    assert!(drone_warnings.contains(&"audio_analysis.tonal_drone".to_string()));
    // A short tonal beep is legitimately tonal and must not be flagged as a drone.
    assert!(!beep_warnings.contains(&"audio_analysis.tonal_drone".to_string()));
}

#[test]
fn audio_analysis_air_ratio_tracks_brightness() {
    let bright = write_fixture(
        "bright_tone",
        wav_pcm16(44_100, 1, &sine_samples(6_000.0, 0.5, 0.5)),
    );
    let dull = write_fixture(
        "dull_tone",
        wav_pcm16(44_100, 1, &sine_samples(300.0, 0.5, 0.5)),
    );

    let bright_analysis = analyze_wav_file(&bright).unwrap();
    let dull_analysis = analyze_wav_file(&dull).unwrap();

    assert!(number(&bright_analysis, "air_ratio") > 0.9);
    assert!(number(&dull_analysis, "air_ratio") < 0.1);
    // The dull (low) tone has no high-frequency air; the bright tone does.
    assert!(warning_ids(&dull_analysis).contains(&"audio_analysis.dull".to_string()));
    assert!(!warning_ids(&bright_analysis).contains(&"audio_analysis.dull".to_string()));
}

fn pseudo_noise(sample_count: usize, amplitude: f64) -> Vec<f64> {
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    (0..sample_count)
        .map(|_| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((state >> 33) as f64 / (1u64 << 31) as f64) - 1.0
        })
        .map(|unit| unit * amplitude)
        .collect()
}

fn sine_samples(frequency: f64, seconds: f64, amplitude: f64) -> Vec<f64> {
    let sample_rate = 44_100.0;
    let sample_count = (sample_rate * seconds) as usize;
    (0..sample_count)
        .map(|idx| {
            let phase = 2.0 * PI * frequency * idx as f64 / sample_rate;
            phase.sin() * amplitude
        })
        .collect()
}

fn wav_pcm16(sample_rate: u32, channels: u16, mono_samples: &[f64]) -> Vec<u8> {
    let mut data = Vec::new();
    for sample in mono_samples {
        let clamped = sample.clamp(-1.0, 1.0);
        let value = if clamped >= 1.0 {
            i16::MAX
        } else if clamped <= -1.0 {
            i16::MIN
        } else {
            (clamped * f64::from(i16::MAX)) as i16
        };
        for _ in 0..channels {
            data.extend_from_slice(&value.to_le_bytes());
        }
    }

    let block_align = channels * 2;
    let byte_rate = sample_rate * u32::from(block_align);
    let riff_size = 4 + 8 + 16 + 8 + data.len() as u32;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&riff_size.to_le_bytes());
    bytes.extend_from_slice(b"WAVE");
    bytes.extend_from_slice(b"fmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&channels.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&byte_rate.to_le_bytes());
    bytes.extend_from_slice(&block_align.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&data);
    bytes
}

fn write_fixture(name: &str, bytes: Vec<u8>) -> PathBuf {
    let path = temp_path(name).with_extension("wav");
    fs::write(&path, bytes).unwrap();
    path
}

fn temp_path(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("godot_qa_{name}_{unique}"))
}

fn number(value: &Value, key: &str) -> f64 {
    value[key].as_f64().unwrap()
}

fn warning_ids(value: &Value) -> Vec<String> {
    value["warning_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap().to_string())
        .collect()
}
