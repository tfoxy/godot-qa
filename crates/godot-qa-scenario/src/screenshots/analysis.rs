use image::{ImageError, RgbaImage};
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Immutable decoded screenshot shared by finalization checks. Quality is computed only
/// when requested; standalone comparisons need only the pixels.
#[derive(Debug)]
pub(crate) struct ScreenshotAnalysis {
    pub(super) pixels: RgbaImage,
    path: PathBuf,
    quality: OnceLock<Value>,
}

impl ScreenshotAnalysis {
    fn load(path: &Path) -> Result<Self, String> {
        Ok(Self {
            pixels: load_rgba_image(path)?,
            path: path.to_path_buf(),
            quality: OnceLock::new(),
        })
    }

    pub(crate) fn quality(&self) -> &Value {
        self.quality
            .get_or_init(|| analyze_screenshot_pixels(&self.path, &self.pixels))
    }

    pub(crate) fn measure_target(&self, rect: [f64; 4]) -> Value {
        measure_saved_target_analysis(self, rect)
    }
}

/// Lives only for one scenario's finalization. Retain at most two decoded
/// frames, even in long runs; quality summaries survive eviction for later
/// assertions without repeating whole-image color analysis.
#[derive(Default)]
pub(crate) struct ScreenshotAnalysisCache {
    entries: HashMap<PathBuf, Result<ScreenshotAnalysis, String>>,
    recent: VecDeque<PathBuf>,
    quality: HashMap<PathBuf, Value>,
}

impl ScreenshotAnalysisCache {
    pub(crate) fn get(&mut self, path: &Path) -> Result<&ScreenshotAnalysis, String> {
        const MAX_DECODED_SCREENSHOTS: usize = 2;
        if !self.entries.contains_key(path) {
            if self.entries.len() == MAX_DECODED_SCREENSHOTS
                && let Some(oldest) = self.recent.pop_front()
                && let Some(Ok(analysis)) = self.entries.remove(&oldest)
                && let Some(quality) = analysis.quality.get()
            {
                self.quality.insert(oldest, quality.clone());
            }
            let analysis = ScreenshotAnalysis::load(path).inspect(|analysis| {
                if let Some(quality) = self.quality.get(path) {
                    // This frame's immutable quality was computed before eviction.
                    let _ = analysis.quality.set(quality.clone());
                }
            });
            self.entries.insert(path.to_path_buf(), analysis);
        }
        self.recent.retain(|recent| recent != path);
        self.recent.push_back(path.to_path_buf());
        self.entries[path].as_ref().map_err(Clone::clone)
    }
}

pub fn analyze_screenshot_file(path: &Path) -> Result<Value, String> {
    Ok(ScreenshotAnalysis::load(path)?.quality().clone())
}

fn analyze_screenshot_pixels(path: &Path, pixels: &RgbaImage) -> Value {
    let (width, height) = pixels.dimensions();
    let pixel_count = u64::from(width) * u64::from(height);
    let mut counts: HashMap<[u8; 4], u64> = HashMap::new();
    for pixel in pixels.pixels() {
        *counts.entry(pixel.0).or_insert(0) += 1;
    }

    let mut dominant_colors: Vec<([u8; 4], u64)> = counts.into_iter().collect();
    dominant_colors.sort_by(|(left_color, left_count), (right_color, right_count)| {
        right_count
            .cmp(left_count)
            .then_with(|| left_color.cmp(right_color))
    });
    let background_count = dominant_colors
        .first()
        .map(|(_, count)| *count)
        .unwrap_or(0);
    let non_background_pixel_count = pixel_count.saturating_sub(background_count);
    let non_background_ratio = if pixel_count == 0 {
        0.0
    } else {
        non_background_pixel_count as f64 / pixel_count as f64
    };
    let dominant_ratio = if pixel_count == 0 {
        0.0
    } else {
        background_count as f64 / pixel_count as f64
    };

    let mut reasons = Vec::new();
    if width < 128 || height < 128 {
        reasons.push(json!("small_dimensions"));
    }
    if pixel_count == 0 {
        reasons.push(json!("empty_image"));
    }
    if pixel_count > 0 && non_background_ratio < 0.01 {
        reasons.push(json!("flat_or_blank"));
    }
    if pixel_count > 0 && dominant_ratio >= 0.98 {
        reasons.push(json!("dominant_color_overwhelms"));
    }

    let dominant_color_values = dominant_colors
        .into_iter()
        .take(5)
        .map(|(color, count)| {
            json!({
                "rgba": color,
                "count": count,
                "ratio": if pixel_count == 0 {
                    0.0
                } else {
                    count as f64 / pixel_count as f64
                },
            })
        })
        .collect::<Vec<_>>();

    json!({
        "path": path.to_string_lossy().into_owned(),
        "width": width,
        "height": height,
        "pixel_count": pixel_count,
        "dominant_colors": dominant_color_values,
        "non_background_pixel_count": non_background_pixel_count,
        "non_background_ratio": non_background_ratio,
        "low_information": !reasons.is_empty(),
        "low_information_reasons": reasons,
    })
}

