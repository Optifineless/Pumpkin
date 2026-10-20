use super::happy_ghast_test_support::*;
use crate::entity::{Entity, EntityBase, ageable::AgeableMob};
use pumpkin_data::{
    Block, data_component_impl::EquipmentSlot, entity::EntityType, item::Item,
    item_stack::ItemStack,
};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use pumpkin_world::world::BlockFlags;
use std::sync::{Arc, atomic::Ordering::Relaxed};

// Verbatim pre-slice species source, git blob 285f3846b4a94258f3360e159d67c60edf89c16d.
// Common readers remain unchanged from 0a91143efb7a2f366489716756931eb054104ae5.
#[path = "happy_ghast_baseline_reader.rs"]
mod baseline;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_happy_ghast_s1_home_selection_and_real_reload() {
    let fixture = Fixture::new(true).await;
    let entity = fixture.summon("14.5 74 8.5", "{Age:0,ForcedAge:30,AgeLocked:1b}");
    // First candidate is air 14 blocks above a real floor, accepted by the actual random goal.
    draws(&entity, &[0.75, 0.625, 0.75].repeat(100));
    fixture.tick(1);
    assert_eq!(target(&entity), Vector3::new(22.5, 78.0, 16.5));
    let home = nbt(&entity);
    assert_eq!(
        home.get_int("home_radius"),
        Some(64),
        "adult AI did not establish a home"
    );
    assert_eq!(home.get_int_array("home_pos"), Some([14, 74, 8].as_slice()));
    // Real canonical BODY equipment selects the smaller radius on the next species AI step.
    ghast(&entity).mob_entity.set_item_slot(
        &EquipmentSlot::BODY,
        ItemStack::new(1, &Item::WHITE_HARNESS),
    );
    fixture.tick(1);
    assert_eq!(nbt(&entity).get_int("home_radius"), Some(32));
    wanted(&entity, Vector3::new(30.5, 78.0, 16.5));
    fixture.tick(32);
    assert!(
        entity.get_entity().pos.load().x > 16.0,
        "flight never crossed the storage chunk border"
    );
    let saved = fixture.saved_record(&entity).await;
    let uuid = entity.get_entity().entity_uuid;
    let chunk_pos = entity.get_entity().chunk_pos.load();
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        fixture.world.level.mark_chunks_as_not_watched([chunk_pos]),
    )
    .await
    .unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        fixture.world.remove_entities_in_chunks([chunk_pos]),
    )
    .await
    .unwrap();
    fixture.world.process_entity_chunk_lifecycle();
    assert!(entity.get_entity().is_removed());
    let restored = fixture.restore(chunk_pos, uuid).await;
    assert_eq!(
        fixture
            .world
            .entities
            .load()
            .iter()
            .filter(|e| e.get_entity().entity_uuid == uuid)
            .count(),
        1
    );
    let reload = nbt(&restored);
    for key in [
        "home_pos",
        "home_radius",
        "Age",
        "ForcedAge",
        "AgeLocked",
        "equipment",
    ] {
        assert_eq!(
            reload.child_tags.get(key),
            saved.child_tags.get(key),
            "lost {key} on real reload"
        );
    }
    draws(&restored, &[0.75, 0.625, 0.75].repeat(100));
    let start = restored.get_entity().pos.load();
    fixture.tick(12);
    assert!(
        (restored.get_entity().pos.load() - start).length() > 0.05,
        "reload did not move: age={}, removed={}, velocity={:?}, target={:?}, loaded={}",
        restored.get_entity().age.load(Relaxed),
        restored.get_entity().is_removed(),
        restored.get_entity().velocity.load(),
        target(&restored),
        fixture
            .world
            .level
            .is_chunk_loaded(&restored.get_entity().chunk_pos.load())
    );
    assert!(fixture.world.forced_chunks.lock().unwrap().is_empty());
    drop(fixture.close().await);
    selection_fallback_and_recenter().await;
    strict_home_candidates_and_vehicle().await;
}

