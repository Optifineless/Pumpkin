use super::{
    Arc, Fixture, Item, Ordering, Owner, PermissionLvl, TestPlayer, combat_test_support,
    packet_hook,
};
use crate::{
    data::{advancement_data::AdvancementManager, player_server::ServerPlayerData},
    entity::EntityBase,
};
use pumpkin_data::Advancement;
use pumpkin_inventory::Inventory;
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_world::data::player_data::PlayerDataStorage;
use std::time::Duration;

async fn returning_player() -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let player_path = directory.path().join("players");
    let mut server = combat_test_support::server(directory.path());
    let setup_server = Arc::get_mut(&mut server).unwrap();
    setup_server.player_data_storage =
        ServerPlayerData::new(&player_path, Duration::from_secs(300), true);
    setup_server.advancement_manager = Arc::new(AdvancementManager::new(directory.path(), true));
    let world = combat_test_support::world(&server, directory.path());
    server.worlds.store(Arc::new(vec![world.clone()]));
    let mut recipient = TestPlayer::new(&world);
    let uuid = recipient.player.gameprofile.id;
    let mut saved = NbtCompound::new();
    EntityBase::write_nbt(recipient.player.as_ref(), &mut saved);
    // Player.saveWithoutId's inventory/XP tags, also used by older fork builds.
    saved.put_int("XpLevel", 30);
    saved.put_int("XpSeed", 42);
    saved.put_int("SelectedItemSlot", 0);
    let mut item = NbtCompound::new();
    item.put_byte("Slot", 0);
    item.put_string("id", Item::DIRT.registry_key.to_owned());
    item.put_int("count", 3);
    saved.put("Inventory", NbtTag::List(vec![NbtTag::Compound(item)]));
    PlayerDataStorage::new(&player_path, true)
        .save_player_data(&uuid, saved)
        .unwrap();
    // Pre-checker fork advancement JSON; the criterion comes from story/root's game data.
    std::fs::write(
        server.advancement_manager.advancement_path.join(format!("{uuid}.json")),
        r#"{"minecraft:story/root":{"crafting_table":{"secs_since_epoch":1,"nanos_since_epoch":0}}}"#,
    ).unwrap();
    world.players.store(Arc::new(Vec::new()));
    world
        .entity_tracker
        .remove_entity(recipient.player.as_ref(), &world);
    let (player, _) = server
        .add_player(
            recipient.player.client.clone(),
            recipient.player.gameprofile.clone(),
            None,
        )
        .await
        .unwrap();
    recipient.player = player;
    recipient.client().set_player(recipient.player.clone());
    recipient.player.set_client_loaded(true);
    recipient.player.permission_lvl.store(PermissionLvl::Four);
    combat_test_support::publish_empty_chunk(
        &world,
        pumpkin_util::math::vector2::Vector2::new(0, 0),
    );
    Fixture {
        _directory: directory,
        server,
        world,
        player: recipient,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guard_boundary_old_save_real_login_preserves_inventory_xp_and_advancement_callbacks() {
    let fixture = returning_player().await;
    let player = &fixture.player.player;
    assert!(player.has_played_before.load(Ordering::Relaxed));
    assert_eq!(player.experience_level.load(Ordering::Relaxed), 30);
    assert_eq!(player.enchantment_seed.load(Ordering::Relaxed), 42);
    let item = player.inventory.get_stack(0);
    assert_eq!(item.item, &Item::DIRT);
    assert_eq!(item.item_count, 3);
    assert!(
        player
            .advancements
            .try_lock()
            .unwrap()
            .progress
            .map
            .get(&Advancement::STORY_ROOT)
            .unwrap()
            .is_done()
    );
    let calls = packet_hook(&fixture, Owner::Advancements);
    player.tick(&fixture.server);
    assert!(calls.load(Ordering::Relaxed) > 0);
    assert!(
        !player
            .advancements
            .try_lock()
            .unwrap()
            .progress_changed
            .is_empty()
    );
    player.tick(&fixture.server);
    assert!(
        player
            .advancements
            .try_lock()
            .unwrap()
            .progress_changed
            .is_empty()
    );
    fixture.close().await;
}
