//! Dependency-free waveform + log-frequency spectrogram PNG renderer.
//!
//! Ports `tools/analyze_audio.py`'s `render`/`write_png` so audio quality is visible to an
//! eyes-only reviewer (human or agent) without pulling in an image, FFT, or zlib crate. The
//! PNG is emitted with a hand-rolled *stored* (uncompressed) `deflate` stream wrapped in a
//! minimal zlib container — valid PNG, zero third-party dependencies, matching the proven
//! pure-stdlib Python tool this generalizes.

use std::f64::consts::PI;
use std::io;
use std::path::Path;

const WIDTH: usize = 720;
const WAVEFORM_HEIGHT: usize = 120;
const SPECTROGRAM_HEIGHT: usize = 260;
const HEIGHT: usize = WAVEFORM_HEIGHT + SPECTROGRAM_HEIGHT;
const FFT_WINDOW: usize = 1024;
const SPECTROGRAM_DB_RANGE: f64 = 60.0;
const MIN_SPECTROGRAM_HZ: f64 = 60.0;
const BACKGROUND: [u8; 3] = [12, 12, 18];
const WAVEFORM_COLOR: [u8; 3] = [90, 200, 160];

/// Render a waveform (top) + log-frequency spectrogram (bottom) PNG for `samples`.
pub fn render_to_png(path: &Path, samples: &[f64], sample_rate: u32) -> io::Result<()> {
    let mut image = vec![BACKGROUND; WIDTH * HEIGHT];
    draw_waveform(&mut image, samples);
    draw_spectrogram(&mut image, samples, sample_rate);
    write_png(path, WIDTH, HEIGHT, &image)
}

fn draw_waveform(image: &mut [[u8; 3]], samples: &[f64]) {
    let n = samples.len();
    if n == 0 {
        return;
    }
    let center = WAVEFORM_HEIGHT as f64 / 2.0;
    let amplitude = center - 2.0;
    for x in 0..WIDTH {
        let start = x * n / WIDTH;
        let end = ((x + 1) * n / WIDTH).max(start + 1).min(n);
        let segment = &samples[start..end];
        let mut low = f64::INFINITY;
        let mut high = f64::NEG_INFINITY;
        for &sample in segment {
            low = low.min(sample);
            high = high.max(sample);
        }
        if !low.is_finite() {
            low = 0.0;
            high = 0.0;
        }
        let y_high = (center - high * amplitude).round() as isize;
        let y_low = (center - low * amplitude).round() as isize;
        for y in y_high.min(y_low)..=y_high.max(y_low) {
            if (0..WAVEFORM_HEIGHT as isize).contains(&y) {
                image[y as usize * WIDTH + x] = WAVEFORM_COLOR;
            }
        }
    }
}

fn draw_spectrogram(image: &mut [[u8; 3]], samples: &[f64], sample_rate: u32) {
    let n = samples.len();
    if n < FFT_WINDOW || sample_rate == 0 {
        return;
    }
    let hop = (n / WIDTH).max(64);
    let hann = (0..FFT_WINDOW)
        .map(|i| 0.5 - 0.5 * ((2.0 * PI * i as f64) / (FFT_WINDOW as f64 - 1.0)).cos())
        .collect::<Vec<_>>();
    let half = FFT_WINDOW / 2;

    let mut columns: Vec<Vec<f64>> = Vec::new();
    let mut start = 0;
    while start + FFT_WINDOW <= n {
        let mut real = vec![0.0; FFT_WINDOW];
        let mut imag = vec![0.0; FFT_WINDOW];
        for (j, slot) in real.iter_mut().enumerate() {
            *slot = samples[start + j] * hann[j];
        }
        fft_in_place(&mut real, &mut imag);
        let column = (0..half)
            .map(|k| {
                let magnitude = (real[k] * real[k] + imag[k] * imag[k]).sqrt();
                20.0 * (magnitude + 1e-6).log10()
            })
            .collect::<Vec<_>>();
        columns.push(column);
        start += hop;
    }
    if columns.is_empty() {
        return;
    }

    let max_db = columns
        .iter()
        .flat_map(|column| column.iter().copied())
        .fold(f64::NEG_INFINITY, f64::max);
    let min_db = max_db - SPECTROGRAM_DB_RANGE;
    let nyquist = f64::from(sample_rate) / 2.0;
    let column_count = columns.len();
    for x in 0..WIDTH {
        let column = if column_count > 1 {
            &columns[x * (column_count - 1) / WIDTH]
        } else {
            &columns[0]
        };
        for y in 0..SPECTROGRAM_HEIGHT {
            let fraction = 1.0 - y as f64 / SPECTROGRAM_HEIGHT as f64;
            let frequency = MIN_SPECTROGRAM_HZ * (nyquist / MIN_SPECTROGRAM_HZ).powf(fraction);
            let bin = ((frequency / nyquist) * half as f64) as usize;
            let bin = bin.min(half - 1);
            let intensity = (column[bin] - min_db) / (max_db - min_db + 1e-9);
            image[(WAVEFORM_HEIGHT + y) * WIDTH + x] = viridis(intensity);
        }
    }
}

