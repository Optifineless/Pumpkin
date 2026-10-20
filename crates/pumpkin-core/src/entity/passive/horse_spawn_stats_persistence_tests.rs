use pumpkin_data::{Block, biome::Biome, entity::EntityType};
use pumpkin_nbt::{Nbt, compound::NbtCompound, tag::NbtTag};
use pumpkin_util::math::vector2::Vector2;

use super::spawn_test_support::{Fixture as WorldFixture, proto, publish};
use crate::entity::{
    mob::spawn::load_spawn_entity,
    passive::{horse_spawn_stats_test_rng::Script, horse_spawn_stats_test_support::*},
};

#[tokio::test]
async fn feat_horses_s1_generation_publication_does_not_reroll() {
    tokio::time::timeout(std::time::Duration::from_secs(90), async {
        let fixture = WorldFixture::new();
        let mut cache = proto(&Biome::PLAINS, &Block::GRASS_BLOCK);
        let script = Script::new([3, 4], [0.25, 0.5, 0.75, 0.125, 0.25, 0.375], 1);
        assert!(super::generation_spawning::spawn_mob(
            &fixture.world,
            &mut cache,
            &EntityType::HORSE,
            POS,
            &mut None
        ));
        script.consumed();
        assert!(fixture.world.entities.load().is_empty());
        let retained = cache.pending_entities[0].clone();
        let uuid = retained.get_uuid("UUID").unwrap();
        let script = Script::new([0, 0], [0.0; 6], 0);
        publish(&fixture.world, cache);
        fixture.world.publish_generated_entities(Vector2::new(0, 0));
        let horse = fixture.world.get_entity_by_uuid(uuid).unwrap();
        assert_stats(horse.as_ref(), INTERNAL, 22.0);
        assert_saved(&retained, &save(horse.as_ref()));
        script.consumed();

        if let Ok(path) = std::env::var("FEAT_HORSES_S1_CAPTURE") {
            std::fs::write(path, Nbt::new(String::new(), retained.clone()).write()).unwrap();
        }
        let fixture = unload_restart(fixture).await;
        let chunk = fixture
            .world
            .level
            .get_entity_chunk(Vector2::new(0, 0))
            .await
            .unwrap();
        let script = Script::new([0, 0], [0.0; 6], 0);
        fixture.world.make_chunk_entities_live(&chunk, None);
        fixture.world.make_chunk_entities_live(&chunk, None);
        let horse = fixture.world.get_entity_by_uuid(uuid).unwrap();
        assert_saved(&retained, &save(horse.as_ref()));
        script.consumed();
        fixture.finish().await;
        structure_birth().await;
    })
    .await
    .unwrap();
}

async fn unload_restart(fixture: WorldFixture) -> WorldFixture {
    let pos = Vector2::new(0, 0);
    let chunk = fixture.world.level.get_entity_chunk(pos).await.unwrap();
    fixture.world.make_chunk_entities_live(&chunk, None);
    fixture.world.remove_entities_in_chunks([pos]).await;
    fixture.world.process_entity_chunk_lifecycle();
    assert!(fixture.world.entities.load().is_empty());
    fixture.world.level.clean_entity_chunks([pos]);
    fixture.restart().await
}

async fn structure_birth() {
    let fixture = WorldFixture::new();
    let mut cache = proto(&Biome::PLAINS, &Block::GRASS_BLOCK);
    let mut nbt = custom_record(40.0, 0.37, 0.91, 7.0);
    nbt.put_list(
        "Pos",
        vec![
            NbtTag::Double(POS.x),
            NbtTag::Double(POS.y),
            NbtTag::Double(POS.z),
        ],
    );
    let script = Script::new([3, 4], [0.25, 0.5, 0.75, 0.125, 0.25, 0.375], 1);
    fixture
        .world
        .retain_structure_entities(&mut cache, vec![nbt]);
    script.consumed();
    let retained = cache.pending_entities[0].clone();
    assert_eq!(retained.get_float("Health"), Some(7.0));
    let uuid = retained.get_uuid("UUID").unwrap();
    let script = Script::new([0, 0], [0.0; 6], 0);
    publish(&fixture.world, cache);
    fixture.world.publish_generated_entities(Vector2::new(0, 0));
    let horse = fixture.world.get_entity_by_uuid(uuid).unwrap();
    assert_stats(horse.as_ref(), INTERNAL, 7.0);
    assert_permanent_modifiers(horse.as_ref());
    assert_saved(&retained, &save(horse.as_ref()));
    script.consumed();
    fixture.finish().await;
}

fn custom_record(max: f64, speed: f64, jump: f64, health: f32) -> NbtCompound {
    let mut nbt = NbtCompound::new();
    nbt.put_string("id", "minecraft:horse".to_owned());
    nbt.put_float("Health", health);
    nbt.put_int("Variant", 258);
    nbt.put_list(
        "attributes",
        ATTRS
            .iter()
            .zip([max, speed, jump])
            .map(|(attribute, base)| {
                let mut packed = NbtCompound::new();
                packed.put_string("id", attribute.name.to_owned());
                packed.put_double("base", base);
                let mut modifier = NbtCompound::new();
                modifier.put_string("id", "test:permanent".to_owned());
                modifier.put_double("amount", 0.125);
                modifier.put_string("operation", "add_value".to_owned());
                packed.put_list("modifiers", vec![NbtTag::Compound(modifier)]);
                NbtTag::Compound(packed)
            })
            .collect(),
    );
    nbt
}

