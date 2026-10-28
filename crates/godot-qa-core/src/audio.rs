use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::io::{self, Read};
use std::path::Path;

const MAX_WAV_BYTES: usize = 64 * 1024 * 1024;
const MAX_DECODED_MONO_BYTES: usize = 512 * 1024 * 1024;
const SILENCE_THRESHOLD: f64 = 0.001;
const NEAR_SILENCE_RMS_THRESHOLD: f64 = 0.01;
const CLIPPING_THRESHOLD: f64 = 0.999;
const TRANSIENT_DELTA_THRESHOLD: f64 = 0.35;
const EVENT_SPAM_TRANSIENTS_PER_SECOND: f64 = 40.0;
const HIGH_TRANSIENTS_PER_SECOND: f64 = 80.0;
const LOOP_DISCONTINUITY_THRESHOLD: f64 = 0.5;
const AIR_FREQUENCY_HZ: f64 = 4_000.0;
const MAX_SPECTRUM_FRAMES: usize = 256;
const DRONE_FLATNESS_THRESHOLD: f64 = 0.02;
const DRONE_MAX_STRONG_PEAKS: u64 = 4;
const DRONE_MIN_DURATION_SECONDS: f64 = 3.0;
const DULL_AIR_RATIO_THRESHOLD: f64 = 0.01;
const DEFAULT_WINDOW_SECONDS: f64 = 1.0;
const DEFAULT_HOP_SECONDS: f64 = 0.5;
const MIN_WINDOW_SECONDS: f64 = 0.01;
const MAX_WINDOW_SECONDS: f64 = 30.0;
const MIN_HOP_SECONDS: f64 = 0.01;
const MAX_HOP_SECONDS: f64 = 30.0;
const MAX_WINDOW_COUNT: usize = 4096;

