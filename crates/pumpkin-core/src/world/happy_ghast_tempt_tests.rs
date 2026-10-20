use super::happy_ghast_test_support::*;
use crate::{
    entity::{EntityBase, player::Player},
    net::java::combat_test_support::TestPlayer,
    plugin::{EventHandler, EventPriority, api::events::world::world_save::WorldSaveEvent},
    server::Server,
};
use futures::future::BoxFuture;
use pumpkin_data::{data_component_impl::EquipmentSlot, item::Item, item_stack::ItemStack};
use pumpkin_util::{
    GameMode, Hand,
    math::{vector2::Vector2, vector3::Vector3},
};
use std::{sync::Arc, time::Duration};

fn prepare_temptation(
    fixture: &Fixture,
    entity: &Arc<dyn EntityBase>,
    item: &'static Item,
    hand: Hand,
) {
    fixture
        .player
        .player
        .get_entity()
        .set_pos(Vector3::new(16.5, 80.0, 12.5));
    fixture
        .player
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::EMPTY.clone());
    fixture
        .player
        .player
        .inventory()
        .set_stack_in_hand(Hand::Left, ItemStack::EMPTY.clone());
    fixture
        .player
        .player
        .inventory()
        .set_stack_in_hand(hand, ItemStack::new(7, item));
    draws(entity, &[0.5; 200]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_happy_ghast_s1_non_pathfinder_temptation() {
    let fixture = Fixture::new(true).await;
    let entity = fixture.summon("8.5 74 8.5", "");
    prepare_temptation(&fixture, &entity, &Item::WHITE_HARNESS, Hand::Left);
    let before = fixture.player.player.inventory().off_hand_item();
    fixture.tick(1);
    let eye = fixture.player.player.get_entity().get_eye_pos();
    let expected = Vector3::new(8.5, 74.0, 8.5) + (eye - Vector3::new(8.5, 74.0, 8.5)) * 0.5;
    assert_eq!(
        target(&entity),
        expected,
        "temptation must steer directly toward the eye in 3D"
    );
    assert_eq!(
        entity.get_entity().yaw.load().to_bits(),
        0xc27d_bd63,
        "requested look did not use vanilla float evaluation"
    );
    fixture.tick(8);
    assert_flying(&entity, Vector3::new(8.5, 74.0, 8.5));
    assert!(
        fixture
            .player
            .player
            .inventory()
            .off_hand_item()
            .are_equal(&before)
    );
    ghast(&entity).mob_entity.set_item_slot(
        &EquipmentSlot::BODY,
        ItemStack::new(1, &Item::WHITE_HARNESS),
    );
    fixture.tick(1);
    assert!(
        !ghast(&entity).temptation_running(),
        "equipped adult still followed harness"
    );
    prepare_temptation(&fixture, &entity, &Item::SNOWBALL, Hand::Right);
    let stock = fixture.player.player.inventory().held_item();
    for _ in 0..49 {
        fixture.tick(1);
        assert!(
            !ghast(&entity).temptation_running(),
            "temptation ignored cooldown"
        );
    }
    fixture.tick(1);
    assert!(
        ghast(&entity).temptation_running(),
        "temptation did not resume after cooldown"
    );
    assert!(target(&entity).y > entity.get_entity().pos.load().y);
    assert!(
        fixture
            .player
            .player
            .inventory()
            .held_item()
            .are_equal(&stock)
    );
    strict_stop_boundary(&fixture);
    drop(fixture.close().await);
}

fn strict_stop_boundary(fixture: &Fixture) {
    // Exact representable coordinates avoid rounding an arbitrary live X+7 below seven.
    let entity = fixture.summon("-8.5 74 -8.5", "");
    draws(&entity, &[0.5; 200]);
    fixture
        .player
        .player
        .get_entity()
        .set_pos(Vector3::new(-1.5, 74.0, -8.5));
    assert_eq!(
        entity
            .get_entity()
            .pos
            .load()
            .squared_distance_to_vec(&fixture.player.player.get_entity().pos.load()),
        49.0
    );
    fixture.tick(1);
    assert!(
        ghast(&entity)
            .mob_entity
            .move_control
            .lock()
            .unwrap()
            .has_wanted(),
        "exact stop-distance boundary should still steer"
    );
    fixture
        .player
        .player
        .get_entity()
        .set_pos(entity.get_entity().pos.load() + Vector3::new(6.99, 0.0, 0.0));
    fixture.tick(2);
    assert!(
        !ghast(&entity)
            .mob_entity
            .move_control
            .lock()
            .unwrap()
            .has_wanted()
    );
    fixture
        .player
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::new(7, &Item::DIRT));
    fixture.tick(1);
    assert!(!ghast(&entity).temptation_running());
    assert_eq!(fixture.player.player.inventory().held_item().item_count, 7);
}

struct ChangeHand(Arc<Player>);
impl EventHandler<WorldSaveEvent> for ChangeHand {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        _: &'a mut WorldSaveEvent,
    ) -> BoxFuture<'a, ()> {
        self.0
            .inventory()
            .set_stack_in_hand(Hand::Right, ItemStack::new(3, &Item::DIRT));
        Box::pin(async {})
    }
}

