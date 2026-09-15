//! A deliberately boring deterministic card simulation.
//!
//! Every experiment in Phase 4 runs this. It is built to be reproducible rather
//! than interesting: the arithmetic is restricted to IEEE-exact operations
//! (`+ - *` and comparisons on values that round identically everywhere), the
//! RNG is an explicit integer sequence, and nothing reads wall-clock time.
//!
//! Transcendental functions (`sin`, `cos`, `powf`) are deliberately *absent*.
//! They route through platform libm and are a known cross-machine determinism
//! hazard that has nothing to do with Bevy; E41 probes them separately so the
//! golden is not contaminated by that question.

use core::time::Duration;

use bevy::app::{Main, MainSchedulePlugin, SubApp, Update};
use bevy::ecs::prelude::*;
use bevy::ecs::reflect::{AppTypeRegistry, ReflectComponent, ReflectResource};
use bevy::ecs::schedule::{IntoScheduleConfigs, ScheduleLabel};
use bevy::math::Vec2;
use bevy::reflect::Reflect;
use bevy::time::{Time, Virtual};

/// One simulated tick. 1/64 s, chosen because it is exactly representable as an
/// `f32` so `delta_secs()` is the same bit pattern on every machine.
pub const FIXED_STEP: Duration = Duration::from_nanos(15_625_000);

/// Half-width of the box particles bounce around in.
const BOUND: f32 = 64.0;

// ---------------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------------

#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
#[reflect(Component)]
pub struct Position(pub Vec2);

#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
#[reflect(Component)]
pub struct Velocity(pub Vec2);

#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
#[reflect(Component)]
pub struct Age {
    pub ticks: u32,
}

/// Stands in for "a component the card author added later". E42 inserts this on
/// a single entity to measure how much of the golden file churns as a result.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq, Default)]
#[reflect(Component)]
pub struct Selected;

// ---------------------------------------------------------------------------
// Resources
// ---------------------------------------------------------------------------

#[derive(Resource, Reflect, Debug, Clone)]
#[reflect(Resource)]
pub struct CardConfig {
    pub name: String,
    pub seed: u64,
    /// Spawn one particle every this many ticks.
    pub spawn_every: u64,
    /// Despawn a particle once it reaches this age, so population plateaus.
    pub max_age: u32,
    /// E42 scenario C: spawn one extra entity at startup, shifting every
    /// subsequent entity id.
    pub extra_startup_entity: bool,
}

impl CardConfig {
    pub fn new(name: &str, seed: u64) -> Self {
        Self {
            name: name.to_string(),
            seed,
            spawn_every: 3,
            max_age: 120,
            extra_startup_entity: false,
        }
    }
}

#[derive(Resource, Reflect, Debug, Clone, Default)]
#[reflect(Resource)]
pub struct CardClock {
    pub tick: u64,
}

/// xorshift64*. An explicit integer sequence beats any crate here: the whole
/// point is that the reader can see there is nowhere for entropy to enter.
#[derive(Resource, Reflect, Debug, Clone)]
#[reflect(Resource)]
pub struct Rng {
    pub state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        // A zero state is a fixed point for xorshift, so fold the seed into a
        // guaranteed-nonzero starting value.
        Self {
            state: seed ^ 0x9E37_79B9_7F4A_7C15,
        }
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A value in `[-1, 1)`, built from 24 integer bits so the conversion and
    /// both arithmetic steps are exact in `f32`.
    pub fn next_unit(&mut self) -> f32 {
        let bits = (self.next_u64() >> 40) as u32;
        (bits as f32 / 16_777_216.0) * 2.0 - 1.0
    }
}

/// The per-card fixed step. Host-side configuration, deliberately not
/// registered for reflection so it stays out of the snapshot.
#[derive(Resource, Debug, Clone, Copy)]
pub struct CardStep(pub Duration);

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Advance this card's own clock. Nothing here reads real time, so a card that
/// is skipped for a thousand host frames resumes with the same delta it always
/// had. That property is what Phase 1's E13 will lean on.
fn advance_card_time(
    step: Res<CardStep>,
    mut virt: ResMut<Time<Virtual>>,
    mut generic: ResMut<Time>,
) {
    virt.advance_by(step.0);
    *generic = virt.as_generic();
}

fn tick_clock(mut clock: ResMut<CardClock>) {
    clock.tick += 1;
}

