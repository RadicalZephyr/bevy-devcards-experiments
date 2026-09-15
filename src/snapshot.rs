//! Reflection snapshots of a card world.
//!
//! Bevy 0.19 note: `DynamicScene` is gone. `bevy_scene` was rewritten around BSN
//! and no longer serializes worlds at all (it has no serde dependency). The
//! reflection-snapshot path moved to the new `bevy_world_serialization` crate,
//! re-exported as `bevy::world_serialization`, behind the `bevy_world_serialization`
//! and `serialize` features. `DynamicWorld` is the direct replacement for
//! `DynamicScene` and this module is the whole adapter.

use bevy::ecs::reflect::AppTypeRegistry;
use bevy::ecs::world::World;
use bevy::world_serialization::DynamicWorld;

/// Serialize every registered component and resource in `world` to RON.
pub fn snapshot(world: &World) -> String {
    let app_registry = world.resource::<AppTypeRegistry>();
    let registry = app_registry.read();
    DynamicWorld::from_world_with(world, &registry)
        .serialize(&registry)
        .expect("DynamicWorld serialization failed")
}

/// FNV-1a 64. A hand-rolled hash keeps the spike dependency-free; it is a
/// fingerprint for result tables, not a security primitive.
pub fn fingerprint(s: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in s.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// A compact description of a snapshot for a results table.
pub struct Fingerprint {
    pub bytes: usize,
    pub lines: usize,
    pub hash: u64,
}

impl Fingerprint {
    pub fn of(s: &str) -> Self {
        Self {
            bytes: s.len(),
            lines: s.lines().count(),
            hash: fingerprint(s),
        }
    }
}

impl core::fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{:>7} bytes  {:>5} lines  fnv1a64={:016x}",
            self.bytes, self.lines, self.hash
        )
    }
}
