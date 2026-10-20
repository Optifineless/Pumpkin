use super::*;
use pumpkin_data::attributes::Attributes;
use pumpkin_nbt::{Nbt, compound::NbtCompound};

fn record(entity: &Arc<dyn EntityBase>) -> NbtCompound {
    let mut nbt = NbtCompound::new();
    entity.write_nbt(&mut nbt);
    nbt
}

fn assert_committed(entity: &Arc<dyn EntityBase>, owner: Uuid) {
    let saved = record(entity);
    assert_eq!(saved.get_bool("Tame"), Some(true));
    assert_eq!(saved.get_uuid("Owner"), Some(owner));
    assert_eq!(saved.get_int("Temper"), Some(5));
    assert_eq!(saved.get_int("Variant"), Some(771));
    assert_eq!(saved.get_float("Health"), Some(8.5));
    let living = entity.get_living_entity().unwrap();
    assert_eq!(living.get_attribute_base(&Attributes::MAX_HEALTH), 27.0);
    assert_eq!(living.get_attribute_base(&Attributes::MOVEMENT_SPEED), 0.29);
    assert_eq!(living.get_attribute_base(&Attributes::JUMP_STRENGTH), 0.7);
    assert!(entity.get_mob().unwrap().is_tamed());
    assert_eq!(entity.get_mob().unwrap().get_owner_uuid(), Some(owner));
    assert!(!horse(entity).has_flag(FLAG_STANDING));
    assert!(!entity.get_entity().has_passengers());
}

fn customize_saved_attributes(entity: &Arc<dyn EntityBase>) {
    horse(entity).set_variant(771);
    let living = entity.get_living_entity().unwrap();
    living.set_attribute_base(&Attributes::MAX_HEALTH, 27.0);
    living.set_attribute_base(&Attributes::MOVEMENT_SPEED, 0.29);
    living.set_attribute_base(&Attributes::JUMP_STRENGTH, 0.7);
    living.set_health(8.5);
}

