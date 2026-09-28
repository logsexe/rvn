//! Internal events that drive state changes.

use super::state::Surface;

/// Events that can be produced by UI, hardware adapters, or timers.
#[derive(Debug, Clone)]
pub enum AppEvent {
    /// User selected a surface from the home grid
    SurfaceSelected(Surface),
    /// User requested return to home
    GoHome,
    /// Clock tick (once per second)
    Tick,
    /// Hardware status was refreshed
    HardwareUpdated,
}
