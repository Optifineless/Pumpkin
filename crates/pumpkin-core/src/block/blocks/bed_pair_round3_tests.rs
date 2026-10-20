use pumpkin_protocol::{
    bedrock::server::player_action::{PlayerActionType, SPlayerAction as BedrockPlayerAction},
    codec::var_ulong::VarULong,
};
use pumpkin_util::sync::Mutex;

use crate::{entity::player::Player, net::bedrock::combat_test_support::TestBedrockPlayer};

use super::{
    Arc, BlockBreakEvent, BlockPos, EntityBase, FOOT, GameMode, HEAD, Item, PermissionLvl,
    PlayerFixture, Relaxed, TestPlayer, TestRecords, VarInt, Vector3, World, destroy, drops,
    finish, place, register,
};

enum Breaker {
    Java(TestPlayer),
    Bedrock(TestBedrockPlayer),
}

impl Breaker {
    fn player(&self) -> &Arc<Player> {
        match self {
            Self::Java(client) => &client.player,
            Self::Bedrock(client) => &client.player,
        }
    }

    fn destroy(&self, world: &World, position: BlockPos) {
        match self {
            Self::Java(client) => destroy(client, world, position),
            Self::Bedrock(client) => {
                let server = world.server.upgrade().unwrap();
                let creative = client.player.gamemode.load() == GameMode::Creative;
                for action in [
                    if creative {
                        PlayerActionType::CreativeDestroyBlock
                    } else {
                        PlayerActionType::StartDestroyBlock
                    },
                    PlayerActionType::StopDestroyBlock,
                ] {
                    client.client().handle_player_action(
                        &client.player,
                        &server,
                        &BedrockPlayerAction {
                            player_runtime_id: VarULong(client.player.entity_id() as u64),
                            action,
                            block_position: position,
                            result_pos: position,
                            face: VarInt(1),
                        },
                    );
                    if creative {
                        break;
                    }
                    client.player.tick_counter.fetch_add(100, Relaxed);
                }
            }
        }
    }

    async fn close(&self) {
        if let Self::Bedrock(client) = self {
            client.close().await;
        }
    }
}

