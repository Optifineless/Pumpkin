use super::*;
use crate::{
    entity::{
        Entity, EntityBase,
        passive::{llama::LlamaEntity, trader_llama::TraderLlamaEntity},
    },
    net::java::combat_test_support::TestPlayer,
    server::combat_test_support,
    world::World,
};
use pumpkin_data::Block;
use pumpkin_util::math::{vector2::Vector2, vector3::Vector3};
use pumpkin_world::chunk::ChunkData;
use std::{cell::Cell, sync::atomic::Ordering::Relaxed};

/// Installs the stone floor used by avoidance regression tests.
pub(in crate::entity::ai::goal) fn floor(world: &World) {
    for cx in -1..=1 {
        for cz in -1..=1 {
            let chunk = ChunkData::empty_sync(cx, cz);
            for x in 0..16 {
                for z in 0..16 {
                    chunk.set_block_absolute_y(x, 63, z, Block::STONE.default_state.id);
                }
            }
            world
                .level
                .loaded_chunks
                .insert(Vector2::new(cx, cz), chunk);
        }
    }
}

#[tokio::test]
async fn avoidance_followup_tamed_cat_stops_fleeing_and_ignores_strangers() {
    let directory = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(directory.path());
    let world = combat_test_support::world(&server, directory.path());
    floor(&world);
    let owner = TestPlayer::new(&world);
    let stranger = TestPlayer::new(&world);
    world.players.store(Arc::new(vec![
        owner.player.clone(),
        stranger.player.clone(),
    ]));
    owner
        .player
        .get_entity()
        .set_pos(Vector3::new(11.5, 64.0, 8.5));
    stranger
        .player
        .get_entity()
        .set_pos(Vector3::new(10.5, 64.0, 8.5));
    let cat = CatEntity::new(Entity::new(
        world,
        Vector3::new(8.5, 64.0, 8.5),
        &EntityType::CAT,
    ));
    cat.get_entity().on_ground.store(true, Relaxed);
    // Exercise the actual constructor's registration, not a separately assembled goal.
    let mut selector = cat.mob_entity.goals_selector.lock().unwrap();
    let mut goals: Vec<_> = selector.goals_for_test::<CatAvoidEntityGoal>().collect();
    assert_eq!(goals.len(), 1);
    let goal = &mut *goals[0];
    assert!((0..256).any(|_| goal.can_start(cat.as_ref())));
    goal.start(cat.as_ref());
    assert!(goal.should_continue(cat.as_ref()));
    cat.set_owner(Some(owner.player.get_entity().entity_uuid));
    cat.set_tame(true, Some(owner.player.get_entity().entity_uuid));
    assert!(!goal.should_continue(cat.as_ref()));
    assert!(!(0..256).any(|_| goal.can_start(cat.as_ref())));
}

#[tokio::test]
async fn avoidance_followup_trusting_ocelot_stops_fleeing() {
    let directory = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(directory.path());
    let world = combat_test_support::world(&server, directory.path());
    floor(&world);
    let player = TestPlayer::new(&world);
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(10.5, 64.0, 8.5));
    let ocelot = OcelotEntity::new(Entity::new(
        world,
        Vector3::new(8.5, 64.0, 8.5),
        &EntityType::OCELOT,
    ));
    ocelot.get_entity().on_ground.store(true, Relaxed);
    let mut selector = ocelot.mob_entity.goals_selector.lock().unwrap();
    let mut goals: Vec<_> = selector.goals_for_test::<OcelotAvoidEntityGoal>().collect();
    assert_eq!(goals.len(), 1);
    let goal = &mut *goals[0];
    assert!((0..256).any(|_| goal.can_start(ocelot.as_ref())));
    goal.start(ocelot.as_ref());
    assert!(goal.should_continue(ocelot.as_ref()));
    ocelot.set_trusting(true);
    assert!(!goal.should_continue(ocelot.as_ref()));
    assert!(!(0..256).any(|_| goal.can_start(ocelot.as_ref())));
}

