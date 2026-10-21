use super::{adapter_tests::use_on, test_support::Scenario};
use crate::{
    block::{
        BlockBehaviour, OnScheduledTickArgs, blocks::dried_ghast::DriedGhastBlock,
        entities::BlockEntity,
    },
    entity::{
        Entity, EntityBase,
        mob::{Mob, ravager::RavagerEntity},
    },
    world::explosion::{BlockInteraction, Explosion},
};
use pumpkin_data::{
    Block,
    block_properties::{AttachFace, DriedGhastLikeProperties, LeverLikeProperties},
    entity::EntityType,
    game_event::GameEvent,
    item::Item,
    item_stack::ItemStack,
};
use pumpkin_util::{
    Hand,
    math::{position::BlockPos, vector3::Vector3},
};
use pumpkin_world::world::BlockFlags;
use std::sync::atomic::Ordering::Relaxed;

const TARGET: BlockPos = BlockPos::new(8, 65, 8);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s2_actorless_lever_explosion_and_hydration() {
    let s = Scenario::new(true);
    s.capture(None);
    let mut props = LeverLikeProperties::default(&Block::LEVER);
    props.face = AttachFace::Floor;
    s.world.set_block_state(
        &TARGET,
        props.to_state_id(&Block::LEVER),
        BlockFlags::FORCE_STATE,
    );
    use_on(&s, TARGET);
    assert!(LeverLikeProperties::from_state_id(s.world.get_block_state_id(&TARGET)).powered);
    // Both exact 26.3 LeverBlock entry points call pull(..., null).
    Explosion::new(
        1.0,
        TARGET.to_centered_f64(),
        BlockInteraction::TriggerBlock,
    )
    .explode(&s.world);
    assert!(!LeverLikeProperties::from_state_id(s.world.get_block_state_id(&TARGET)).powered);
    let dry_before = scheduled_hydration(&s, false, 2, 1);
    let wet_before = scheduled_hydration(&s, true, 1, 2);
    let sink = s.sink.lock().unwrap();
    let events: Vec<_> = sink
        .forwarded
        .iter()
        .filter(|e| {
            matches!(
                e.event,
                GameEvent::BlockActivate | GameEvent::BlockDeactivate | GameEvent::BlockChange
            )
        })
        .collect();
    assert_eq!(events.len(), 4);
    assert_eq!(
        events.iter().map(|e| e.event).collect::<Vec<_>>(),
        [
            GameEvent::BlockActivate,
            GameEvent::BlockDeactivate,
            GameEvent::BlockChange,
            GameEvent::BlockChange
        ]
    );
    assert!(events.iter().all(|e| e.source.is_none()));
    assert!(events[..2].iter().all(|e| e.affected_state.is_none()));
    assert_eq!(events[2].affected_state, Some(dry_before));
    assert_eq!(events[3].affected_state, Some(wet_before));
    assert_eq!(events[0].position, TARGET.to_centered_f64());
    assert_eq!(events[2].position, TARGET.to_f64());
}

fn scheduled_hydration(
    s: &Scenario,
    waterlogged: bool,
    before: u8,
    after: u8,
) -> pumpkin_data::BlockStateId {
    let mut props = DriedGhastLikeProperties::default(&Block::DRIED_GHAST);
    props.waterlogged = waterlogged;
    props.hydration = before;
    let state = props.to_state_id(&Block::DRIED_GHAST);
    s.world
        .set_block_state(&TARGET, state, BlockFlags::FORCE_STATE);
    DriedGhastBlock.on_scheduled_tick(OnScheduledTickArgs {
        world: &s.world,
        block: &Block::DRIED_GHAST,
        position: &TARGET,
    });
    assert_eq!(
        DriedGhastLikeProperties::from_state_id(s.world.get_block_state_id(&TARGET)).hydration,
        after
    );
    state
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s2_actorless_container_and_no_invented_place_break() {
    let s = Scenario::new(true);
    s.capture(None);
    s.world.set_block_state(
        &TARGET,
        Block::BARREL.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let barrel = std::sync::Arc::new(crate::block::entities::barrel::BarrelBlockEntity::new(
        TARGET,
    ));
    s.world.add_block_entity(barrel.clone());
    use_on(&s, TARGET);
    barrel.tick(&s.world);
    s.owner.player.close_handled_screen();
    barrel.tick(&s.world);
    let sink = s.sink.lock().unwrap();
    assert_eq!(sink.plugins.len(), 2);
    assert_eq!(
        sink.forwarded.iter().map(|e| e.event).collect::<Vec<_>>(),
        [GameEvent::ContainerOpen, GameEvent::ContainerClose]
    );
    assert!(
        sink.forwarded
            .iter()
            .all(|e| e.source.is_none() && e.affected_state.is_none())
    );
    drop(sink);
    s.world.set_block_state(
        &TARGET,
        Block::AIR.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    s.owner
        .player
        .get_entity()
        .set_pos(Vector3::new(8.5, 65.0, 10.5));
    s.owner
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::new(2, &Item::DIRT));
    use_on(&s, TARGET.down());
    assert_eq!(s.world.get_block(&TARGET), &Block::DIRT);
    assert_eq!(s.owner.player.inventory().held_item().item_count, 1);
    s.world
        .break_block(&TARGET, Some(&s.owner.player), BlockFlags::FORCE_STATE);
    assert_eq!(s.world.get_block(&TARGET), &Block::AIR);
    assert_eq!(
        s.sink.lock().unwrap().plugins.len(),
        2,
        "ordinary place/break have no existing game-event notification"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s2_ravager_action_source() {
    let s = Scenario::new(true);
    s.capture(None);
    let ravager = RavagerEntity::new(Entity::new(
        s.world.clone(),
        Vector3::new(14.5, 65.0, 14.5),
        &EntityType::RAVAGER,
    ));
    assert!(s.world.spawn_entity(ravager.clone()));
    ravager.blocking.roar.store(11, Relaxed);
    ravager.mob_tick(ravager.as_ref());
    assert_eq!(ravager.blocking.roar.load(Relaxed), 10);
    let sink = s.sink.lock().unwrap();
    assert_eq!(sink.plugins.len(), 1);
    assert_eq!(sink.forwarded.len(), 1);
    assert_eq!(sink.forwarded[0].event, GameEvent::EntityAction);
    assert_eq!(
        sink.forwarded[0].source.as_ref().map(|s| s.uuid),
        Some(ravager.get_entity().entity_uuid)
    );
    assert!(sink.forwarded[0].affected_state.is_none());
}
