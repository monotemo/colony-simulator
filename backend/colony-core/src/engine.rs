//! The simulation engine: owns the world and drives time forward.

use crate::snapshot::WorldSnapshot;
use crate::world::World;

/// Drives the simulation one fixed timestep at a time.
///
/// The engine owns the [`World`] and a monotonically increasing tick counter.
/// Collision detection and other cross-entity systems will hang off `step`
/// here as the simulation grows.
#[derive(Debug, Clone)]
pub struct Engine {
    world: World,
    tick: u64,
}

impl Engine {
    /// Create an engine wrapping the given world, starting at tick 0.
    pub fn new(world: World) -> Self {
        Self { world, tick: 0 }
    }

    /// Create an engine with the default seeded world.
    pub fn seeded() -> Self {
        Self::new(World::seeded())
    }

    /// Reset the engine back to a fresh seeded world at tick 0.
    pub fn reset(&mut self) {
        self.world = World::seeded();
        self.tick = 0;
    }

    /// Advance the simulation by one fixed timestep of `dt` seconds.
    pub fn step(&mut self, dt: f64) {
        self.world.step(dt);
        self.tick += 1;
    }

    /// The current tick count.
    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// Read-only access to the world.
    pub fn world(&self) -> &World {
        &self.world
    }

    /// Capture a serializable snapshot of the current state.
    pub fn snapshot(&self) -> WorldSnapshot {
        WorldSnapshot::capture(&self.world, self.tick)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_advances_tick_and_moves_bees() {
        let mut engine = Engine::seeded();
        let before = engine.snapshot();
        engine.step(1.0 / 30.0);
        let after = engine.snapshot();

        assert_eq!(after.tick, before.tick + 1);
        // At least one bee should have moved.
        assert_ne!(after.bees, before.bees);
    }

    #[test]
    fn reset_returns_to_tick_zero() {
        let mut engine = Engine::seeded();
        for _ in 0..10 {
            engine.step(1.0 / 30.0);
        }
        assert_eq!(engine.tick(), 10);
        engine.reset();
        assert_eq!(engine.tick(), 0);
    }

    #[test]
    fn stepping_from_the_seed_is_deterministic() {
        // The engine has no RNG, so two runs from the same seed must produce
        // bit-identical trajectories. This is the contract that collision
        // avoidance (which reads neighbours and sums forces) must not break —
        // re-run it after any change to `World::step`.
        fn run(steps: u64) -> WorldSnapshot {
            let mut engine = Engine::seeded();
            for _ in 0..steps {
                engine.step(1.0 / 30.0);
            }
            engine.snapshot()
        }

        assert_eq!(run(600), run(600));
    }

    #[test]
    fn behavior_breakdown_shows_a_mix_over_a_run() {
        use crate::bee::BeeState;

        // Bees seed with staggered energy and have nectar to forage, so the
        // colony spreads across states instead of moving in lockstep: hungry
        // bees peel off to forage while the rest still wander. Over a run there
        // must be a tick where at least two states coexist, and foraging — the
        // behavior this slice adds — must show up. That is what lights up the
        // frontend behavior breakdown.
        let mut engine = Engine::seeded();
        let mut saw_foraging = false;
        let mut saw_two_states = false;
        for _ in 0..1_200 {
            engine.step(1.0 / 30.0);
            let snap = engine.snapshot();
            let (mut wandering, mut foraging, mut resting) = (0, 0, 0);
            for bee in &snap.bees {
                match bee.state {
                    BeeState::Wandering => wandering += 1,
                    BeeState::Foraging => foraging += 1,
                    BeeState::Resting => resting += 1,
                    // The caste-specific states (worker build, queen laying,
                    // drone loaf/flight) also exist now; this test only asserts
                    // the worker forage/wander/rest mix lights up.
                    BeeState::BuildingComb
                    | BeeState::LayingEggs
                    | BeeState::Loafing
                    | BeeState::Flying => {}
                }
            }
            saw_foraging |= foraging > 0;
            let distinct = (wandering > 0) as u8 + (foraging > 0) as u8 + (resting > 0) as u8;
            saw_two_states |= distinct >= 2;
        }
        assert!(saw_foraging, "foraging should occur when nectar is available");
        assert!(saw_two_states, "behavior breakdown should show a mix of states");
    }

    #[test]
    fn snapshot_serializes_honey_with_camelcase_key() {
        // The store field is `honey_stored` in Rust but must reach the frontend
        // as `honeyStored`, the key the rail already reads. Guard the rename so
        // the wire contract can't silently drift back to snake_case.
        let engine = Engine::seeded();
        let json = serde_json::to_string(&engine.snapshot()).expect("serialize");
        assert!(
            json.contains("\"honeyStored\""),
            "wire key should be camelCase honeyStored: {json}"
        );
    }

    #[test]
    fn snapshot_serializes_caste_and_wax_wire_keys() {
        // The new bee fields must reach the frontend under the agreed wire keys:
        // multi-word fields camelCase, enum values snake_case. Guard them so the
        // contract with `models.ts` can't silently drift.
        let engine = Engine::seeded();
        let json = serde_json::to_string(&engine.snapshot()).expect("serialize");
        for key in ["\"beeClass\"", "\"sex\"", "\"waxScales\"", "\"waxGrams\""] {
            assert!(json.contains(key), "wire should carry {key}: {json}");
        }
        // The seeded colony has a queen, so her caste must serialize snake_case.
        assert!(json.contains("\"queen\""), "caste should be snake_case: {json}");
    }

    #[test]
    fn snapshot_round_trips_through_json() {
        let engine = Engine::seeded();
        let snap = engine.snapshot();
        let json = serde_json::to_string(&snap).expect("serialize");
        let back: WorldSnapshot = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(snap, back);
    }
}
