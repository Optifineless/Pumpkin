use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g03_beacon_native_write_clears_fresh_canonical_lock() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let mut fixture = Fixture::new(0);
        let title_tag = styled_name("Canonical-only lock", "gold");
        let title = CustomNameImpl::read_data(&title_tag).unwrap().name;
        let loaded = load_beacon(
            &fixture,
            canonical_beacon(&title_tag, &styled_name("Original key", "red")),
        );
        assert_denied(&mut fixture, &title, "fresh canonical-only lock is active");

        let beacon = loaded
            .as_any()
            .downcast_ref::<BeaconBlockEntity>()
            .expect("loaded entity is a beacon");
        *beacon.lock_key.lock().unwrap() = None;
        fixture.use_beacon();
        assert_open_title(&mut fixture, &title);

        let mut saved = NbtCompound::new();
        loaded.write_internal(&mut saved);
        assert!(
            saved.get("lock").is_none(),
            "clearing removes canonical lock"
        );
        assert!(saved.get("Lock").is_none(), "no legacy lock remains");

        let reloaded = load_beacon(&fixture, saved);
        fixture.use_beacon();
        assert_open_title(&mut fixture, &title);
        let mut saved_again = NbtCompound::new();
        reloaded.write_internal(&mut saved_again);
        assert!(saved_again.get("lock").is_none());
        fixture.finish().await;
    })
    .await
    .expect("bounded fresh canonical-only lock clear regression timed out");
}
