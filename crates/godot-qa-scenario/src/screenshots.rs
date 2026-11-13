use godot_qa_core::results::{CommandResult, FailureDetail};
use serde_json::{Map, Value};

mod analysis;
mod burst;
mod capture;
mod compare;
mod output_paths;
mod sequence;

pub(crate) use analysis::{ScreenshotAnalysis, ScreenshotAnalysisCache};
pub use analysis::{analyze_screenshot_file, measure_saved_target_pixels};
pub use capture::{
    LiveScreenshotCapture, ScreenshotStateGuards, capture_live_screenshot, capture_screenshot,
};
pub use compare::{compare_presented_startup_files, compare_screenshot_files, compare_screenshots};
pub(crate) use compare::{compare_screenshot_files_cached, compare_screenshot_files_for_review};
pub use sequence::{
    ScreenshotContactSheetOptions, ScreenshotSequenceOptions, build_contact_sheet, capture_sequence,
};

fn args_invalid(command: &str, message: &str, actual: Value, expected: Value) -> CommandResult {
    screenshot_args_invalid(
        command,
        "screenshot comparison failed",
        "screenshot_compare_args_invalid",
        message,
        actual,
        expected,
    )
}

fn screenshot_args_invalid(
    command: &str,
    result_message: &str,
    kind: &str,
    message: &str,
    actual: Value,
    expected: Value,
) -> CommandResult {
    CommandResult {
        ok: false,
        command: command.to_string(),
        message: result_message.to_string(),
        data: Map::new(),
        failures: vec![FailureDetail {
            kind: kind.to_string(),
            command: Some(command.to_string()),
            message: Some(message.to_string()),
            expected: Some(expected),
            actual: Some(actual),
            ..FailureDetail::default()
        }],
        exit_code: 1,
    }
}

#[cfg(test)]
mod tests;