pub fn measure_saved_target_pixels(path: &Path, rect: [f64; 4]) -> Result<Value, String> {
    Ok(ScreenshotAnalysis::load(path)?.measure_target(rect))
}

fn measure_saved_target_analysis(analysis: &ScreenshotAnalysis, rect: [f64; 4]) -> Value {
    let path = &analysis.path;
    let quality = analysis.quality();
    if quality
        .get("low_information")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return json!({
            "source_screenshot": path.to_string_lossy().into_owned(),
            "screenshot_quality": quality,
            "low_information": true,
            "low_information_reasons": quality
                .get("low_information_reasons")
                .cloned()
                .unwrap_or_else(|| json!([])),
            "target_rect": rect,
            "useful_pixel_count": 0_u64,
            "useful_pixel_ratio": 0.0,
        });
    }

    let pixels = &analysis.pixels;
    let (width, height) = pixels.dimensions();
    let x0 = rect[0].floor().max(0.0).min(f64::from(width)) as u32;
    let y0 = rect[1].floor().max(0.0).min(f64::from(height)) as u32;
    let x1 = (rect[0] + rect[2]).ceil().max(0.0).min(f64::from(width)) as u32;
    let y1 = (rect[1] + rect[3]).ceil().max(0.0).min(f64::from(height)) as u32;
    let measured_pixel_count = u64::from(x1.saturating_sub(x0)) * u64::from(y1.saturating_sub(y0));

    let dominant = quality
        .get("dominant_colors")
        .and_then(Value::as_array)
        .and_then(|colors| colors.first())
        .and_then(|entry| entry.get("rgba"))
        .and_then(Value::as_array)
        .and_then(|rgba| {
            Some([
                rgba.first()?.as_u64()? as u8,
                rgba.get(1)?.as_u64()? as u8,
                rgba.get(2)?.as_u64()? as u8,
                rgba.get(3)?.as_u64()? as u8,
            ])
        });

    let mut useful_pixel_count = 0_u64;
    for y in y0..y1 {
        for x in x0..x1 {
            let rgba = pixels.get_pixel(x, y).0;
            let visible = rgba[3] > 0;
            let non_black = rgba[0] > 0 || rgba[1] > 0 || rgba[2] > 0;
            let differs_from_background = dominant != Some(rgba);
            if visible && non_black && differs_from_background {
                useful_pixel_count += 1;
            }
        }
    }
    let useful_pixel_ratio = if measured_pixel_count == 0 {
        0.0
    } else {
        useful_pixel_count as f64 / measured_pixel_count as f64
    };

    json!({
        "source_screenshot": path.to_string_lossy().into_owned(),
        "screenshot_quality": quality,
        "low_information": false,
        "low_information_reasons": [],
        "target_rect": rect,
        "measured_pixel_count": measured_pixel_count,
        "useful_pixel_count": useful_pixel_count,
        "useful_pixel_ratio": useful_pixel_ratio,
    })
}

pub(super) fn load_rgba_image(path: &Path) -> Result<RgbaImage, String> {
    image::open(path)
        .map(image::DynamicImage::into_rgba8)
        .map_err(image_error_message)
}

fn image_error_message(error: ImageError) -> String {
    error.to_string()
}

#[cfg(test)]
#[path = "analysis_tests.rs"]
mod tests;
