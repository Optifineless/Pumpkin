use super::test_support::{Forwarded, Scenario};
use crate::entity::EntityBase;
use pumpkin_data::{Block, game_event::GameEvent, item::Item, item_stack::ItemStack};
use pumpkin_protocol::{
    VarInt,
    java::server::play::{SInteract, SUseItemOn},
};
use pumpkin_util::{
    Hand,
    math::{position::BlockPos, vector3::Vector3},
};
use pumpkin_world::world::BlockFlags;

const TARGET: BlockPos = BlockPos::new(8, 65, 8);
const ACTOR: Vector3<f64> = Vector3::new(8.5, 65.0, 10.5);

fn assert_actor(s: &Scenario, event: &Forwarded, position: Vector3<f64>) {
    assert_eq!(
        event.source.as_ref().map(|source| source.uuid),
        Some(s.owner.player.get_entity().entity_uuid),
        "literal player source"
    );
    s.assert_source(event.source.as_ref().unwrap(), position);
}

pub(super) fn use_on(s: &Scenario, position: BlockPos) {
    s.owner
        .client()
        .handle_use_item_on(
            &s.owner.player,
            &SUseItemOn {
                hand: VarInt(0),
                position,
                face: VarInt(1),
                cursor_pos: Vector3::new(0.5, 0.5, 0.5),
                inside_block: false,
                is_against_world_border: false,
                sequence: VarInt(3),
            },
            &s.server,
        )
        .unwrap();
}

