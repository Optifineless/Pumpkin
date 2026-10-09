use super::*;
use crate::{
    data::datapack::loot_table_loader::parse_loot_table,
    entity::{death_test_world::DeathTestWorld, r#type::from_type},
    world::spawn_test_support::{proto, publish},
};
use pumpkin_data::{Block, biome::Biome, entity::EntityType, item::Item};
use pumpkin_util::{identifier::Identifier, math::vector3::Vector3};
use serde_json::json;

fn drops(world: &crate::world::World, item: &Item) -> u32 {
    world
        .entities
        .load()
        .iter()
        .filter_map(|entity| entity.get_item_entity())
        .map(|entity| {
            let stack = entity.get_item_stack().lock().unwrap();
            if stack.item == item {
                u32::from(stack.item_count)
            } else {
                0
            }
        })
        .sum()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shearing_uses_root_datapack_and_species_context() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    for (kind, name) in [
        (&EntityType::SHEEP, "sheep"),
        (&EntityType::MOOSHROOM, "mooshroom"),
        (&EntityType::SNOW_GOLEM, "snow_golem"),
        (&EntityType::BOGGED, "bogged"),
    ] {
        let sequence = format!("test:shearing_{name}");
        let predicate = if name == "sheep" {
            json!({"type":"minecraft:sheep", "minecraft:type_specific/sheep":{"sheared":false}})
        } else {
            json!({"type":format!("minecraft:{name}")})
        };
        let table = parse_loot_table(&json!({
            "type":"minecraft:shearing", "random_sequence":sequence,
            "pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:diamond",
                "condition":{"type":"minecraft:all_of","terms":[
                    {"type":"minecraft:match_tool","predicate":{"items":"minecraft:shears"}},
                    {"type":"minecraft:entity_properties","entity":"this","predicate":predicate},
                    {"type":"minecraft:random_chance","chance":1}
                ]}}]}]
        }).to_string()).unwrap();
        fixture
            .server
            .datapack_manager
            .insert_loot_table(format!("minecraft:shearing/{name}"), Arc::new(table));
        let id = Identifier::parse(&sequence).unwrap();
        let before = fixture
            .server
            .random_sequences
            .lock()
            .unwrap()
            .get_or_create(&id, 0)
            .random()
            .state();
        let mob = from_type(
            kind,
            Vector3::new(8.5, 64.0, 8.5),
            &world,
            uuid::Uuid::new_v4(),
        );
        world.add_entity_silent(mob.clone());
        let shearable = mob.get_mob().unwrap().as_shearable().unwrap();
        let count = drops(&world, &Item::DIAMOND);
        assert!(shearable.ready_for_shearing());
        assert!(shearable.shear(SoundCategory::Players, &ItemStack::new(1, &Item::SHEARS)));
        assert_eq!(
            drops(&world, &Item::DIAMOND),
            count + 1,
            "root loot for {name}"
        );
        let after = fixture
            .server
            .random_sequences
            .lock()
            .unwrap()
            .get_or_create(&id, 0)
            .random()
            .state();
        assert_ne!(before, after, "named sequence for {name}");
    }
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ordinary_sheep_root_keeps_color_drops() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let sheep = crate::entity::passive::sheep::SheepEntity::new(Entity::new(
        world.clone(),
        Vector3::new(8.5, 64.0, 8.5),
        &EntityType::SHEEP,
    ));
    sheep.set_color(pumpkin_data::dye_color::DyeColor::Purple as u8);
    assert!(sheep.shear(SoundCategory::Players, &ItemStack::new(1, &Item::SHEARS)));
    assert!((1..=3).contains(&drops(&world, &Item::PURPLE_WOOL)));
    assert_eq!(drops(&world, &Item::WHITE_WOOL), 0);
    assert!(!sheep.shear(SoundCategory::Players, &ItemStack::new(1, &Item::SHEARS)));
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mooshroom_root_loot_observes_transferred_vehicle_and_first_passenger() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let mooshroom = fixture.mob(&EntityType::MOOSHROOM);
    let vehicle = fixture.mob(&EntityType::COW);
    let first = fixture.mob(&EntityType::SHEEP);
    let second = fixture.mob(&EntityType::PIG);
    vehicle
        .get_entity()
        .add_passenger(vehicle.clone(), mooshroom.clone());
    mooshroom
        .get_entity()
        .add_passenger(mooshroom.clone(), first);
    mooshroom
        .get_entity()
        .add_passenger(mooshroom.clone(), second);
    let table = parse_loot_table(&json!({"type":"minecraft:shearing", "pools":[{"rolls":1,
        "entries":[{"type":"minecraft:item","name":"minecraft:diamond", "condition":{
            "type":"minecraft:all_of", "terms":[
                {"type":"minecraft:inverted", "term":{"type":"minecraft:entity_properties", "entity":"this", "predicate":{"minecraft:vehicle":{}}}},
                {"type":"minecraft:inverted", "term":{"type":"minecraft:entity_properties", "entity":"this", "predicate":{"minecraft:passenger":{"type":"minecraft:sheep"}}}},
                {"type":"minecraft:entity_properties", "entity":"this", "predicate":{"minecraft:passenger":{"type":"minecraft:pig"}}}
            ]}}]}]}).to_string()).unwrap();
    fixture
        .server
        .datapack_manager
        .insert_loot_table("minecraft:shearing/mooshroom".into(), Arc::new(table));
    assert!(
        mooshroom
            .get_mob()
            .unwrap()
            .as_shearable()
            .unwrap()
            .shear(SoundCategory::Players, &ItemStack::new(1, &Item::SHEARS))
    );
    assert_eq!(drops(&world, &Item::DIAMOND), 1);
    fixture.server.shutdown().await;
}
