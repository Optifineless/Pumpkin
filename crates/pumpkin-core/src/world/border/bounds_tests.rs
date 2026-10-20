#![allow(clippy::unwrap_used)]

use super::{
    Worldborder,
    test_support::{Fixture, initialization},
};
use pumpkin_util::math::vector3::Vector3;

fn assert_bounds(border: &Worldborder, expected: (f64, f64, f64, f64)) {
    let (min_x, min_z, max_x, max_z) = expected;
    assert_eq!(border.bounds(0.0), expected);
    assert!(border.contains(min_x, min_z));
    assert!(!border.contains(max_x, min_z));
    assert!(!border.contains(min_x, max_z));
    assert!(!border.contains(min_x - 0.001, min_z));
    assert_eq!(
        border.contains_block(min_x.ceil() as i32, min_z.ceil() as i32),
        min_x.ceil() < max_x && min_z.ceil() < max_z
    );
    let epsilon = f64::from(1.0e-5f32);
    assert_eq!(
        border.clamp_teleport_position(Vector3::new(-1.0e9, 73.0, 1.0e9)),
        Vector3::new(min_x, 73.0, max_z - epsilon)
    );
    assert_eq!(
        border.clamp_teleport_position(Vector3::new(1.0e9, 73.0, -1.0e9)),
        Vector3::new(max_x - epsilon, 73.0, min_z)
    );
}