async fn strict_home_candidates_and_vehicle() {
    let fixture = Fixture::new(true).await;
    fixture.world.set_block_state(
        &BlockPos::new(13, 100, 8),
        Block::STONE.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let entity = fixture.summon("8.5 100 8.5", "{home_radius:1,home_pos:[I;8,100,8]}");
    draws(&entity, &[0.53125, 0.515625, 0.5, 0.5, 0.515625, 0.5]);
    fixture.tick(1);
    assert_eq!(
        target(&entity),
        Vector3::new(8.5, 100.5, 8.5),
        "candidate exactly on block-center radius was admitted"
    );
    assert!(ghast(&entity).random_draws.lock().unwrap().is_empty());
    let rider = crate::entity::r#type::from_type(
        &EntityType::CHICKEN,
        entity.get_entity().pos.load(),
        &fixture.world,
        uuid::Uuid::new_v4(),
    );
    assert!(fixture.world.spawn_entity(rider.clone()));
    entity.get_entity().add_passenger(entity.clone(), rider);
    assert!(entity.get_entity().has_passengers());
    ghast(&entity).mob_entity.set_item_slot(
        &EquipmentSlot::BODY,
        ItemStack::new(1, &Item::WHITE_HARNESS),
    );
    fixture.tick(1);
    assert_eq!(
        nbt(&entity).get_int("home_radius"),
        Some(64),
        "vehicle recentered after equipment change"
    );
    let free = fixture.summon("8.5 100 8.5", "{}");
    draws(&free, &[0.99, 0.515625, 0.5]);
    fixture.tick(1);
    assert!(
        target(&free).x > 23.0,
        "unrestricted candidate was rejected"
    );
    drop(fixture.close().await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_happy_ghast_s1_unrestricted_and_unloaded_controls() {
    let fixture = Fixture::new(true).await;
    fixture.world.set_block_state(
        &BlockPos::new(13, 100, 8),
        Block::STONE.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let entity = fixture.summon("8.5 100 8.5", "{}");
    draws(&entity, &[0.99, 0.515625, 0.5]);
    fixture.tick(1);
    assert!(
        target(&entity).x > 23.0,
        "unrestricted near-block candidate rejected"
    );
    let edge = fixture.summon("38.5 74 8.5", "{}");
    draws(&edge, &[0.99, 0.5, 0.5].repeat(64));
    fixture.tick(1);
    assert!(target(&edge).x > 47.0);
    assert_eq!(
        edge.get_entity().velocity.load(),
        Vector3::new(0.0, 0.0, 0.0),
        "missing terrain was treated as air"
    );
    assert!(
        fixture
            .world
            .get_block_state_if_loaded(&BlockPos::new(54, 74, 8))
            .is_none()
    );
    assert!(fixture.world.forced_chunks.lock().unwrap().is_empty());
    drop(fixture.close().await);
}

async fn selection_fallback_and_recenter() {
    let fixture = Fixture::new(true).await;
    // All 64 candidates fail strict block-center radius zero. Final unrestricted candidate survives.
    let entity = fixture.summon("8.5 100 8.5", "{home_radius:0,home_pos:[I;8,100,8]}");
    let mut sequence = vec![0.5; 64 * 3];
    sequence.extend_from_slice(&[0.75, 0.75, 0.75]);
    draws(&entity, &sequence);
    fixture.tick(1);
    assert_eq!(
        target(&entity),
        Vector3::new(16.5, 92.0, 16.5),
        "all-rejected heightmap fallback did not reflect upward Y"
    );
    assert!(
        ghast(&entity).random_draws.lock().unwrap().is_empty(),
        "random goal did not consume 64 candidates plus fallback"
    );
    // A restored home at exactly radius+16 recenters, using integer block distance.
    let entity = fixture.summon("8.5 100 -8.5", "{home_radius:64,home_pos:[I;88,100,-9]}");
    draws(&entity, &[0.5; 192]);
    fixture.tick(1);
    assert_eq!(
        nbt(&entity).get_int_array("home_pos"),
        Some([8, 100, -9].as_slice())
    );
    let anchor = nbt(&entity).get_int_array("home_pos").unwrap().to_vec();
    // Existing leash path suppresses radius changes without introducing feature tickets.
    entity.get_entity().leash_to(fixture.player.player.clone());
    ghast(&entity).mob_entity.set_item_slot(
        &EquipmentSlot::BODY,
        ItemStack::new(1, &Item::WHITE_HARNESS),
    );
    fixture
        .player
        .player
        .get_entity()
        .set_pos(entity.get_entity().pos.load() + Vector3::new(2.0, 0.0, 0.0));
    fixture.tick(1);
    assert_eq!(nbt(&entity).get_int("home_radius"), Some(64));
    assert_eq!(
        nbt(&entity).get_int_array("home_pos"),
        Some(anchor.as_slice())
    );
    entity.get_entity().unleash();
    fixture.tick(1);
    assert_eq!(nbt(&entity).get_int("home_radius"), Some(32));
    // Empty-column min-Y fallback does not reflect the last failed upward candidate.
    let entity = fixture.summon("-8.5 100 -8.5", "{}");
    for x in -26..=8 {
        for z in -26..=8 {
            fixture.world.set_block_state(
                &BlockPos::new(x, 63, z),
                Block::AIR.default_state.id,
                BlockFlags::FORCE_STATE,
            );
        }
    }
    draws(&entity, &[0.5, 0.75, 0.5].repeat(64));
    fixture.tick(1);
    assert_eq!(
        target(&entity).y,
        108.0,
        "min-Y fallback incorrectly reflected target"
    );
    drop(fixture.close().await);
}

fn assert_old_reader(record: &NbtCompound, world: &Arc<super::World>) {
    let old = baseline::HappyGhastEntity::new(Entity::from_uuid(
        record.get_uuid("UUID").unwrap(),
        world.clone(),
        Vector3::new(0.0, 0.0, 0.0),
        &EntityType::HAPPY_GHAST,
    ));
    old.read_nbt_non_mut(record);
    assert!(!old.is_on_still_timeout());
    let mut reread = NbtCompound::new();
    old.write_nbt(&mut reread);
    for key in [
        "Age",
        "ForcedAge",
        "AgeLocked",
        "still_timeout",
        "Pos",
        "Motion",
        "UUID",
        "attributes",
        "equipment",
    ] {
        assert_eq!(
            reread.child_tags.get(key),
            record.child_tags.get(key),
            "baseline reader rejected/changed {key}"
        );
    }
    assert!(
        reread.get_int("home_radius").is_none(),
        "baseline must ignore additive home tags"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_happy_ghast_s1_save_and_config_compatibility() {
    let fixture = Fixture::new(true).await;
    let entity = fixture.summon("8.5 74 8.5", "{Age:0,ForcedAge:40,AgeLocked:1b}");
    ghast(&entity).mob_entity.set_item_slot(
        &EquipmentSlot::BODY,
        ItemStack::new(1, &Item::WHITE_HARNESS),
    );
    draws(&entity, &[0.75, 0.625, 0.75]);
    fixture.tick(1);
    let saved = fixture.saved_record(&entity).await;
    assert_old_reader(&saved, &fixture.world);
    let uuid = entity.get_entity().entity_uuid;
    let chunk = entity.get_entity().chunk_pos.load();
    let fixture = fixture.restart(false).await;
    let restored = fixture.restore(chunk, uuid).await;
    let before = nbt(&restored);
    fixture.tick(4);
    let after = nbt(&restored);
    for key in [
        "home_pos",
        "home_radius",
        "ForcedAge",
        "AgeLocked",
        "equipment",
    ] {
        assert_eq!(
            before.child_tags.get(key),
            after.child_tags.get(key),
            "disabled mode changed {key}"
        );
    }
    assert!(
        restored.get_entity().pos.load().y
            < saved.get_list("Pos").unwrap()[1].extract_double().unwrap(),
        "disabled: age={}, removed={}, velocity={:?}, loaded={}, health={}, enabled={}",
        restored.get_entity().age.load(Relaxed),
        restored.get_entity().is_removed(),
        restored.get_entity().velocity.load(),
        fixture.world.level.is_chunk_loaded(&chunk),
        restored.get_living_entity().unwrap().health.load(),
        fixture.server.advanced_config.happy_ghast.enabled
    );
    let fixture = fixture.restart(true).await;
    let restored = fixture.restore(chunk, uuid).await;
    let start = restored.get_entity().pos.load();
    draws(&restored, &[0.75, 0.625, 0.75]);
    fixture.tick(32);
    let delta = restored.get_entity().pos.load().sub(&start);
    assert!(delta.length_squared() > 0.25 && delta.length_squared() < 900.0);
    assert!(
        restored.get_entity().velocity.load().y > 0.0,
        "adult flight did not overcome inherited falling inertia"
    );
    assert_eq!(ghast(&restored).floating_goal_count(), 1);
    loaded_vanilla_body_radius(&fixture).await;
    defaults_and_vanilla_fixture(&fixture, &restored);
    drop(fixture.close().await);
}

async fn loaded_vanilla_body_radius(fixture: &Fixture) {
    // Source-authored vanilla 26.3 fixture: Mob.java:369-410 home codec and
    // HappyGhast.java:448-459,532-545 restriction/timeout; canonical common equipment codec.
    let loaded = fixture.summon("8.5 100 -8.5", r#"{Age:0,ForcedAge:21,AgeLocked:1b,home_radius:64,home_pos:[I;8,100,-9],still_timeout:0,equipment:{body:{id:"minecraft:white_harness",count:1}}}"#);
    assert_eq!(nbt(&loaded).get_int("home_radius"), Some(64));
    assert_eq!(
        ghast(&loaded)
            .mob_entity
            .living_entity
            .entity_equipment
            .lock()
            .unwrap()
            .get(&EquipmentSlot::BODY)
            .item,
        &Item::WHITE_HARNESS
    );
    draws(&loaded, &[0.75, 0.625, 0.75]);
    fixture.tick(1);
    assert_eq!(
        nbt(&loaded).get_int("home_radius"),
        Some(32),
        "loaded canonical BODY did not change adult home radius"
    );
    assert_old_reader(&fixture.saved_record(&loaded).await, &fixture.world);
}

fn defaults_and_vanilla_fixture(fixture: &Fixture, reused: &Arc<dyn EntityBase>) {
    // Vanilla 26.3 Mob/HappyGhast save schema supplied with this task; source-authored fixture,
    // not a claim of a vanilla binary capture. Old-fork record comes from retained baseline writer.
    let old = baseline::HappyGhastEntity::new(Entity::new(
        fixture.world.clone(),
        Vector3::new(8.5, 74.0, 8.5),
        &EntityType::HAPPY_GHAST,
    ));
    old.set_age(-2000);
    old.get_ageable_data().forced_age.store(20, Relaxed);
    old.set_age_locked(true);
    old.mob_entity.set_item_slot(
        &EquipmentSlot::BODY,
        ItemStack::new(1, &Item::WHITE_HARNESS),
    );
    let mut legacy = NbtCompound::new();
    old.write_nbt(&mut legacy);
    let fresh = fixture.egg(BlockPos::new(-8, 74, -8));
    for entity in [&fresh, reused] {
        ghast(entity).set_server_still_timeout(90);
        for pos in [None, Some(vec![9, 9])] {
            let mut vanilla = legacy.clone();
            vanilla.child_tags.remove("still_timeout");
            vanilla.put_int("home_radius", 32);
            if let Some(pos) = pos {
                vanilla.put("home_pos", NbtTag::IntArray(pos));
            }
            entity.read_nbt_non_mut(&vanilla);
            assert_eq!(
                ghast(entity).server_still_timeout.load(Relaxed),
                0,
                "absent timeout retained reused state"
            );
            assert_eq!(
                nbt(entity).get_int_array("home_pos"),
                Some([0, 0, 0].as_slice())
            );
        }
        entity.read_nbt_non_mut(&legacy);
        assert!(
            nbt(entity).get_int("home_radius").is_none(),
            "old save retained reused home"
        );
        assert_eq!(ghast(entity).get_age(), -2000);
        assert_eq!(
            ghast(entity)
                .mob_entity
                .living_entity
                .entity_equipment
                .lock()
                .unwrap()
                .get(&EquipmentSlot::BODY)
                .item,
            &Item::WHITE_HARNESS
        );
    }
}
