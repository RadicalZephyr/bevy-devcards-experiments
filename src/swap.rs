//! Phase 1's mechanism: one `App`, N `World`s, swap the active world into the
//! App's main world slot.
//!
//! The swap itself is one line — `core::mem::swap` on two `&mut World`. What
//! makes this a phase rather than a function is everything that turns out to
//! live *in* the main world and therefore gets swapped out with it.
//!
//! The plan says "render world, device, and asset storage are shared by
//! construction because it's the same App". Two of those three hold. Asset
//! storage does not: `Assets<Image>`, `AssetServer`, and everything else the
//! renderer installs on the main app are plain resources in the main world, and
//! a whole-world swap takes them along. So a swap has to move a set of
//! resources back — which is Phase 2's E20 arriving early, as a prerequisite
//! for E10 rather than a follow-on to it.
//!
//! [`MigrationSet`] is that set, and it is deliberately an explicit list rather
//! than something inferred. The list is the artifact: it is what the crate would
//! have to hand a user whose plugin will not start in a bare card world.

use core::marker::PhantomData;

use bevy::ecs::resource::Resource;
use bevy::ecs::world::World;

/// One resource that belongs to the host rather than to a card, and therefore
/// has to be carried across a world swap.
trait Migrate: Send + Sync + 'static {
    fn migrate(&self, from: &mut World, to: &mut World) -> bool;
    fn name(&self) -> &'static str;
    fn present_in(&self, world: &World) -> bool;
}

struct MigrateResource<T: Resource>(PhantomData<fn() -> T>);

impl<T: Resource> Migrate for MigrateResource<T> {
    fn migrate(&self, from: &mut World, to: &mut World) -> bool {
        match from.remove_resource::<T>() {
            Some(resource) => {
                to.insert_resource(resource);
                true
            }
            None => false,
        }
    }

    fn name(&self) -> &'static str {
        core::any::type_name::<T>()
    }

    fn present_in(&self, world: &World) -> bool {
        world.get_resource::<T>().is_some()
    }
}

/// The set of resources that stay with the App when the world under it changes.
#[derive(Default)]
pub struct MigrationSet {
    entries: Vec<Box<dyn Migrate>>,
}

impl MigrationSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Declare that `T` belongs to the host, not to the card.
    pub fn with<T: Resource>(mut self) -> Self {
        self.entries.push(Box::new(MigrateResource::<T>(PhantomData)));
        self
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.entries.iter().map(|e| e.name()).collect()
    }

    /// Move every declared resource from `from` into `to`.
    ///
    /// Returns the names of the resources that were declared but absent, which
    /// is the diagnostic E22 asks about: a missing entry here is the difference
    /// between "your plugin needs X, which the card world doesn't have" and a
    /// bare `Res<T>` panic three systems later.
    pub fn migrate(&self, from: &mut World, to: &mut World) -> Vec<&'static str> {
        let mut missing = Vec::new();
        for entry in &self.entries {
            if !entry.migrate(from, to) {
                missing.push(entry.name());
            }
        }
        missing
    }

    /// Which declared resources a world is currently holding. Used to report the
    /// state of a world before anything is moved.
    pub fn present_in(&self, world: &World) -> Vec<&'static str> {
        self.entries
            .iter()
            .filter(|e| e.present_in(world))
            .map(|e| e.name())
            .collect()
    }
}

/// N card worlds, at most one of which is installed in the App at a time.
///
/// Dormant worlds are held here rather than inside the App, for the obvious
/// reason that anything inside the App's world would be swapped out along with
/// it. Slots are `Option<World>` so a swap moves worlds and allocates nothing —
/// otherwise the cost of the swap measurement would be the cost of building a
/// throwaway `World`.
pub struct CardWorlds {
    /// `None` marks the card currently installed in the App.
    worlds: Vec<Option<World>>,
    /// The App's original world, parked once the first card is swapped in. It
    /// holds whatever the renderer installed that the migration set does not
    /// name — which is precisely what E20 is trying to enumerate.
    host: Option<World>,
    active: Option<usize>,
    migration: MigrationSet,
    swaps: u64,
}

impl CardWorlds {
    pub fn new(worlds: Vec<World>, migration: MigrationSet) -> Self {
        Self {
            worlds: worlds.into_iter().map(Some).collect(),
            host: None,
            active: None,
            migration,
            swaps: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.worlds.len()
    }

    pub fn is_empty(&self) -> bool {
        self.worlds.is_empty()
    }

    /// `None` while the App still holds its original host world.
    pub fn active(&self) -> Option<usize> {
        self.active
    }

    pub fn swaps(&self) -> u64 {
        self.swaps
    }

    pub fn migration(&self) -> &MigrationSet {
        &self.migration
    }

    /// The dormant world for `slot`, or `None` if that card is the active one.
    pub fn dormant(&self, slot: usize) -> Option<&World> {
        self.worlds[slot].as_ref()
    }

    /// The parked host world, once a card has been swapped in.
    pub fn host(&self) -> Option<&World> {
        self.host.as_ref()
    }

    /// Install card `slot` into `app_world`, parking whatever was there.
    ///
    /// Returns the resources the migration set declared but could not find in
    /// the outgoing world.
    pub fn swap_to(&mut self, app_world: &mut World, slot: usize) -> Vec<&'static str> {
        if self.active == Some(slot) {
            return Vec::new();
        }

        let mut incoming = self.worlds[slot]
            .take()
            .expect("dormant world missing — a slot was left empty");

        // Host-owned resources move out of the outgoing world and into the
        // incoming one *before* the swap, so they never travel into storage.
        let missing = self.migration.migrate(app_world, &mut incoming);

        core::mem::swap(app_world, &mut incoming);
        // `incoming` now holds the outgoing world. Park it.
        match self.active {
            Some(previous) => self.worlds[previous] = Some(incoming),
            None => self.host = Some(incoming),
        }

        self.active = Some(slot);
        self.swaps += 1;
        missing
    }

    /// Advance to the next card, round-robin. The plan's scheduling policy:
    /// every card ticks at `fps/N`.
    pub fn swap_next(&mut self, app_world: &mut World) -> Vec<&'static str> {
        let next = match self.active {
            Some(active) => (active + 1) % self.worlds.len(),
            None => 0,
        };
        self.swap_to(app_world, next)
    }
}
