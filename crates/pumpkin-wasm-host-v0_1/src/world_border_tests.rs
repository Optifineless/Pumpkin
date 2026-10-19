use crate::pumpkin::plugin::world::{HostWorldBorder, WorldBorder};
use pumpkin_core::server::combat_test_support;
use pumpkin_wasm_host_common::state::PluginHostState;
use std::{sync::Arc, time::Duration};
use wasmtime::component::Resource;

#[tokio::test]
async fn b1020_g04_wit_border_uses_live_size_and_documented_ticks() {
    let directory = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(directory.path());
    let world = combat_test_support::world(&server, directory.path());
    server.worlds.store(Arc::new(vec![world.clone()]));
    let mut host = PluginHostState::new();
    let resource = host.add::<WorldBorder>(world.clone()).unwrap();
    let border = || Resource::new_borrow(resource.rep());

    host.set_size(border(), 100.0).unwrap();
    host.set_diameter(border(), 20.0, Some(200)).unwrap();
    for _ in 0..50 {
        world.tick_environment();
    }
    assert_eq!(host.get_diameter(border()).unwrap(), 80.0);
    assert_eq!(host.get_size(border()).unwrap(), 80.0);
    assert_eq!(host.get_target_diameter(border()).unwrap(), 20.0);
    assert_eq!(host.get_target_speed(border()).unwrap(), 150);

    host.set_diameter(border(), 90.0, Some(20)).unwrap();
    assert_eq!(host.get_size(border()).unwrap(), 80.0);
    assert_eq!(host.get_target_speed(border()).unwrap(), 20);
    for _ in 0..20 {
        world.tick_environment();
    }
    assert_eq!(host.get_size(border()).unwrap(), 90.0);
    assert_eq!(host.get_target_speed(border()).unwrap(), 0);

    host.set_size_transition(border(), 50.0, 2).unwrap();
    assert_eq!(host.get_target_speed(border()).unwrap(), 40);
    for _ in 0..10 {
        world.tick_environment();
    }
    assert_eq!(host.get_size(border()).unwrap(), 80.0);
    assert_eq!(host.get_target_speed(border()).unwrap(), 30);
    host.set_size(border(), 33.0).unwrap();
    world.tick_environment();
    assert_eq!(host.get_size(border()).unwrap(), 33.0);
    assert_eq!(host.get_target_speed(border()).unwrap(), 0);
    tokio::time::timeout(Duration::from_secs(10), world.level.shutdown())
        .await
        .unwrap()
        .unwrap();
}
