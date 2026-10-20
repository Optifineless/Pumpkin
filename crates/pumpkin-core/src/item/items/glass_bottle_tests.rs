#![allow(clippy::unwrap_used)]

#[path = "glass_bottle_test_support.rs"]
mod support;

use std::sync::Arc;

use crate::{
    command::CommandSender,
    entity::{
        EntityBase, area_effect_cloud::AreaEffectCloudEntity, item::ItemEntity,
        living::LivingEntity,
    },
    net::java::sound_test_support::decode_sounds,
    plugin::{
        EventPriority,
        api::events::{
            player::{
                player_interact_event::InteractAction,
                player_statistic_increment::PlayerStatisticIncrementEvent,
            },
            world::generic_game::GenericGameEvent,
        },
    },
};
use pumpkin_data::{
    Block,
    block_properties::WaterLikeProperties,
    entity::EntityType,
    game_event::GameEvent,
    item::Item,
    item_stack::ItemStack,
    sound::{Sound, SoundCategory},
};
use pumpkin_inventory::Inventory;
use pumpkin_util::{
    GameMode, Hand, PermissionLvl,
    math::{position::BlockPos, vector3::Vector3},
};
use support::*;
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn source_water_packet_uses_vanilla_hit_and_sound_event_stat_order() {
    let mut fixture = Fixture::new();
    fixture.put_source_water(WATER_POSITION);
    fixture.hold_bottle(Hand::Right, 1);
    let interact = register_interact(&fixture, Hand::Right, false, None);
    let game = Arc::new(GameRecorder {
        player: fixture.actor.player.clone(),
        observer: fixture.observer.clone(),
        events: RecorderMutex::new(Vec::new()),
    });
    let stat = Arc::new(StatRecorder {
        player: fixture.actor.player.clone(),
        events: RecorderMutex::new(Vec::new()),
    });
    fixture
        .server
        .plugin_manager
        .register::<GenericGameEvent, _>(game.clone(), EventPriority::Normal, true);
    fixture
        .server
        .plugin_manager
        .register::<PlayerStatisticIncrementEvent, _>(stat.clone(), EventPriority::Normal, true);

    fixture.use_item(Hand::Right);

    let interactions = interact.events.lock().unwrap();
    assert_eq!(interactions.len(), 1);
    assert!(matches!(
        interactions[0].action,
        InteractAction::RightClickBlock
    ));
    assert_eq!(interactions[0].clicked_pos, Some(WATER_POSITION));
    assert_eq!(interactions[0].block, &Block::WATER);
    drop(interactions);

    let events = game.events.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].key, GameEvent::FluidPickup.name());
    assert_eq!(events[0].position, WATER_POSITION.to_centered_f64());
    assert_eq!(events[0].hand_item, &Item::GLASS_BOTTLE);
    assert_eq!(events[0].used_stat, 0);
    assert_eq!(events[0].sounds_already_sent.len(), 1);
    assert_eq!(
        events[0].sounds_already_sent[0].sound_id,
        Sound::ItemBottleFill as u16
    );
    assert_eq!(
        events[0].sounds_already_sent[0].category,
        SoundCategory::Neutral as i32
    );
    assert_eq!(
        events[0].sounds_already_sent[0].position,
        Vector3::new(68, 512, 68)
    );
    drop(events);

    let stats = stat.events.lock().unwrap();
    assert_eq!(stats.len(), 1);
    assert_eq!(
        stats[0].statistic_id,
        format!("Used:{}", Item::GLASS_BOTTLE.id)
    );
    assert_eq!(stats[0].amount, 1);
    assert_eq!(stats[0].hand_item, &Item::GLASS_BOTTLE);
    assert_eq!(stats[0].previous_value, 0);
    drop(stats);

    assert_eq!(fixture.used_bottle_stat(), 1);
    assert_eq!(
        fixture.actor.player.inventory().held_item().item,
        &Item::POTION
    );
    assert_eq!(fixture.potion_count(), 1);
    assert!(decode_sounds(&fixture.actor.take_packets()).is_empty());
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn flowing_water_is_not_bottled() {
    let fixture = Fixture::new();
    let flowing = WaterLikeProperties { level: 1 }
        .to_state_id(&Block::WATER)
        .to_state();
    fixture.put_state(WATER_POSITION, flowing);
    fixture.hold_bottle(Hand::Right, 1);

    fixture.use_item(Hand::Right);

    assert_eq!(
        fixture.actor.player.inventory().held_item().item,
        &Item::GLASS_BOTTLE
    );
    assert_eq!(fixture.potion_count(), 0);
    assert_eq!(fixture.used_bottle_stat(), 0);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn waterlogged_fluid_is_not_bottled_as_a_source_block() {
    let fixture = Fixture::new();
    let waterlogged_stairs = Block::OAK_STAIRS
        .default_state
        .set_waterlogged(true)
        .unwrap();
    fixture.put_state(WATER_POSITION, waterlogged_stairs);
    fixture.hold_bottle(Hand::Right, 1);

    fixture.use_item(Hand::Right);

    assert_eq!(
        fixture.actor.player.inventory().held_item().item,
        &Item::GLASS_BOTTLE
    );
    assert_eq!(fixture.potion_count(), 0);
    assert_eq!(fixture.used_bottle_stat(), 0);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nearer_solid_outline_occludes_source_water() {
    let fixture = Fixture::new();
    let obstruction = BlockPos::new(8, 65, 10);
    fixture.put_source_water(WATER_POSITION);
    fixture.put_state(obstruction, Block::STONE.default_state);
    fixture.hold_bottle(Hand::Right, 1);

    fixture.use_item(Hand::Right);

    assert_eq!(
        fixture.actor.player.inventory().held_item().item,
        &Item::GLASS_BOTTLE
    );
    assert_eq!(fixture.potion_count(), 0);
    assert_eq!(fixture.used_bottle_stat(), 0);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn offhand_bottle_uses_the_main_hand_collision_context() {
    let fixture = Fixture::new();
    let light_position = BlockPos::new(8, 65, 10);
    fixture.put_source_water(WATER_POSITION);
    fixture.put_state(light_position, Block::LIGHT.default_state);
    fixture
        .actor
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::new(1, &Item::LIGHT));
    fixture.hold_bottle(Hand::Left, 1);
    let interact = register_interact(&fixture, Hand::Left, false, None);

    fixture.use_item(Hand::Left);

    let events = interact.events.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0].action, InteractAction::RightClickBlock));
    assert_eq!(events[0].clicked_pos, Some(light_position));
    assert_eq!(events[0].block, &Block::LIGHT);
    drop(events);
    assert_eq!(
        fixture.actor.player.inventory().off_hand_item().item,
        &Item::GLASS_BOTTLE
    );
    assert_eq!(fixture.potion_count(), 0);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn survival_reach_targets_a_source_within_four_and_a_half_blocks() {
    let fixture = Fixture::new();
    let target = BlockPos::new(8, 65, 12);
    fixture.put_source_water(target);
    fixture.hold_bottle(Hand::Right, 1);
    let interact = register_interact(&fixture, Hand::Right, false, None);

    fixture.use_item(Hand::Right);

    assert_eq!(
        fixture.actor.player.inventory().held_item().item,
        &Item::POTION
    );
    let events = interact.events.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0].action, InteractAction::RightClickBlock));
    assert_eq!(events[0].clicked_pos, Some(target));
    drop(events);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn creative_reach_and_preuse_event_include_the_five_block_hit() {
    let fixture = Fixture::new();
    let target = BlockPos::new(8, 65, 13);
    fixture.put_source_water(target);
    fixture.actor.player.set_gamemode(GameMode::Creative);
    fixture.hold_bottle(Hand::Right, 1);
    let interact = register_interact(&fixture, Hand::Right, false, None);

    fixture.use_item(Hand::Right);

    assert_eq!(
        fixture.actor.player.inventory().held_item().item,
        &Item::GLASS_BOTTLE
    );
    assert_eq!(fixture.potion_count(), 1);
    let events = interact.events.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0].action, InteractAction::RightClickBlock));
    assert_eq!(events[0].clicked_pos, Some(target));
    drop(events);
    fixture.finish().await;
}