fn spawn(
    mut commands: Commands,
    cfg: Res<CardConfig>,
    clock: Res<CardClock>,
    mut rng: ResMut<Rng>,
) {
    if clock.tick % cfg.spawn_every != 0 {
        return;
    }
    let px = rng.next_unit() * 8.0;
    let py = rng.next_unit() * 8.0;
    let vx = rng.next_unit() * 48.0;
    let vy = rng.next_unit() * 48.0;
    commands.spawn((
        Position(Vec2::new(px, py)),
        Velocity(Vec2::new(vx, vy)),
        Age { ticks: 0 },
    ));
}

fn integrate(time: Res<Time>, mut q: Query<(&mut Position, &Velocity)>) {
    let dt = time.delta_secs();
    for (mut p, v) in &mut q {
        p.0.x += v.0.x * dt;
        p.0.y += v.0.y * dt;
    }
}

fn bounce(mut q: Query<(&mut Position, &mut Velocity)>) {
    for (mut p, mut v) in &mut q {
        if p.0.x < -BOUND {
            p.0.x = -BOUND;
            v.0.x = -v.0.x;
        } else if p.0.x > BOUND {
            p.0.x = BOUND;
            v.0.x = -v.0.x;
        }
        if p.0.y < -BOUND {
            p.0.y = -BOUND;
            v.0.y = -v.0.y;
        } else if p.0.y > BOUND {
            p.0.y = BOUND;
            v.0.y = -v.0.y;
        }
    }
}

fn age(mut q: Query<&mut Age>) {
    for mut a in &mut q {
        a.ticks += 1;
    }
}

/// Reaping by a per-entity predicate rather than by "oldest first" keeps the
/// result independent of query iteration order. The *order* the despawns are
/// queued in still decides which freed ids get recycled first, which is exactly
/// the kind of hidden ordering E41 is meant to catch.
fn reap(mut commands: Commands, cfg: Res<CardConfig>, q: Query<(Entity, &Age)>) {
    for (entity, a) in &q {
        if a.ticks >= cfg.max_age {
            commands.entity(entity).despawn();
        }
    }
}

// ---------------------------------------------------------------------------
// Construction
// ---------------------------------------------------------------------------

/// Register exactly the types the snapshot is allowed to contain.
///
/// `DynamicWorld` extracts what the registry knows about and nothing else, so
/// this list *is* the snapshot schema. Worth noticing early: that makes
/// "declare a subset" (the API question E42 raises) a registry question rather
/// than a new mechanism.
pub fn register_card_types(registry: &AppTypeRegistry) {
    let mut w = registry.write();
    w.register::<Position>();
    w.register::<Velocity>();
    w.register::<Age>();
    w.register::<Selected>();
    w.register::<CardConfig>();
    w.register::<CardClock>();
    w.register::<Rng>();
}

/// A type registry shared by every card, the way cards in one process would
/// share the host's.
pub fn shared_registry() -> AppTypeRegistry {
    let registry = AppTypeRegistry::default();
    register_card_types(&registry);
    registry
}

/// Build one card as a [`SubApp`]: its own `World`, its own schedules, its own
/// clock, and a *shared* type registry.
pub fn card_sub_app(registry: &AppTypeRegistry, cfg: CardConfig, step: Duration) -> SubApp {
    let mut sub = SubApp::new();
    sub.update_schedule = Some(Main.intern());
    sub.insert_resource(registry.clone());
    sub.add_plugins(MainSchedulePlugin);

    sub.init_resource::<Time>();
    sub.init_resource::<Time<Virtual>>();
    sub.insert_resource(CardStep(step));
    sub.insert_resource(Rng::new(cfg.seed));
    sub.init_resource::<CardClock>();

    let extra = cfg.extra_startup_entity;
    sub.insert_resource(cfg);

    if extra {
        sub.world_mut().spawn((
            Position(Vec2::new(0.0, 0.0)),
            Velocity(Vec2::new(0.0, 0.0)),
            Age { ticks: 0 },
        ));
    }

    sub.add_systems(
        Update,
        (
            advance_card_time,
            tick_clock,
            spawn,
            integrate,
            bounce,
            age,
            reap,
        )
            .chain(),
    );

    sub
}

/// Run a freshly built card forward `ticks` steps and hand back its `SubApp`.
///
/// A `SubApp` is drivable on its own, so experiments that do not care about the
/// host-plus-sub-apps plumbing skip the `App` entirely.
pub fn run_card(registry: &AppTypeRegistry, cfg: CardConfig, ticks: u64) -> SubApp {
    let mut sub = card_sub_app(registry, cfg, FIXED_STEP);
    for _ in 0..ticks {
        sub.update();
    }
    sub
}