#[tokio::test]
async fn avoidance_followup_wolf_rolls_for_nearest_llama_and_trader_llama() {
    let directory = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(directory.path());
    let world = combat_test_support::world(&server, directory.path());
    floor(&world);
    let wolf = WolfEntity::new(Entity::new(
        world.clone(),
        Vector3::new(8.5, 64.0, 8.5),
        &EntityType::WOLF,
    ));
    wolf.get_entity().on_ground.store(true, Relaxed);
    let llama = LlamaEntity::new(Entity::new(
        world.clone(),
        Vector3::new(11.5, 64.0, 8.5),
        &EntityType::LLAMA,
    ));
    llama.set_strength(3);
    let trader = TraderLlamaEntity::new(Entity::new(
        world.clone(),
        Vector3::new(13.5, 64.0, 8.5),
        &EntityType::TRADER_LLAMA,
    ));
    trader.set_strength(5);
    world.entities.store(Arc::new(vec![llama, trader.clone()]));
    let registered = wolf
        .mob_entity
        .goals_selector
        .lock()
        .unwrap()
        .goals_for_test::<WolfAvoidEntityGoal>()
        .count();
    assert_eq!(registered, 1);
    let mut goal = WolfAvoidEntityGoal::new(&wolf);
    assert!((0..256).any(|_| goal.can_start_with_roll(wolf.as_ref(), || 3)));
    let rolls = Cell::new(0);
    for _ in 0..256 {
        assert!(!goal.can_start_with_roll(wolf.as_ref(), || {
            rolls.set(rolls.get() + 1);
            4
        }));
    }
    assert!(
        rolls.get() > 0,
        "the path succeeded and the strength roll rejected avoidance"
    );
    wolf.set_tame(true);
    assert!(!(0..256).any(|_| goal.can_start_with_roll(wolf.as_ref(), || 0)));
    wolf.set_tame(false);
    world.entities.store(Arc::new(vec![trader.clone()]));
    wolf.mob_entity.sensing.lock().unwrap().tick();
    assert!((0..256).any(|_| goal.can_start_with_roll(wolf.as_ref(), || 4)));
    wolf.mob_entity.set_target(Some(trader.clone()));
    goal.start(wolf.as_ref());
    assert!(wolf.mob_entity.get_target().is_none());
    wolf.mob_entity.set_target(Some(trader));
    goal.tick(wolf.as_ref());
    assert!(wolf.mob_entity.get_target().is_none());
}

#[tokio::test]
async fn avoidance_followup_optional_predicate() {
    let directory = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(directory.path());
    let world = combat_test_support::world(&server, directory.path());
    floor(&world);
    let player = TestPlayer::new(&world);
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(10.5, 64.0, 8.5));
    let cat = CatEntity::new(Entity::new(
        world,
        Vector3::new(8.5, 64.0, 8.5),
        &EntityType::CAT,
    ));
    cat.get_entity().on_ground.store(true, Relaxed);
    let mut rejecting = AvoidEntityGoal::new(
        &EntityType::PLAYER,
        8.0,
        1.6,
        1.4,
        Some(Box::new(|_, _| false)),
    );
    assert!(!(0..256).any(|_| rejecting.can_start(cat.as_ref())));
    let mut accepting = AvoidEntityGoal::new(&EntityType::PLAYER, 8.0, 1.6, 1.4, None);
    assert!((0..256).any(|_| accepting.can_start(cat.as_ref())));
}

#[tokio::test]
async fn avoidance_followup_creative_and_spectator_filter() {
    let directory = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(directory.path());
    let world = combat_test_support::world(&server, directory.path());
    let player = TestPlayer::new(&world);
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(10.5, 64.0, 8.5));
    let cat = CatEntity::new(Entity::new(
        world,
        Vector3::new(8.5, 64.0, 8.5),
        &EntityType::CAT,
    ));
    assert!(AvoidEntityGoal::find_threat(cat.as_ref(), 8.0, |_| true).is_some());
    for mode in [
        pumpkin_util::GameMode::Creative,
        pumpkin_util::GameMode::Spectator,
    ] {
        player.player.gamemode.store(mode);
        assert!(AvoidEntityGoal::find_threat(cat.as_ref(), 8.0, |_| true).is_none());
    }
}
