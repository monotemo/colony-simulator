//! Per-tick cost of `Engine::step` across colony sizes.
//!
//! This is the measurement scaffold that brackets the collision-avoidance work:
//! capture a baseline before separation steering lands, then re-run with
//! `--baseline <name>` afterwards to read criterion's per-size regression
//! verdicts. The 30 Hz frame budget is ~33 ms, so the large-N rows show whether
//! a step still fits inside a frame.
//!
//! Run from `backend/` — scope to `--bench step` so criterion's flags aren't
//! handed to the lib's libtest harness, which rejects them:
//! ```bash
//! cargo bench -p colony-core --bench step -- --save-baseline pre-collision
//! cargo bench -p colony-core --bench step -- --baseline pre-collision
//! ```

use std::time::Duration;

use colony_core::{Engine, World};
use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};

/// One fixed timestep at the simulation's 30 Hz cadence.
const DT: f64 = 1.0 / 30.0;

/// Cold single step: each timed iteration steps a freshly-cloned, never-stepped
/// engine, so the broad-phase scratch buffers are empty and the step pays their
/// full allocation. Represents the first step after construction/reset.
fn bench_step(c: &mut Criterion) {
    let mut group = c.benchmark_group("engine_step");

    for &n in &[24usize, 100, 500, 1000, 2000] {
        // Build a pristine engine of `n` bees once; each timed iteration clones
        // it (untimed, via `iter_batched`) so we measure a single step from the
        // same fresh state every time rather than a drifting trajectory.
        let engine = Engine::new(World::seeded_with_count(n));
        group.bench_with_input(BenchmarkId::from_parameter(n), &engine, |b, engine| {
            b.iter_batched(
                || engine.clone(),
                |mut engine| engine.step(DT),
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

/// Steady-state step: warm one engine, then time consecutive steps in place so
/// the broad-phase scratch buffers are reused tick to tick. This is what the
/// server and wasm runtimes actually do (~30 steps/sec forever), so it is the
/// representative sustained cost — and the one the reusable buffers target.
fn bench_step_warm(c: &mut Criterion) {
    let mut group = c.benchmark_group("engine_step_warm");

    for &n in &[24usize, 100, 500, 1000, 2000] {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            let mut engine = Engine::new(World::seeded_with_count(n));
            // Size the buffers and let the swarm settle into a realistic spread
            // before timing, so we measure warm reuse rather than the cold step.
            for _ in 0..100 {
                engine.step(DT);
            }
            b.iter(|| engine.step(DT));
        });
    }

    group.finish();
}

criterion_group! {
    name = benches;
    // Larger populations are slower to sample; give criterion a touch more time
    // so the big-N estimates stay stable without dragging the small ones out.
    config = Criterion::default().measurement_time(Duration::from_secs(8));
    targets = bench_step, bench_step_warm
}
criterion_main!(benches);