fn enable_spawn_protection(fixture: &Fixture, spawn: BlockPos) {
    fixture.server.level_info.rcu(|info| {
        let mut next = (**info).clone();
        next.spawn_x = spawn.0.x;
        next.spawn_y = spawn.0.y;
        next.spawn_z = spawn.0.z;
        next
    });
    fixture
        .server
        .data
        .operator_config
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .ops
        .push(pumpkin_config::op::Op::new(
            uuid::Uuid::new_v4(),
            "glass-bottle-test-op".to_owned(),
            PermissionLvl::Zero,
            false,
        ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn may_interact_denial_prevents_spawn_protected_bottle_effects() {
    let fixture = Fixture::new();
    fixture.put_source_water(WATER_POSITION);
    fixture.hold_bottle(Hand::Right, 1);
    fixture
        .actor
        .player
        .permission_lvl
        .store(PermissionLvl::Zero);
    enable_spawn_protection(&fixture, BlockPos::new(8, 64, 8));
    let interact = register_interact(&fixture, Hand::Right, false, None);
    let game = Arc::new(GameRecorder {
        player: fixture.actor.player.clone(),
        observer: fixture.observer.clone(),
        events: RecorderMutex::new(Vec::new()),
    });
    fixture
        .server
        .plugin_manager
        .register::<GenericGameEvent, _>(game.clone(), EventPriority::Normal, true);

    fixture.use_item(Hand::Right);

    assert_eq!(
        fixture.actor.player.inventory().held_item().item,
        &Item::GLASS_BOTTLE
    );
    assert_eq!(fixture.potion_count(), 0);
    assert_eq!(fixture.used_bottle_stat(), 0);
    assert!(game.events.lock().unwrap().is_empty());
    let sounds = decode_sounds(&fixture.observer.lock().unwrap().take_packets());
    assert!(sounds.is_empty());
    let events = interact.events.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].clicked_pos, Some(WATER_POSITION));
    drop(events);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn may_interact_denial_prevents_outside_border_bottle_effects() {
    let fixture = Fixture::new();
    fixture.put_source_water(WATER_POSITION);
    fixture.hold_bottle(Hand::Right, 1);
    fixture
        .world
        .worldborder
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .set_diameter(&fixture.world, 1.0, None);
    let game = Arc::new(GameRecorder {
        player: fixture.actor.player.clone(),
        observer: fixture.observer.clone(),
        events: RecorderMutex::new(Vec::new()),
    });
    fixture
        .server
        .plugin_manager
        .register::<GenericGameEvent, _>(game.clone(), EventPriority::Normal, true);

    fixture.use_item(Hand::Right);

    assert_eq!(
        fixture.actor.player.inventory().held_item().item,
        &Item::GLASS_BOTTLE
    );
    assert_eq!(fixture.potion_count(), 0);
    assert_eq!(fixture.used_bottle_stat(), 0);
    assert!(game.events.lock().unwrap().is_empty());
    assert!(decode_sounds(&fixture.observer.lock().unwrap().take_packets()).is_empty());
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g10_last_border_block_admits_real_use() {
    let fixture = Fixture::new();
    fixture
        .server
        .worlds
        .store(Arc::new(vec![fixture.world.clone()]));
    let source = CommandSender::Console
        .into_source(&fixture.server)
        .with_world(fixture.world.clone());
    fixture
        .server
        .command_dispatcher
        .load()
        .execute_input("worldborder set 10", &source)
        .unwrap();

    for (x, player_x, yaw, admitted) in [
        (3, 3.5, 0.0, true),
        (4, 4.5, 0.0, true),
        (5, 4.5, -16.0, false),
    ] {
        let position = BlockPos::new(x, 65, 0);
        fixture.put_source_water(position);
        fixture
            .actor
            .player
            .get_entity()
            .set_pos(Vector3::new(player_x, 64.0, -3.5));
        fixture.actor.player.get_entity().set_rotation(yaw, 0.0);
        fixture.hold_bottle(Hand::Right, 1);
        fixture.use_item(Hand::Right);

        assert_eq!(
            fixture.used_bottle_stat(),
            if admitted { x - 2 } else { 2 },
            "bottle use statistic at x={x}"
        );
        assert_eq!(
            fixture.potion_count(),
            u32::from(admitted),
            "one use replaces the held bottle with one potion at x={x}"
        );
        if !admitted {
            assert_eq!(
                fixture.actor.player.inventory().held_item().item,
                &Item::GLASS_BOTTLE,
                "the x=5 denial must retain the bottle"
            );
        }
        fixture.world.set_block_state(
            &position,
            Block::AIR.default_state.id,
            crate::world::BlockFlags::FORCE_STATE,
        );
    }
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellable_preuse_event_receives_the_bottle_hit_and_stops_filling() {
    let fixture = Fixture::new();
    fixture.put_source_water(WATER_POSITION);
    fixture.hold_bottle(Hand::Right, 1);
    let interact = register_interact(&fixture, Hand::Right, true, None);

    fixture.use_item(Hand::Right);

    let events = interact.events.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0].action, InteractAction::RightClickBlock));
    assert_eq!(events[0].clicked_pos, Some(WATER_POSITION));
    drop(events);
    assert_eq!(
        fixture.actor.player.inventory().held_item().item,
        &Item::GLASS_BOTTLE
    );
    assert_eq!(fixture.potion_count(), 0);
    assert_eq!(fixture.used_bottle_stat(), 0);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plugin_hand_replacement_survives_the_admitted_bottle_hit() {
    let fixture = Fixture::new();
    fixture.put_source_water(WATER_POSITION);
    fixture.hold_bottle(Hand::Right, 1);
    let replacement = ItemStack::new(2, &Item::GLASS_BOTTLE);
    let interact = register_interact(&fixture, Hand::Right, false, Some(replacement.clone()));
    let game = Arc::new(GameRecorder {
        player: fixture.actor.player.clone(),
        observer: fixture.observer.clone(),
        events: RecorderMutex::new(Vec::new()),
    });
    fixture
        .server
        .plugin_manager
        .register::<GenericGameEvent, _>(game.clone(), EventPriority::Normal, true);

    fixture.use_item(Hand::Right);

    let held = fixture.actor.player.inventory().held_item();
    assert_eq!(held.uid, replacement.uid);
    assert_eq!(held.item, &Item::GLASS_BOTTLE);
    assert_eq!(held.item_count, 2);
    assert_eq!(fixture.potion_count(), 0);
    assert_eq!(fixture.used_bottle_stat(), 0);
    assert!(game.events.lock().unwrap().is_empty());
    assert!(decode_sounds(&fixture.observer.lock().unwrap().take_packets()).is_empty());

    let events = interact.events.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0].action, InteractAction::RightClickBlock));
    assert_eq!(events[0].clicked_pos, Some(WATER_POSITION));
    drop(events);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_inventory_overflow_drops_at_the_player_not_the_water() {
    let fixture = Fixture::new();
    fixture.put_source_water(WATER_POSITION);
    let inventory = fixture.actor.player.inventory();
    inventory.set_stack(0, ItemStack::new(2, &Item::GLASS_BOTTLE));
    for slot in 1..inventory.size() {
        inventory.set_stack(slot, ItemStack::new(64, &Item::STONE));
    }

    fixture.use_item(Hand::Right);

    assert_eq!(inventory.held_item().item, &Item::GLASS_BOTTLE);
    assert_eq!(inventory.held_item().item_count, 1);
    let dropped_potion = fixture
        .world
        .entities
        .load()
        .iter()
        .filter_map(|entity| entity.cast_any().downcast_ref::<ItemEntity>())
        .find_map(|item| {
            let stack = item
                .get_item_stack()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (stack.item == &Item::POTION).then(|| (item.get_entity().pos.load(), stack.clone()))
        })
        .expect("the overflowing water potion should be dropped");
    assert_eq!(dropped_potion.1.item_count, 1);
    let player_position = fixture.actor.player.position();
    let player_drop_block = player_position.to_block_pos();
    assert_eq!(
        BlockPos::floored_v(dropped_potion.0),
        player_drop_block,
        "overflow potion drop at {:?}, player's drop block at {:?}",
        dropped_potion.0,
        player_drop_block
    );
    let drop_center_x = f64::from(player_drop_block.0.x) + 0.5;
    let drop_center_z = f64::from(player_drop_block.0.z) + 0.5;
    assert!(
        (dropped_potion.0.x - drop_center_x).abs() <= 0.25,
        "drop at {:?}, player's drop block at {:?}",
        dropped_potion.0,
        player_drop_block
    );
    assert!(
        (dropped_potion.0.z - drop_center_z).abs() <= 0.25,
        "drop at {:?}, player's drop block at {:?}",
        dropped_potion.0,
        player_drop_block
    );
    assert_ne!(player_drop_block.0.z, WATER_POSITION.0.z);
    assert!((dropped_potion.0.z - (f64::from(WATER_POSITION.0.z) + 0.5)).abs() > 1.5);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dragon_breath_keeps_precedence_over_source_water() {
    let fixture = Fixture::new();
    fixture.put_source_water(WATER_POSITION);
    fixture.hold_bottle(Hand::Right, 1);
    let player_position = fixture.actor.player.position();
    let dragon = Arc::new(LivingEntity::new(crate::entity::Entity::new(
        fixture.world.clone(),
        player_position,
        &EntityType::ENDER_DRAGON,
    )));
    let cloud = AreaEffectCloudEntity::new(crate::entity::Entity::new(
        fixture.world.clone(),
        player_position,
        &EntityType::AREA_EFFECT_CLOUD,
    ));
    let cloud_entity = cloud
        .cast_any()
        .downcast_ref::<AreaEffectCloudEntity>()
        .unwrap();
    cloud_entity.set_owner(Some(dragon.as_ref()));
    fixture
        .world
        .entities
        .store(Arc::new(vec![dragon, cloud.clone()]));

    fixture.use_item(Hand::Right);

    assert_eq!(
        fixture.actor.player.inventory().held_item().item,
        &Item::DRAGON_BREATH
    );
    assert_eq!(cloud_entity.radius(), 2.5);
    assert_eq!(fixture.world.get_block(&WATER_POSITION), &Block::WATER);
    assert_eq!(fixture.potion_count(), 0);
    fixture.finish().await;
}