async fn breaker(fixture: &PlayerFixture, bedrock: bool) -> Breaker {
    let java = place(fixture, &Item::RED_BED);
    let breaker = if bedrock {
        Breaker::Bedrock(TestBedrockPlayer::new(&fixture.world).await)
    } else {
        Breaker::Java(java)
    };
    breaker
        .player()
        .get_entity()
        .set_pos(Vector3::new(5.5, 64.0, 2.5));
    breaker
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g05_both_editions_admit_both_halves_with_the_breaking_player() {
    for bedrock in [false, true] {
        for mode in [GameMode::Survival, GameMode::Creative] {
            for position in [FOOT, HEAD] {
                let fixture = PlayerFixture::new();
                let client = breaker(&fixture, bedrock).await;
                client.player().gamemode.store(mode);
                let actor = client.player().gameprofile.id;
                let events = Arc::new(Mutex::<_, TestRecords>::new(Vec::new()));
                let seen = events.clone();
                register::<BlockBreakEvent>(&fixture.world, move |event| {
                    let player = event.player.as_ref().map(|player| player.gameprofile.id);
                    seen.lock().unwrap().push((event.block_position, player));
                    event.cancelled = player.is_none();
                });
                client.destroy(&fixture.world, position);
                let partner = if position == FOOT { HEAD } else { FOOT };
                assert_eq!(
                    *events.lock().unwrap(),
                    [(position, Some(actor)), (partner, Some(actor))]
                );
                assert!(fixture.world.get_block(&FOOT).is_air());
                assert!(fixture.world.get_block(&HEAD).is_air());
                assert_eq!(
                    drops(&fixture.world, &Item::RED_BED),
                    i32::from(mode == GameMode::Survival)
                );
                client.close().await;
                finish(fixture).await;
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g05_both_editions_preserve_a_vetoed_partner_without_retry() {
    for bedrock in [false, true] {
        for mode in [GameMode::Survival, GameMode::Creative] {
            for position in [FOOT, HEAD] {
                let fixture = PlayerFixture::new();
                let client = breaker(&fixture, bedrock).await;
                client.player().gamemode.store(mode);
                let partner = if position == FOOT { HEAD } else { FOOT };
                let expected = fixture.world.get_block_state_id(&partner);
                let actor = client.player().gameprofile.id;
                let events = Arc::new(Mutex::<_, TestRecords>::new(Vec::new()));
                let seen = events.clone();
                register::<BlockBreakEvent>(&fixture.world, move |event| {
                    assert_eq!(event.player.as_ref().unwrap().gameprofile.id, actor);
                    seen.lock().unwrap().push(event.block_position);
                    event.cancelled = event.block_position == partner;
                });
                client.destroy(&fixture.world, position);
                assert_eq!(*events.lock().unwrap(), [position, partner]);
                assert!(fixture.world.get_block(&position).is_air());
                assert_eq!(fixture.world.get_block_state_id(&partner), expected);
                assert_eq!(
                    drops(&fixture.world, &Item::RED_BED),
                    i32::from(mode == GameMode::Survival && position == HEAD)
                );
                client.close().await;
                finish(fixture).await;
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g05_creative_partner_veto_is_not_retried_after_callback_changes_gamemode() {
    for bedrock in [false, true] {
        let fixture = PlayerFixture::new();
        let client = breaker(&fixture, bedrock).await;
        client.player().gamemode.store(GameMode::Creative);
        let head = fixture.world.get_block_state_id(&HEAD);
        let events = Arc::new(Mutex::<_, TestRecords>::new(Vec::new()));
        let seen = events.clone();
        register::<BlockBreakEvent>(&fixture.world, move |event| {
            seen.lock().unwrap().push(event.block_position);
            if event.block_position == HEAD {
                event.cancelled = true;
                event
                    .player
                    .as_ref()
                    .unwrap()
                    .gamemode
                    .store(GameMode::Survival);
            }
        });
        client.destroy(&fixture.world, FOOT);
        assert_eq!(*events.lock().unwrap(), [FOOT, HEAD]);
        assert!(fixture.world.get_block(&FOOT).is_air());
        assert_eq!(fixture.world.get_block_state_id(&HEAD), head);
        assert_eq!(drops(&fixture.world, &Item::RED_BED), 0);
        client.close().await;
        finish(fixture).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g05_both_editions_keep_the_partner_inside_spawn_protection() {
    for bedrock in [false, true] {
        for mode in [GameMode::Survival, GameMode::Creative] {
            let fixture = PlayerFixture::new();
            let client = breaker(&fixture, bedrock).await;
            client.player().gamemode.store(mode);
            client.player().permission_lvl.store(PermissionLvl::Zero);
            let server = fixture.world.server.upgrade().unwrap();
            server.level_info.rcu(|info| {
                let mut info = (**info).clone();
                info.spawn_x = HEAD.0.x + 16;
                info.spawn_z = HEAD.0.z;
                info
            });
            server
                .data
                .operator_config
                .write()
                .unwrap()
                .ops
                .push(pumpkin_config::op::Op::new(
                    uuid::Uuid::new_v4(),
                    "other-operator".to_owned(),
                    PermissionLvl::Four,
                    false,
                ));
            assert!(!fixture.world.is_in_spawn_protection(client.player(), &FOOT));
            assert!(fixture.world.is_in_spawn_protection(client.player(), &HEAD));
            let expected = fixture.world.get_block_state_id(&HEAD);
            let events = Arc::new(Mutex::<_, TestRecords>::new(Vec::new()));
            let seen = events.clone();
            register::<BlockBreakEvent>(&fixture.world, move |event| {
                seen.lock().unwrap().push(event.block_position);
            });
            client.destroy(&fixture.world, FOOT);
            assert!(fixture.world.get_block(&FOOT).is_air());
            assert_eq!(fixture.world.get_block_state_id(&HEAD), expected);
            assert_eq!(*events.lock().unwrap(), [FOOT]);
            assert_eq!(drops(&fixture.world, &Item::RED_BED), 0);
            client.close().await;
            finish(fixture).await;
        }
    }
}