#[tokio::test]
async fn feat_horses_s1_loads_and_explicit_overrides_keep_saved_stats() {
    tokio::time::timeout(std::time::Duration::from_secs(90), async {
    let fixture = Fixture::new(true);
    let mut legacy = NbtCompound::new();
    legacy.put_string("id", "minecraft:horse".to_owned());
    legacy.put_float("Health", 53.0);
    let mut upper = legacy.clone();
    let mut old_attribute = NbtCompound::new();
    old_attribute.put_string("Name", "minecraft:generic.max_health".to_owned());
    old_attribute.put_double("Base", 19.0);
    upper.put_list("Attributes", vec![NbtTag::Compound(old_attribute)]);
    let script = Script::new([0, 0], [0.0; 6], 0);
    for (nbt, expected, health) in [
        (legacy.clone(), default_bases(&EntityType::HORSE), 53.0),
        (upper, default_bases(&EntityType::HORSE), 53.0),
        (
            custom_record(53.0, 0.22499999403953552, 0.7, 53.0),
            [53.0, 0.22499999403953552, 0.7],
            53.0,
        ),
        (
            custom_record(37.0, 0.31, 0.86, 6.0),
            [37.0, 0.31, 0.86],
            6.0,
        ),
    ] {
        let horse = load_spawn_entity(&fixture.world, &nbt, POS).unwrap();
        assert_stats(horse.as_ref(), expected, health);
        if nbt.get_list("attributes").is_some() { assert_permanent_modifiers(horse.as_ref()); }
        let saved = save(horse.as_ref());
        let horse = load_spawn_entity(&fixture.world, &saved, POS).unwrap();
        assert_saved(&saved, &save(horse.as_ref()));
        horse.init_data_tracker();
        horse.init_data_tracker();
        assert_saved(&saved, &save(horse.as_ref()));
    }
    let horse = fixture.summon("horse 8.5 64 8.5 {}");
    assert_stats(horse.as_ref(), default_bases(&EntityType::HORSE), 53.0);
    let horse = fixture.summon("horse 8.5 64 8.5 {Health:6f,attributes:[{id:'minecraft:max_health',base:37d},{id:'minecraft:movement_speed',base:0.31d},{id:'minecraft:jump_strength',base:0.86d}]}");
    assert_stats(horse.as_ref(), [37.0, 0.31, 0.86], 6.0);
    script.consumed();
    fixture.finish().await;
    config_restart().await;

    }).await.unwrap();
}

#[tokio::test]
async fn feat_horses_s1_egg_entity_data_overrides_after_roll() {
    tokio::time::timeout(std::time::Duration::from_secs(90), async {
        let mut fixture = Fixture::new(true);
        let mut egg = horse_egg();
        egg.set_data_component(pumpkin_data::data_component_impl::EntityDataImpl {
            nbt: Some(custom_record(37.0, 0.31, 0.86, 6.0)),
        });
        let script = Script::new([3, 4], [0.25, 0.5, 0.75, 0.125, 0.25, 0.375], 1);
        let horse = fixture.egg(egg);
        assert_stats(horse.as_ref(), [37.0, 0.31, 0.86], 6.0);
        assert_permanent_modifiers(horse.as_ref());
        let properties = assert_observer(
            &fixture.actor.take_packets(),
            horse.as_ref(),
            [37.0, 0.31, 0.86],
            6.0,
        );
        for property in properties {
            assert_eq!(
                property.modifiers,
                vec![
                    pumpkin_protocol::java::client::play::AttributeModifier::new(
                        "test:permanent".to_owned(),
                        0.125,
                        0
                    )
                ]
            );
        }
        script.consumed();
        fixture.finish().await;
    })
    .await
    .unwrap();
}

async fn config_restart() {
    let mut fixture = Fixture::new(true);
    let horse =
        load_spawn_entity(&fixture.world, &custom_record(37.0, 0.31, 0.86, 6.0), POS).unwrap();
    let record = save(horse.as_ref());
    let uuid = horse.get_entity().entity_uuid;
    assert!(fixture.world.spawn_entity(horse));
    for enabled in [false, true] {
        let pos = Vector2::new(0, 0);
        let chunk = fixture.world.level.get_entity_chunk(pos).await.unwrap();
        fixture.world.make_chunk_entities_live(&chunk, None);
        fixture.world.remove_entities_in_chunks([pos]).await;
        fixture.world.process_entity_chunk_lifecycle();
        fixture.world.level.clean_entity_chunks([pos]);
        fixture = fixture.restart(enabled).await;
        let chunk = fixture.world.level.get_entity_chunk(pos).await.unwrap();
        let script = Script::new([0, 0], [0.0; 6], 0);
        fixture.world.make_chunk_entities_live(&chunk, None);
        let restored = fixture.world.get_entity_by_uuid(uuid).unwrap();
        assert_saved(&record, &save(restored.as_ref()));
        script.consumed();
    }
    fixture.finish().await;
}