fn tempting_adult(fixture: &Fixture) -> Arc<dyn EntityBase> {
    let entity = fixture.summon("8.5 74 8.5", "");
    prepare_temptation(fixture, &entity, &Item::SNOWBALL, Hand::Right);
    fixture.tick(1);
    assert!(ghast(&entity).temptation_running());
    entity
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_happy_ghast_s1_temptation_callback_and_session_controls() {
    let fixture = Fixture::new(true).await;
    let entity = tempting_adult(&fixture);
    fixture.server.plugin_manager.register::<WorldSaveEvent, _>(
        Arc::new(ChangeHand(fixture.player.player.clone())),
        EventPriority::Normal,
        true,
    );
    tokio::time::timeout(Duration::from_secs(10), fixture.world.save())
        .await
        .unwrap();
    fixture.tick(1);
    assert!(
        !ghast(&entity).temptation_running(),
        "callback hand replacement remained eligible"
    );
    assert_eq!(
        fixture.player.player.inventory().held_item().item,
        &Item::DIRT
    );
    assert_eq!(fixture.player.player.inventory().held_item().item_count, 3);
    entity.get_entity().remove();
    let entity = tempting_adult(&fixture);
    let stock = fixture.player.player.inventory().held_item();
    fixture.player.player.set_gamemode(GameMode::Spectator);
    fixture.tick(1);
    assert!(
        !ghast(&entity).temptation_running(),
        "spectator with valid food remained eligible"
    );
    assert!(
        fixture
            .player
            .player
            .inventory()
            .held_item()
            .are_equal(&stock)
    );
    fixture.player.player.set_gamemode(GameMode::Survival);
    entity.get_entity().remove();
    let entity = tempting_adult(&fixture);
    let replacement =
        TestPlayer::with_profile(&fixture.world, fixture.player.player.gameprofile.clone());
    replacement
        .player
        .get_entity()
        .set_pos(fixture.player.player.get_entity().pos.load());
    fixture.tick(1);
    assert!(
        !ghast(&entity).temptation_running(),
        "replaced session stayed eligible through retained Arc"
    );
    assert!(
        fixture
            .player
            .player
            .inventory()
            .held_item()
            .are_equal(&stock)
    );
    replacement.client().close();
    drop(fixture.close().await);
    removed_and_closed_controls().await;
    other_world_control().await;
}

async fn removed_and_closed_controls() {
    for removed in [false, true] {
        let fixture = Fixture::new(true).await;
        let entity = tempting_adult(&fixture);
        let stock = fixture.player.player.inventory().held_item();
        if removed {
            tokio::time::timeout(
                Duration::from_secs(10),
                fixture.world.remove_player(&fixture.player.player, false),
            )
            .await
            .unwrap();
        } else {
            fixture.player.client().close();
        }
        // A live observer keeps ordinary chunk ticking active after the target leaves.
        let observer = TestPlayer::new(&fixture.world);
        if !removed {
            fixture.world.players.store(Arc::new(vec![
                fixture.player.player.clone(),
                observer.player.clone(),
            ]));
        }
        fixture.tick(1);
        assert!(
            !ghast(&entity).temptation_running(),
            "departed valid-food player stayed eligible"
        );
        assert!(
            fixture
                .player
                .player
                .inventory()
                .held_item()
                .are_equal(&stock)
        );
        observer.client().close();
        drop(fixture.close().await);
    }
}

async fn other_world_control() {
    let mut fixture = Fixture::new(true).await;
    let entity = tempting_adult(&fixture);
    let stock = fixture.player.player.inventory().held_item();
    let directory = tempfile::tempdir().unwrap();
    let destination = crate::server::combat_test_support::world(&fixture.server, directory.path());
    for x in -2..=2 {
        for z in -2..=2 {
            crate::server::combat_test_support::publish_empty_chunk(
                &destination,
                Vector2::new(x, z),
            );
        }
    }
    fixture
        .server
        .worlds
        .store(Arc::new(vec![fixture.world.clone(), destination.clone()]));
    let player = fixture.player.player.clone();
    tokio::time::timeout(
        Duration::from_secs(15),
        fixture.player.with_outgoing_writer(player.teleport_world(
            destination.clone(),
            Vector3::new(8.5, 80.0, 8.5),
            None,
            None,
        )),
    )
    .await
    .unwrap();
    assert!(Arc::ptr_eq(&player.world(), &destination));
    let observer = TestPlayer::new(&fixture.world);
    tokio::time::timeout(
        Duration::from_secs(10),
        fixture
            .world
            .level
            .mark_chunks_as_newly_watched(&[entity.get_entity().chunk_pos.load()]),
    )
    .await
    .unwrap();
    fixture.tick(1);
    assert!(
        !entity.get_entity().is_removed(),
        "source entity retired during eligibility control"
    );
    assert!(
        !ghast(&entity).temptation_running(),
        "other-world valid-food player remained eligible"
    );
    assert!(player.inventory().held_item().are_equal(&stock));
    observer.client().close();
    drop(fixture.close().await);
}
