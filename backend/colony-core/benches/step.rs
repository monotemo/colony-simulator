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

criterion_group! {
    name = benches;
    // Larger populations are slower to sample; give criterion a touch more time
    // so the big-N estimates stay stable without dragging the small ones out.
    config = Criterion::default().measurement_time(Duration::from_secs(8));
    targets = bench_step
}
criterion_main!(benches);
