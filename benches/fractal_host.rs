//! CPU cost of the fractal explorer's host evaluator: one distance sample,
//! the schedule expansion inside it, and one camera step in dense geometry.

use std::collections::HashMap;
use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use varda::fractal::{
    Controls, Flight, FlightMode, Pose, Quat, Stack, Vec3, default_param, stack_from_params,
};

fn stack(overrides: &[(&str, f64)]) -> Stack {
    let values: HashMap<&str, f64> = overrides.iter().copied().collect();
    stack_from_params(|name| {
        Some(
            values
                .get(name)
                .copied()
                .unwrap_or_else(|| default_param(name)),
        )
    })
}

/// The gnarled temple: a box then Gnarl, which runs the Jacobian path.
fn temple() -> Stack {
    stack(&[
        ("slot1_a", -1.8),
        ("slot1_rot_x", 0.0),
        ("slot1_rot_y", 0.0),
        ("slot2_formula", 10.0),
    ])
}

/// Points spread through the structure.
#[allow(clippy::approx_constant)] // 6.283 is a probe frequency, not 2 pi.
fn points() -> Vec<Vec3> {
    (0..64)
        .map(|i| {
            let t = f64::from(i) * 0.618_034;
            Vec3::new(
                3.0 * (t * 6.283).sin(),
                3.0 * (t * 4.1).cos(),
                3.0 * (t * 2.3).sin(),
            )
        })
        .collect()
}

fn bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("fractal_host");
    let points = points();
    for (name, stack) in [("default", stack(&[])), ("temple", temple())] {
        group.bench_function(format!("sample_64/{name}"), |b| {
            b.iter(|| {
                for p in &points {
                    black_box(stack.sample(*p, 0.0));
                }
            });
        });
        group.bench_function(format!("schedules/{name}"), |b| {
            b.iter(|| black_box(stack.schedules()));
        });
    }
    // A camera flying forward in the temple's crust, where every probe is
    // close to a surface.
    let stack = temple();
    let pose = Pose {
        position: Vec3::new(
            -1.363_989_544_874_340_4,
            2.069_652_430_434_301,
            1.206_989_544_874_341,
        ),
        orientation: Quat {
            w: 0.880_476_239_217_149_3,
            x: 0.279_848_142_331_276_3,
            y: -0.364_705_199_631_000_9,
            z: 0.115_916_895_959_295_16,
        },
    };
    let controls = Controls {
        throttle: 1.0,
        strafe_x: 0.0,
        strafe_y: 0.0,
        yaw_rate: 0.0,
        pitch_rate: 0.0,
        roll_rate: 0.0,
        speed: 0.5,
        mode: FlightMode::Walk,
    };
    group.bench_function("flight_step/temple_crust", |b| {
        b.iter_batched(
            || {
                let mut flight = Flight::new(pose);
                flight.scale = 1.137;
                flight
            },
            |mut flight| black_box(flight.step(&stack, &controls, 1.0 / 60.0)),
            criterion::BatchSize::SmallInput,
        );
    });
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
