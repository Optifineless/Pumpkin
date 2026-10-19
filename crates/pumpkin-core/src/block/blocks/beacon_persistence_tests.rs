use super::*;
use std::time::Duration;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn beacon_paid_selection_survives_chunk_save_and_reload() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let fixture = Fixture::new(4);
        fixture.use_beacon();
        fixture
            .player
            .player
            .inventory
            .set_stack(0, ItemStack::new(1, &Item::EMERALD));
        click(
            &fixture.player.player,
            &fixture.server,
            28,
            0,
            SlotActionType::QuickMove,
        );
        let payment = beacon_payment(&fixture.player.player);
        assert_eq!(payment.get_stack(0).item_count, 1);

        set_beacon(&fixture.player, Some(2), Some(9));
        assert!(payment.get_stack(0).is_empty());
        assert_eq!(emerald_count(&fixture.player.player), 0);
        assert_eq!(fixture.beacon.primary_effect.load(Ordering::Relaxed), 2);
        assert_eq!(fixture.beacon.secondary_effect.load(Ordering::Relaxed), 9);
        fixture.player.player.close_handled_screen();

        let chunk_pos = fixture.pos.chunk_position();
        let chunk = fixture
            .world
            .level
            .loaded_chunks
            .get(&chunk_pos)
            .unwrap()
            .clone();
        fixture
            .world
            .level
            .chunk_saver
            .save_chunks(&fixture.world.level.level_folder, vec![(chunk_pos, chunk)])
            .await
            .unwrap();
        fixture.world.block_entities.remove(&chunk_pos);
        fixture.world.level.loaded_chunks.remove(&chunk_pos);
        let (send, mut receive) = tokio::sync::mpsc::channel(1);
        fixture
            .world
            .level
            .chunk_saver
            .fetch_chunks(&fixture.world.level.level_folder, &[chunk_pos], send)
            .await;
        let LoadedData::Loaded(reloaded) =
            tokio::time::timeout(Duration::from_secs(10), receive.recv())
                .await
                .expect("beacon chunk reload timed out")
                .expect("beacon chunk reload channel closed")
        else {
            panic!("saved beacon chunk missing")
        };
        fixture
            .world
            .level
            .loaded_chunks
            .insert(chunk_pos, reloaded);
        let reloaded_entity = fixture.world.get_block_entity(&fixture.pos).unwrap();
        let original: Arc<dyn crate::block::entities::BlockEntity> = fixture.beacon.clone();
        assert!(!Arc::ptr_eq(&reloaded_entity, &original));
        let reloaded_beacon = reloaded_entity
            .as_any()
            .downcast_ref::<BeaconBlockEntity>()
            .unwrap();
        let effects = (
            reloaded_beacon.primary_effect.load(Ordering::Relaxed),
            reloaded_beacon.secondary_effect.load(Ordering::Relaxed),
        );
        fixture.finish().await;
        assert_eq!(effects, (2, 9), "paid beacon effects must survive reload");
    })
    .await
    .expect("beacon persistence regression timed out");
}
