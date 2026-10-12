use super::*;
use crate::world::chunker::test_support::{self as watcher_steps, Phase, Step};
use crate::{
    block::entities::chest::ChestBlockEntity,
    entity::{Entity, item::ItemEntity},
};
use pumpkin_data::{item::Item, item_stack::ItemStack};
use pumpkin_inventory::Inventory;
use pumpkin_world::world::BlockFlags;

const CHEST: BlockPos = BlockPos::new(8, 64, 8);

async fn watcher_step(
    scenario: &Scenario,
    steps: &mut UnboundedReceiver<Step>,
    expected: Phase,
) -> Step {
    let step = scenario
        .drive("teleport watcher barrier", steps.recv())
        .await
        .unwrap();
    assert_eq!(
        step.phase, expected,
        "teleport watcher scheduling order changed"
    );
    step
}

pub(super) fn contents(world: &Arc<World>) -> Vec<Uuid> {
    // ChestBlockEntity.saveAdditional/loadAdditional, Entity.saveWithoutId and ItemEntity's item codec.
    world.set_block_state(
        &CHEST,
        Block::CHEST.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let chest = Arc::new(ChestBlockEntity::new(CHEST));
    world.add_block_entity(chest.clone());
    // Change the live inventory after its initial chunk NBT exists; stale NBT must not pass.
    chest.set_stack(0, ItemStack::new(17, &Item::DIAMOND));
    let pig = Scenario::mob(world, &EntityType::PIG, Vector3::new(10.5, 64.0, 10.5));
    let cow = Scenario::mob(world, &EntityType::COW, Vector3::new(12.5, 64.0, 10.5));
    let item: Arc<dyn EntityBase> = Arc::new(ItemEntity::new_with_velocity(
        Entity::new(
            world.clone(),
            Vector3::new(10.5, 64.0, 12.5),
            &EntityType::ITEM,
        ),
        ItemStack::new(5, &Item::GOLD_INGOT),
        Vector3::new(0.0, 0.0, 0.0),
        0,
    ));
    world.add_entity_silent(item.clone());
    [pig, cow, item]
        .iter()
        .map(|entity| entity.get_entity().entity_uuid)
        .collect()
}

pub(super) fn assert_contents(world: &Arc<World>, ids: &[Uuid]) {
    assert_eq!(
        world.entities.load().len(),
        ids.len(),
        "entity count changed across save/load"
    );
    for id in ids {
        assert!(
            world.get_entity_by_uuid(*id).is_some(),
            "saved entity {id} missing after reload"
        );
    }
    let item = world.get_entity_by_uuid(ids[2]).unwrap();
    let stack = item
        .get_item_entity()
        .unwrap()
        .get_item_stack()
        .lock()
        .unwrap()
        .clone();
    assert_eq!(stack.item, &Item::GOLD_INGOT);
    assert_eq!(stack.item_count, 5);
    let chest = world
        .get_block_entity(&CHEST)
        .unwrap_or_else(|| {
            panic!(
                "saved chest missing after reload: state {}",
                world.level.get_block_state(&CHEST)
            )
        })
        .get_inventory()
        .unwrap();
    let stack = chest.get_stack(0);
    assert_eq!(stack.item, &Item::DIAMOND);
    assert_eq!(stack.item_count, 17, "filled chest changed across unload");
}

#[tokio::test]
#[ignore = "known bug: audit row 5"]
async fn s4_destroyed_mount_detaches_and_preserves_passenger_on_reload() {
    run(async {
        // Entity.setRemoved destroys the vehicle link before Entity.save chooses root records.
        let scenario = Scenario::new().await;
        let id = Uuid::new_v4();
        let player = scenario.join(id, "MountSave").await;
        let world = player.player.world();
        let cart = Scenario::mob(
            &world,
            &EntityType::MINECART,
            Vector3::new(10.5, 64.0, 10.5),
        );
        let passenger = Scenario::mob(&world, &EntityType::CHICKEN, Vector3::new(10.5, 64.0, 10.5));
        cart.get_entity()
            .add_passenger(cart.clone(), passenger.clone());
        let passenger_id = passenger.get_entity().entity_uuid;
        scenario.command(&format!("kill {}", cart.get_entity().entity_uuid));
        assert!(
            cart.get_entity().is_removed(),
            "kill command did not remove mount"
        );
        let detached = !passenger.get_entity().has_vehicle();
        scenario.leave(&player).await;
        scenario.join(id, "MountSave").await;
        let preserved = world.get_entity_by_uuid(passenger_id).is_some();
        scenario.stop().await;
        assert!(
            detached,
            "audit row 5: destroyed mount leaves passenger.vehicle set; passenger skipped by save (reloaded={preserved})"
        );
        assert!(preserved, "audit row 5: passenger vanished on chunk reload");
    })
    .await;
}

#[tokio::test]
async fn s5_chunk_unload_reload_preserves_entities_items_and_chest() {
    run(async {
        // ChunkMap.move -> PersistentEntitySectionManager.processChunkUnload/processPendingLoads.
        let scenario = Scenario::new().await;
        let player = scenario.join(Uuid::new_v4(), "Storage").await;
        let world = player.player.world();
        let ids = contents(&world);
        scenario.ticks(3).await;
        let ages: Vec<_> = ids
            .iter()
            .map(|id| {
                world
                    .get_entity_by_uuid(*id)
                    .unwrap()
                    .get_entity()
                    .age
                    .load(std::sync::atomic::Ordering::Relaxed)
            })
            .collect();
        assert!(
            ages.iter().all(|age| *age >= 3),
            "entities must actually tick before unload"
        );
        scenario.prepare_area(&world, Vector2::new(10, 0)).await;
        scenario.command("tp Storage 163.5 64 3.5");
        player.loaded(&scenario.server);
        scenario
            .until("movement unload task", || {
                scenario.server.tasks.is_empty() && player.is_idle()
            })
            .await;
        scenario.ticks(1).await;
        assert!(world.entities.load().is_empty());
        assert!(world.block_entities.get(&Vector2::new(0, 0)).is_none());
        assert!(
            world
                .level
                .get_entity_chunk_sync(&Vector2::new(0, 0))
                .is_none()
        );
        scenario.command("tp Storage 3.5 64 3.5");
        player.loaded(&scenario.server);
        scenario
            .until("movement reload task", || {
                scenario.server.tasks.is_empty() && player.is_idle()
            })
            .await;
        scenario.ticks(1).await;
        assert_contents(&world, &ids);
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn s5_rewatch_while_unload_pending_preserves_live_records() {
    run(async {
        // PersistentEntitySectionManager.processUnloads cancels unload when visibility returns.
        let scenario = Scenario::new().await;
        let player = scenario.join(Uuid::new_v4(), "Rewatch").await;
        let world = player.player.world();
        let ids = contents(&world);
        scenario.ticks(2).await;
        let pos = Vector2::new(0, 0);
        scenario.prepare_area(&world, Vector2::new(10, 0)).await;
        // ChunkMap.move: /tp schedules update_position; pause its last unwatch before retirement.
        let mut away = watcher_steps::capture(|| {
            scenario.command("tp Rewatch 163.5 64 3.5");
        });
        player.loaded(&scenario.server);
        watcher_step(&scenario, &mut away, Phase::Watched)
            .await
            .release
            .send(())
            .unwrap();
        let pending = watcher_step(&scenario, &mut away, Phase::Unwatched).await;
        assert!(
            pending.chunks.contains(&pos),
            "teleport did not unwatch origin"
        );
        assert!(!world.level.is_chunk_watched(&pos));
        let mut back = watcher_steps::capture(|| {
            scenario.command("tp Rewatch 3.5 64 3.5");
        });
        player.loaded(&scenario.server);
        let rewatched = watcher_step(&scenario, &mut back, Phase::Watched).await;
        assert!(
            rewatched.chunks.contains(&pos),
            "return teleport did not rewatch origin"
        );
        assert!(world.level.is_chunk_watched(&pos));
        rewatched.release.send(()).unwrap();
        watcher_step(&scenario, &mut back, Phase::Unwatched)
            .await
            .release
            .send(())
            .unwrap();
        scenario
            .until("teleport entity activation", || player.is_idle())
            .await;
        scenario.ticks(1).await;
        assert!(Scenario::entities_live(&world, pos));
        pending.release.send(()).unwrap();
        scenario
            .until("teleport watcher retirement", || {
                scenario.server.tasks.is_empty()
            })
            .await;
        // Use the real shutdown snapshot and real login activation to expose a saved empty live copy.
        let scenario = scenario.restart().await;
        let world = scenario.world(&Dimension::OVERWORLD);
        scenario.join(Uuid::new_v4(), "ReloadRewatch").await;
        let preserved = world.entities.load().len() == ids.len()
            && ids.iter().all(|id| world.get_entity_by_uuid(*id).is_some());
        if preserved {
            assert_contents(&world, &ids);
        }
        scenario.stop().await;
        assert!(
            preserved,
            "audit row 4: rewatch before pending unload loses the live chunk's entity records"
        );
    })
    .await;
}

#[tokio::test]
async fn s6_stop_restart_preserves_players_entities_chest_and_level_data() {
    run(async {
        // MinecraftServer.stopServer -> PlayerList.saveAll/removeAll and saveAllChunks.
        let scenario = Scenario::new().await;
        let id = Uuid::new_v4();
        let player = scenario.join(id, "Shutdown").await;
        let world = player.player.world();
        let ids = contents(&world);
        player
            .player
            .inventory
            .set_slot(0, ItemStack::new(7, &Item::EMERALD));
        player.player.set_experience(10, 0.0, 160);
        let saved_position = Vector3::new(5.5, 64.0, 5.5);
        scenario.command(&format!(
            "tp Shutdown {} {} {}",
            saved_position.x, saved_position.y, saved_position.z
        ));
        player.loaded(&scenario.server);
        scenario.command("time set 6000");
        scenario.command("gamerule keep_inventory true");
        scenario.ticks(3).await;
        let scenario = scenario.restart().await;
        let player = scenario.join(id, "Shutdown").await;
        let world = player.player.world();
        assert_eq!(player.player.position(), saved_position);
        assert_eq!(player.player.inventory.get_slot(0).item, &Item::EMERALD);
        assert_eq!(player.player.inventory.get_slot(0).item_count, 7);
        assert_eq!(
            player
                .player
                .experience_level
                .load(std::sync::atomic::Ordering::Relaxed),
            10
        );
        assert!(scenario.server.level_info.load().game_rules.keep_inventory);
        assert_contents(&world, &ids);
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
#[ignore = "new: shutdown/restart resets saved game time and world clock"]
async fn s6_stop_restart_preserves_world_clock() {
    run(async {
        // PrimaryLevelData.parse/setTagData persist game time; ServerClockManager.packState/init
        // persist the 26.3 world clock independently of that game time.
        let scenario = Scenario::new().await;
        let id = Uuid::new_v4();
        scenario.join(id, "ClockSave").await;
        scenario.command("time set 6000");
        let world = scenario.world(&Dimension::OVERWORLD);
        let age_before_ticks = world.level_time.lock().unwrap().world_age;
        scenario.ticks(3).await;
        let before = {
            let time = world.level_time.lock().unwrap();
            (time.world_age, time.time_of_day)
        };
        assert_eq!(
            before,
            (age_before_ticks + 3, 6003),
            "clock setup did not run real ticks"
        );
        let scenario = scenario.restart().await;
        scenario.join(id, "ClockSave").await;
        let world = scenario.world(&Dimension::OVERWORLD);
        let after = {
            let time = world.level_time.lock().unwrap();
            (time.world_age, time.time_of_day)
        };
        scenario.stop().await;
        assert_eq!(
            after, before,
            "shutdown/restart resets saved game time and world clock"
        );
    })
    .await;
}
