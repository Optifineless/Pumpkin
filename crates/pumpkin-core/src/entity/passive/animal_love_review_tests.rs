use super::review_test_support::Fixture;
use crate::entity::passive::cat::CatEntity;
use pumpkin_data::{
    entity::EntityType, item::Item, item_stack::ItemStack, packet::clientbound::play,
};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::codec::var_int::VarInt;
use std::sync::atomic::Ordering::Relaxed;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review116_positive_age_does_not_accept_love_food() {
    let fixture = Fixture::new();
    let inventory = fixture.player.player.inventory();
    for (kind, food) in [
        (&EntityType::CAT, &Item::COD),
        (&EntityType::COW, &Item::WHEAT),
    ] {
        let animal = fixture.spawn(kind, 20);
        inventory.set_stack(0, ItemStack::new(1, food));
        fixture.interact(animal.as_ref());
        assert_eq!(animal.get_entity().age.load(Relaxed), 20);
        assert!(
            !animal.is_in_love(),
            "{} ignored age cooldown",
            kind.resource_name
        );
        assert_eq!(inventory.held_item().item_count, 1);
        if let Some(cat) = animal.cast_any().downcast_ref::<CatEntity>() {
            assert!(cat.is_sitting(), "unhandled owner use still orders sitting");
        }
    }
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review116_legacy_animal_keeps_its_existing_love_food_path() {
    let fixture = Fixture::new();
    let entity = fixture.spawn(&EntityType::OCELOT, 20);
    let ocelot = entity
        .cast_any()
        .downcast_ref::<crate::entity::passive::ocelot::OcelotEntity>()
        .unwrap();
    ocelot.set_trusting(true);
    assert!(entity.get_mob().unwrap().as_ageable().is_none());
    fixture
        .player
        .player
        .inventory()
        .set_stack(0, ItemStack::new(1, &Item::COD));
    fixture.interact(entity.as_ref());
    assert!(entity.is_in_love());
    assert!(fixture.player.player.inventory().held_item().is_empty());
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review116_love_food_sends_status_without_duplicate_heart_particles() {
    let mut fixture = Fixture::new();
    let cow = fixture.spawn(&EntityType::COW, 0);
    let chunk = cow.get_entity().chunk_pos.load();
    fixture
        .player
        .player
        .chunk_sender
        .lock()
        .unwrap()
        .mark_sent_out_of_band(chunk);
    fixture.world.entity_tracker.update_player_chunks(
        &fixture.player.player,
        &fixture.world,
        &[chunk],
    );
    assert!(fixture.world.is_tracked_by_any_player(cow.get_entity()));
    fixture
        .player
        .player
        .inventory()
        .set_stack(0, ItemStack::new(1, &Item::WHEAT));
    fixture.player.take_packets();
    fixture.interact(cow.as_ref());
    assert!(cow.is_in_love());
    let packets = fixture.player.take_packets();
    let ids: Vec<_> = packets
        .iter()
        .map(|packet| VarInt::decode(&mut packet.as_ref()).unwrap().0)
        .collect();
    assert_eq!(
        ids.iter().filter(|&&id| id == play::ENTITY_EVENT.0).count(),
        1
    );
    assert_eq!(
        ids.iter()
            .filter(|&&id| id == play::LEVEL_PARTICLES.0)
            .count(),
        0,
        "the client makes seven hearts from the love status itself"
    );
    fixture.finish().await;
}
