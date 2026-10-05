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

/// Initial window geometry: keep the default window inside the monitor's
/// available (work) area so the consent footer stays visible on small
/// panels — project-management#380 (375 finding 8): at a 1280x800 work area
/// the 1024x768 default plus WM title bar overflows the panel and the bottom
/// of the UI (consent footer) rendered off-screen.
pub mod window_fit {
    /// Default inner window size (logical px), set in `main.rs`.
    pub const DEFAULT_WIDTH: f64 = 1024.0;
    pub const DEFAULT_HEIGHT: f64 = 768.0;
    /// Budget for the WM title bar / frame drawn outside the client area
    /// (logical px). Typical GNOME/Mutter title bars are ~37px.
    pub const TITLEBAR_BUDGET: f64 = 40.0;
    /// Budget for the taskbar/dock strip along the panel edge (logical px).
    /// tao 0.30 does not expose the WM work area (no `work_area()` on
    /// `MonitorHandle`), so the panel is budgeted conservatively; on panels
    /// without a dock this costs a few extra pixels only.
    pub const PANEL_BUDGET: f64 = 48.0;
    /// Breathing room between the window edge and the panel edge.
    pub const EDGE_MARGIN: f64 = 12.0;

    /// Clamp a requested window size so it fits inside the monitor area,
    /// leaving room for the WM title bar, the panel strip, and a small edge
    /// margin. Only ever shrinks: a window that already fits is left alone.
    ///
    /// Pure (f64 in / f64 out) so the geometry is unit-testable without a
    /// GUI; the live-window application lives in [`apply`].
    pub fn clamp_to_fit(
        (width, height): (f64, f64),
        (monitor_w, monitor_h): (f64, f64),
    ) -> (f64, f64) {
        let max_w = monitor_w - 2.0 * EDGE_MARGIN;
        let max_h = monitor_h - TITLEBAR_BUDGET - PANEL_BUDGET - 2.0 * EDGE_MARGIN;
        (width.min(max_w), height.min(max_h))
    }

    /// Apply the clamp to the live window once at startup (desktop builds).
    /// Shrink-only: a window the user has since resized is never touched
    /// again (the caller runs this exactly once).
    #[cfg(feature = "desktop")]
    pub fn apply(window: &dioxus::desktop::tao::window::Window) {
        let Some(monitor) = window
            .current_monitor()
            .or_else(|| window.primary_monitor())
        else {
            return; // No monitor info (headless/CI); the default size stands.
        };
        let scale = monitor.scale_factor();
        let mon = monitor.size();
        let mon_logical = (mon.width as f64 / scale, mon.height as f64 / scale);
        let current = window.inner_size();
        let current_logical = (current.width as f64 / scale, current.height as f64 / scale);
        let (w, h) = clamp_to_fit(current_logical, mon_logical);
        if (w - current_logical.0).abs() > 0.5 || (h - current_logical.1).abs() > 0.5 {
            window.set_inner_size(dioxus::desktop::LogicalSize::new(w, h));
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn small_panel_is_clamped_so_footer_fits() {
            // 1280x800: the 1024x768 default no longer overflows once the
            // title bar + panel budgets are taken off.
            assert_eq!(
                clamp_to_fit((DEFAULT_WIDTH, DEFAULT_HEIGHT), (1280.0, 800.0)),
                (1024.0, 688.0)
            );
        }

        #[test]
        fn large_panel_is_untouched() {
            assert_eq!(
                clamp_to_fit((DEFAULT_WIDTH, DEFAULT_HEIGHT), (2560.0, 1440.0)),
                (DEFAULT_WIDTH, DEFAULT_HEIGHT)
            );
        }

        #[test]
        fn width_is_clamped_independently_of_height() {
            assert_eq!(
                clamp_to_fit((DEFAULT_WIDTH, DEFAULT_HEIGHT), (1000.0, 1200.0)),
                (976.0, 768.0)
            );
        }

        #[test]
        fn never_grows() {
            let small = (640.0, 480.0);
            assert_eq!(clamp_to_fit(small, (1280.0, 800.0)), small);
        }
    }
}
