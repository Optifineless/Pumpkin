use super::{
    GameEventContext,
    test_support::{DEPARTURE, Scenario},
};
use crate::entity::{Entity, EntityBase};
use pumpkin_data::{Block, entity::EntityType, item::Item, item_stack::ItemStack};
use pumpkin_util::{
    Hand,
    math::{position::BlockPos, vector3::Vector3},
};
use std::sync::{Arc, atomic::Ordering::Relaxed};

fn costs(s: &Scenario) -> (usize, usize) {
    (
        s.world.game_event_sources.probes.swap(0, Relaxed),
        s.world.game_event_sources.comparisons.swap(0, Relaxed),
    )
}

fn background(s: &Scenario, count: usize) {
    for i in 0..count {
        let entity = crate::entity::r#type::from_type(
            if i % 2 == 0 {
                &EntityType::WARDEN
            } else {
                &EntityType::MARKER
            },
            Vector3::new(1000.5 + i as f64, 65.0, 1000.5),
            &s.world,
            uuid::Uuid::new_v4(),
        );
        s.world.add_entity_silent(entity);
    }
    // Use the production publisher for independent player sessions, without TestPlayer's
    // deliberately replacing single-player fixture semantics.
    for i in 0..count {
        use crate::{
            entity::player::Player,
            net::{ClientPlatform, java::combat_test_support::captured_client},
        };
        let profile = crate::net::GameProfile {
            id: uuid::Uuid::new_v4(),
            name: format!("background-{i}"),
            properties: arc_swap::ArcSwap::from_pointee(Vec::new()),
            profile_actions: None,
        };
        let (client, _packets) = captured_client(profile.clone());
        let player = Arc::new(Player::new(
            Arc::new(ClientPlatform::Java(client)),
            profile,
            crate::net::PlayerConfig::default(),
            &s.world,
            pumpkin_util::GameMode::Survival,
        ));
        s.world.add_pending_player(&player);
    }
    // Real block-entity candidates in unrelated loaded sections are background only:
    // s1 has no registered production vibration listeners.
    for i in 0..(count / 128) {
        let chunk = pumpkin_world::chunk::ChunkData::empty_sync(64 + i as i32, 64);
        s.world.level.loaded_chunks.insert(
            pumpkin_util::math::vector2::Vector2::new(64 + i as i32, 64),
            chunk,
        );
        let position = BlockPos::new((64 + i as i32) * 16, 64, 64 * 16);
        s.world.set_block_state(
            &position,
            Block::SCULK_SENSOR.default_state.id,
            pumpkin_world::world::BlockFlags::FORCE_STATE,
        );
        s.world.add_block_entity(Arc::new(
            crate::block::entities::sculk_sensor::SculkSensorBlockEntity::new(position),
        ));
        assert!(s.world.get_block_entity(&position).is_some());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s2_indexed_source_cost_ignores_unrelated_listeners() {
    for enabled in [true, false] {
        for population in [0, 128, 2048] {
            let s = Scenario::new(enabled);
            background(&s, population);
            s.capture(None);
            s.rod(Some(true));
            costs(&s);
            s.use_item();
            let hook = s
                .world
                .get_entity_by_id(s.owner.player.fishing_bobber.load(Relaxed))
                .unwrap();
            hook.get_entity().on_ground.store(true, Relaxed);
            s.use_item();
            assert!(hook.get_entity().is_removed());
            assert_eq!(s.owner.player.inventory().held_item().get_damage(), 2);
            assert_eq!(
                costs(&s),
                if enabled { (4, 6) } else { (0, 0) },
                "population {population}, enabled {enabled}"
            );
            let sink = s.sink.lock().unwrap();
            assert_eq!(sink.plugins.len(), 2);
            assert_eq!(sink.forwarded.len(), if enabled { 2 } else { 0 });
            if enabled {
                for event in &sink.forwarded {
                    s.assert_source(event.source.as_ref().unwrap(), DEPARTURE);
                }
            }
            drop(sink);
            release_and_new_adapters(&s, enabled);
            let missing = Entity::new(s.world.clone(), DEPARTURE, &EntityType::MARKER);
            s.world.emit_game_event_with_context(
                "entity_action",
                DEPARTURE,
                GameEventContext {
                    source: Some(&missing),
                    affected_state: None,
                },
            );
            assert_eq!(
                costs(&s),
                if enabled { (1, 0) } else { (0, 0) },
                "missing ID must never scan"
            );
        }
    }
}

fn release_and_new_adapters(s: &Scenario, enabled: bool) {
    use pumpkin_data::{
        data_component_impl::UseEffectsImpl, enchantment::LevelBasedValue, game_event::GameEvent,
    };
    use pumpkin_protocol::{
        VarInt,
        java::server::play::{SPlayerAction, Status},
    };
    let mut shield = ItemStack::new(1, &Item::SHIELD);
    shield.set_data_component(UseEffectsImpl {
        interact_vibrations: true,
        ..UseEffectsImpl::DEFAULT
    });
    let uid = shield.uid;
    s.owner
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, shield);
    s.use_item();
    s.owner.client().handle_player_action(
        &s.owner.player,
        &SPlayerAction {
            status: VarInt(Status::ReleaseItemInUse as i32),
            position: BlockPos::new(0, 0, 0),
            face: 0,
            sequence: VarInt(2),
        },
        &s.server,
    );
    assert_eq!(s.owner.player.inventory().held_item().uid, uid);
    assert_eq!(
        costs(s),
        if enabled { (2, 3) } else { (0, 0) },
        "real item release cost"
    );
    super::adapter_tests::flower(s);
    assert_eq!(
        costs(s),
        if enabled { (4, 6) } else { (0, 0) },
        "real flower-pot adapter cost"
    );
    let actor: Arc<dyn EntityBase> =
        Arc::new(Entity::new(s.world.clone(), DEPARTURE, &EntityType::MARKER));
    s.world.add_entity_silent(actor.clone());
    let target = BlockPos::new(10, 65, 8);
    s.world.set_block_state(
        &target,
        Block::WHITE_WOOL.default_state.id,
        pumpkin_world::world::BlockFlags::FORCE_STATE,
    );
    let effect = crate::enchantment::effects::replace_disk::ReplaceDisk::simple(
        LevelBasedValue::Constant(1.0),
        LevelBasedValue::Constant(1.0),
        Vector3::new(0, 0, 0),
        Block::AIR.default_state,
        Some(GameEvent::BlockChange),
    );
    effect.apply(
        &s.world,
        1,
        Some(actor.get_entity()),
        target.to_centered_f64(),
    );
    assert_eq!(
        s.world.get_block(&target),
        &Block::AIR,
        "actual effect destroys wool before notification"
    );
    let sink = s.sink.lock().unwrap();
    assert_eq!(sink.plugins.len(), 5);
    if enabled {
        let event = sink.forwarded.last().unwrap();
        assert_eq!(
            event.source.as_ref().map(|source| source.uuid),
            Some(actor.get_entity().entity_uuid)
        );
        assert!(
            event.affected_state.is_none(),
            "ReplaceDisk uses actor-only context even for wool"
        );
    }
    drop(sink);
    assert_eq!(
        costs(s),
        if enabled { (4, 6) } else { (0, 0) },
        "real nonplayer effect adapter cost"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s2_index_lifecycle_reentry_and_off() {
    for enabled in [true, false] {
        let s = Scenario::new(enabled);
        let player = s.owner.player.clone();
        let world = s.world.clone();
        s.capture(Some(Box::new(move |_, _| {
            assert!(world.withdraw_game_event_player(&player).is_some());
            world.add_pending_player(&player);
        })));
        s.rod(Some(true));
        costs(&s);
        s.use_item();
        assert_eq!(s.sink.lock().unwrap().plugins.len(), 1);
        assert!(
            s.sink.lock().unwrap().forwarded.is_empty(),
            "same Arc re-admission must reject the outer event"
        );
        assert_eq!(costs(&s), if enabled { (2, 2) } else { (0, 0) });
    }
    let s = Scenario::new(true);
    let old: Arc<dyn EntityBase> =
        Arc::new(Entity::new(s.world.clone(), DEPARTURE, &EntityType::MARKER));
    s.world.add_entity_silent(old.clone());
    assert!(
        s.world
            .entity_tracker
            .get_tracked_entity(old.get_entity().entity_id)
            .is_none()
    );
    s.capture(None);
    s.world.game_event_item_interact_finish(old.get_entity());
    assert_eq!(
        s.sink.lock().unwrap().forwarded[0]
            .source
            .as_ref()
            .unwrap()
            .uuid,
        old.get_entity().entity_uuid
    );
    old.get_entity().remove();
    assert!(
        s.world
            .game_event_source_membership(old.get_entity().entity_id)
            .is_none()
    );
    let replacement: Arc<dyn EntityBase> = Arc::new(Entity::from_uuid_with_id(
        old.get_entity().entity_id,
        uuid::Uuid::new_v4(),
        s.world.clone(),
        DEPARTURE,
        &EntityType::MARKER,
    ));
    s.world.add_entity_silent(replacement.clone());
    s.world.retire_game_event_source(old.get_entity());
    s.world
        .game_event_item_interact_finish(replacement.get_entity());
    assert_eq!(
        s.sink.lock().unwrap().forwarded.len(),
        2,
        "late old teardown cannot remove replacement membership"
    );
    s.world.game_event_item_interact_finish(old.get_entity());
    assert_eq!(s.sink.lock().unwrap().forwarded.len(), 2);
    let hand = ItemStack::new(1, &Item::STONE);
    s.owner
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, hand);
    assert_eq!(s.world.get_block(&BlockPos::new(8, 64, 8)), &Block::STONE);
}
