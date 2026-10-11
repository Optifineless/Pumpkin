use super::review_test_support::Fixture;
use crate::entity::{Entity, EntityBase};
use pumpkin_data::{entity::EntityType, item::Item, item_stack::ItemStack, tracked_data};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::java::client::play::Metadata;
use pumpkin_util::{math::vector3::Vector3, version::JavaMinecraftVersion};
use std::sync::{Arc, atomic::Ordering::Relaxed};

const FEEDING_SPECIES: [(&EntityType, &Item); 6] = [
    (&EntityType::CAT, &Item::COD),
    (&EntityType::COW, &Item::WHEAT),
    (&EntityType::WOLF, &Item::BEEF),
    (&EntityType::CHICKEN, &Item::WHEAT_SEEDS),
    (&EntityType::PIG, &Item::CARROT),
    (&EntityType::SHEEP, &Item::WHEAT),
];

fn assert_baby_metadata(entity: &dyn EntityBase, baby: bool) {
    let version = JavaMinecraftVersion::V_26_3;
    let metadata = entity
        .get_entity()
        .synched_data
        .get_non_default_values_for_version(&version)
        .unwrap();
    let mut expected = Vec::new();
    Metadata::new(tracked_data::ageable_mob::DATA_BABY_ID, baby)
        .write(&mut expected, &version)
        .unwrap();
    assert!(
        metadata
            .windows(expected.len())
            .any(|bytes| bytes == expected),
        "{} has stale baby metadata after its world tick",
        entity.get_entity().entity_type.resource_name
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review116_fed_babies_finish_growth_through_world_tick() {
    let fixture = Fixture::new();
    let inventory = fixture.player.player.inventory();
    let mut babies = Vec::new();
    for (kind, food) in FEEDING_SPECIES {
        let baby = fixture.spawn(kind, -1);
        inventory.set_stack(0, ItemStack::new(1, food));
        fixture.interact(baby.as_ref());
        assert!(
            inventory.held_item().is_empty(),
            "{} refused food",
            kind.resource_name
        );
        assert_eq!(baby.get_entity().age.load(Relaxed), -1);
        assert_baby_metadata(baby.as_ref(), true);
        babies.push(baby);
    }
    fixture.tick();
    for baby in babies {
        assert_eq!(baby.get_entity().age.load(Relaxed), 0);
        assert_baby_metadata(baby.as_ref(), false);
        assert_eq!(
            baby.get_mob()
                .unwrap()
                .get_mob_entity()
                .ticks_lived
                .load(Relaxed),
            1
        );
    }
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review116_age_locked_babies_stay_locked_through_world_tick() {
    let fixture = Fixture::new();
    let babies: Vec<_> = FEEDING_SPECIES
        .iter()
        .map(|(kind, _)| fixture.spawn(kind, -199))
        .collect();
    for baby in &babies {
        baby.get_mob()
            .unwrap()
            .as_ageable()
            .unwrap()
            .set_age_locked(true);
    }
    fixture.tick();
    for baby in babies {
        assert_eq!(
            baby.get_entity().age.load(Relaxed),
            -199,
            "{} age lock",
            baby.get_entity().entity_type.resource_name
        );
        assert_baby_metadata(baby.as_ref(), true);
    }
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review116_ageable_world_tick_counts_down_cooldown_and_stops_at_zero() {
    let fixture = Fixture::new();
    let mut animals: Vec<_> = FEEDING_SPECIES
        .iter()
        .map(|(kind, _)| fixture.spawn(kind, 2))
        .collect();
    animals.push(fixture.spawn(&EntityType::GOAT, 2));
    for expected in [1, 0, 0] {
        fixture.tick();
        for animal in &animals {
            assert_eq!(
                animal.get_entity().age.load(Relaxed),
                expected,
                "{} breeding cooldown",
                animal.get_entity().entity_type.resource_name
            );
        }
    }
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review116_previously_wired_ageable_species_grow_once_per_world_tick() {
    let fixture = Fixture::new();
    let species = [
        &EntityType::ARMADILLO,
        &EntityType::AXOLOTL,
        &EntityType::BEE,
        &EntityType::CAMEL,
        &EntityType::DONKEY,
        &EntityType::FOX,
        &EntityType::FROG,
        &EntityType::GOAT,
        &EntityType::HAPPY_GHAST,
        &EntityType::HORSE,
        &EntityType::LLAMA,
        &EntityType::MOOSHROOM,
        &EntityType::MULE,
        &EntityType::PANDA,
        &EntityType::POLAR_BEAR,
        &EntityType::RABBIT,
        &EntityType::SKELETON_HORSE,
        &EntityType::SNIFFER,
        &EntityType::STRIDER,
        &EntityType::TADPOLE,
        &EntityType::TRADER_LLAMA,
        &EntityType::TURTLE,
        &EntityType::ZOMBIE_HORSE,
    ];
    let animals: Vec<_> = species
        .into_iter()
        .map(|kind| fixture.spawn(kind, -20))
        .collect();
    fixture.tick();
    for animal in animals {
        assert_eq!(
            animal.get_entity().age.load(Relaxed),
            -19,
            "{} must grow once",
            animal.get_entity().entity_type.resource_name
        );
    }
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review116_world_tick_keeps_independent_mob_and_non_mob_clocks() {
    let fixture = Fixture::new();
    let cat = fixture.spawn(&EntityType::CAT, 0);
    let zombie = fixture.spawn(&EntityType::ZOMBIE, 0);
    let plain = Arc::new(Entity::new(
        fixture.world.clone(),
        Vector3::new(8.5, 64.0, 9.5),
        &EntityType::ARMOR_STAND,
    ));
    assert!(fixture.world.spawn_entity(plain.clone()));
    for elapsed in 1..=3 {
        fixture.tick();
        assert_eq!(cat.get_entity().age.load(Relaxed), 0);
        assert_eq!(
            cat.get_mob()
                .unwrap()
                .get_mob_entity()
                .ticks_lived
                .load(Relaxed),
            elapsed
        );
        assert_eq!(zombie.get_entity().age.load(Relaxed), elapsed);
        assert_eq!(
            zombie
                .get_mob()
                .unwrap()
                .get_mob_entity()
                .ticks_lived
                .load(Relaxed),
            elapsed
        );
        assert_eq!(plain.age.load(Relaxed), elapsed);
    }
    fixture.finish().await;
}
