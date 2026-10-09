use super::*;
use crate::{
    net::java::combat_test_support::TestPlayer,
    server::combat_test_support::{server, world},
};
use pumpkin_data::{
    Block, attributes::Attributes, biome::Biome, entity::EntityType, item_stack::ItemStack,
};
use pumpkin_util::math::position::BlockPos;
use std::sync::Arc;

pub(super) fn hook(world: &Arc<World>, player: &Player) -> FishingBobberEntity {
    player
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::new(1, &Item::FISHING_ROD));
    FishingBobberEntity::new_with_rotation(
        Entity::new(
            world.clone(),
            player.position(),
            &EntityType::FISHING_BOBBER,
        ),
        player,
        0.0,
        0.0,
        0,
        0,
        Hand::Right,
    )
}

#[test]
fn fishing_throw_uses_vanilla_pitch_clamp_and_independent_axis_noise() {
    let (offset, forward) = throw_setup(0.0, 0.0, Vector3::new(0.5, 0.5, 0.5));
    assert!((offset.z - 0.3).abs() < 1e-5);
    assert!((forward.z - 1.1).abs() < 1e-5);
    assert!(forward.x.abs() < 1e-5 && forward.y.abs() < 1e-5);
    for (pitch, expected) in [(90.0, -5.0), (-90.0, 5.0)] {
        let (_, launch) = throw_setup(0.0, pitch, Vector3::default());
        assert!((launch.y / launch.z - expected).abs() < 1e-5);
    }
    let (_, clean) = throw_setup(45.0, 30.0, Vector3::default());
    let (_, noisy) = throw_setup(45.0, 30.0, Vector3::new(0.1, 0.2, 0.3));
    assert!((noisy.x / clean.x - noisy.y / clean.y).abs() > 0.01);
    assert!((noisy.y / clean.y - noisy.z / clean.z).abs() > 0.01);
}

#[test]
fn fishing_bobber_settles_and_exact_equilibrium_does_not_get_a_spurious_push() {
    assert_eq!(
        bob_velocity(Vector3::default(), 64.5, 64, 0.5, 0.5),
        Vector3::default()
    );
    for mut y in [63.5, 66.5] {
        let mut velocity = Vector3::default();
        for _ in 0..200 {
            velocity = bob_velocity(velocity, y, 64, 0.5, 0.5);
            y += velocity.y;
            velocity = velocity * FishingBobberEntity::INERTIA;
        }
        assert!((y - 64.5).abs() < 0.2);
    }
    assert_eq!(
        retrieve::catch_velocity(Vector3::new(0.0, 0.0, 16.0)),
        Vector3::new(0.0, 0.32, 1.6)
    );
}

#[test]
fn fishing_weather_modifies_the_approach_clock_at_strict_probability_boundaries() {
    use catching_fish::fishing_speed;
    assert_eq!(fishing_speed(true, true, 0.24, 0.9), 2);
    assert_eq!(fishing_speed(true, true, 0.25, 0.9), 1);
    assert_eq!(fishing_speed(false, false, 0.1, 0.49), 0);
    assert_eq!(fishing_speed(false, false, 0.1, 0.5), 1);
    assert_eq!(fishing_speed(true, false, 0.1, 0.1), 1);
}

