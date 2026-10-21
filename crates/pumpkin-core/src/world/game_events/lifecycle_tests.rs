use super::test_support::{Capture, watch_world};
use crate::{
    entity::{Entity, EntityBase, item::ItemEntity, projectile::arrow::ArrowEntity},
    plugin::EventPriority,
    server::scenario::{Scenario, Session, run},
};
use pumpkin_data::{entity::EntityType, item::Item, item_stack::ItemStack};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::{
    VarInt,
    java::server::play::{SClientCommand, SUseItem},
};
use pumpkin_util::{Hand, math::vector3::Vector3};
use std::{
    path::Path,
    sync::{Arc, atomic::Ordering::Relaxed},
};
use uuid::Uuid;

async fn transfer(s: &Scenario, session: &Session, dimension: &str) {
    let teleport = session.player.teleport_id_count.load(Relaxed);
    s.command(&format!(
        "execute in minecraft:{dimension} run tp Indexed 3.5 64 3.5"
    ));
    s.until("indexed source dimension transfer", || {
        session.player.world().dimension.minecraft_name == format!("minecraft:{dimension}")
            && session.player.teleport_id_count.load(Relaxed) > teleport
            && s.server.tasks.is_empty()
            && session.is_idle()
    })
    .await;
    session.loaded(&s.server);
}

fn persisted_entities(world: &Arc<crate::world::World>, player: &Session) -> Vec<Uuid> {
    let pig = Scenario::mob(world, &EntityType::PIG, Vector3::new(10.5, 64.0, 10.5));
    pig.get_living_entity().unwrap().set_health(7.0);
    pig.get_living_entity()
        .unwrap()
        .set_attribute_base(&pumpkin_data::attributes::Attributes::MOVEMENT_SPEED, 0.123);
    pig.get_mob().unwrap().as_ageable().unwrap().set_age(-24000);
    let mut gold = ItemStack::new(5, &Item::GOLD_INGOT);
    gold.set_custom_name("IndexedGold".into());
    let item: Arc<dyn EntityBase> = Arc::new(ItemEntity::new_with_velocity(
        Entity::new(
            world.clone(),
            Vector3::new(10.5, 64.0, 12.5),
            &EntityType::ITEM,
        ),
        gold,
        Vector3::default(),
        0,
    ));
    world.add_entity_silent(item.clone());
    let arrow = Arc::new(ArrowEntity::new(
        Entity::new(
            world.clone(),
            Vector3::new(12.5, 66.0, 12.5),
            &EntityType::ARROW,
        ),
        Some(player.player.entity_id()),
    ));
    world.add_entity_silent(arrow.clone());
    arrow.projectile.tick(arrow.get_entity());
    [
        pig.get_entity().entity_uuid,
        item.get_entity().entity_uuid,
        arrow.get_entity().entity_uuid,
    ]
    .to_vec()
}

fn assert_rebuilt(world: &crate::world::World, ids: &[Uuid]) {
    for uuid in ids {
        let entity = world
            .get_entity_by_uuid(*uuid)
            .expect("persistent entity survives real restart");
        let membership = world
            .game_event_source_membership(entity.get_entity().entity_id)
            .unwrap();
        assert!(std::ptr::eq(
            membership.entity().unwrap().get_entity(),
            entity.get_entity()
        ));
        let mut nbt = pumpkin_nbt::compound::NbtCompound::new();
        entity.write_nbt(&mut nbt);
        assert!(nbt.child_tags.keys().all(|key| !key.contains("game_event")
            && !key.contains("source_index")
            && !key.contains("source_membership")));
    }
    let pig = world.get_entity_by_uuid(ids[0]).unwrap();
    assert_eq!(pig.get_living_entity().unwrap().health.load(), 7.0);
    let item = world.get_entity_by_uuid(ids[1]).unwrap();
    assert_eq!(
        item.get_item_entity()
            .unwrap()
            .get_item_stack()
            .lock()
            .unwrap()
            .item_count,
        5
    );
}