#[tokio::test]
async fn feat_horses_s2_committed_taming_survives_unload_restart() {
    run(async {
        let scenario = Scenario::new().await;
        let owner_id = Uuid::new_v4();
        let owner = scenario.join(owner_id, "HorseSave").await;
        let world = owner.player.world();
        let entity = spawn(&world);
        let entity_id = entity.get_entity().entity_uuid;
        customize_saved_attributes(&entity);
        interact(&scenario, &owner, &entity);
        script(&entity, 0, 0);
        opportunity(&scenario, &entity).await;
        assert_eq!(horse(&entity).get_temper(), 5);
        remount(&scenario, &owner, &entity).await;
        script(&entity, 0, 4);
        opportunity(&scenario, &entity).await;
        assert!(
            horse(&entity).is_tame(),
            "persistence fixture must tame by actual ride ticks"
        );
        entity
            .get_entity()
            .remove_passenger(owner.player.entity_id());
        owner.confirm_teleport(&scenario.server);
        assert_committed(&entity, owner_id);
        if let Some(path) = std::env::var_os("HORSES_S2_BASELINE_RECORD") {
            std::fs::write(path, Nbt::new(String::new(), record(&entity)).write()).unwrap();
        }
        scenario.prepare_area(&world, Vector2::new(10, 0)).await;
        scenario.command("tp HorseSave 163.5 64 3.5");
        owner.loaded(&scenario.server);
        scenario
            .until("horse unload watcher tasks", || {
                scenario.server.tasks.is_empty() && owner.is_idle()
            })
            .await;
        scenario.ticks(1).await;
        assert!(
            entity.get_entity().is_removed(),
            "horse must actually retire"
        );
        assert!(world.get_entity_by_uuid(entity_id).is_none());
        assert!(!entity.get_entity().has_passengers());
        scenario.command("tp HorseSave 3.5 64 3.5");
        owner.loaded(&scenario.server);
        scenario
            .until("horse activation watcher tasks", || {
                scenario.server.tasks.is_empty() && owner.is_idle()
            })
            .await;
        scenario.ticks(1).await;
        let loaded = world.get_entity_by_uuid(entity_id).unwrap();
        assert!(!Arc::ptr_eq(&loaded, &entity));
        assert_committed(&loaded, owner_id);
        assert!(
            statuses(&owner, &loaded).is_empty(),
            "activation replayed heart or buck"
        );
        // Persist a second, failed riding attempt while its stand timer is still live.
        let pending = spawn(&world);
        let pending_id = pending.get_entity().entity_uuid;
        interact(&scenario, &owner, &pending);
        script(&pending, 0, 0);
        opportunity(&scenario, &pending).await;
        assert!(horse(&pending).has_flag(FLAG_STANDING));
        let scenario = scenario.restart_with_horse_taming(false).await;
        let owner = scenario.join(owner_id, "HorseSave").await;
        let world = owner.player.world();
        let loaded = world.get_entity_by_uuid(entity_id).unwrap();
        assert_committed(&loaded, owner_id);
        let pending = world.get_entity_by_uuid(pending_id).unwrap();
        assert_eq!(horse(&pending).get_temper(), 5);
        assert!(
            !horse(&pending).has_flag(FLAG_STANDING),
            "standing timer persisted through restart"
        );
        script(&pending, 0, 4);
        interact(&scenario, &owner, &pending);
        assert_mounted(&owner, &pending);
        owner.packets();
        scenario.ticks(12).await;
        assert!(!horse(&pending).is_tame());
        assert_eq!(horse(&pending).get_temper(), 5);
        assert!(statuses(&owner, &pending).is_empty());
        let scenario = scenario.restart_with_horse_taming(true).await;
        let owner = scenario.join(owner_id, "HorseSave").await;
        let loaded = owner.player.world().get_entity_by_uuid(entity_id).unwrap();
        assert_committed(&loaded, owner_id);
        let resumed = owner.player.world().get_entity_by_uuid(pending_id).unwrap();
        assert_eq!(horse(&resumed).get_temper(), 5);
        interact(&scenario, &owner, &resumed);
        script(&resumed, 0, 4);
        opportunity(&scenario, &resumed).await;
        assert!(
            horse(&resumed).is_tame(),
            "reenabled riding must resume committed temper"
        );
        assert_eq!(horse(&resumed).get_owner(), Some(owner_id));
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn feat_horses_s2_legacy_loaded_state_keeps_missing_fields_and_arbitrary_temper() {
    run(async {
        let scenario = Scenario::new().await;
        let owner = scenario.join(Uuid::new_v4(), "HorseLegacy").await;
        let world = owner.player.world();
        for temper in [None, Some(-42), Some(147)] {
            let entity =
                crate::entity::r#type::from_type(&EntityType::HORSE, SPAWN, &world, Uuid::new_v4());
            let mut nbt = NbtCompound::new();
            if let Some(temper) = temper {
                nbt.put_int("Temper", temper);
                nbt.put_bool("Tame", true);
                nbt.put_uuid("Owner", owner.player.gameprofile.id);
                nbt.put_int("Variant", 771);
            }
            entity.read_nbt_non_mut(&nbt);
            world.add_entity_silent(entity.clone());
            script(&entity, 0, 0);
            let saved = record(&entity);
            assert_eq!(
                horse(&entity).get_temper(),
                temper.unwrap_or(0),
                "direct NBT must not clamp arbitrary loaded tags"
            );
            assert_eq!(saved.get_int("Temper"), Some(temper.unwrap_or(0)));
            assert_eq!(horse(&entity).is_tame(), temper.is_some());
            assert_eq!(
                horse(&entity).get_owner(),
                temper.map(|_| owner.player.gameprofile.id)
            );
            if temper.is_some() {
                assert_eq!(horse(&entity).get_variant(), 771);
            }
            scenario.ticks(6).await;
            assert!(
                horse(&entity)
                    .taming_random
                    .rolls
                    .lock()
                    .unwrap()
                    .is_empty()
            );
        }
        scenario.stop().await;
    })
    .await;
}