#[tokio::test]
async fn fishing_collision_preserves_owner_grace_and_hooks_other_players_then_detaches() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let owner = TestPlayer::new(&world);
    let target = TestPlayer::new(&world);
    world
        .players
        .store(Arc::new(vec![owner.player.clone(), target.player.clone()]));
    target
        .player
        .get_entity()
        .set_pos(Vector3::new(0.0, 64.0, 3.0));
    owner
        .player
        .get_entity()
        .set_pos(Vector3::new(0.0, 64.0, 0.0));
    let bobber = hook(&world, &owner.player);
    bobber.entity.set_pos(Vector3::new(0.0, 65.0, 0.3));
    let mut velocity = Vector3::new(0.0, 0.0, -0.5);
    bobber.entity.velocity.store(velocity);
    bobber.projectile.check_left_owner(&bobber.entity);
    assert!(!bobber.projectile.left_owner.load(Relaxed));
    bobber.check_collision(&world, &mut velocity);
    assert_eq!(bobber.hooked_entity_id.load(Relaxed), -1);
    bobber.entity.set_pos(Vector3::new(0.0, 65.0, 2.0));
    velocity = Vector3::new(0.0, 0.0, 1.0);
    bobber.entity.velocity.store(velocity);
    bobber.check_collision(&world, &mut velocity);
    assert_eq!(
        bobber.hooked_entity_id.load(Relaxed),
        target.player.get_entity().entity_id
    );
    bobber.process_tick(&bobber);
    assert_eq!(bobber.state.load(), HookState::HookedInEntity);
    assert_eq!(bobber.entity.velocity.load(), Vector3::default());
    target.player.living_entity.health.store(0.0);
    bobber.process_tick(&bobber);
    assert_eq!(bobber.hooked_entity_id.load(Relaxed), -1);
    assert_eq!(bobber.state.load(), HookState::Flying);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn fishing_stranded_bobbing_hook_at_life_limit_clears_only_its_own_reference() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let fixture = TestPlayer::new(&world);
    let bobber = Arc::new(hook(&world, &fixture.player));
    world.spawn_entity(bobber.clone());
    fixture
        .player
        .fishing_bobber
        .store(bobber.entity.entity_id, Relaxed);
    // FishingHook.tick's life limit still applies to a bobbing hook that stays grounded.
    bobber.state.store(HookState::Bobbing);
    bobber.entity.on_ground.store(true, Relaxed);
    bobber.life.store(1199, Relaxed);
    bobber.process_tick(bobber.as_ref());
    assert!(bobber.entity.is_removed());
    assert_eq!(fixture.player.fishing_bobber.load(Relaxed), -1);
    fixture.player.fishing_bobber.store(12345, Relaxed);
    bobber.clear_owner();
    assert_eq!(fixture.player.fishing_bobber.load(Relaxed), 12345);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn fishing_timers_start_in_water_apply_lure_and_approach_before_a_bite() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let fixture = TestPlayer::new(&world);
    let mut bobber = hook(&world, &fixture.player);
    assert_eq!(bobber.wait_countdown.load(Relaxed), 0);
    bobber.lure_speed = 600;
    let mut velocity = Vector3::default();
    bobber.catching_fish(&world, &BlockPos::new(0, 64, 0), &mut velocity);
    assert!((-500..=0).contains(&bobber.wait_countdown.load(Relaxed)));
    bobber.wait_countdown.store(1, Relaxed);
    // Fixture air has full skylight: no sheltered slowdown.
    let proto = crate::world::spawn_test_support::proto(&Biome::PLAINS, &Block::AIR);
    crate::world::spawn_test_support::publish(&world, proto);
    bobber.catching_fish(&world, &BlockPos::new(0, 64, 0), &mut velocity);
    assert!((20..=80).contains(&bobber.hook_countdown.load(Relaxed)));
    assert_eq!(bobber.bite_countdown.load(Relaxed), 0);
    bobber.hook_countdown.store(1, Relaxed);
    bobber.entity.synched_data.clear_dirty();
    bobber.catching_fish(&world, &BlockPos::new(0, 64, 0), &mut velocity);
    assert!((20..=40).contains(&bobber.bite_countdown.load(Relaxed)));
    assert_eq!(
        regression_tests::tracked_value(&bobber.entity, tracked_data::fishing_bobber::DATA_BITING),
        [1]
    );
    assert!((f64::from(-0.4f32)..=-0.24).contains(&velocity.y));
    bobber.bite_countdown.store(1, Relaxed);
    bobber.wait_countdown.store(88, Relaxed);
    bobber.entity.synched_data.clear_dirty();
    bobber.catching_fish(&world, &BlockPos::new(0, 64, 0), &mut velocity);
    assert_eq!(
        regression_tests::tracked_value(&bobber.entity, tracked_data::fishing_bobber::DATA_BITING),
        [0]
    );
    assert_eq!(bobber.bite_countdown.load(Relaxed), 0);
    assert_eq!(bobber.wait_countdown.load(Relaxed), 0);
    assert_eq!(bobber.hook_countdown.load(Relaxed), 0);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn fishing_hook_stops_on_rod_swap_death_spectator_and_beyond_32_blocks() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let fixture = TestPlayer::new(&world);
    let bobber = hook(&world, &fixture.player);
    let owner = &fixture.player;
    bobber
        .entity
        .set_pos(owner.position().add_raw(32.0, 0.0, 0.0));
    assert!(!bobber.should_stop_fishing(owner));
    bobber
        .entity
        .set_pos(owner.position().add_raw(32.001, 0.0, 0.0));
    assert!(bobber.should_stop_fishing(owner));
    bobber.entity.set_pos(owner.position());
    owner
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::EMPTY.clone());
    assert!(bobber.should_stop_fishing(owner));
    owner
        .inventory()
        .set_stack_in_hand(Hand::Left, ItemStack::new(1, &Item::FISHING_ROD));
    assert!(!bobber.should_stop_fishing(owner));
    owner.gamemode.store(pumpkin_util::GameMode::Spectator);
    assert!(bobber.should_stop_fishing(owner));
    owner.gamemode.store(pumpkin_util::GameMode::Survival);
    owner.living_entity.health.store(0.0);
    assert!(bobber.should_stop_fishing(owner));
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn fishing_context_retains_the_rod_hook_open_water_and_combined_luck() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let fixture = TestPlayer::new(&world);
    let mut bobber = hook(&world, &fixture.player);
    bobber.luck = 3;
    fixture
        .player
        .living_entity
        .set_attribute_base(&Attributes::LUCK, 2.0);
    let mut rod = ItemStack::new(1, &Item::FISHING_ROD);
    rod.add_enchantment(&pumpkin_data::Enchantment::LURE, 2);
    let params = bobber.fishing_loot_context(&fixture.player, &rod);
    assert_eq!(params.luck, 5.0);
    assert!(params.tool.unwrap().are_items_and_components_equal(&rod));
    assert_eq!(params.position, Some(bobber.entity.pos.load()));
    assert_eq!(params.this_entity, Some(&EntityType::FISHING_BOBBER));
    assert_eq!(
        params.this_entity_state.unwrap().fishing_open_water,
        Some(true)
    );
    assert!(Arc::ptr_eq(
        &params.registry.unwrap(),
        &server.datapack_manager
    ));
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn fishing_treasure_predicate_requires_open_water_and_keeps_nested_loot_tables() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let fixture = TestPlayer::new(&world);
    let bobber = hook(&world, &fixture.player);
    let rod = fixture.player.inventory().held_item();
    let table = world.get_loot_table("minecraft:gameplay/fishing").unwrap();
    // A high player luck makes treasure the only positive-weight category in vanilla data.
    fixture
        .player
        .living_entity
        .set_attribute_base(&Attributes::LUCK, 100.0);
    let params = bobber.fishing_loot_context(&fixture.player, &rod);
    let items = table.generate_loot_with_context(1, &params);
    assert!(!items.is_empty());
    assert!(items.iter().all(|stack| stack.item != &Item::COD
        && stack.item != &Item::SALMON
        && stack.item != &Item::STICK));
    bobber.open_water.store(false, Relaxed);
    let params = bobber.fishing_loot_context(&fixture.player, &rod);
    assert!(table.generate_loot_with_context(1, &params).is_empty());
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn fishing_open_water_requires_uniform_source_layers_and_clear_air_above() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let fixture = TestPlayer::new(&world);
    let bobber = hook(&world, &fixture.player);
    let make_water = |obstruction: Option<Block>| {
        let mut proto = crate::world::spawn_test_support::proto(&Biome::PLAINS, &Block::AIR);
        for y in 63..=64 {
            for x in 6..=10 {
                for z in 6..=10 {
                    proto.set_block_state(x, y, z, Block::WATER.default_state);
                }
            }
        }
        if let Some(block) = obstruction {
            proto.set_block_state(8, 65, 8, block.default_state);
        }
        crate::world::spawn_test_support::publish(&world, proto);
    };
    make_water(None);
    assert!(bobber.calculate_open_water(&BlockPos::new(8, 64, 8)));
    make_water(Some(Block::LILY_PAD));
    assert!(bobber.calculate_open_water(&BlockPos::new(8, 64, 8)));
    make_water(Some(Block::STONE));
    assert!(!bobber.calculate_open_water(&BlockPos::new(8, 64, 8)));
    assert!(!bobber.calculate_open_water(&BlockPos::new(6, 64, 8)));
    bobber.entity.set_pos(Vector3::new(8.0, 64.5, 8.0));
    bobber.bite_countdown.store(5, Relaxed);
    bobber.bob_tick(&world, &BlockPos::new(8, 64, 8), 1.0, Vector3::default());
    assert!(!bobber.is_open_water_fishing());
    make_water(None);
    bobber.bob_tick(&world, &BlockPos::new(8, 64, 8), 1.0, Vector3::default());
    assert!(!bobber.is_open_water_fishing());
    bobber.bite_countdown.store(0, Relaxed);
    bobber.hook_countdown.store(0, Relaxed);
    bobber.bob_tick(&world, &BlockPos::new(8, 64, 8), 1.0, Vector3::default());
    assert!(bobber.is_open_water_fishing());
    bobber.hook_countdown.store(10, Relaxed);
    bobber.out_of_water_time.store(10, Relaxed);
    bobber.bob_tick(&world, &BlockPos::new(8, 64, 8), 1.0, Vector3::default());
    assert!(!bobber.is_open_water_fishing());
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn fishing_retrieval_costs_and_pull_distinguish_items_mobs_ground_and_empty_reels() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let fixture = TestPlayer::new(&world);
    for (kind, grounded, expected) in [
        (None, false, 0),
        (None, true, 2),
        (Some(&EntityType::ITEM), false, 3),
        (Some(&EntityType::COW), false, 5),
        (Some(&EntityType::COW), true, 2),
    ] {
        let bobber = Arc::new(hook(&world, &fixture.player));
        world.spawn_entity(bobber.clone());
        fixture
            .player
            .fishing_bobber
            .store(bobber.entity.entity_id, Relaxed);
        bobber
            .entity
            .set_pos(fixture.player.position().add_raw(5.0, 0.0, 0.0));
        bobber.entity.on_ground.store(grounded, Relaxed);
        let target =
            kind.map(|kind| Arc::new(Entity::new(world.clone(), bobber.entity.pos.load(), kind)));
        if let Some(target) = &target {
            world.add_entity_silent(target.clone());
            bobber.set_hooked_entity(Some(target.entity_id));
        }
        let cost = bobber.reel_in(
            &fixture.player,
            &fixture.player.inventory().held_item(),
            Hand::Right,
        );
        assert_eq!(cost, expected);
        if let Some(target) = target {
            assert_eq!(target.velocity.load(), Vector3::new(-0.5, 0.0, 0.0));
        }
        assert_eq!(fixture.player.fishing_bobber.load(Relaxed), -1);
        assert!(bobber.entity.is_removed());
    }
    crate::server::fixture_lifecycle::finish().await;
}
