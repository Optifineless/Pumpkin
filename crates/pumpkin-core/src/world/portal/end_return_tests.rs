#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "Portal regressions require successful fixtures"
)]

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use futures::future::BoxFuture;
use pumpkin_data::{
    Block,
    block_properties::{
        BedPart, HorizontalFacing, RespawnAnchorLikeProperties, WhiteBedLikeProperties,
    },
    dimension::Dimension,
    entity::EntityType,
};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use pumpkin_world::world::BlockFlags;
use tokio::sync::oneshot;

use super::end_return_test_support::{
    DEADLINE, Fixture, PERSONAL, PORTAL, contact, load_personal, transfer,
};
use crate::{
    command::CommandSender,
    entity::{Entity, EntityBase, living::LivingEntity},
    net::java::combat_test_support::TestPlayer,
    plugin::{
        EventHandler, EventPriority,
        api::events::{
            entity::entity_portal::EntityPortalEvent,
            player::player_change_world::PlayerChangeWorldEvent,
        },
    },
    server::Server,
    world::World,
};

pub(super) fn install_bed(world: &Arc<World>) {
    let mut props = WhiteBedLikeProperties::default(&Block::WHITE_BED);
    props.facing = HorizontalFacing::South;
    props.part = BedPart::Foot;
    world.set_block_state(
        &BlockPos::new(8, 64, 7),
        props.to_state_id(&Block::WHITE_BED),
        BlockFlags::FORCE_STATE,
    );
    props.part = BedPart::Head;
    world.set_block_state(
        &PERSONAL,
        props.to_state_id(&Block::WHITE_BED),
        BlockFlags::FORCE_STATE,
    );
}

pub(super) fn install_anchor(world: &Arc<World>) {
    let mut props = RespawnAnchorLikeProperties::default(&Block::RESPAWN_ANCHOR);
    props.charges = 3;
    world.set_block_state(
        &PERSONAL,
        props.to_state_id(&Block::RESPAWN_ANCHOR),
        BlockFlags::FORCE_STATE,
    );
}

