pub mod app;
pub mod components;
pub mod theme;

pub use app::App;

use sh_core::SharedBridgeState;
use std::sync::OnceLock;

static BRIDGE_STATE: OnceLock<SharedBridgeState> = OnceLock::new();

/// Store the bridge state so the custom protocol handler can access it.
pub fn set_bridge_state(state: SharedBridgeState) {
    let _ = BRIDGE_STATE.set(state);
}

/// Retrieve the bridge state (returns `None` if not yet initialised).
pub fn get_bridge_state() -> Option<&'static SharedBridgeState> {
    BRIDGE_STATE.get()
}
