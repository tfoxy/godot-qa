//! Debug-host support for `godot-qa` engine-faithful hot reload.
//!
//! Godot's state-preserving script soft reload and live resource reload are only
//! reachable over the engine's remote-debugger wire protocol, and the game always
//! connects OUT as a TCP client (`--remote-debug tcp://IP:PORT`). This crate provides
//! the persistent LISTENER side of that protocol (the "debug host"), plus the minimal
//! Godot binary Variant codec needed to frame the handful of reload messages we send.
//!
//! See `variant` for the wire codec and `host` for the listener + control server.

pub mod host;
pub mod variant;

pub use host::{ControlRequest, ControlResponse, DebugHost, DebugHostConfig, ReloadKind};