#[test]
fn b1020b_g10_clamp_block_nan_diameter_does_not_panic() {
    for border in [
        Worldborder::new(0.0, 0.0, f64::NAN, 0, 5, 15),
        Worldborder::new(f64::NAN, f64::NAN, 10.0, 0, 5, 15),
    ] {
        assert_eq!(border.clamp_block(100, -100), (0, 0));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020_g04_clamps_share_the_moving_extent() {
    let fixture = Fixture::new();
    fixture.run("worldborder center 7.0 -13.0");
    // At tick 50, getSize is 80 while bounds(0) uses previousSize=80.4.
    for (from, to, samples) in [
        (100, 20, [100.0, 80.4, 40.4, 20.0]),
        (20, 100, [20.0, 39.6, 79.6, 100.0]),
    ] {
        if fixture.run("worldborder get") != from {
            fixture.run(&format!("worldborder set {from}"));
        }
        fixture.run(&format!("worldborder set {to} 200t"));
        for (advance, previous) in [0, 50, 100, 50].into_iter().zip(samples) {
            fixture.ticks(advance);
            let half = previous / 2.0;
            assert_bounds(
                &fixture.world.worldborder.lock().unwrap(),
                (7.0 - half, -13.0 - half, 7.0 + half, -13.0 + half),
            );
        }
    }
    fixture.run("worldborder set 20");
    fixture.run("worldborder set 100 200t");
    fixture.ticks(50);
    fixture.run("worldborder set 60 20t");
    assert_bounds(
        &fixture.world.worldborder.lock().unwrap(),
        (-13.0, -33.0, 27.0, 7.0),
    );
    fixture.ticks(20);
    assert_bounds(
        &fixture.world.worldborder.lock().unwrap(),
        (-23.0, -43.0, 37.0, 17.0),
    );
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020_g04_partial_ticks_and_absolute_limits_follow_vanilla() {
    let fixture = Fixture::new();
    fixture.resize(100, 20);
    fixture.run("worldborder center 0.0 0.0");
    fixture.ticks(50);
    let border = fixture.world.worldborder.lock().unwrap();
    assert_eq!(border.bounds(0.0), (-40.2, -40.2, 40.2, 40.2));
    assert_eq!(border.bounds(0.5), (-40.1, -40.1, 40.1, 40.1));
    assert_eq!(border.bounds(1.0), (-40.0, -40.0, 40.0, 40.0));
    assert!(
        border.contains(40.1, 0.0),
        "default queries retain previous-size bounds"
    );
    assert!(!border.contains(40.2, 0.0));
    drop(border);
    fixture.run("worldborder center 29999980.0 -29999980.0");
    let border = fixture.world.worldborder.lock().unwrap();
    assert_eq!(
        border.bounds(0.0),
        (29_999_939.8, -29_999_984.0, 29_999_984.0, -29_999_939.8)
    );
    assert!(!border.contains(29_999_984.0, -29_999_980.0));
    assert_eq!(
        border.clamp_teleport_position(Vector3::new(3.0e7, 64.0, -3.0e7)),
        Vector3::new(29_999_984.0 - f64::from(1.0e-5f32), 64.0, -29_999_984.0)
    );
    drop(border);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020_g04_immediate_reset_warning_and_static_boundary_controls() {
    let mut fixture = Fixture::new();
    fixture.resize(100, 20);
    fixture.run("worldborder center 0.0 0.0");
    fixture.ticks(50);
    fixture.run("worldborder set 60");
    fixture.ticks(200);
    assert_bounds(
        &fixture.world.worldborder.lock().unwrap(),
        (-30.0, -30.0, 30.0, 30.0),
    );
    fixture.run("worldborder set 1");
    assert!(
        fixture
            .world
            .worldborder
            .lock()
            .unwrap()
            .contains(-0.5, -0.5)
    );
    assert!(!fixture.world.worldborder.lock().unwrap().contains(0.5, 0.0));
    fixture.run("worldborder set 59999970");
    assert!(
        fixture
            .world
            .worldborder
            .lock()
            .unwrap()
            .contains(-29_999_984.0, 0.0)
    );
    assert!(
        !fixture
            .world
            .worldborder
            .lock()
            .unwrap()
            .contains(29_999_984.0, 0.0)
    );
    fixture.run("worldborder center 2.0 -3.0");
    fixture.run("worldborder warning distance 7");
    fixture.run("worldborder warning time 40t");
    fixture.actor.take_packets();
    fixture.world.send_world_info(&fixture.actor.player);
    let state = initialization(&mut fixture.actor);
    assert_eq!(state.center, (2.0, -3.0));
    assert_eq!(state.warning, (7, 40));
    fixture.run("worldborder set 100 200t");
    fixture
        .world
        .worldborder
        .lock()
        .unwrap()
        .reset(&fixture.world);
    fixture.ticks(250);
    assert_eq!(fixture.run("worldborder get"), 29_999_984);
    let border = fixture.world.worldborder.lock().unwrap();
    assert_eq!(border.get_lerp_time(), 0);
    assert_eq!(
        (
            border.center_x,
            border.center_z,
            border.warning_blocks,
            border.warning_time
        ),
        (0.0, 0.0, 5, 15)
    );
    assert_eq!(
        border.clamp_block(i32::MAX, i32::MIN),
        (14_999_991, -14_999_992)
    );
    drop(border);
    for diameter in [0.0, 0.5] {
        let narrow = Worldborder::new(0.5, 0.5, diameter, 0, 5, 15);
        assert_eq!(narrow.clamp_block(100, -100), (0, 0));
    }
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020_g04_legacy_native_milliseconds_convert_to_ticks() {
    let fixture = Fixture::new();
    fixture.run("worldborder set 100");
    fixture
        .world
        .worldborder
        .lock()
        .unwrap()
        .set_diameter(&fixture.world, 20.0, Some(10_000));
    fixture.ticks(50);
    assert_eq!(fixture.run("worldborder get"), 80);
    let border = fixture.world.worldborder.lock().unwrap();
    assert_eq!(border.get_lerp_time(), 150);
    assert_eq!(
        border.speed, 7_500,
        "native public speed retains milliseconds"
    );
    let cloned = border.clone();
    assert_eq!(cloned.get_size(), 80.0);
    assert_eq!(cloned.get_lerp_time(), 150);
    assert_eq!(cloned.bounds(0.0), border.bounds(0.0));
    drop(border);
    let mut border = fixture.world.worldborder.lock().unwrap();
    border.add_diameter(&fixture.world, 10.0, Some(1_000));
    assert_eq!(border.get_size(), 80.0);
    assert_eq!(border.get_lerp_target(), 90.0);
    assert_eq!(border.get_lerp_time(), 170);
    drop(border);
    fixture
        .world
        .worldborder
        .lock()
        .unwrap()
        .set_diameter(&fixture.world, 60.0, Some(51));
    assert_eq!(fixture.run("worldborder get"), 80);
    assert_eq!(fixture.world.worldborder.lock().unwrap().get_lerp_time(), 2);
    fixture.ticks(1);
    assert_eq!(fixture.run("worldborder get"), 70);
    fixture.ticks(1);
    assert_eq!(fixture.run("worldborder get"), 60);
    assert_eq!(fixture.world.worldborder.lock().unwrap().get_lerp_time(), 0);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020_g04_static_and_narrow_border_controls() {
    let fixture = Fixture::new();
    fixture.run("worldborder set 10");
    let border = fixture.world.worldborder.lock().unwrap();
    assert!(border.contains(-5.0, -5.0));
    assert!(!border.contains(5.0, 0.0));
    assert!(border.contains_block(-5, -5));
    assert!(border.contains_block(4, 0));
    assert!(!border.contains_block(5, 0));
    assert_eq!(border.clamp_block(100, -100), (4, -5));
    assert_eq!(
        border.clamp_teleport_position(Vector3::new(-10.0, 74.0, 0.0)),
        Vector3::new(-5.0, 74.0, 0.0)
    );
    drop(border);
    fixture.run("worldborder set 1");
    let border = fixture.world.worldborder.lock().unwrap();
    assert!(border.contains(-0.5, -0.5));
    assert!(!border.contains(0.5, 0.0));
    drop(border);
    fixture.run("worldborder set 59999970");
    assert!(
        fixture
            .world
            .worldborder
            .lock()
            .unwrap()
            .contains(29_999_983.0, 0.0)
    );
    assert!(
        !fixture
            .world
            .worldborder
            .lock()
            .unwrap()
            .contains(29_999_985.0, 0.0)
    );
    for diameter in [0.0, 0.5] {
        let narrow = Worldborder::new(0.5, 0.5, diameter, 0, 5, 15);
        assert_eq!(narrow.clamp_block(100, -100), (0, 0));
    }
    let off_grid_narrow = Worldborder::new(1.0, -1.0, 0.5, 0, 5, 15);
    assert_eq!(off_grid_narrow.clamp_block(100, -100), (1, -1));
    fixture.finish().await;
}

#[test]
fn b1020b_g10_fractional_block_clamp_floors_the_clamped_float() {
    let fractional = Worldborder::new(0.0, 0.0, 10.4, 0, 5, 15);
    assert_eq!(fractional.clamp_block(100, -100), (5, -6));

    let negative_center = Worldborder::new(-0.2, -0.2, 10.4, 0, 5, 15);
    assert_eq!(negative_center.clamp_block(100, -100), (4, -6));

    let absolute_limit = Worldborder::new(29_999_980.0, 0.0, 100.0, 0, 5, 15);
    assert_eq!(absolute_limit.clamp_block(i32::MAX, 0).0, 29_999_983);
}