pub(super) fn flower(s: &Scenario) {
    s.owner.player.get_entity().set_pos(ACTOR);
    s.world.set_block_state(
        &TARGET,
        Block::FLOWER_POT.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    s.owner
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::new(1, &Item::DANDELION));
    use_on(s, TARGET);
    assert_eq!(s.world.get_block(&TARGET), &Block::POTTED_DANDELION);
    assert!(s.owner.player.inventory().held_item().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s2_real_block_and_item_context() {
    for enabled in [true, false] {
        let s = Scenario::new(enabled);
        s.capture(None);
        flower(&s);
        use_on(&s, TARGET);
        assert_eq!(s.world.get_block(&TARGET), &Block::FLOWER_POT);
        assert_eq!(
            s.owner.player.inventory().held_item().item,
            &Item::DANDELION
        );
        let sink = s.sink.lock().unwrap();
        assert_eq!(sink.plugins.len(), 2);
        assert!(
            sink.plugins
                .iter()
                .all(|p| p.key == "block_change" && p.position == TARGET.to_centered_f64())
        );
        assert_eq!(sink.forwarded.len(), if enabled { 2 } else { 0 });
        for event in &sink.forwarded {
            assert_eq!(event.event, GameEvent::BlockChange);
            assert_eq!(event.position, TARGET.to_centered_f64());
            assert!(
                event.affected_state.is_none(),
                "FlowerPotBlock uses actor-only context"
            );
            assert_actor(&s, event, ACTOR);
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s2_transform_state_is_emission_snapshot() {
    let s = Scenario::new(true);
    s.owner.player.get_entity().set_pos(ACTOR);
    s.world.set_block_state(
        &TARGET,
        Block::OAK_LOG.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    s.owner
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::new(1, &Item::IRON_AXE));
    let world = s.world.clone();
    s.capture(Some(Box::new(move |_, event| {
        assert_eq!(event.event_key, "block_change");
        world.set_block_state(
            &TARGET,
            Block::GOLD_BLOCK.default_state.id,
            BlockFlags::FORCE_STATE,
        );
    })));
    use_on(&s, TARGET);
    assert_eq!(s.world.get_block(&TARGET), &Block::GOLD_BLOCK);
    assert_eq!(s.owner.player.inventory().held_item().get_damage(), 1);
    let sink = s.sink.lock().unwrap();
    assert_eq!(sink.plugins.len(), 1);
    assert_eq!(sink.forwarded.len(), 1);
    assert_eq!(
        sink.forwarded[0].affected_state,
        Some(Block::STRIPPED_OAK_LOG.default_state.id),
        "transform context retains committed updatedShape"
    );
    assert_actor(&s, &sink.forwarded[0], ACTOR);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s2_bucket_bottle_context_and_missing_pickup_control() {
    let s = Scenario::new(true);
    let actor = Vector3::new(8.5, 64.0, 8.5);
    let clicked = BlockPos::new(8, 65, 11);
    let placed = BlockPos::new(8, 65, 10);
    s.owner.player.get_entity().set_pos(actor);
    s.world.set_block_state(
        &clicked,
        Block::STONE.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    s.capture(None);
    s.owner
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::new(1, &Item::WATER_BUCKET));
    s.use_item();
    assert_eq!(s.world.get_block(&placed), &Block::WATER);
    assert_eq!(s.owner.player.inventory().held_item().item, &Item::BUCKET);
    let sink = s.sink.lock().unwrap();
    assert_eq!(sink.plugins.len(), 1);
    assert_eq!(sink.forwarded[0].event, GameEvent::FluidPlace);
    assert_eq!(sink.forwarded[0].position, placed.to_f64());
    assert!(sink.forwarded[0].affected_state.is_none());
    assert_actor(&s, &sink.forwarded[0], actor);
    drop(sink);
    s.use_item();
    assert_eq!(s.world.get_block(&placed), &Block::AIR);
    assert_eq!(
        s.owner.player.inventory().held_item().item,
        &Item::WATER_BUCKET
    );
    assert_eq!(
        s.sink.lock().unwrap().plugins.len(),
        1,
        "ordinary bucket pickup has no pre-existing game-event emitter"
    );
    s.world.set_block_state(
        &clicked,
        Block::WATER.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    s.owner
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::new(1, &Item::GLASS_BOTTLE));
    s.use_item();
    assert_eq!(s.owner.player.inventory().held_item().item, &Item::POTION);
    let sink = s.sink.lock().unwrap();
    assert_eq!(sink.plugins.len(), 2);
    assert_eq!(
        sink.plugins[1].count, 1,
        "bottle notification precedes item conversion"
    );
    assert_eq!(sink.forwarded[1].event, GameEvent::FluidPickup);
    assert_eq!(sink.forwarded[1].position, clicked.to_centered_f64());
    assert!(sink.forwarded[1].affected_state.is_none());
    assert_actor(&s, &sink.forwarded[1], actor);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s2_living_damage_consume_and_shear_sources() {
    use pumpkin_data::damage::DamageType;
    let s = Scenario::new(true);
    s.capture(None);
    assert!(
        s.owner
            .player
            .damage(s.owner.player.as_ref(), 4.0, DamageType::GENERIC)
    );
    assert_eq!(s.owner.player.living_entity.health.load(), 16.0);
    let sink = s.sink.lock().unwrap();
    assert_eq!(
        sink.plugins[0].health, 16.0,
        "damage notification follows committed health"
    );
    assert_actor(&s, &sink.forwarded[0], s.owner.player.position());
    drop(sink);
    consume(&s);
    shear(&s);
}

fn consume(s: &Scenario) {
    use pumpkin_data::data_component_impl::{ConsumableImpl, ConsumeAnimation, UseEffectsImpl};
    for animation in [ConsumeAnimation::Eat, ConsumeAnimation::Drink] {
        let mut stack = ItemStack::new(1, &Item::POTION);
        let mut consumable = stack
            .get_data_component::<ConsumableImpl>()
            .unwrap()
            .clone();
        consumable.animation = animation;
        stack.set_data_component(consumable);
        stack.set_data_component(UseEffectsImpl {
            interact_vibrations: false,
            ..UseEffectsImpl::DEFAULT
        });
        s.owner
            .player
            .inventory()
            .set_stack_in_hand(Hand::Right, stack);
        s.use_item();
        s.owner
            .player
            .living_entity
            .complete_using_item(s.owner.player.as_ref());
        assert_eq!(
            s.owner.player.inventory().held_item().item,
            &Item::GLASS_BOTTLE
        );
        let sink = s.sink.lock().unwrap();
        let event = sink.forwarded.last().unwrap();
        assert_eq!(
            event.event,
            if animation == ConsumeAnimation::Eat {
                GameEvent::Eat
            } else {
                GameEvent::Drink
            }
        );
        assert!(event.affected_state.is_none());
        assert_actor(s, event, s.owner.player.position());
    }
}

fn shear(s: &Scenario) {
    use pumpkin_data::entity::EntityType;
    let sheep = crate::entity::r#type::from_type(
        &EntityType::SHEEP,
        Vector3::new(8.5, 65.2, 10.5),
        &s.world,
        uuid::Uuid::new_v4(),
    );
    assert!(s.world.spawn_entity(sheep.clone()));
    s.owner
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::new(1, &Item::SHEARS));
    let before = s.sink.lock().unwrap().plugins.len();
    s.owner.client().handle_interact(
        &s.owner.player,
        &SInteract {
            entity_id: VarInt(sheep.get_entity().entity_id),
            r#type: VarInt(0),
            target_position: None,
            hand: Some(VarInt(0)),
            sneaking: false,
        },
        &s.server,
    );
    assert!(
        !sheep
            .get_mob()
            .unwrap()
            .as_shearable()
            .unwrap()
            .ready_for_shearing()
    );
    assert_eq!(s.owner.player.inventory().held_item().get_damage(), 1);
    let sink = s.sink.lock().unwrap();
    assert_eq!(sink.plugins.len(), before + 1);
    assert_eq!(
        sink.plugins.last().unwrap().damage,
        0,
        "shear event stays before tool durability"
    );
    let event = sink.forwarded.last().unwrap();
    assert_eq!(event.event, GameEvent::Shear);
    assert_eq!(event.position, sheep.get_entity().pos.load());
    assert_actor(s, event, s.owner.player.position());
}