#[derive(Debug, Clone, Serialize)]
pub struct AudioAnalysis {
    pub duration_seconds: f64,
    pub sample_rate: u32,
    pub channels: u16,
    pub window_seconds: f64,
    pub hop_seconds: f64,
    pub peak: f64,
    pub rms: f64,
    pub silence_ratio: f64,
    pub near_silence_ratio: f64,
    pub near_silence: bool,
    pub clipping_sample_count: u64,
    pub clipping_region_count: u64,
    pub loop_boundary_delta: f64,
    pub transient_count: u64,
    pub transients_per_second: f64,
    pub zero_crossing_rate: f64,
    pub spectral_bands: SpectralBands,
    pub spectral_centroid_hz: f64,
    pub spectral_flatness: f64,
    pub air_ratio: f64,
    pub crest_factor: f64,
    pub strong_peak_count: u64,
    pub peak_by_window: Vec<WindowMetric>,
    pub rms_by_window: Vec<WindowMetric>,
    pub transients_by_window: Vec<WindowTransientMetric>,
    pub suspected_event_spam_regions: Vec<AudioRegion>,
    pub clipping_regions: Vec<AudioRegion>,
    pub silence_regions: Vec<AudioRegion>,
    pub warning_ids: Vec<String>,
    pub warnings: Vec<AudioWarning>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SpectralBands {
    pub low: f64,
    pub mid: f64,
    pub high: f64,
}

/// Timbre metrics derived from a single magnitude spectrum. `spectral_bands` keeps the
/// existing low/mid/high energy split; the remaining fields port the "is this a flat
/// programmer-art drone / thin chiptune" cues from `tools/analyze_audio.py`.
#[derive(Debug, Clone, Default)]
struct SpectrumMetrics {
    bands: SpectralBands,
    centroid_hz: f64,
    flatness: f64,
    air_ratio: f64,
    strong_peak_count: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct WindowMetric {
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub value: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct WindowTransientMetric {
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AudioRegion {
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub duration_seconds: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AudioWarning {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub severity: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy)]
pub struct AudioAnalysisOptions {
    pub window_seconds: f64,
    pub hop_seconds: f64,
}

impl Default for AudioAnalysisOptions {
    fn default() -> Self {
        Self {
            window_seconds: DEFAULT_WINDOW_SECONDS,
            hop_seconds: DEFAULT_HOP_SECONDS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioAnalysisError {
    pub code: &'static str,
    pub message: String,
}

impl std::fmt::Display for AudioAnalysisError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for AudioAnalysisError {}

pub fn analyze_wav_file(path: &Path) -> Result<Value, AudioAnalysisError> {
    analyze_wav_file_with_options(path, AudioAnalysisOptions::default())
}

pub fn analyze_wav_file_with_options(
    path: &Path,
    options: AudioAnalysisOptions,
) -> Result<Value, AudioAnalysisError> {
    validate_options(options)?;
    DecodedWav::read(path)?.analyze(options)
}

pub fn write_audio_analysis_artifact(path: &Path, analysis: &Value) -> io::Result<()> {
    fs::write(
        path,
        serde_json::to_string_pretty(analysis).unwrap_or_else(|_| "{}".to_string()),
    )
}

fn validate_options(options: AudioAnalysisOptions) -> Result<(), AudioAnalysisError> {
    for (name, value, min, max) in [
        (
            "window_seconds",
            options.window_seconds,
            MIN_WINDOW_SECONDS,
            MAX_WINDOW_SECONDS,
        ),
        (
            "hop_seconds",
            options.hop_seconds,
            MIN_HOP_SECONDS,
            MAX_HOP_SECONDS,
        ),
    ] {
        if !value.is_finite() || value < min || value > max {
            return Err(err(
                "audio_analysis_invalid_options",
                format!("{name} must be finite and between {min} and {max} seconds"),
            ));
        }
    }
    Ok(())
}

fn parse_wav(bytes: &[u8]) -> Result<(WavFormat, &[u8]), AudioAnalysisError> {
    if bytes.len() > MAX_WAV_BYTES {
        return Err(err(
            "audio_wav_too_large",
            format!(
                "WAV file is {} bytes; maximum supported analysis size is {} bytes",
                bytes.len(),
                MAX_WAV_BYTES
            ),
        ));
    }
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(err(
            "audio_invalid_riff",
            "expected RIFF/WAVE header".to_string(),
        ));
    }

    let riff_size = read_u32_le(bytes, 4)? as usize;
    if riff_size.saturating_add(8) > bytes.len() {
        return Err(err(
            "audio_invalid_riff",
            "RIFF size extends past file length".to_string(),
        ));
    }

    let mut offset = 12usize;
    let mut format = None;
    let mut data = None;
    while offset.saturating_add(8) <= bytes.len() {
        let chunk_id = &bytes[offset..offset + 4];
        let chunk_size = read_u32_le(bytes, offset + 4)? as usize;
        let chunk_start = offset + 8;
        let chunk_end = chunk_start.checked_add(chunk_size).ok_or_else(|| {
            err(
                "audio_invalid_chunk",
                "WAV chunk size overflowed parser bounds".to_string(),
            )
        })?;
        if chunk_end > bytes.len() {
            return Err(err(
                "audio_invalid_chunk",
                format!(
                    "WAV chunk {} extends past file length",
                    String::from_utf8_lossy(chunk_id)
                ),
            ));
        }

        match chunk_id {
            b"fmt " => format = Some(parse_format_chunk(&bytes[chunk_start..chunk_end])?),
            b"data" => data = Some(&bytes[chunk_start..chunk_end]),
            _ => {}
        }

        offset = chunk_end + (chunk_size % 2);
    }

    let format = format.ok_or_else(|| {
        err(
            "audio_missing_fmt_chunk",
            "WAV file does not contain a fmt chunk".to_string(),
        )
    })?;
    let data = data.ok_or_else(|| {
        err(
            "audio_missing_data_chunk",
            "WAV file does not contain a data chunk".to_string(),
        )
    })?;
    Ok((format, data))
}

/// Decode a WAV file to a mono `f64` sample buffer (used by the spectrogram renderer).
pub fn load_wav_mono(path: &Path) -> Result<(Vec<f64>, u32), AudioAnalysisError> {
    let wav = DecodedWav::read(path)?;
    Ok((wav.mono, wav.format.sample_rate))
}

/// Render a waveform and spectrogram from a bounded WAV file.
pub fn write_spectrogram_artifact(
    wav_path: &Path,
    png_path: &Path,
) -> Result<(), AudioAnalysisError> {
    DecodedWav::read(wav_path)?.write_spectrogram(png_path)
}

/// Immutable decoded audio shared by analysis and spectrogram rendering.
#[derive(Debug)]
pub struct DecodedWav {
    format: WavFormat,
    byte_count: usize,
    mono: Vec<f64>,
    peak: f64,
    sum_squares: f64,
    silent_samples: u64,
    clipping_sample_count: u64,
    zero_crossings: u64,
    transient_count: u64,
}

/// Container and sample metadata; no analysis or browser compatibility is implied.
#[derive(Debug, Clone, Serialize)]
pub struct WavMetadata {
    pub byte_count: usize,
    pub duration_seconds: f64,
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
    pub encoding: &'static str,
}

impl DecodedWav {
    pub fn read(path: &Path) -> Result<Self, AudioAnalysisError> {
        let bytes =
            read_bounded_wav(fs::File::open(path).map_err(|error| read_error(path, error))?)
                .map_err(|error| match error {
                    BoundedReadError::Io(error) => read_error(path, error),
                    BoundedReadError::TooLarge => wav_too_large(MAX_WAV_BYTES + 1),
                })?;
        Self::from_bytes(&bytes)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, AudioAnalysisError> {
        let (format, data) = parse_wav(bytes)?;
        decode_wav(format, data, bytes.len())
    }

    pub fn metadata(&self) -> WavMetadata {
        WavMetadata {
            byte_count: self.byte_count,
            duration_seconds: self.mono.len() as f64 / f64::from(self.format.sample_rate),
            sample_rate: self.format.sample_rate,
            channels: self.format.channels,
            bits_per_sample: self.format.bits_per_sample,
            encoding: if self.format.audio_format == 1 {
                "pcm"
            } else {
                "ieee_float"
            },
        }
    }

    pub fn analyze(&self, options: AudioAnalysisOptions) -> Result<Value, AudioAnalysisError> {
        validate_options(options)?;
        let analysis =
            serde_json::to_value(decoded_to_analysis(self, options)?).map_err(|error| {
                AudioAnalysisError {
                    code: "audio_analysis_serialization_failed",
                    message: format!("failed to serialize WAV analysis: {error}"),
                }
            })?;
        // Analysis has no nullable fields. serde_json maps nonfinite derived f64
        // values to null, which must never masquerade as successful metrics.
        if contains_null(&analysis) {
            return Err(err(
                "audio_invalid_sample",
                "WAV samples overflow finite derived analysis metrics".to_string(),
            ));
        }
        Ok(analysis)
    }

    pub fn write_spectrogram(&self, path: &Path) -> Result<(), AudioAnalysisError> {
        crate::spectrogram::render_to_png(path, &self.mono, self.format.sample_rate).map_err(
            |error| AudioAnalysisError {
                code: "audio_spectrogram_write_failed",
                message: format!(
                    "failed to write spectrogram PNG {}: {error}",
                    path.display()
                ),
            },
        )
    }
}

fn contains_null(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Array(items) => items.iter().any(contains_null),
        Value::Object(items) => items.values().any(contains_null),
        _ => false,
    }
}

enum BoundedReadError {
    Io(io::Error),
    TooLarge,
}

fn read_bounded_wav(reader: impl Read) -> Result<Vec<u8>, BoundedReadError> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_WAV_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(BoundedReadError::Io)?;
    if bytes.len() > MAX_WAV_BYTES {
        return Err(BoundedReadError::TooLarge);
    }
    Ok(bytes)
}

fn read_error(path: &Path, error: io::Error) -> AudioAnalysisError {
    err(
        "audio_read_failed",
        format!("failed to read WAV file {}: {error}", path.display()),
    )
}

fn wav_too_large(size: usize) -> AudioAnalysisError {
    err(
        "audio_wav_too_large",
        format!(
            "WAV file is at least {size} bytes; maximum supported analysis size is {MAX_WAV_BYTES} bytes"
        ),
    )
}

#[derive(Debug, Clone, Copy)]
struct WavFormat {
    audio_format: u16,
    channels: u16,
    sample_rate: u32,
    block_align: u16,
    bits_per_sample: u16,
}

fn parse_format_chunk(chunk: &[u8]) -> Result<WavFormat, AudioAnalysisError> {
    if chunk.len() < 16 {
        return Err(err(
            "audio_invalid_fmt_chunk",
            "fmt chunk must be at least 16 bytes".to_string(),
        ));
    }
    let format = WavFormat {
        audio_format: read_u16_le(chunk, 0)?,
        channels: read_u16_le(chunk, 2)?,
        sample_rate: read_u32_le(chunk, 4)?,
        block_align: read_u16_le(chunk, 12)?,
        bits_per_sample: read_u16_le(chunk, 14)?,
    };
    if format.channels == 0 {
        return Err(err(
            "audio_invalid_fmt_chunk",
            "channel count must be nonzero".to_string(),
        ));
    }
    if format.sample_rate == 0 {
        return Err(err(
            "audio_invalid_fmt_chunk",
            "sample rate must be nonzero".to_string(),
        ));
    }
    if !matches!(format.audio_format, 1 | 3) {
        return Err(err(
            "audio_unsupported_format",
            format!(
                "unsupported WAV format {}; only PCM and IEEE float are supported",
                format.audio_format
            ),
        ));
    }
    let expected_bits = match format.audio_format {
        1 => matches!(format.bits_per_sample, 8 | 16 | 24 | 32),
        3 => matches!(format.bits_per_sample, 32 | 64),
        _ => false,
    };
    if !expected_bits {
        return Err(err(
            "audio_unsupported_bits_per_sample",
            format!(
                "unsupported bits per sample {} for WAV format {}",
                format.bits_per_sample, format.audio_format
            ),
        ));
    }
    let expected_block_align =
        u32::from(format.channels) * u32::from(format.bits_per_sample).div_ceil(8);
    if u32::from(format.block_align) != expected_block_align {
        return Err(err(
            "audio_invalid_fmt_chunk",
            format!(
                "block_align {} does not match channels and bits_per_sample",
                format.block_align
            ),
        ));
    }
    Ok(format)
}

fn reserve_mono(frame_count: usize) -> Result<Vec<f64>, AudioAnalysisError> {
    let bytes = frame_count.checked_mul(std::mem::size_of::<f64>())
        .filter(|bytes| *bytes <= MAX_DECODED_MONO_BYTES)
        .ok_or_else(|| err("audio_resource_limit", format!("decoded mono buffer for {frame_count} frames exceeds maximum {MAX_DECODED_MONO_BYTES} bytes")))?;
    let mut mono = Vec::new();
    mono.try_reserve_exact(frame_count).map_err(|error| {
        err(
            "audio_resource_limit",
            format!("cannot allocate {bytes} decoded mono bytes for {frame_count} frames: {error}"),
        )
    })?;
    Ok(mono)
}

fn decode_wav(
    format: WavFormat,
    data: &[u8],
    byte_count: usize,
) -> Result<DecodedWav, AudioAnalysisError> {
    if data.is_empty() {
        return Err(err(
            "audio_empty_data_chunk",
            "WAV data chunk is empty".to_string(),
        ));
    }
    let block_align = usize::from(format.block_align);
    if block_align == 0 || !data.len().is_multiple_of(block_align) {
        return Err(err(
            "audio_invalid_data_chunk",
            "data chunk length is not aligned to complete sample frames".to_string(),
        ));
    }

    let bytes_per_sample = usize::from(format.bits_per_sample.div_ceil(8));
    let frame_count = data.len() / block_align;
    if frame_count == 0 {
        return Err(err(
            "audio_empty_data_chunk",
            "WAV data chunk contains no complete frames".to_string(),
        ));
    }

    let mut mono = reserve_mono(frame_count)?;
    let mut peak = 0.0f64;
    let mut sum_squares = 0.0f64;
    let mut silent_samples = 0u64;
    let mut clipping_sample_count = 0u64;
    let mut previous_mono = None;
    let mut zero_crossings = 0u64;
    let mut transient_count = 0u64;

    for frame_index in 0..frame_count {
        let mut frame_sum = 0.0f64;
        for channel_index in 0..usize::from(format.channels) {
            let sample_offset = frame_index * block_align + channel_index * bytes_per_sample;
            let sample = decode_sample(
                format,
                &data[sample_offset..sample_offset + bytes_per_sample],
            )?;
            if !sample.is_finite() {
                return Err(err(
                    "audio_invalid_sample",
                    format!(
                        "WAV sample at frame {frame_index}, channel {channel_index} must be finite; got {sample}"
                    ),
                ));
            }
            let abs = sample.abs();
            peak = peak.max(abs);
            sum_squares += sample * sample;
            if abs <= SILENCE_THRESHOLD {
                silent_samples += 1;
            }
            if abs >= CLIPPING_THRESHOLD {
                clipping_sample_count += 1;
            }
            frame_sum += sample;
            if !sum_squares.is_finite() || !frame_sum.is_finite() {
                return Err(err(
                    "audio_invalid_sample",
                    format!(
                        "WAV sample at frame {frame_index}, channel {channel_index} overflows finite energy or downmix accumulation; got {sample}"
                    ),
                ));
            }
        }
        let mono_sample = frame_sum / f64::from(format.channels);
        if let Some(previous) = previous_mono {
            if sign_changed(previous, mono_sample) {
                zero_crossings += 1;
            }
            if (mono_sample - previous).abs() >= TRANSIENT_DELTA_THRESHOLD {
                transient_count += 1;
            }
        }
        previous_mono = Some(mono_sample);
        mono.push(mono_sample);
    }

    Ok(DecodedWav {
        format,
        byte_count,
        mono,
        peak,
        sum_squares,
        silent_samples,
        clipping_sample_count,
        zero_crossings,
        transient_count,
    })
}

fn decoded_to_analysis(
    wav: &DecodedWav,
    options: AudioAnalysisOptions,
) -> Result<AudioAnalysis, AudioAnalysisError> {
    let format = wav.format;
    let mono = &wav.mono;
    let frame_count = mono.len();
    let sample_count = frame_count * usize::from(format.channels);
    let peak = wav.peak;
    let sum_squares = wav.sum_squares;
    let silent_samples = wav.silent_samples;
    let clipping_sample_count = wav.clipping_sample_count;
    let zero_crossings = wav.zero_crossings;
    let transient_count = wav.transient_count;

    let rms = (sum_squares / sample_count as f64).sqrt();
    let duration_seconds = frame_count as f64 / f64::from(format.sample_rate);
    let loop_boundary_delta = if mono.len() >= 2 {
        (mono[0] - mono[mono.len() - 1]).abs()
    } else {
        0.0
    };
    let spectrum = spectrum_metrics(&mono[..mono.len().min(262_144)], format.sample_rate);
    let crest_factor = if rms > f64::EPSILON { peak / rms } else { 0.0 };
    let window_samples = seconds_to_samples(options.window_seconds, format.sample_rate);
    let hop_samples = seconds_to_samples(options.hop_seconds, format.sample_rate);
    let window_count = window_count(frame_count, window_samples, hop_samples);
    if window_count > MAX_WINDOW_COUNT {
        return Err(err(
            "audio_analysis_window_limit_exceeded",
            format!(
                "window configuration would produce {window_count} windows; maximum is {MAX_WINDOW_COUNT}"
            ),
        ));
    }

    let window_summaries = summarize_windows(
        mono,
        format.sample_rate,
        window_samples,
        hop_samples,
        window_count,
    );
    let peak_by_window = window_summaries
        .iter()
        .map(|summary| WindowMetric {
            start_seconds: summary.start_seconds,
            end_seconds: summary.end_seconds,
            value: summary.peak,
        })
        .collect::<Vec<_>>();
    let rms_by_window = window_summaries
        .iter()
        .map(|summary| WindowMetric {
            start_seconds: summary.start_seconds,
            end_seconds: summary.end_seconds,
            value: summary.rms,
        })
        .collect::<Vec<_>>();
    let transients_by_window = window_summaries
        .iter()
        .map(|summary| WindowTransientMetric {
            start_seconds: summary.start_seconds,
            end_seconds: summary.end_seconds,
            count: summary.transients,
        })
        .collect::<Vec<_>>();
    let clipping_regions =
        regions_for_threshold(mono, format.sample_rate, CLIPPING_THRESHOLD, true);
    let silence_regions = regions_for_threshold(mono, format.sample_rate, SILENCE_THRESHOLD, false);
    let suspected_event_spam_regions = event_spam_regions(&window_summaries);
    let near_silent_frames = mono
        .iter()
        .filter(|sample| sample.abs() <= NEAR_SILENCE_RMS_THRESHOLD)
        .count();
    let near_silence_ratio = near_silent_frames as f64 / frame_count as f64;
    let clipping_region_count = clipping_regions.len() as u64;
    let transients_per_second = if duration_seconds > 0.0 {
        transient_count as f64 / duration_seconds
    } else {
        0.0
    };
    let warning_ids = warning_ids(
        rms,
        peak,
        clipping_sample_count,
        transients_per_second,
        !suspected_event_spam_regions.is_empty(),
        loop_boundary_delta,
        duration_seconds,
        &spectrum,
    );
    let warnings = warning_ids
        .iter()
        .map(|id| AudioWarning {
            id: id.clone(),
            kind: id.clone(),
            severity: "warning".to_string(),
            message: warning_message(id).to_string(),
        })
        .collect::<Vec<_>>();

    Ok(AudioAnalysis {
        duration_seconds,
        sample_rate: format.sample_rate,
        channels: format.channels,
        window_seconds: options.window_seconds,
        hop_seconds: options.hop_seconds,
        peak,
        rms,
        silence_ratio: silent_samples as f64 / sample_count as f64,
        near_silence_ratio,
        near_silence: rms <= NEAR_SILENCE_RMS_THRESHOLD || peak <= SILENCE_THRESHOLD,
        clipping_sample_count,
        clipping_region_count,
        loop_boundary_delta,
        transient_count,
        transients_per_second,
        zero_crossing_rate: if frame_count > 1 {
            zero_crossings as f64 / (frame_count - 1) as f64
        } else {
            0.0
        },
        spectral_bands: spectrum.bands,
        spectral_centroid_hz: spectrum.centroid_hz,
        spectral_flatness: spectrum.flatness,
        air_ratio: spectrum.air_ratio,
        crest_factor,
        strong_peak_count: spectrum.strong_peak_count,
        peak_by_window,
        rms_by_window,
        transients_by_window,
        suspected_event_spam_regions,
        clipping_regions,
        silence_regions,
        warning_ids,
        warnings,
    })
}

#[derive(Debug, Clone)]
struct WindowSummary {
    start_seconds: f64,
    end_seconds: f64,
    peak: f64,
    rms: f64,
    transients: u64,
}

fn seconds_to_samples(seconds: f64, sample_rate: u32) -> usize {
    (seconds * f64::from(sample_rate)).ceil().max(1.0) as usize
}

fn window_count(frame_count: usize, window_samples: usize, hop_samples: usize) -> usize {
    if frame_count == 0 {
        return 0;
    }
    if frame_count <= window_samples {
        return 1;
    }
    ((frame_count - window_samples).div_ceil(hop_samples)) + 1
}

fn summarize_windows(
    mono: &[f64],
    sample_rate: u32,
    window_samples: usize,
    hop_samples: usize,
    window_count: usize,
) -> Vec<WindowSummary> {
    let mut summaries = Vec::with_capacity(window_count);
    for window_index in 0..window_count {
        let start = window_index * hop_samples;
        let end = (start + window_samples).min(mono.len());
        if start >= end {
            break;
        }
        let window = &mono[start..end];
        let mut peak = 0.0f64;
        let mut sum_squares = 0.0f64;
        let mut transients = 0u64;
        let mut previous: Option<f64> = None;
        for sample in window {
            peak = peak.max(sample.abs());
            sum_squares += sample * sample;
            if let Some(previous_sample) = previous
                && (*sample - previous_sample).abs() >= TRANSIENT_DELTA_THRESHOLD
            {
                transients += 1;
            }
            previous = Some(*sample);
        }
        summaries.push(WindowSummary {
            start_seconds: start as f64 / f64::from(sample_rate),
            end_seconds: end as f64 / f64::from(sample_rate),
            peak,
            rms: (sum_squares / window.len() as f64).sqrt(),
            transients,
        });
    }
    summaries
}

fn regions_for_threshold(
    mono: &[f64],
    sample_rate: u32,
    threshold: f64,
    at_or_above: bool,
) -> Vec<AudioRegion> {
    let mut regions = Vec::new();
    let mut start = None;
    for (index, sample) in mono.iter().enumerate() {
        let matched = if at_or_above {
            sample.abs() >= threshold
        } else {
            sample.abs() <= threshold
        };
        match (matched, start) {
            (true, None) => start = Some(index),
            (false, Some(start_index)) => {
                regions.push(region(start_index, index, sample_rate));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(start_index) = start {
        regions.push(region(start_index, mono.len(), sample_rate));
    }
    regions
}

fn event_spam_regions(windows: &[WindowSummary]) -> Vec<AudioRegion> {
    windows
        .iter()
        .filter(|window| {
            let duration = (window.end_seconds - window.start_seconds).max(f64::EPSILON);
            window.transients as f64 / duration >= EVENT_SPAM_TRANSIENTS_PER_SECOND
        })
        .map(|window| AudioRegion {
            start_seconds: window.start_seconds,
            end_seconds: window.end_seconds,
            duration_seconds: window.end_seconds - window.start_seconds,
        })
        .collect()
}

fn region(start: usize, end: usize, sample_rate: u32) -> AudioRegion {
    let start_seconds = start as f64 / f64::from(sample_rate);
    let end_seconds = end as f64 / f64::from(sample_rate);
    AudioRegion {
        start_seconds,
        end_seconds,
        duration_seconds: end_seconds - start_seconds,
    }
}

#[allow(clippy::too_many_arguments)]
fn warning_ids(
    rms: f64,
    peak: f64,
    clipping_sample_count: u64,
    transients_per_second: f64,
    event_spam: bool,
    loop_boundary_delta: f64,
    duration_seconds: f64,
    spectrum: &SpectrumMetrics,
) -> Vec<String> {
    let mut ids = Vec::new();
    let near_silent = rms <= NEAR_SILENCE_RMS_THRESHOLD || peak <= SILENCE_THRESHOLD;
    if near_silent {
        ids.push("audio_analysis.near_silent".to_string());
    }
    if clipping_sample_count > 0 {
        ids.push("audio_analysis.clipping_risk".to_string());
    }
    if transients_per_second >= HIGH_TRANSIENTS_PER_SECOND {
        ids.push("audio_analysis.high_transient_density".to_string());
    }
    if event_spam {
        ids.push("audio_analysis.event_spam_risk".to_string());
    }
    if loop_boundary_delta >= LOOP_DISCONTINUITY_THRESHOLD {
        ids.push("audio_analysis.loop_discontinuity_high".to_string());
    }
    // Quality cues only make sense on a clip that actually has audible content.
    if !near_silent {
        if spectrum.flatness <= DRONE_FLATNESS_THRESHOLD
            && spectrum.strong_peak_count <= DRONE_MAX_STRONG_PEAKS
            && duration_seconds >= DRONE_MIN_DURATION_SECONDS
        {
            ids.push("audio_analysis.tonal_drone".to_string());
        }
        if spectrum.air_ratio <= DULL_AIR_RATIO_THRESHOLD {
            ids.push("audio_analysis.dull".to_string());
        }
    }
    ids
}

fn warning_message(id: &str) -> &'static str {
    match id {
        "audio_analysis.near_silent" => "audio peak or RMS is near silent",
        "audio_analysis.clipping_risk" => "one or more samples reached the clipping threshold",
        "audio_analysis.high_transient_density" => "transient density is high",
        "audio_analysis.event_spam_risk" => "one or more windows have high transient density",
        "audio_analysis.loop_discontinuity_high" => "loop boundary delta is high",
        "audio_analysis.tonal_drone" => {
            "spectrum is flat and tonal over a sustained clip (possible static drone)"
        }
        "audio_analysis.dull" => "almost no spectral energy above 4 kHz (dull/muffled timbre)",
        _ => "audio analysis warning",
    }
}

fn decode_sample(format: WavFormat, bytes: &[u8]) -> Result<f64, AudioAnalysisError> {
    match (format.audio_format, format.bits_per_sample) {
        (1, 8) => Ok((f64::from(bytes[0]) - 128.0) / 128.0),
        (1, 16) => Ok(f64::from(i16::from_le_bytes([bytes[0], bytes[1]])) / 32768.0),
        (1, 24) => {
            let raw = i32::from_le_bytes([
                bytes[0],
                bytes[1],
                bytes[2],
                if bytes[2] & 0x80 == 0 { 0 } else { 0xff },
            ]);
            Ok(f64::from(raw) / 8_388_608.0)
        }
        (1, 32) => Ok(
            f64::from(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
                / 2_147_483_648.0,
        ),
        (3, 32) => Ok(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f64),
        (3, 64) => Ok(f64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ])),
        _ => Err(err(
            "audio_unsupported_format",
            "unsupported WAV sample encoding".to_string(),
        )),
    }
}

fn spectrum_metrics(samples: &[f64], sample_rate: u32) -> SpectrumMetrics {
    if samples.len() < 2 {
        return SpectrumMetrics::default();
    }

    // Frame the signal at its NATIVE rate (Hann window, FFT) and average magnitude spectra,
    // matching tools/analyze_audio.py. Native framing keeps the full 0..nyquist range
    // resolvable — decimating the whole clip would push multi-second tones out of range.
    let window = largest_power_of_two_at_most(samples.len().min(2048));
    if window < 2 {
        return SpectrumMetrics::default();
    }
    let half = window / 2;
    let hann = (0..window)
        .map(|i| 0.5 - 0.5 * ((2.0 * std::f64::consts::PI * i as f64) / (window - 1) as f64).cos())
        .collect::<Vec<_>>();

    // Spread up to MAX_SPECTRUM_FRAMES Hann frames across the clip and average them.
    let max_start = samples.len() - window;
    let possible_frames = max_start / (window / 2).max(1) + 1;
    let frame_count = possible_frames.clamp(1, MAX_SPECTRUM_FRAMES);
    let frame_step = if frame_count > 1 {
        max_start / (frame_count - 1)
    } else {
        0
    };
    let mut accumulated = vec![0.0f64; half];
    let mut real = vec![0.0f64; window];
    let mut imag = vec![0.0f64; window];
    for frame_index in 0..frame_count {
        let start = frame_index * frame_step;
        for (i, slot) in real.iter_mut().enumerate() {
            *slot = samples[start + i] * hann[i];
        }
        imag.iter_mut().for_each(|value| *value = 0.0);
        crate::spectrogram::fft_in_place(&mut real, &mut imag);
        for (bin, value) in accumulated.iter_mut().enumerate() {
            *value += (real[bin] * real[bin] + imag[bin] * imag[bin]).sqrt();
        }
    }

    // Magnitude spectrum (skip the DC bin so a small offset can't dominate flatness).
    let mut freqs = Vec::with_capacity(half.saturating_sub(1));
    let mut mags = Vec::with_capacity(half.saturating_sub(1));
    let mut low = 0.0;
    let mut mid = 0.0;
    let mut high = 0.0;
    for (bin, &accumulated_magnitude) in accumulated.iter().enumerate().take(half).skip(1) {
        let frequency = bin as f64 * f64::from(sample_rate) / window as f64;
        let magnitude = accumulated_magnitude / frame_count as f64;
        let energy = magnitude * magnitude;
        if frequency < 250.0 {
            low += energy;
        } else if frequency < AIR_FREQUENCY_HZ {
            mid += energy;
        } else {
            high += energy;
        }
        freqs.push(frequency);
        mags.push(magnitude);
    }

    let total_energy = low + mid + high;
    let bands = if total_energy <= f64::EPSILON {
        SpectralBands::default()
    } else {
        SpectralBands {
            low: low / total_energy,
            mid: mid / total_energy,
            high: high / total_energy,
        }
    };

    // Magnitude-domain timbre cues (mirrors tools/analyze_audio.py `metrics`).
    let total_mag: f64 = mags.iter().sum();
    let (centroid_hz, air_ratio) = if total_mag > f64::EPSILON {
        let centroid = freqs
            .iter()
            .zip(&mags)
            .map(|(frequency, magnitude)| frequency * magnitude)
            .sum::<f64>()
            / total_mag;
        let air = freqs
            .iter()
            .zip(&mags)
            .filter(|(frequency, _)| **frequency > AIR_FREQUENCY_HZ)
            .map(|(_, magnitude)| *magnitude)
            .sum::<f64>()
            / total_mag;
        (centroid, air)
    } else {
        (0.0, 0.0)
    };

    // Spectral flatness = geometric mean / arithmetic mean over non-trivial bins
    // (≈0 for a pure tone/drone, →1 for a rich/noisy spectrum).
    let nonzero = mags
        .iter()
        .copied()
        .filter(|magnitude| *magnitude > 1e-9)
        .collect::<Vec<_>>();
    let flatness = if nonzero.is_empty() {
        0.0
    } else {
        let geometric_mean = (nonzero.iter().map(|magnitude| magnitude.ln()).sum::<f64>()
            / nonzero.len() as f64)
            .exp();
        let arithmetic_mean = nonzero.iter().sum::<f64>() / nonzero.len() as f64;
        if arithmetic_mean > f64::EPSILON {
            geometric_mean / arithmetic_mean
        } else {
            0.0
        }
    };

    // Count prominent spectral peaks; a handful of evenly spaced peaks is the
    // signature of a single-oscillator "80s" voice.
    let peak_threshold = 0.15 * mags.iter().copied().fold(0.0_f64, f64::max);
    let mut strong_peak_count = 0u64;
    if peak_threshold > 0.0 && mags.len() >= 5 {
        for bin in 2..mags.len() - 2 {
            if mags[bin] > peak_threshold && mags[bin] >= mags[bin - 1] && mags[bin] > mags[bin + 1]
            {
                strong_peak_count += 1;
            }
        }
    }

    SpectrumMetrics {
        bands,
        centroid_hz,
        flatness,
        air_ratio,
        strong_peak_count,
    }
}

fn largest_power_of_two_at_most(value: usize) -> usize {
    if value < 1 {
        return 0;
    }
    let mut power = 1;
    while power * 2 <= value {
        power *= 2;
    }
    power
}

fn sign_changed(previous: f64, current: f64) -> bool {
    (previous < 0.0 && current >= 0.0) || (previous >= 0.0 && current < 0.0)
}

fn read_u16_le(bytes: &[u8], offset: usize) -> Result<u16, AudioAnalysisError> {
    let slice = bytes.get(offset..offset + 2).ok_or_else(|| {
        err(
            "audio_invalid_chunk",
            "unexpected end of WAV chunk".to_string(),
        )
    })?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32_le(bytes: &[u8], offset: usize) -> Result<u32, AudioAnalysisError> {
    let slice = bytes.get(offset..offset + 4).ok_or_else(|| {
        err(
            "audio_invalid_chunk",
            "unexpected end of WAV chunk".to_string(),
        )
    })?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn err(code: &'static str, message: String) -> AudioAnalysisError {
    AudioAnalysisError { code, message }
}

#[cfg(test)]
mod decoded_tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    fn pcm_stereo() -> Vec<u8> {
        let data: Vec<u8> = [32767_i16, -32767, 16384, -16384, 0, 0]
            .into_iter()
            .flat_map(i16::to_le_bytes)
            .collect();
        let mut wav = b"RIFF".to_vec();
        wav.extend_from_slice(&(36_u32 + data.len() as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16_u32.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&2_u16.to_le_bytes());
        wav.extend_from_slice(&8000_u32.to_le_bytes());
        wav.extend_from_slice(&32000_u32.to_le_bytes());
        wav.extend_from_slice(&4_u16.to_le_bytes());
        wav.extend_from_slice(&16_u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
        wav.extend_from_slice(&data);
        wav
    }

    #[test]
    fn shared_decode_preserves_channel_metrics_when_stereo_cancels() {
        let bytes = pcm_stereo();
        let wav = DecodedWav::from_bytes(&bytes).unwrap();
        let analysis = wav.analyze(AudioAnalysisOptions::default()).unwrap();
        assert!(wav.mono.iter().all(|sample| *sample == 0.0));
        assert!(analysis["peak"].as_f64().unwrap() > 0.99);
        assert!(analysis["rms"].as_f64().unwrap() > 0.6);
        assert_eq!(analysis["clipping_sample_count"], 2);
        assert_eq!(analysis["silence_ratio"], 1.0 / 3.0);
        let metadata = wav.metadata();
        assert_eq!(metadata.byte_count, bytes.len());
        assert_eq!(metadata.channels, 2);
        assert_eq!(metadata.encoding, "pcm");
        assert_eq!(metadata.bits_per_sample, 16);
        assert_eq!(metadata.duration_seconds, 3.0 / 8000.0);
        assert_eq!(
            wav.analyze(AudioAnalysisOptions::default()).unwrap(),
            analysis
        );
    }

    fn sample_wav(encoding: u16, bits: u16, data: &[u8]) -> Vec<u8> {
        let padded_size = data.len() + data.len() % 2;
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&(36_u32 + padded_size as u32).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16_u32.to_le_bytes());
        bytes.extend_from_slice(&encoding.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&8000_u32.to_le_bytes());
        bytes.extend_from_slice(&(8000_u32 * u32::from(bits / 8)).to_le_bytes());
        bytes.extend_from_slice(&(bits / 8).to_le_bytes());
        bytes.extend_from_slice(&bits.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(data);
        if !data.len().is_multiple_of(2) {
            bytes.push(0);
        }
        bytes
    }

    #[test]
    fn shared_decode_rejects_nonfinite_ieee_float_samples() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            for (bits, data) in [
                (32, (value as f32).to_le_bytes().to_vec()),
                (64, value.to_le_bytes().to_vec()),
            ] {
                let error = DecodedWav::from_bytes(&sample_wav(3, bits, &data)).unwrap_err();
                assert_eq!(error.code, "audio_invalid_sample");
                assert!(error.message.contains("frame 0, channel 0"));
            }
        }
    }

    #[test]
    fn shared_decode_keeps_supported_pcm_and_float_sample_values() {
        for (encoding, bits, data) in [
            (1_u16, 8_u16, vec![192]),
            (1, 16, 16384_i16.to_le_bytes().to_vec()),
            (1, 24, vec![0, 0, 64]),
            (1, 32, 1_073_741_824_i32.to_le_bytes().to_vec()),
            (3, 32, 0.5_f32.to_le_bytes().to_vec()),
            (3, 64, 0.5_f64.to_le_bytes().to_vec()),
        ] {
            let bytes = sample_wav(encoding, bits, &data);
            let wav = DecodedWav::from_bytes(&bytes).unwrap();
            assert_eq!(wav.mono, vec![0.5], "{encoding}/{bits}");
            let analysis = wav.analyze(AudioAnalysisOptions::default()).unwrap();
            assert_eq!(analysis["peak"], 0.5);
            assert_eq!(analysis["rms"], 0.5);
            assert_eq!(wav.metadata().bits_per_sample, bits);
        }
    }

    #[test]
    fn shared_decode_rejects_overflowing_finite_float_samples() {
        for samples in [vec![1e308_f64, 1e308], vec![1e154, 1e154, 1e154]] {
            let data: Vec<_> = samples.into_iter().flat_map(f64::to_le_bytes).collect();
            let error = DecodedWav::from_bytes(&sample_wav(3, 64, &data)).unwrap_err();
            assert_eq!(error.code, "audio_invalid_sample");
            assert!(error.message.contains("overflows finite"));
        }
    }

    #[test]
    fn analysis_rejects_nonfinite_derived_spectral_metrics() {
        let data: Vec<_> = std::iter::repeat_n(1e152_f64, 2048)
            .flat_map(f64::to_le_bytes)
            .collect();
        let wav = DecodedWav::from_bytes(&sample_wav(3, 64, &data)).unwrap();
        let error = wav.analyze(AudioAnalysisOptions::default()).unwrap_err();
        assert_eq!(error.code, "audio_invalid_sample");
    }

    #[test]
    fn decoded_mono_reservation_checks_size_before_allocating() {
        assert_eq!(
            reserve_mono(usize::MAX).unwrap_err().code,
            "audio_resource_limit"
        );
        assert_eq!(
            reserve_mono(MAX_DECODED_MONO_BYTES / 8 + 1)
                .unwrap_err()
                .code,
            "audio_resource_limit"
        );
        assert_eq!(reserve_mono(4).unwrap().capacity(), 4);
    }

    struct CountingReader {
        remaining: usize,
        observed: Rc<Cell<usize>>,
    }
    impl Read for CountingReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let count = buffer.len().min(self.remaining);
            buffer[..count].fill(0);
            self.remaining -= count;
            self.observed.set(self.observed.get() + count);
            Ok(count)
        }
    }

    #[test]
    fn bounded_reader_accepts_limit_and_consumes_only_one_extra_byte() {
        let observed = Rc::new(Cell::new(0));
        let bytes = read_bounded_wav(CountingReader {
            remaining: MAX_WAV_BYTES,
            observed: observed.clone(),
        });
        assert_eq!(bytes.ok().unwrap().len(), MAX_WAV_BYTES);
        observed.set(0);
        let result = read_bounded_wav(CountingReader {
            remaining: MAX_WAV_BYTES * 2,
            observed: observed.clone(),
        });
        assert!(matches!(result, Err(BoundedReadError::TooLarge)));
        assert_eq!(observed.get(), MAX_WAV_BYTES + 1);
    }

    #[test]
    fn from_bytes_rejects_oversized_input_before_parsing() {
        let error = DecodedWav::from_bytes(&vec![0; MAX_WAV_BYTES + 1]).unwrap_err();
        assert_eq!(error.code, "audio_wav_too_large");
    }
}