fn copy_tree(source: &Path, destination: &Path) {
    std::fs::create_dir_all(destination).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s2_real_login_respawn_rejoin_transfer_and_restarts() {
    run(async {
        let mut s = Scenario::new_with_game_events(false).await;
        let id = Uuid::new_v4();
        let (world, ids, old_runtime) = first_session(&s, id).await;
        s = s.restart_with_game_events(true).await;
        for old in &old_runtime {
            assert!(
                world.game_event_source_membership(*old).is_none(),
                "retained world must close source authority on shutdown"
            );
        }
        let restored = s.join(id, "Indexed").await;
        assert_eq!(restored.player.inventory().get_stack(4).item_count, 17);
        assert_eq!(restored.player.living_entity.health.load(), 13.0);
        let restored_world = restored.player.world();
        assert_rebuilt(&restored_world, &ids);
        for old in old_runtime {
            assert!(
                restored_world.game_event_source_membership(old).is_none(),
                "restart must not retain old runtime handles"
            );
        }
        let events = watch_world(&restored_world);
        restored_world.game_event_item_interact_finish(restored.player.get_entity());
        assert_eq!(
            events.lock().unwrap().forwarded[0]
                .source
                .as_ref()
                .unwrap()
                .uuid,
            id
        );
        s = s.restart_with_game_events(false).await;
        let restored = s.join(id, "Indexed").await;
        assert_rebuilt(&restored.player.world(), &ids);
        assert_eq!(restored.player.inventory().get_stack(4).item_count, 17);
        s.stop().await;
        if let Some(path) = std::env::var_os("S2_EXPORT_WORLD") {
            let target = std::path::PathBuf::from(path);
            assert!(!target.exists(), "export destination must be new");
            copy_tree(s.directory.path(), &target);
        }
    })
    .await;
}

async fn first_session(s: &Scenario, id: Uuid) -> (Arc<crate::world::World>, Vec<Uuid>, Vec<i32>) {
    let first = s.join(id, "Indexed").await;
    let world = first.player.world();
    let sink = watch_world(&world);
    s.server
        .plugin_manager
        .register::<crate::plugin::api::events::world::generic_game::GenericGameEvent, _>(
            Arc::new(Capture {
                player: first.player.clone(),
                sink: sink.clone(),
                edit: None,
            }),
            EventPriority::Normal,
            true,
        );
    first
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::new(1, &Item::FISHING_ROD));
    world.game_event_sources.probes.store(0, Relaxed);
    first.send(
        &s.server,
        &SUseItem {
            hand: VarInt(0),
            sequence: VarInt(1),
            yaw: 0.0,
            pitch: 0.0,
        },
    );
    assert_eq!(sink.lock().unwrap().plugins.len(), 1);
    assert!(sink.lock().unwrap().forwarded.is_empty());
    assert_eq!(
        world.game_event_sources.probes.load(Relaxed),
        0,
        "OFF through real login/cast performs no capture"
    );
    transfer(s, &first, "the_nether").await;
    assert!(
        world
            .game_event_source_membership(first.player.entity_id())
            .is_none()
    );
    assert!(
        first
            .player
            .world()
            .game_event_source_membership(first.player.entity_id())
            .is_some()
    );
    transfer(s, &first, "overworld").await;
    let life = first.player.living_entity.damage_lifecycle();
    s.command("kill Indexed");
    first.send(
        &s.server,
        &SClientCommand::new(VarInt(SClientCommand::PERFORM_RESPAWN)),
    );
    s.until("indexed source real respawn", || s.server.tasks.is_empty())
        .await;
    first.confirm_teleport(&s.server);
    assert!(first.player.living_entity.damage_lifecycle() > life);
    assert!(first.player.living_entity.health.load() > 0.0);
    s.leave(&first).await;
    assert!(
        world
            .game_event_source_membership(first.player.entity_id())
            .is_none()
    );
    let rejoined = s.join(id, "Indexed").await;
    assert!(!Arc::ptr_eq(&first.player, &rejoined.player));
    s.drive(
        "late old session removal",
        world.remove_player(&first.player, false),
    )
    .await;
    assert!(
        world
            .game_event_source_membership(rejoined.player.entity_id())
            .is_some()
    );
    rejoined
        .player
        .inventory()
        .set_stack(4, ItemStack::new(17, &Item::EMERALD));
    rejoined.player.living_entity.set_health(13.0);
    let ids = persisted_entities(&world, &rejoined);
    let old_runtime: Vec<_> = ids
        .iter()
        .map(|id| {
            world
                .get_entity_by_uuid(*id)
                .unwrap()
                .get_entity()
                .entity_id
        })
        .collect();
    (world, ids, old_runtime)
}
