use super::{
    Arc, Biome, Block, BlockPos, DamageType, DeathTestWorld, Dimension, Duration, EntityBase,
    EntityType, GameMode, NBTStorage, NbtCompound, Player, RespawnPoint, SClientCommand,
    TestPlayer, Vector2, Vector3, World, proto, publish,
};
use pumpkin_world::data::player_data::PlayerDataStorage;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lifecycle_pending_root_vehicle_from_old_save_cannot_reattach_after_life_end() {
    for phase in 0..3 {
        tokio::time::timeout(Duration::from_secs(60), async {
            let fixture = DeathTestWorld::new().await;
            let world = fixture.world();
            publish(&world, proto(&Biome::PLAINS, &Block::STONE));
            let pos = Vector2::new(0, 0);
            let chunk = world.level.get_entity_chunk(pos).await.unwrap();
            world.make_chunk_entities_live(&chunk, None);
            let mut client = TestPlayer::new(&world);
            let player = client.player.clone();
            player.get_entity().set_pos(Vector3::new(8.5, 64.0, 8.5));
            let mount = fixture.mob(&EntityType::MINECART);
            mount.get_entity().set_pos(Vector3::new(8.5, 64.0, 8.5));
            let mount_uuid = mount.get_entity().entity_uuid;
            load_legacy_pending_vehicle(&world, &player, &mount, phase == 1, &fixture).await;
            *player.respawn_point.lock().unwrap() = Some(RespawnPoint {
                dimension: Dimension::OVERWORLD,
                position: BlockPos::new(8, 64, 8),
                yaw: 0.0,
                force: true,
            });
            match phase {
                0 => {
                    player.living_entity.damage(
                        player.as_ref(),
                        f32::MAX,
                        DamageType::GENERIC_KILL,
                    );
                }
                1 => {
                    player.living_entity.health.store(0.0);
                    client.client().handle_client_status(
                        &player,
                        &SClientCommand::new(SClientCommand::PERFORM_RESPAWN.into()),
                    );
                    client
                        .with_outgoing_writer(player.client.java().unwrap().await_tasks())
                        .await;
                }
                _ => {
                    assert!(player.set_gamemode(GameMode::Spectator));
                }
            }
            // Drive the real chunk-load path that consumes a saved RootVehicle.Attach.
            world.remove_entities_in_chunks([pos]).await;
            world.level.clean_entity_chunks([pos]);
            let chunk = world.level.get_entity_chunk(pos).await.unwrap();
            world.make_chunk_entities_live(&chunk, Some(&player));
            let restored_mount = world.get_entity_by_uuid(mount_uuid).unwrap();
            assert!(
                !player.get_entity().has_vehicle(),
                "pending legacy mount attached after phase {phase}"
            );
            assert!(!restored_mount.get_entity().has_passengers());
            player.storage_session.lock().unwrap().take();
            fixture.server.shutdown().await;
        })
        .await
        .unwrap();
    }
}

async fn load_legacy_pending_vehicle(
    world: &Arc<World>,
    player: &Arc<Player>,
    mount: &Arc<dyn EntityBase>,
    vanilla_shape: bool,
    fixture: &DeathTestWorld,
) {
    let mut saved = NbtCompound::new();
    NBTStorage::write_nbt(player.as_ref(), &mut saved);
    // ServerPlayer.saveParentVehicle stores Entity and Attach; older fork saves only Attach.
    let mut root_vehicle = NbtCompound::new();
    root_vehicle.put_uuid("Attach", mount.get_entity().entity_uuid);
    if vanilla_shape {
        let mut entity = NbtCompound::new();
        mount.write_nbt(&mut entity);
        root_vehicle.put_compound("Entity", entity);
    }
    saved.put_compound("RootVehicle", root_vehicle);
    let legacy = PlayerDataStorage::new(
        world.level.level_folder.root_folder.join("players/data"),
        true,
    );
    legacy
        .save_player_data(&player.gameprofile.id, saved)
        .unwrap();
    let (session, saved) = fixture
        .server
        .player_data_storage
        .load_data(&player.gameprofile.id)
        .await
        .unwrap();
    NBTStorage::read_nbt_non_mut(player.as_ref(), &saved.unwrap());
    *player.storage_session.lock().unwrap() = Some(session);
}
