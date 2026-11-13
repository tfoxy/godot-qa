// Included by lib.rs to preserve the original crate namespace.

#[path = "session/android.rs"]
mod session_android;

pub use session_android::session_start_target_result;
use session_android::*;

#[path = "session/store.rs"]
mod session_store;

use session_store::*;
pub use session_store::{
    clear_session_files, load_session_record, session_json_path, session_token_path,
};

#[path = "session/desktop.rs"]
mod session_desktop;
#[path = "session/hot_reload.rs"]
mod session_hot_reload;

use session_desktop::*;
pub use session_desktop::{
    resolve_live_frame_pacing, session_list_result, session_liveness, session_profile_result,
    session_start_result, session_status_result, session_stop_result,
};
use session_hot_reload::*;
pub use session_hot_reload::{
    RESOURCE_RELOAD_CAVEAT, STRUCTURE_RELOAD_CAVEAT, debug_host_available, debug_host_json_path,
    load_debug_host_record, session_reload_resources_result, session_reload_result,
    session_reload_scripts_result, session_reload_structure_result,
};

#[cfg(test)]
#[path = "session/tests.rs"]
mod granular_reload_reporting_tests;
