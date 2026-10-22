#![allow(
    clippy::collapsible_if,
    clippy::items_after_test_module,
    clippy::nonminimal_bool,
    clippy::redundant_closure,
    clippy::result_large_err,
    clippy::too_many_arguments,
    clippy::type_complexity
)]

pub mod artifact_query;
pub mod artifact_summary;
pub mod artifacts;
pub mod assertions;
pub mod asset_trial;
pub mod audio;
pub mod bundle;
pub mod bundle_multi;
pub mod bundle_schedule;
pub mod bundle_selection;
pub mod check;
pub mod contact_sheet;
pub mod edit;
pub mod frame_pacing;
pub mod init;
pub mod layout;
pub mod lint;
pub mod live_gate;
pub mod mcp;
pub mod record;
pub mod retention;
pub mod review;
pub mod run;
pub mod scenario_doc;
pub mod screenshots;
pub mod snapshot_context;
pub mod sweep;
pub mod template;
pub mod time_control;
pub mod visual_audit;
pub mod visual_isolate;
pub mod visual_risk;

pub fn migration_marker() -> &'static str {
    "m31"
}
