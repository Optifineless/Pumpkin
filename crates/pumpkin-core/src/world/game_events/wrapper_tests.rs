use super::test_support::{DEPARTURE, Scenario};
use crate::entity::EntityBase;
use pumpkin_data::{
    damage::DamageType,
    data_component_impl::{DeathEffect, DeathProtectionImpl, UseEffectsImpl},
    game_event::GameEvent,
    item::Item,
    item_stack::ItemStack,
};
use pumpkin_protocol::{
    VarInt,
    java::server::play::{SPlayerAction, Status},
};
use pumpkin_util::{
    Hand,
    math::{position::BlockPos, vector2::Vector2, vector3::Vector3},
};

const ORIGIN: Vector3<f64> = Vector3::new(8.5, 66.2, 8.5);

fn release(enabled: bool) {
    let scenario = Scenario::new(true);
    scenario.capture(None);
    let mut shield = ItemStack::new(1, &Item::SHIELD);
    shield.set_data_component(UseEffectsImpl {
        interact_vibrations: enabled,
        ..UseEffectsImpl::DEFAULT
    });
    let uid = shield.uid;
    scenario
        .owner
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, shield);
    scenario.use_item();
    assert_eq!(
        *scenario
            .owner
            .player
            .living_entity
            .active_hand
            .lock()
            .unwrap(),
        Some(Hand::Right)
    );
    scenario.owner.client().handle_player_action(
        &scenario.owner.player,
        &SPlayerAction {
            status: VarInt(Status::ReleaseItemInUse as i32),
            position: BlockPos::new(0, 0, 0),
            face: 0,
            sequence: VarInt(2),
        },
        &scenario.server,
    );
    assert_eq!(
        *scenario
            .owner
            .player
            .living_entity
            .active_hand
            .lock()
            .unwrap(),
        None
    );
    let hand = scenario.owner.player.inventory().held_item();
    assert_eq!(hand.uid, uid);
    assert_eq!(hand.item_count, 1);
    assert_eq!(hand.get_damage(), 0);
    let sink = scenario.sink.lock().unwrap();
    if !enabled {
        assert!(sink.plugins.is_empty());
        assert!(sink.forwarded.is_empty());
        return;
    }
    assert_eq!(sink.plugins.len(), 1);
    assert_eq!(sink.plugins[0].key, "item_interact_finish");
    assert_eq!(sink.order, ["plugin", "dispatch"]);
    assert_eq!(sink.forwarded.len(), 1);
    assert_eq!(sink.forwarded[0].event, GameEvent::ItemInteractFinish);
    assert_eq!(sink.forwarded[0].position, DEPARTURE);
    assert!(sink.forwarded[0].affected_state.is_none());
    scenario.assert_source(
        sink.forwarded[0]
            .source
            .as_ref()
            .expect("release must preserve its source"),
        DEPARTURE,
    );
}

fn protect(teleport: bool, floor: bool, vibrations: bool) {
    let scenario = Scenario::new(true);
    scenario.owner.player.get_entity().set_pos(ORIGIN);
    scenario
        .server
        .datapack_manager
        .load_vanilla_consume_effect_tags_for_test();
    scenario.capture(None);
    if !floor {
        scenario
            .world
            .level
            .loaded_chunks
            .remove(&Vector2::new(0, 0));
    }
    let mut totem = ItemStack::new(1, &Item::TOTEM_OF_UNDYING);
    totem.set_data_component(UseEffectsImpl {
        interact_vibrations: vibrations,
        ..UseEffectsImpl::DEFAULT
    });
    // Diameter zero and the support one block below give a deterministic, distinct departure.
    if teleport {
        totem.set_data_component(DeathProtectionImpl {
            death_effects: vec![DeathEffect::TeleportRandomly {
                diameter: 0.0,
                directional_particles: false,
            }]
            .into(),
        });
    }
    scenario
        .owner
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, totem);
    assert!(scenario.owner.player.damage(
        scenario.owner.player.as_ref(),
        40.0,
        DamageType::GENERIC
    ));
    assert!(scenario.owner.player.inventory().held_item().is_empty());
    assert_eq!(scenario.owner.player.living_entity.health.load(), 1.0);
    assert_protection(&scenario, teleport && floor, vibrations);
}

