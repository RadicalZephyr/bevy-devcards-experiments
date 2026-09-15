//! Throwaway scaffolding for the bevy-devcards feasibility spike.
//!
//! This is not a crate design. It exists so the Phase 4 experiments can share a
//! card simulation and a snapshot helper instead of copy-pasting them, and every
//! line of it is expected to be deleted once the gates are decided.

pub mod card;
pub mod diff;
pub mod snapshot;

/// The exact Bevy patch these results are valid for. Printed by every
/// experiment, because the answers expire on version bumps.
pub const BEVY_VERSION: &str = "0.19.1";

/// Build profile the binary was compiled with, so a result line can say which.
pub const PROFILE: &str = if cfg!(debug_assertions) {
    "debug"
} else {
    "release"
};
