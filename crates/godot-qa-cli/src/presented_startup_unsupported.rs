use crate::{Cli, PresentedHelperArgs, PresentedStartupArgs};
use godot_qa_core::config::ProjectConfig;
use godot_qa_core::results::{CommandResult, FailureDetail};
use serde_json::json;

fn unsupported() -> CommandResult {
    CommandResult {
        ok: false,
        command: "screenshot assert-startup-presented".into(),
        message: "isolated compositor capture is unsupported on this platform".into(),
        data: Default::default(),
        failures: vec![FailureDetail {
            kind: "presented_capture_unsupported".into(),
            command: Some("screenshot assert-startup-presented".into()),
            expected: Some(json!("Linux headless Gamescope")),
            actual: Some(json!(std::env::consts::OS)),
            ..FailureDetail::default()
        }],
        exit_code: 1,
    }
}

pub(crate) fn assert_startup_presented(
    _: &ProjectConfig,
    _: &PresentedStartupArgs,
) -> CommandResult {
    unsupported()
}

pub(crate) fn helper_result(_: &Cli, _: &PresentedHelperArgs) -> CommandResult {
    unsupported()
}
