use super::*;
use crate::plugin::api::events::world::generic_game::GenericGameEvent;
use crossbeam::channel::bounded;
use pumpkin_data::{
    BlockStateId,
    block_properties::{BedPart, HorizontalFacing, WhiteBedLikeProperties},
    game_event::GameEvent,
};
use std::time::Duration;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_projectile_invalidated_lazy_faces_are_not_consumed() {
    let fixture = Fixture::new();
    let position = crafted_content_pot(&fixture);
    let arrow = launch(&fixture, position, ItemStack::new(1, &Item::ARROW));
    let entity = fixture.world.get_block_entity(&position).unwrap();
    let mut saved = NbtCompound::new();
    entity.write_internal(&mut saved);
    fixture.world.remove_block_entity(&position);
    fixture.world.add_block_entity_nbt(position, &saved);
    let (entered, wait_for_entry) = bounded(1);
    let (resume, wait_for_resume) = bounded(1);
    let calls = AtomicUsize::new(0);
    *fixture.world.persistence_test_hooks.lazy.lock().unwrap() = Some(Arc::new(move || {
        if calls.fetch_add(1, Relaxed) == 0 {
            entered.send(()).unwrap();
            wait_for_resume
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
        }
    }));
    let finished = super::overlap::hit_on_worker(arrow, position);
    wait_for_entry.recv_timeout(Duration::from_secs(5)).unwrap();
    // The supported native/Wasm NBT setter invalidates even an identical pending image.
    fixture.world.add_block_entity_nbt(position, &saved);
    resume.send(()).unwrap();
    finished.recv_timeout(Duration::from_secs(5)).unwrap();
    *fixture.world.persistence_test_hooks.lazy.lock().unwrap() = None;
    if fixture.world.get_block(&position) == &Block::DECORATED_POT {
        let pending = fixture
            .world
            .level
            .read_chunk_sync(&position.chunk_position(), |chunk| {
                chunk
                    .pending_block_entities
                    .lock()
                    .unwrap()
                    .get(&position)
                    .cloned()
            })
            .flatten()
            .unwrap();
        assert_eq!(
            pending, saved,
            "denied removal leaves the pending image unchanged"
        );
        assert_retained(&fixture, position);
        let current = fixture.world.get_block_entity(&position).unwrap();
        let mut after = NbtCompound::new();
        current.write_internal(&mut after);
        // Promotion may add inert raw-source backups; the actual faces and contents stay exact.
        assert_eq!(after.get("sherds"), saved.get("sherds"));
        assert_eq!(after.get("item"), saved.get("item"));
    } else {
        assert_shattered(&fixture, position);
    }
    fixture.shutdown().await;
}

#[derive(Default)]
struct RejectedHitEvents {
    breaks: AtomicUsize,
    destroys: AtomicUsize,
}

impl EventHandler<BlockBreakEvent> for RejectedHitEvents {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        _event: &'a mut BlockBreakEvent,
    ) -> BoxFuture<'a, ()> {
        self.breaks.fetch_add(1, Relaxed);
        Box::pin(async {})
    }
}

impl EventHandler<GenericGameEvent> for RejectedHitEvents {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut GenericGameEvent,
    ) -> BoxFuture<'a, ()> {
        if event.event_key == GameEvent::BlockDestroy.name() {
            self.destroys.fetch_add(1, Relaxed);
        }
        Box::pin(async {})
    }
}

fn replace_pending_pot(
    fixture: &Fixture,
    position: BlockPos,
    block: &Block,
) -> Vec<(BlockPos, BlockStateId)> {
    let states = if block == &Block::RED_BED || block == &Block::STRAW_BED {
        let mut properties = WhiteBedLikeProperties::default(block);
        properties.facing = HorizontalFacing::East;
        properties.part = BedPart::Foot;
        let foot = properties.to_state_id(block);
        properties.part = BedPart::Head;
        vec![
            (position, foot),
            (
                position.offset(BlockDirection::East.to_offset()),
                properties.to_state_id(block),
            ),
        ]
    } else {
        vec![(position, block.default_state.id)]
    };
    for (cell, state) in &states {
        fixture.world.set_block_state(
            cell,
            *state,
            BlockFlags::FORCE_STATE
                | BlockFlags::UPDATE_KNOWN_SHAPE
                | BlockFlags::SKIP_DROPS
                | BlockFlags::SKIP_BLOCK_ENTITY_REPLACED_CALLBACK,
        );
        fixture
            .world
            .set_block_entity_custom_data(cell, "r7", "marker", NbtTag::Int(cell.0.x));
    }
    states
}

async fn prepared_hit_keeps_replacement(block: &Block, adventure: bool) {
    let fixture = Fixture::new();
    let position = crafted_content_pot(&fixture);
    let ammunition = if adventure {
        permitted_arrow()
    } else {
        ItemStack::new(1, &Item::ARROW)
    };
    fixture.player.player.gamemode.store(if adventure {
        GameMode::Adventure
    } else {
        GameMode::Survival
    });
    let arrow = launch(&fixture, position, ammunition);
    let mut saved = NbtCompound::new();
    fixture
        .world
        .get_block_entity(&position)
        .unwrap()
        .write_internal(&mut saved);
    fixture.world.remove_block_entity(&position);
    fixture.world.add_block_entity_nbt(position, &saved);
    let events = Arc::new(RejectedHitEvents::default());
    fixture
        .server
        .plugin_manager
        .register::<BlockBreakEvent, _>(events.clone(), EventPriority::Normal, true);
    fixture
        .server
        .plugin_manager
        .register::<GenericGameEvent, _>(events.clone(), EventPriority::Normal, true);
    let (entered, wait_for_entry) = bounded(1);
    let (resume, wait_for_resume) = bounded(1);
    let calls = AtomicUsize::new(0);
    *fixture.world.persistence_test_hooks.lazy.lock().unwrap() = Some(Arc::new(move || {
        if calls.fetch_add(1, Relaxed) == 0 {
            entered.send(()).unwrap();
            wait_for_resume
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
        }
    }));
    let finished = super::overlap::hit_on_worker(arrow, position);
    wait_for_entry.recv_timeout(Duration::from_secs(5)).unwrap();
    let replacement = replace_pending_pot(&fixture, position, block);
    resume.send(()).unwrap();
    finished.recv_timeout(Duration::from_secs(5)).unwrap();
    *fixture.world.persistence_test_hooks.lazy.lock().unwrap() = None;
    for (cell, state) in replacement {
        assert_eq!(
            fixture.world.get_block_state_id(&cell),
            state,
            "prepared hit must preserve {}",
            block.name
        );
        assert_eq!(
            fixture
                .world
                .get_block_entity_custom_data(&cell, "r7", "marker"),
            Some(NbtTag::Int(cell.0.x))
        );
    }
    assert_eq!(
        events.breaks.load(Relaxed),
        0,
        "mismatched preparation must not dispatch a fresh break"
    );
    assert_eq!(events.destroys.load(Relaxed), 0);
    assert!(
        fixture
            .world
            .entities
            .load()
            .iter()
            .all(|entity| entity.get_item_entity().is_none())
    );
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_prepared_hit_preserves_replacement_beds() {
    for block in [&Block::RED_BED, &Block::STRAW_BED] {
        prepared_hit_keeps_replacement(block, false).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_prepared_hit_adventure_pot_predicate_preserves_replacement_bed() {
    prepared_hit_keeps_replacement(&Block::RED_BED, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_prepared_hit_rejects_other_replacement_states() {
    for block in [&Block::STONE, &Block::AIR] {
        prepared_hit_keeps_replacement(block, false).await;
    }
}
