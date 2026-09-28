//! Hardware adapters.
//! Each adapter talks to real hardware (or a mock) and updates PlatformStatus.

pub mod mock;

pub use mock::MockAdapter;
