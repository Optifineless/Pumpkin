use super::test_support::Scenario;
use crate::entity::{
    Entity, EntityBase,
    projectile::{arrow::ArrowEntity, llama_spit::LlamaSpitEntity},
};
use pumpkin_data::{
    Block, entity::EntityType, game_event::GameEvent, item::Item, item_stack::ItemStack,
};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::{
    VarInt,
    java::server::play::{SPlayerAction, Status},
};
use pumpkin_util::{
    Hand,
    math::{position::BlockPos, vector3::Vector3},
};
use pumpkin_world::world::BlockFlags;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering::Relaxed},
};

const HIT: BlockPos = BlockPos::new(8, 65, 11);

pub(super) fn paid_arrow(s: &Scenario) -> Arc<dyn EntityBase> {
    s.owner
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::new(1, &Item::BOW));
    s.owner
        .player
        .inventory()
        .set_stack(1, ItemStack::new(3, &Item::ARROW));
    s.use_item();
    s.owner
        .player
        .living_entity
        .item_use_time
        .store(72000 - 20, Relaxed);
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
    assert_eq!(s.owner.player.inventory().get_stack(1).item_count, 2);
    assert_eq!(s.owner.player.inventory().held_item().get_damage(), 1);
    let arrows: Vec<_> = s
        .world
        .entities
        .load()
        .iter()
        .filter(|e| e.get_entity().entity_type == &EntityType::ARROW)
        .cloned()
        .collect();
    assert_eq!(
        arrows.len(),
        1,
        "one real bow release admits one paid projectile"
    );
    arrows[0].clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s2_projectile_identity_and_existing_entity_sources() {
    let s = Scenario::new(true);
    let world = s.world.clone();
    s.capture(Some(Box::new(move |_, event| {
        if event.event_key == "projectile_land" {
            world.set_block_state(
                &HIT,
                Block::GOLD_BLOCK.default_state.id,
                BlockFlags::FORCE_STATE,
            );
        }
    })));
    let arrow = paid_arrow(&s);
    arrow.get_entity().set_pos(Vector3::new(8.5, 65.5, 8.5));
    arrow
        .get_entity()
        .velocity
        .store(Vector3::new(0.0, 0.0, 0.2));
    arrow.tick(arrow.as_ref(), &s.server);
    let sink = s.sink.lock().unwrap();
    let shoots: Vec<_> = sink
        .forwarded
        .iter()
        .filter(|e| e.event == GameEvent::ProjectileShoot)
        .collect();
    assert_eq!(shoots.len(), 1);
    s.assert_source(
        shoots[0]
            .source
            .as_ref()
            .expect("SHOOT source must be owner"),
        s.owner.player.position(),
    );
    assert!(shoots[0].affected_state.is_none());
    assert!(
        !sink
            .forwarded
            .iter()
            .any(|e| e.event == GameEvent::ProjectileLand)
    );
    drop(sink);
    s.world.set_block_state(
        &HIT,
        Block::WHITE_WOOL.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    arrow.get_entity().set_pos(Vector3::new(8.5, 65.5, 9.5));
    arrow
        .get_entity()
        .velocity
        .store(Vector3::new(0.0, 0.0, 2.0));
    arrow.tick(arrow.as_ref(), &s.server);
    assert!(!arrow.get_entity().is_removed());
    assert!(
        arrow
            .cast_any()
            .downcast_ref::<ArrowEntity>()
            .unwrap()
            .in_ground
            .load(Relaxed)
    );
    assert_eq!(s.world.get_block(&HIT), &Block::GOLD_BLOCK);
    let sink = s.sink.lock().unwrap();
    let lands: Vec<_> = sink
        .forwarded
        .iter()
        .filter(|e| e.event == GameEvent::ProjectileLand)
        .collect();
    assert_eq!(lands.len(), 1);
    assert_eq!(lands[0].position, Vector3::new(8.5, 65.5, 11.5));
    assert_eq!(
        lands[0].affected_state,
        Some(Block::WHITE_WOOL.default_state.id),
        "land context retains post-hit state before the game-event callback"
    );
    let source = lands[0]
        .source
        .as_ref()
        .expect("LAND source must be projectile");
    assert_eq!(source.id, arrow.get_entity().entity_id);
    assert_eq!(source.uuid, arrow.get_entity().entity_uuid);
    assert_eq!(source.life, None);
    assert_eq!(
        sink.plugins
            .iter()
            .filter(|e| e.key == "projectile_land")
            .count(),
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s2_live_entity_land_and_retired_land_boundary() {
    let s = Scenario::new(true);
    s.capture(None);
    let target = crate::entity::r#type::from_type(
        &EntityType::COW,
        Vector3::new(12.5, 65.0, 8.5),
        &s.world,
        uuid::Uuid::new_v4(),
    );
    assert!(s.world.spawn_entity(target));
    let spit = Arc::new(LlamaSpitEntity::new_shot(
        Entity::new(s.world.clone(), Vector3::default(), &EntityType::LLAMA_SPIT),
        s.owner.player.get_entity(),
    ));
    spit.get_entity().set_pos(Vector3::new(10.5, 65.5, 8.5));
    spit.get_entity()
        .velocity
        .store(Vector3::new(4.0, 0.0, 0.0));
    assert!(s.world.spawn_entity(spit.clone()));
    spit.tick(spit.as_ref(), &s.server);
    assert!(!spit.get_entity().is_removed());
    let sink = s.sink.lock().unwrap();
    let lands: Vec<_> = sink
        .forwarded
        .iter()
        .filter(|e| e.event == GameEvent::ProjectileLand)
        .collect();
    assert_eq!(lands.len(), 1);
    assert!(
        lands[0].affected_state.is_none(),
        "entity hit has no affected block state"
    );
    assert_eq!(
        lands[0].source.as_ref().unwrap().uuid,
        spit.get_entity().entity_uuid
    );
    drop(sink);
    let pos = BlockPos::new(8, 65, 11);
    s.world
        .set_block_state(&pos, Block::STONE.default_state.id, BlockFlags::FORCE_STATE);
    let retired = Arc::new(LlamaSpitEntity::new_shot(
        Entity::new(s.world.clone(), Vector3::default(), &EntityType::LLAMA_SPIT),
        s.owner.player.get_entity(),
    ));
    retired.get_entity().set_pos(Vector3::new(8.5, 65.5, 9.5));
    retired
        .get_entity()
        .velocity
        .store(Vector3::new(0.0, 0.0, 2.0));
    assert!(s.world.spawn_entity(retired.clone()));
    retired.tick(retired.as_ref(), &s.server);
    assert!(
        retired.get_entity().is_removed(),
        "normal spit block hit still discards"
    );
    let sink = s.sink.lock().unwrap();
    assert_eq!(
        sink.plugins
            .iter()
            .filter(|e| e.key == "projectile_land")
            .count(),
        2
    );
    assert_eq!(
        sink.forwarded
            .iter()
            .filter(|e| e.event == GameEvent::ProjectileLand)
            .count(),
        1,
        "removed-land adapter is deferred to hit admission, never source-less forwarding"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s2_projectile_plugin_removal_rejects_outer_land() {
    let s = Scenario::new(true);
    let removed = Arc::new(AtomicBool::new(false));
    let observe = removed.clone();
    let world = s.world.clone();
    s.capture(Some(Box::new(move |_, event| {
        if event.event_key == "projectile_land" {
            let arrow = world
                .entities
                .load()
                .iter()
                .find(|e| e.get_entity().entity_type == &EntityType::ARROW)
                .cloned()
                .unwrap();
            arrow.get_entity().remove();
            observe.store(true, Relaxed);
        }
    })));
    let arrow = paid_arrow(&s);
    s.world
        .set_block_state(&HIT, Block::STONE.default_state.id, BlockFlags::FORCE_STATE);
    arrow.get_entity().set_pos(Vector3::new(8.5, 65.5, 9.5));
    arrow
        .get_entity()
        .velocity
        .store(Vector3::new(0.0, 0.0, 2.0));
    arrow.tick(arrow.as_ref(), &s.server);
    assert!(removed.load(Relaxed));
    assert!(arrow.get_entity().is_removed());
    assert_eq!(s.owner.player.inventory().get_stack(1).item_count, 2);
    let sink = s.sink.lock().unwrap();
    assert_eq!(
        sink.plugins
            .iter()
            .filter(|e| e.key == "projectile_land")
            .count(),
        1
    );
    assert!(
        !sink
            .forwarded
            .iter()
            .any(|e| e.event == GameEvent::ProjectileLand)
    );
}
