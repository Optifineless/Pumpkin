use super::*;
use crate::world::explosion::{BlockInteraction, Explosion};
use pumpkin_world::chunk_system::residency::ChunkResidency;
use std::sync::atomic::Ordering;

const RETIRED_CHEST: BlockPos = BlockPos::new(16, 64, 8);
const PLACED_CHEST: BlockPos = BlockPos::new(17, 64, 8);

async fn retire_loaded_neighbour(scenario: &Scenario, world: &Arc<World>) -> ChunkResidency {
    let pos = RETIRED_CHEST.chunk_position();
    let mut residency = ChunkResidency::new(world.level.chunk_loading.clone());
    residency.add(pos);
    scenario
        .drive(
            "neighbour terrain readiness",
            world.level.get_or_fetch_chunk(pos, |_| ()),
        )
        .await
        .unwrap();
    world.set_block_state(
        &RETIRED_CHEST,
        Block::CHEST.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let chest = world.get_block_entity(&RETIRED_CHEST).unwrap();
    chest
        .get_inventory()
        .unwrap()
        .set_stack(0, ItemStack::new(7, &Item::EMERALD));
    assert!(Scenario::entities_live(world, pos));
    assert!(world.level.mark_chunk_as_not_watched(pos).await);
    scenario.ticks(1).await;
    assert!(!Scenario::entities_live(world, pos));
    assert!(world.level.is_chunk_loaded(&pos));
    assert!(world.block_entities.get(&pos).is_none());
    residency
}

fn dropped_count(world: &World, item: &'static Item) -> u32 {
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

async fn save_evict_reload(
    scenario: &Scenario,
    player: &Session,
    world: &Arc<World>,
    residency: ChunkResidency,
) {
    let pos = RETIRED_CHEST.chunk_position();
    assert!(!Scenario::entities_live(world, pos));
    let old = world.level.loaded_chunks.get(&pos).unwrap().clone();
    scenario
        .drive("reopened neighbour save", world.save())
        .await;
    drop(residency);
    scenario.prepare_area(world, Vector2::new(10, 0)).await;
    scenario.command("tp RetiredNeighbour 163.5 64 3.5");
    player.loaded(&scenario.server);
    scenario
        .until("reopened neighbour watcher departure", || {
            player.is_idle() && scenario.server.tasks.is_empty()
        })
        .await;
    scenario.ticks(1).await;
    scenario
        .drive("reopened neighbour terrain eviction", async {
            while world.level.is_chunk_loaded(&pos) {
                world.level.should_unload.store(true, Ordering::Relaxed);
                world.level.level_channel.notify();
                // New explosion drops may first need async entity storage, then its next tick.
                scenario.ticks(1).await;
            }
        })
        .await;
    let reloaded = scenario
        .drive(
            "reopened neighbour terrain disk reload",
            world.level.get_or_fetch_chunk(pos, Clone::clone),
        )
        .await
        .unwrap();
    assert!(
        !Arc::ptr_eq(&old, &reloaded),
        "terrain reload reused the retired incarnation"
    );
}

#[tokio::test]
async fn explosion_retired_loaded_neighbour_drops_captured_chest_once_and_persists() {
    run(async {
        let scenario = Scenario::new().await;
        let player = scenario.join(Uuid::new_v4(), "RetiredNeighbour").await;
        let world = player.player.world();
        let residency = retire_loaded_neighbour(&scenario, &world).await;
        // ServerExplosion.explode crosses the loaded chunk boundary from its live neighbour.
        let explosion = Explosion::new(
            4.0,
            Vector3::new(15.5, 64.5, 8.5),
            BlockInteraction::Destroy,
        );
        explosion.explode(&world);
        assert_eq!(
            world.get_block_state_id(&RETIRED_CHEST),
            Block::AIR.default_state.id,
            "explosion dropped loot but left the retired chest block"
        );
        assert_eq!(dropped_count(&world, &Item::CHEST), 1);
        assert_eq!(dropped_count(&world, &Item::EMERALD), 7);
        save_evict_reload(&scenario, &player, &world, residency).await;
        assert_eq!(
            world.get_block_state_id(&RETIRED_CHEST),
            Block::AIR.default_state.id,
            "destroyed retired chest returned after terrain disk reload"
        );
        assert!(world.get_block_entity(&RETIRED_CHEST).is_none());
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn command_chest_in_retired_loaded_chunk_persists_after_save_and_terrain_reload() {
    run(async {
        let scenario = Scenario::new().await;
        let player = scenario.join(Uuid::new_v4(), "RetiredNeighbour").await;
        let world = player.player.world();
        let residency = retire_loaded_neighbour(&scenario, &world).await;
        scenario.command("setblock 17 64 8 minecraft:chest");
        assert_eq!(
            world.get_block_state_id(&PLACED_CHEST).to_block(),
            &Block::CHEST,
            "setblock silently refused chest placement in loaded retired terrain"
        );
        scenario.command("item replace block 17 64 8 container.0 with minecraft:diamond 3");
        save_evict_reload(&scenario, &player, &world, residency).await;
        for (position, item, count) in [
            (RETIRED_CHEST, &Item::EMERALD, 7),
            (PLACED_CHEST, &Item::DIAMOND, 3),
        ] {
            let inventory = world
                .get_block_entity(&position)
                .expect("saved reopened chest missing after terrain reload")
                .get_inventory()
                .unwrap();
            let stack = inventory.get_stack(0);
            assert_eq!(stack.item, item);
            assert_eq!(stack.item_count, count);
        }
        scenario.stop().await;
    })
    .await;
}