pub(super) fn assert_anchor(world: &World) {
    assert_eq!(
        RespawnAnchorLikeProperties::from_state_id(world.get_block_state_id(&PERSONAL)).charges,
        3,
        "End return must not spend an anchor charge"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g16_end_return_uses_saved_bed_from_both_save_formats() {
    for (legacy, yaw) in [(false, 27.0), (true, 0.0), (false, 180.0)] {
        let fixture = Fixture::new().await;
        install_bed(&fixture.overworld);
        let mut client = fixture.returning_player();
        let player = client.player.clone();
        load_personal(&player, &Dimension::OVERWORLD, legacy);
        player.respawn_point.lock().unwrap().as_mut().unwrap().yaw = yaw;
        let before = player.respawn_point.lock().unwrap().clone();
        let valid = tokio::time::timeout(DEADLINE, player.calculate_respawn_point())
            .await
            .unwrap()
            .unwrap();
        let expected_x = if yaw == 0.0 { 7.5 } else { 9.5 };
        assert_eq!(
            valid.position,
            Vector3::new(expected_x, 64.0, 8.5),
            "vanilla 26.3 AbstractBedBlock.findStandUpPosition selects a yaw-ordered clear exit"
        );
        assert_eq!(valid.yaw, yaw);
        transfer(&fixture, &mut client).await;
        assert!(Arc::ptr_eq(&player.world(), &fixture.overworld));
        assert_eq!(
            player.position(),
            Vector3::new(if yaw == 0.0 { 7.5 } else { 9.5 }, 64.0, 8.5),
            "registered End return must use the valid saved bed"
        );
        assert_eq!(
            player.get_entity().yaw.load(),
            if yaw == 0.0 { -90.0 } else { 90.0 }
        );
        assert_eq!(*player.respawn_point.lock().unwrap(), before);
        assert!(player.seen_credits.load(Ordering::Relaxed));
        fixture.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g16_end_return_retains_anchor_dimension_and_charges() {
    let fixture = Fixture::new().await;
    install_anchor(&fixture.nether);
    let mut client = fixture.returning_player();
    let player = client.player.clone();
    load_personal(&player, &Dimension::THE_NETHER, false);
    transfer(&fixture, &mut client).await;
    assert!(
        Arc::ptr_eq(&player.world(), &fixture.nether),
        "personal spawn's dimension must survive the asynchronous transfer"
    );
    assert_eq!(player.position(), Vector3::new(8.5, 64.0, 7.5));
    assert!(player.get_entity().yaw.load().abs() < 0.001);
    assert_anchor(&fixture.nether);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g16_end_return_missing_obstructed_and_absent_spawn_use_terrain() {
    for kind in ["absent", "missing", "obstructed", "unloaded"] {
        let fixture = Fixture::new().await;
        let mut client = fixture.returning_player();
        let player = client.player.clone();
        if kind != "absent" {
            let dimension = if kind == "unloaded" {
                Dimension::THE_NETHER
            } else {
                Dimension::OVERWORLD
            };
            load_personal(&player, &dimension, false);
        }
        if kind == "unloaded" {
            install_anchor(&fixture.nether);
            tokio::time::timeout(
                DEADLINE,
                fixture.server.unload_world("minecraft:the_nether"),
            )
            .await
            .unwrap()
            .unwrap();
            assert!(fixture.nether.level.cancel_token.is_cancelled());
        }
        if kind == "obstructed" {
            install_bed(&fixture.overworld);
            for x in 5..=11 {
                for z in 5..=11 {
                    for y in 63..=66 {
                        let pos = BlockPos::new(x, y, z);
                        if pos != PERSONAL {
                            fixture.overworld.set_block_state(
                                &pos,
                                Block::STONE.default_state.id,
                                BlockFlags::FORCE_STATE,
                            );
                        }
                    }
                }
            }
        }
        let before = player.respawn_point.lock().unwrap().clone();
        transfer(&fixture, &mut client).await;
        assert!(Arc::ptr_eq(&player.world(), &fixture.overworld));
        assert_eq!(
            player.position(),
            Vector3::new(2.5, 64.0, 2.5),
            "{kind} personal spawn must fall back onto actual terrain, below placeholder Y=200"
        );
        assert_eq!(
            fixture.server.level_info.load().spawn_y,
            200,
            "return must not rewrite the saved shared spawn suggestion"
        );
        assert_eq!(*player.respawn_point.lock().unwrap(), before);
        fixture.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g10_end_fallback_rejects_waterlogged_body_space() {
    for kind in ["waterlogged", "dry_open", "fluid_below_body", "water"] {
        let fixture = Fixture::new().await;
        let candidate = BlockPos::new(2, 64, 2);
        match kind {
            "waterlogged" => {
                let wet_sign = Block::OAK_SIGN.default_state.set_waterlogged(true).unwrap();
                assert!(wet_sign.is_waterlogged());
                fixture
                    .overworld
                    .set_block_state(&candidate, wet_sign.id, BlockFlags::FORCE_STATE);
                assert!(
                    !fixture
                        .overworld
                        .get_fluid_and_fluid_state(&candidate)
                        .1
                        .is_empty,
                    "waterlogged sign must expose contained fluid to the fallback"
                );
            }
            "dry_open" => {
                fixture.overworld.set_block_state(
                    &candidate,
                    Block::OAK_SIGN.default_state.id,
                    BlockFlags::FORCE_STATE,
                );
            }
            "fluid_below_body" => {
                fixture.overworld.set_block_state(
                    &candidate.down().down(),
                    Block::WATER.default_state.id,
                    BlockFlags::FORCE_STATE,
                );
            }
            "water" => {
                fixture.overworld.set_block_state(
                    &candidate,
                    Block::WATER.default_state.id,
                    BlockFlags::FORCE_STATE,
                );
            }
            _ => {}
        }
        fixture.server.level_info.rcu(|info| {
            let mut info = (**info).clone();
            info.game_rules.respawn_radius = 0;
            info
        });
        let mut client = fixture.returning_player();
        transfer(&fixture, &mut client).await;
        let expected_y = if matches!(kind, "waterlogged" | "water") {
            65.0
        } else {
            64.0
        };
        assert_eq!(
            client.player.position(),
            Vector3::new(2.5, expected_y, 2.5),
            "shared fallback for {kind} must follow the actual player-body fluid shape"
        );
        fixture.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g16_end_return_preserves_admin_spawn_on_elevated_platform() {
    let fixture = Fixture::new().await;
    fixture.overworld.set_block_state(
        &BlockPos::new(2, 80, 2),
        Block::STONE.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let source = CommandSender::Console
        .into_source(&fixture.server)
        .with_world(fixture.overworld.clone());
    assert_eq!(
        fixture
            .server
            .command_dispatcher
            .load()
            .execute_input("setworldspawn 2 81 2", &source)
            .unwrap(),
        1
    );
    let mut client = fixture.returning_player();
    transfer(&fixture, &mut client).await;
    assert_eq!(
        client.player.position(),
        Vector3::new(2.5, 81.0, 2.5),
        "supported administrator-selected height must survive spawn adjustment"
    );
    assert_eq!(fixture.server.level_info.load().spawn_y, 81);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g16_entering_end_keeps_player_platform_behavior() {
    let fixture = Fixture::new().await;
    let mut client = TestPlayer::new(&fixture.overworld);
    client.player.get_entity().set_pos(PORTAL.to_centered_f64());
    transfer(&fixture, &mut client).await;
    assert!(Arc::ptr_eq(&client.player.world(), &fixture.end));
    assert_eq!(client.player.position(), Vector3::new(100.5, 49.0, 0.5));
    assert_eq!(client.player.get_entity().yaw.load(), 90.0);
    for x in 98..=102 {
        for z in -2..=2 {
            assert_eq!(
                fixture.end.get_block(&BlockPos::new(x, 48, z)),
                &Block::OBSIDIAN
            );
            for y in 49..=51 {
                assert_eq!(fixture.end.get_block(&BlockPos::new(x, y, z)), &Block::AIR);
            }
        }
    }
    fixture.finish().await;
}

// Same teleport observer used by the existing portal async-residency regressions.
struct Traveler {
    entity: Entity,
    arrival: Mutex<Option<oneshot::Sender<Vector3<f64>>>>,
}
impl EntityBase for Traveler {
    fn get_entity(&self) -> &Entity {
        &self.entity
    }
    fn get_living_entity(&self) -> Option<&LivingEntity> {
        None
    }
    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }
    fn teleport(
        &self,
        position: Vector3<f64>,
        yaw: Option<f32>,
        pitch: Option<f32>,
        world: Arc<World>,
    ) {
        self.entity.teleport(position, yaw, pitch, &world);
        if let Some(done) = self.arrival.lock().unwrap().take() {
            let _ = done.send(position);
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g16_non_player_end_return_keeps_shared_spawn_behavior() {
    let fixture = Fixture::new().await;
    let (done, completed) = oneshot::channel();
    let traveler = Arc::new(Traveler {
        entity: Entity::new(
            fixture.end.clone(),
            PORTAL.to_centered_f64(),
            &EntityType::COW,
        ),
        arrival: Mutex::new(Some(done)),
    });
    fixture.end.add_entity_silent(traveler.clone());
    contact(&fixture.server, traveler.as_ref());
    let position = tokio::time::timeout(DEADLINE, completed)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        position,
        Vector3::new(2.5, 200.0, 2.5),
        "non-player End exits retain their separate destination behavior"
    );
    assert_eq!(traveler.entity.pos.load(), position);
    fixture.finish().await;
}

struct ChangeWorld {
    cancel: bool,
    redirect: Option<Arc<World>>,
    calls: AtomicUsize,
    observed: Mutex<Vec<(Dimension, Dimension, Vector3<f64>)>>,
}
impl EventHandler<PlayerChangeWorldEvent> for ChangeWorld {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerChangeWorldEvent,
    ) -> BoxFuture<'a, ()> {
        self.observed.lock().unwrap().push((
            event.previous_world.dimension.clone(),
            event.new_world.dimension.clone(),
            event.position,
        ));
        self.calls.fetch_add(1, Ordering::Relaxed);
        event.cancelled = self.cancel;
        if let Some(world) = &self.redirect {
            event.new_world = world.clone();
            event.position = Vector3::new(12.5, 64.0, 12.5);
            event.yaw = 71.0;
            event.pitch = 12.0;
        }
        Box::pin(async {})
    }
}

pub(super) async fn drain(fixture: &Fixture, client: &mut TestPlayer) {
    fixture.server.tasks.close();
    tokio::time::timeout(DEADLINE, fixture.server.tasks.wait())
        .await
        .unwrap();
    let player = client.player.clone();
    let java = player.client.java().unwrap();
    tokio::time::timeout(DEADLINE, client.with_outgoing_writer(java.await_tasks()))
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g16_end_return_cancellation_and_redirection_keep_anchor_charges() {
    for cancel in [true, false] {
        let fixture = Fixture::new().await;
        install_anchor(&fixture.nether);
        let mut client = fixture.returning_player();
        let player = client.player.clone();
        load_personal(&player, &Dimension::THE_NETHER, false);
        let initial = player.position();
        let handler = Arc::new(ChangeWorld {
            cancel,
            redirect: (!cancel).then(|| fixture.overworld.clone()),
            calls: AtomicUsize::new(0),
            observed: Mutex::new(Vec::new()),
        });
        fixture
            .server
            .plugin_manager
            .register::<PlayerChangeWorldEvent, _>(handler.clone(), EventPriority::Normal, true);
        contact(&fixture.server, player.as_ref());
        drain(&fixture, &mut client).await;
        assert_eq!(handler.calls.load(Ordering::Relaxed), 1);
        assert_eq!(
            *handler.observed.lock().unwrap(),
            vec![(
                Dimension::THE_END,
                Dimension::THE_NETHER,
                Vector3::new(8.5, 64.0, 7.5)
            )],
            "change-world event must observe the resolved personal dimension and position",
        );
        if cancel {
            assert!(Arc::ptr_eq(&player.world(), &fixture.end));
            assert_eq!(player.position(), initial);
        } else {
            assert!(Arc::ptr_eq(&player.world(), &fixture.overworld));
            assert_eq!(player.position(), Vector3::new(12.5, 64.0, 12.5));
            assert_eq!(player.get_entity().yaw.load(), 71.0);
            assert_eq!(player.get_entity().pitch.load(), 12.0);
        }
        assert_anchor(&fixture.nether);
        fixture.finish().await;
    }
}

struct CloseAtContact {
    removed: bool,
    player: Arc<crate::entity::player::Player>,
    calls: AtomicUsize,
}
impl EventHandler<EntityPortalEvent> for CloseAtContact {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut EntityPortalEvent,
    ) -> BoxFuture<'a, ()> {
        assert_eq!(event.entity_id, self.player.get_entity().entity_id);
        self.calls.fetch_add(1, Ordering::Relaxed);
        if self.removed {
            self.player
                .get_entity()
                .removal_reason
                .store(Some(crate::entity::RemovalReason::Discarded));
            return Box::pin(async {});
        }
        match self.player.client.as_ref() {
            crate::net::ClientPlatform::Java(client) => client.close(),
            crate::net::ClientPlatform::Bedrock(_) => panic!("Java fixture"),
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g16_closed_or_removed_session_at_contact_cannot_commit_return() {
    for removed in [false, true] {
        let fixture = Fixture::new().await;
        install_anchor(&fixture.nether);
        let mut client = fixture.returning_player();
        let player = client.player.clone();
        load_personal(&player, &Dimension::THE_NETHER, false);
        let initial = player.position();
        let handler = Arc::new(CloseAtContact {
            removed,
            player: player.clone(),
            calls: AtomicUsize::new(0),
        });
        fixture
            .server
            .plugin_manager
            .register::<EntityPortalEvent, _>(handler.clone(), EventPriority::Normal, true);
        contact(&fixture.server, player.as_ref());
        drain(&fixture, &mut client).await;
        assert_eq!(handler.calls.load(Ordering::Relaxed), 1);
        assert!(
            Arc::ptr_eq(&player.world(), &fixture.end),
            "closed/removed old session must not transfer (removed={removed})"
        );
        assert_eq!(player.position(), initial);
        assert!(
            fixture
                .end
                .get_player_by_uuid(player.gameprofile.id)
                .is_some_and(|current| Arc::ptr_eq(&current, &player))
        );
        assert!(
            fixture
                .overworld
                .get_player_by_uuid(player.gameprofile.id)
                .is_none()
        );
        assert!(
            fixture
                .nether
                .get_player_by_uuid(player.gameprofile.id)
                .is_none()
        );
        assert_anchor(&fixture.nether);
        fixture.finish().await;
    }
}
