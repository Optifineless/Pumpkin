#[path = "b1020d_g10_transform_test_support.rs"]
mod support;
use crate::{
    block::entities::BlockEntity,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::player::player_interact_event::PlayerInteractEvent,
    },
    server::Server,
};
use pumpkin_data::{
    Block,
    block_properties::{ChestLikeProperties, ChestType, HorizontalFacing},
    game_event::GameEvent,
    item::Item,
    sound::Sound,
};
use pumpkin_world::world::BlockFlags;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use support::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g10_mixed_copper_transform_and_bedrock_effects() {
    let mut fixture = Fixture::new().await;
    for bedrock_actor in [false, true] {
        for clicked in [LEFT, RIGHT] {
            for (source, partner, target, java_particle, bedrock_particle, sound, bedrock_sound) in [
                (
                    &Block::WEATHERED_COPPER_CHEST,
                    &Block::OXIDIZED_COPPER_CHEST,
                    &Block::EXPOSED_COPPER_CHEST,
                    3005,
                    2032,
                    Sound::ItemAxeScrape,
                    "scrape",
                ),
                (
                    &Block::WAXED_EXPOSED_COPPER_CHEST,
                    &Block::WAXED_OXIDIZED_COPPER_CHEST,
                    &Block::EXPOSED_COPPER_CHEST,
                    3004,
                    2031,
                    Sound::ItemAxeWaxOff,
                    "copper.wax.off",
                ),
            ] {
                if clicked == LEFT {
                    fixture.pair(source, partner);
                } else {
                    fixture.pair(partner, source);
                }
                fixture.reload_pair().await;
                let entities: Vec<Arc<dyn BlockEntity>> = [LEFT, RIGHT]
                    .iter()
                    .map(|pos| fixture.world.get_block_entity(pos).unwrap())
                    .collect();
                fixture.clear();
                if bedrock_actor {
                    fixture.use_bedrock(clicked);
                } else {
                    fixture.use_java(clicked);
                }
                for (index, position) in [LEFT, RIGHT].into_iter().enumerate() {
                    assert_eq!(
                        fixture.world.get_block(&position).name,
                        target.name,
                        "mixed pair: Bedrock actor={bedrock_actor}, clicked={clicked:?}, source={}, checking={position:?}",
                        source.name
                    );
                    let entity = fixture.world.get_block_entity(&position).unwrap();
                    assert!(
                        Arc::ptr_eq(&entities[index], &entity),
                        "stage changes retain container identity"
                    );
                    let stack = entity.get_inventory().unwrap().get_stack(0);
                    assert_eq!(stack.item, &Item::DIAMOND);
                    assert_eq!(stack.item_count, [7, 19][index]);
                    let props = ChestLikeProperties::from_state_id(
                        fixture.world.get_block_state_id(&position),
                    );
                    assert_eq!(props.r#type, [ChestType::Left, ChestType::Right][index]);
                    assert_eq!(props.facing, HorizontalFacing::North);
                }
                verify_effects(
                    &mut fixture,
                    clicked,
                    bedrock_actor,
                    java_particle,
                    bedrock_particle,
                    sound,
                    bedrock_sound,
                );
            }
        }
    }
    fixture.close().await;
}

fn verify_effects(
    fixture: &mut Fixture,
    clicked: pumpkin_util::math::position::BlockPos,
    bedrock_actor: bool,
    java_particle: i32,
    bedrock_particle: i32,
    sound: Sound,
    bedrock_sound: &str,
) {
    if bedrock_actor {
        assert_eq!(
            fixture
                .bedrock
                .player
                .inventory()
                .off_hand_item()
                .get_damage(),
            1
        );
    } else {
        assert_eq!(fixture.java.player.inventory().held_item().get_damage(), 1);
    }
    let other = if clicked == LEFT { RIGHT } else { LEFT };
    let positions = [clicked, other];
    let java_actor_packets = fixture.java.take_packets();
    let java_observer_packets = fixture.java_observer.take_packets();
    java_effects(
        &java_observer_packets,
        java_particle,
        sound as u16,
        &positions,
    );
    if bedrock_actor {
        java_effects(&java_actor_packets, java_particle, sound as u16, &positions);
    } else {
        assert!(
            java_particles(&java_actor_packets).is_empty(),
            "Java actor predicts particles"
        );
        assert!(
            crate::net::java::sound_test_support::decode_sounds(&java_actor_packets).is_empty(),
            "Java actor predicts sound"
        );
    }
    bedrock_effects(
        &fixture.bedrock.take_packets(),
        bedrock_particle,
        bedrock_sound,
        &positions,
        clicked,
    );
    bedrock_effects(
        &fixture.bedrock_observer.take_packets(),
        bedrock_particle,
        bedrock_sound,
        &positions,
        clicked,
    );
    let changes = fixture
        .events
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|(name, _)| name == GameEvent::BlockChange.name())
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        changes,
        positions
            .iter()
            .map(|pos| (
                GameEvent::BlockChange.name().to_owned(),
                pos.to_centered_f64()
            ))
            .collect::<Vec<_>>()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g10_mixed_transform_conditional_writes_and_invalid_partner_controls() {
    let mut fixture = Fixture::new().await;
    fixture.pair(
        &Block::WEATHERED_COPPER_CHEST,
        &Block::WEATHERED_COPPER_CHEST,
    );
    let called = Arc::new(AtomicBool::new(false));
    let observed = called.clone();
    let world = Arc::downgrade(&fixture.world);
    *fixture
        .world
        .persistence_test_hooks
        .transform_write
        .lock()
        .unwrap() = Some(Arc::new(move |position| {
        if position == RIGHT && !observed.swap(true, Ordering::Relaxed) {
            world.upgrade().unwrap().set_block_state(
                &RIGHT,
                Block::DIAMOND_BLOCK.default_state.id,
                BlockFlags::FORCE_STATE,
            );
        }
    }));
    fixture.clear();
    fixture.use_java(LEFT);
    assert!(called.load(Ordering::Relaxed));
    assert_eq!(fixture.world.get_block(&RIGHT), &Block::DIAMOND_BLOCK);
    assert_eq!(fixture.world.get_block(&LEFT), &Block::EXPOSED_COPPER_CHEST);
    assert_eq!(
        java_particles(&fixture.java_observer.take_packets()).len(),
        1,
        "no companion particle for a replaced source"
    );
    *fixture
        .world
        .persistence_test_hooks
        .transform_write
        .lock()
        .unwrap() = None;

    for (block, kind, facing) in [
        (&Block::CHEST, ChestType::Right, HorizontalFacing::North),
        (
            &Block::OXIDIZED_COPPER_CHEST,
            ChestType::Single,
            HorizontalFacing::North,
        ),
        (
            &Block::OXIDIZED_COPPER_CHEST,
            ChestType::Right,
            HorizontalFacing::South,
        ),
        (
            &Block::OXIDIZED_COPPER_CHEST,
            ChestType::Left,
            HorizontalFacing::North,
        ),
    ] {
        fixture.pair(
            &Block::WEATHERED_COPPER_CHEST,
            &Block::OXIDIZED_COPPER_CHEST,
        );
        let mut props = ChestLikeProperties::default(block);
        props.r#type = kind;
        props.facing = facing;
        let invalid = props.to_state_id(block);
        fixture
            .world
            .set_block_state(&RIGHT, invalid, BlockFlags::FORCE_STATE);
        fixture.clear();
        fixture.use_java(LEFT);
        assert_eq!(
            fixture.world.get_block_state_id(&RIGHT),
            invalid,
            "unrelated or nonreciprocal inventory remains untouched"
        );
        assert_eq!(
            java_particles(&fixture.java_observer.take_packets()).len(),
            1
        );
    }
    fixture.close().await;
}

struct CancelUse;
impl EventHandler<PlayerInteractEvent> for CancelUse {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerInteractEvent,
    ) -> BoxFuture<'a, ()> {
        event.cancelled = true;
        Box::pin(async {})
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g10_cancelled_transform_keeps_mixed_pair_and_effects_silent() {
    let mut fixture = Fixture::new().await;
    fixture.pair(
        &Block::WEATHERED_COPPER_CHEST,
        &Block::OXIDIZED_COPPER_CHEST,
    );
    fixture
        .server
        .plugin_manager
        .register::<PlayerInteractEvent, _>(Arc::new(CancelUse), EventPriority::Normal, true);
    fixture.clear();
    fixture.use_java(LEFT);
    assert_eq!(
        fixture.world.get_block(&LEFT),
        &Block::WEATHERED_COPPER_CHEST
    );
    assert_eq!(
        fixture.world.get_block(&RIGHT),
        &Block::OXIDIZED_COPPER_CHEST
    );
    assert_eq!(fixture.java.player.inventory().held_item().get_damage(), 0);
    assert!(java_particles(&fixture.java_observer.take_packets()).is_empty());
    assert!(fixture.bedrock.take_packets().is_empty());
    assert!(fixture.events.0.lock().unwrap().is_empty());
    fixture.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g10_clicked_transform_replacement_preserves_new_source() {
    let mut fixture = Fixture::new().await;
    fixture.pair(
        &Block::WEATHERED_COPPER_CHEST,
        &Block::OXIDIZED_COPPER_CHEST,
    );
    let called = Arc::new(AtomicBool::new(false));
    let observed = called.clone();
    let world = Arc::downgrade(&fixture.world);
    *fixture
        .world
        .persistence_test_hooks
        .transform_write
        .lock()
        .unwrap() = Some(Arc::new(move |position| {
        if position == LEFT && !observed.swap(true, Ordering::Relaxed) {
            world.upgrade().unwrap().set_block_state(
                &LEFT,
                Block::DIAMOND_BLOCK.default_state.id,
                BlockFlags::FORCE_STATE,
            );
        }
    }));
    fixture.clear();
    fixture.use_java(LEFT);
    assert!(called.load(Ordering::Relaxed));
    assert_eq!(fixture.world.get_block(&LEFT), &Block::DIAMOND_BLOCK);
    assert_eq!(fixture.java.player.inventory().held_item().get_damage(), 0);
    assert_eq!(
        fixture
            .world
            .get_block_entity(&RIGHT)
            .unwrap()
            .get_inventory()
            .unwrap()
            .get_stack(0)
            .item_count,
        19,
        "companion-first ordering retains the companion contents when the clicked write is rejected"
    );
    let packets = fixture.java_observer.take_packets();
    assert!(java_particles(&packets).is_empty());
    assert!(crate::net::java::sound_test_support::decode_sounds(&packets).is_empty());
    assert!(fixture.events.0.lock().unwrap().is_empty());
    fixture.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g10_java_transform_prediction_control() {
    let mut fixture = Fixture::new().await;
    fixture.world.set_block_state(
        &LEFT,
        Block::WEATHERED_COPPER.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    fixture.clear();
    fixture.use_java(LEFT);
    assert_eq!(fixture.world.get_block(&LEFT), &Block::EXPOSED_COPPER);
    assert_eq!(fixture.java.player.inventory().held_item().get_damage(), 1);
    let actor = fixture.java.take_packets();
    assert!(
        java_particles(&actor).is_empty(),
        "Java actor predicts particles"
    );
    assert!(
        crate::net::java::sound_test_support::decode_sounds(&actor).is_empty(),
        "Java actor predicts sound"
    );
    java_effects(
        &fixture.java_observer.take_packets(),
        3005,
        Sound::ItemAxeScrape as u16,
        &[LEFT],
    );
    fixture.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g10_bedrock_main_hand_transform_effects() {
    let mut fixture = Fixture::new().await;
    for (source, target, particle, sound) in [
        (
            &Block::WEATHERED_COPPER,
            &Block::EXPOSED_COPPER,
            2032,
            "scrape",
        ),
        (
            &Block::WAXED_EXPOSED_COPPER,
            &Block::EXPOSED_COPPER,
            2031,
            "copper.wax.off",
        ),
    ] {
        fixture
            .world
            .set_block_state(&LEFT, source.default_state.id, BlockFlags::FORCE_STATE);
        fixture.clear();
        fixture.use_bedrock_with_hand(LEFT, pumpkin_util::Hand::Right);
        assert_eq!(
            fixture.world.get_block(&LEFT).name,
            target.name,
            "ordinary Bedrock main-hand source {}",
            source.name
        );
        assert_eq!(
            fixture.bedrock.player.inventory().held_item().get_damage(),
            1
        );
        bedrock_effects(
            &fixture.bedrock.take_packets(),
            particle,
            sound,
            &[LEFT],
            LEFT,
        );
        bedrock_effects(
            &fixture.bedrock_observer.take_packets(),
            particle,
            sound,
            &[LEFT],
            LEFT,
        );
    }
    fixture.close().await;
}
