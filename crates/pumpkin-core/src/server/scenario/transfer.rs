use super::*;
use pumpkin_data::{
    block_properties::PoweredRailLikeProperties, packet::clientbound::play as outgoing,
};
use pumpkin_protocol::{
    VarInt,
    java::server::play::{ActionType, SInteract, SPlayerInput},
    ser::NetworkReadExt,
};
use pumpkin_world::world::BlockFlags;
use std::sync::atomic::Ordering;

// InteractionHand.STREAM_CODEC encodes MAIN_HAND with id 0.
const MAIN_HAND: VarInt = VarInt(0);

async fn transfer(scenario: &Scenario, player: &Session, dimension: &str) {
    let teleport = player.player.teleport_id_count.load(Ordering::Relaxed);
    scenario.command(&format!(
        "execute in minecraft:{dimension} run tp {} 3.5 64 3.5",
        player.player.gameprofile.name
    ));
    scenario
        .until("dimension transfer packets", || {
            player.player.world().dimension.minecraft_name == format!("minecraft:{dimension}")
                && player.player.teleport_id_count.load(Ordering::Relaxed) > teleport
                && scenario.server.tasks.is_empty()
                && player.is_idle()
        })
        .await;
    player.loaded(&scenario.server);
}

#[tokio::test]
async fn s3_dimension_roundtrip_cleans_lists_trackers_and_tickets() {
    run(async {
        // ServerPlayer.teleport -> ServerLevel.removePlayerImmediately(CHANGED_DIMENSION), ChunkMap.removeEntity.
        let scenario = Scenario::new().await;
        let player = scenario.join(Uuid::new_v4(), "Traveller").await;
        for dimension in ["the_nether", "overworld"] {
            let old_world = player.player.world();
            transfer(&scenario, &player, dimension).await;
            let new_world = player.player.world();
            assert_ne!(old_world.uuid, new_world.uuid);
            assert!(
                old_world.players.load().is_empty(),
                "old dimension retains player"
            );
            assert!(
                old_world
                    .entity_tracker
                    .get_tracked_entity(player.player.entity_id())
                    .is_none(),
                "old tracker retains player"
            );
            assert_eq!(new_world.players.load().len(), 1);
            assert!(
                new_world
                    .entity_tracker
                    .get_tracked_entity(player.player.entity_id())
                    .is_some()
            );
            assert!(
                !player.player.get_entity().is_removed(),
                "dimension transfer retired the active player"
            );
            assert!(
                old_world
                    .level
                    .chunk_loading
                    .lock()
                    .unwrap()
                    .ticket
                    .is_empty(),
                "old dimension retains chunk tickets"
            );
            assert!(
                !old_world.level.is_chunk_watched(&Vector2::new(0, 0)),
                "old dimension still watched"
            );
            assert!(
                player.player.held_chunk_tickets.lock().unwrap().is_some(),
                "destination tickets missing"
            );
            scenario.ticks(2).await;
        }
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
#[ignore = "known bug: audit row 18"]
async fn s3_old_dimension_hostile_drops_transferred_target() {
    run(async {
        // LivingEntity.hasLineOfSight rejects another level; TargetGoal.canContinueToUse drops it.
        let scenario = Scenario::new().await;
        let player = scenario.join(Uuid::new_v4(), "CrossTarget").await;
        let observer = scenario.join(Uuid::new_v4(), "Stay").await;
        scenario.command("gamemode spectator Stay");
        let old_world = player.player.world();
        let zombie = super::lifecycle::targeting_mob(&old_world, &player);
        scenario.ticks(4).await;
        assert_eq!(
            zombie
                .get_mob()
                .unwrap()
                .get_mob_entity()
                .get_target()
                .unwrap()
                .get_entity()
                .entity_id,
            player.player.entity_id()
        );
        transfer(&scenario, &player, "the_nether").await;
        scenario.ticks(80).await; // Beyond TargetGoal's 60-tick unseen memory, including selector cadence.
        let dropped = zombie
            .get_mob()
            .unwrap()
            .get_mob_entity()
            .get_target()
            .is_none();
        scenario.leave(&observer).await;
        scenario.stop().await;
        assert!(
            dropped,
            "audit row 18: old-dimension mob retains the transferred target"
        );
    })
    .await;
}

#[tokio::test]
async fn s3_dimension_transfer_preserves_custom_bossbar() {
    run(async {
        // ServerPlayer.teleport never disconnects the player from CustomBossEvents.
        let scenario = Scenario::new().await;
        let player = scenario.join(Uuid::new_v4(), "BarTraveller").await;
        scenario.command("bossbar add scenario:travel \"Travel\"");
        scenario.command("bossbar set scenario:travel players BarTraveller");
        player.packets();
        for dimension in ["the_nether", "overworld"] {
            transfer(&scenario, &player, dimension).await;
            assert!(
                scenario
                    .server
                    .bossbars
                    .lock()
                    .unwrap()
                    .get_bossbar("scenario:travel")
                    .unwrap()
                    .players
                    .contains(&player.player.gameprofile.id)
            );
            for packet in player.packets() {
                let mut bytes = packet.as_ref();
                if bytes.get_var_int().unwrap().0 == outgoing::BOSS_EVENT.to_id(CURRENT_MC_VERSION)
                {
                    let _id = bytes.get_uuid().unwrap();
                    assert_ne!(
                        bytes.get_var_int().unwrap().0,
                        1,
                        "issue #118: dimension transfer removes the bossbar"
                    );
                }
            }
        }
        scenario.stop().await;
    })
    .await;
}

fn mount(scenario: &Scenario, player: &Session, cart: &Arc<dyn EntityBase>) {
    player.send(
        &scenario.server,
        &SInteract {
            entity_id: cart.get_entity().entity_id.into(),
            r#type: VarInt(ActionType::InteractAt as i32),
            hand: Some(MAIN_HAND),
            target_position: Some(Vector3::new(0.5, 0.5, 0.5)),
            sneaking: false,
        },
    );
    assert!(
        player.player.get_entity().has_vehicle(),
        "interact packet did not mount"
    );
    assert!(cart.get_entity().has_passenger(player.player.entity_id()));
}

#[tokio::test]
async fn s4_mount_dismount_and_powered_activator_ejection() {
    run(async {
        // ServerGamePacketListenerImpl.handleInteract/handlePlayerInput; Minecart.activateMinecart.
        let scenario = Scenario::new().await;
        let player = scenario.join(Uuid::new_v4(), "Rider").await;
        let world = player.player.world();
        let cart = Scenario::mob(&world, &EntityType::MINECART, SPAWN);
        let _lock_probe = super::riding_lock_probe::watch(&cart);
        mount(&scenario, &player, &cart);
        player.send(
            &scenario.server,
            &SPlayerInput {
                input: SPlayerInput::SNEAK,
            },
        );
        assert!(!player.player.get_entity().has_vehicle());
        assert!(!cart.get_entity().has_passengers());
        player.loaded(&scenario.server);
        player.player.get_entity().set_pos(SPAWN);
        mount(&scenario, &player, &cart);
        let rail = BlockPos::new(3, 64, 3);
        world.set_block_state(
            &BlockPos::new(3, 63, 3),
            Block::REDSTONE_BLOCK.default_state.id,
            BlockFlags::FORCE_STATE,
        );
        let mut properties = PoweredRailLikeProperties::default(&Block::ACTIVATOR_RAIL);
        properties.powered = true;
        world.set_block_state(
            &rail,
            properties.to_state_id(&Block::ACTIVATOR_RAIL),
            BlockFlags::FORCE_STATE,
        );
        scenario.ticks(1).await;
        assert!(
            !player.player.get_entity().has_vehicle(),
            "audit row 1: powered activator did not eject rider"
        );
        assert!(!cart.get_entity().has_passengers());
        assert!(!player.client().is_closed());
        assert!(
            cart.get_entity().age.load(Ordering::Relaxed) > 0,
            "minecart was never ticked"
        );
        player.loaded(&scenario.server);
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
#[ignore = "new: LpVector3d decoder consumes two bytes for the one-byte zero vector"]
async fn s4_zero_location_interact_packet_is_accepted() {
    run(async {
        // ServerboundInteractPacket.STREAM_CODEC -> LpVec3.read consumes ONE byte for Vec3.ZERO.
        let scenario = Scenario::new().await;
        let player = scenario.join(Uuid::new_v4(), "ZeroInteract").await;
        let cart = Scenario::mob(&player.player.world(), &EntityType::MINECART, SPAWN);
        player.send(
            &scenario.server,
            &SInteract {
                entity_id: cart.get_entity().entity_id.into(),
                r#type: VarInt(ActionType::InteractAt as i32),
                hand: Some(MAIN_HAND),
                target_position: Some(Vector3::new(0.0, 0.0, 0.0)),
                sneaking: false,
            },
        );
        assert!(player.player.get_entity().has_vehicle());
        scenario.stop().await;
    })
    .await;
}