fn assert_protection(scenario: &Scenario, moved: bool, vibrations: bool) {
    let final_position = if moved {
        Vector3::new(8.5, 65.2, 8.5)
    } else {
        ORIGIN
    };
    assert_eq!(scenario.owner.player.position(), final_position);
    let sink = scenario.sink.lock().unwrap();
    // The real damage entry also emits its existing, source-less entity_damage notification.
    assert_eq!(sink.plugins[0].key, "entity_damage");
    assert_eq!(sink.plugins[0].position, ORIGIN);
    assert_eq!(sink.plugins[0].count, 1);
    assert_eq!(sink.forwarded[0].event, GameEvent::EntityDamage);
    assert_eq!(sink.forwarded[0].position, ORIGIN);
    assert!(sink.forwarded[0].source.is_none());
    let plugins = &sink.plugins[1..];
    let forwarded = &sink.forwarded[1..];
    assert_eq!(plugins.len(), usize::from(vibrations) + usize::from(moved));
    if vibrations {
        assert_eq!(plugins[0].key, "item_interact_finish");
        assert_eq!(
            plugins[0].health, 0.0,
            "totem callback precedes health restoration"
        );
        assert_eq!(plugins[0].count, 0, "totem payment already committed");
    }
    if moved {
        assert_eq!(plugins.last().unwrap().key, "teleport");
        assert_eq!(plugins.last().unwrap().position, ORIGIN);
    }
    assert_eq!(forwarded.len(), plugins.len());
    let mut expected = Vec::new();
    if vibrations {
        expected.push(GameEvent::ItemInteractFinish);
    }
    if moved {
        expected.push(GameEvent::Teleport);
    }
    assert_eq!(
        forwarded
            .iter()
            .map(|payload| payload.event)
            .collect::<Vec<_>>(),
        expected
    );
    for payload in forwarded {
        assert_eq!(
            payload.position, ORIGIN,
            "teleport retains explicit departure"
        );
        assert!(payload.affected_state.is_none());
        scenario.assert_source(
            payload
                .source
                .as_ref()
                .expect("existing wrappers must preserve their source"),
            if payload.event == GameEvent::Teleport {
                final_position
            } else {
                ORIGIN
            },
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s1_existing_wrappers_preserve_finish_and_departure() {
    release(false);
    protect(true, false, false); // Failed teleport emits no teleport notification.
    protect(false, true, false); // Component-disabled ordinary protector.
    protect(false, true, true); // Ordinary protector emits only finish.
    release(true);
    protect(true, true, true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_decorated_pot_destroy_event_retains_projectile_context() {
    use crate::item::items::bow::BowItem;
    use pumpkin_data::{Block, block_properties::DecoratedPotLikeProperties, entity::EntityType};
    use pumpkin_world::world::BlockFlags;
    use std::sync::Arc;

    let scenario = Scenario::new(true);
    let position = BlockPos::new(8, 65, 8);
    let initial = Block::DECORATED_POT.default_state.id;
    scenario.world.set_block_state(
        &position,
        initial,
        BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
    );
    scenario.capture(None);
    let shooter = scenario.owner.player.get_entity();
    shooter.set_pos(Vector3::new(5.5, 65.6 - shooter.get_eye_height(), 8.5));
    shooter.set_rotation(-90.0, 0.0);
    BowItem::shoot(
        scenario.owner.player.as_ref(),
        &ItemStack::new(1, &Item::BOW),
        &[ItemStack::new(1, &Item::ARROW)],
        1.0,
        0.0,
        false,
    );
    let arrow = scenario
        .world
        .entities
        .load()
        .iter()
        .find(|entity| entity.get_entity().entity_type == &EntityType::ARROW)
        .expect("the real bow launch must admit an arrow")
        .clone();
    for _ in 0..4 {
        arrow.tick(arrow.as_ref(), &scenario.server);
        if scenario.world.get_block(&position) == &Block::AIR {
            break;
        }
    }
    assert_eq!(scenario.world.get_block(&position), &Block::AIR);
    tokio::time::timeout(
        std::time::Duration::from_secs(20),
        scenario.server.shutdown(),
    )
    .await
    .expect("projectile-context fixture shutdown must finish");
    let sink = scenario.sink.lock().unwrap();
    let destroyed: Vec<_> = sink
        .forwarded
        .iter()
        .filter(|event| event.event == GameEvent::BlockDestroy)
        .collect();
    assert_eq!(destroyed.len(), 1);
    let event = destroyed[0];
    assert_eq!(event.position, position.to_centered_f64());
    let source = event
        .source
        .as_ref()
        .expect("the arrow is the event source");
    assert_eq!(source.id, arrow.get_entity().entity_id);
    assert_eq!(source.uuid, arrow.get_entity().entity_uuid);
    assert_eq!(
        source.instance,
        std::ptr::from_ref(arrow.get_entity()) as usize
    );
    assert_eq!(source.world, Arc::as_ptr(&scenario.world) as usize);
    assert_eq!(source.life, None);
    let mut properties = DecoratedPotLikeProperties::from_state_id(initial);
    properties.cracked = true;
    assert_eq!(
        event.affected_state,
        Some(properties.to_state_id(&Block::DECORATED_POT))
    );
}