/// Simple perceptual-ish dark→bright ramp (matches the Python tool's `viridis`).
fn viridis(t: f64) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    let channel = |value: f64| (255.0 * value.clamp(0.0, 1.0)) as u8;
    [
        channel(1.5 * t - 0.3),
        channel(t),
        channel(1.0 - (t - 0.35).abs() * 2.0),
    ]
}

/// In-place iterative radix-2 Cooley–Tukey FFT. `re`/`im` length must be a power of two.
pub(crate) fn fft_in_place(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let angle = -2.0 * PI / len as f64;
        let (step_re, step_im) = (angle.cos(), angle.sin());
        let mut block = 0;
        while block < n {
            let (mut w_re, mut w_im) = (1.0f64, 0.0f64);
            for k in 0..len / 2 {
                let a = block + k;
                let b = block + k + len / 2;
                let t_re = w_re * re[b] - w_im * im[b];
                let t_im = w_re * im[b] + w_im * re[b];
                re[b] = re[a] - t_re;
                im[b] = im[a] - t_im;
                re[a] += t_re;
                im[a] += t_im;
                let next_w_re = w_re * step_re - w_im * step_im;
                w_im = w_re * step_im + w_im * step_re;
                w_re = next_w_re;
            }
            block += len;
        }
        len <<= 1;
    }
}

fn write_png(path: &Path, width: usize, height: usize, image: &[[u8; 3]]) -> io::Result<()> {
    let mut raw = Vec::with_capacity(height * (1 + width * 3));
    for y in 0..height {
        raw.push(0); // filter type 0 (none)
        for x in 0..width {
            raw.extend_from_slice(&image[y * width + x]);
        }
    }

    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&(width as u32).to_be_bytes());
    ihdr.extend_from_slice(&(height as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8-bit depth, truecolor RGB
    write_chunk(&mut png, b"IHDR", &ihdr);
    write_chunk(&mut png, b"IDAT", &zlib_store(&raw));
    write_chunk(&mut png, b"IEND", &[]);
    std::fs::write(path, png)
}

fn write_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
}

/// Minimal zlib stream using only *stored* (uncompressed) deflate blocks — no codec needed.
fn zlib_store(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01]; // zlib header: deflate / 32K window / fastest
    let mut offset = 0;
    loop {
        let end = (offset + 0xFFFF).min(data.len());
        let chunk = &data[offset..end];
        let is_final = end >= data.len();
        out.push(u8::from(is_final)); // BFINAL bit + BTYPE 00 (stored)
        let len = chunk.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(chunk);
        offset = end;
        if is_final {
            break;
        }
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    const MOD: u32 = 65_521;
    let mut a = 1u32;
    let mut b = 0u32;
    for &byte in data {
        a = (a + u32::from(byte)) % MOD;
        b = (b + a) % MOD;
    }
    (b << 16) | a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_a_valid_png_header() {
        let samples = (0..8_000)
            .map(|i| (2.0 * PI * 440.0 * i as f64 / 8_000.0).sin() * 0.5)
            .collect::<Vec<_>>();
        let path = std::env::temp_dir().join(format!(
            "godot_qa_spectrogram_{}.png",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        render_to_png(&path, &samples, 8_000).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(
            &bytes[0..8],
            &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]
        );
        // IHDR chunk follows the 8-byte signature: length 13, type "IHDR".
        assert_eq!(&bytes[8..12], &[0, 0, 0, 13]);
        assert_eq!(&bytes[12..16], b"IHDR");
        assert!(bytes.windows(4).any(|window| window == b"IDAT"));
        assert!(bytes.windows(4).any(|window| window == b"IEND"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn adler_and_crc_match_known_vectors() {
        // zlib of empty input has Adler-32 = 1; CRC-32 of "IEND" is a fixed constant.
        assert_eq!(adler32(b""), 1);
        assert_eq!(crc32(b"IEND"), 0xAE42_6082);
    }
}
