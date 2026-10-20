#![expect(clippy::unwrap_used, reason = "Regression fixtures must be valid")]

use std::{sync::Arc, time::Duration};

use pumpkin_data::{
    Block, BlockDirection,
    biome::Biome,
    block_properties::{BedPart, WhiteBedLikeProperties},
    entity::{EntityPose, EntityType},
    item::Item,
    item_stack::ItemStack,
};
use pumpkin_protocol::{
    VarInt,
    bedrock::{
        network_item::NetworkItemDescriptor,
        server::inventory_transaction::{
            HandSlot, SInventoryTransaction, TransactionData, UseItemTransactionData,
        },
    },
    codec::var_uint::VarUInt,
    java::server::play::SUseItemOn,
};
use pumpkin_util::{
    GameMode, Hand,
    math::{position::BlockPos, vector3::Vector3},
};

use crate::{
    block::blocks::player_destroy::test_support::PlayerFixture,
    entity::{EntityBase, player::Player, r#type::from_type},
    net::{bedrock::combat_test_support::TestBedrockPlayer, java::combat_test_support::TestPlayer},
    world::{
        World,
        spawn_test_support::{proto, publish},
    },
};

const FOOT: BlockPos = BlockPos::new(8, 64, 8);
const HEAD: BlockPos = BlockPos::new(9, 64, 8);

enum Client {
    Java(TestPlayer),
    Bedrock(TestBedrockPlayer),
}

impl Client {
    async fn new(world: &Arc<World>, bedrock: bool) -> Self {
        if bedrock {
            Self::Bedrock(TestBedrockPlayer::new(world).await)
        } else {
            Self::Java(TestPlayer::new(world))
        }
    }

    fn player(&self) -> &Arc<Player> {
        match self {
            Self::Java(client) => &client.player,
            Self::Bedrock(client) => &client.player,
        }
    }

    fn use_on(&self, world: &World, position: BlockPos) {
        match self {
            Self::Java(client) => client
                .client()
                .handle_use_item_on(
                    &client.player,
                    &SUseItemOn {
                        hand: VarInt(0),
                        position,
                        face: VarInt(BlockDirection::Up as i32),
                        cursor_pos: Vector3::new(0.5, 1.0, 0.5),
                        inside_block: false,
                        is_against_world_border: false,
                        sequence: VarInt(1),
                    },
                    &world.server.upgrade().unwrap(),
                )
                .unwrap(),
            Self::Bedrock(client) => client.client().handle_inventory_action(
                &client.player,
                SInventoryTransaction {
                    legacy_request_id: VarInt(0),
                    legacy_set_item_slots: Vec::new(),
                    has_value: true,
                    actions: Vec::new(),
                    transaction_type: VarUInt(2),
                    transaction_data: TransactionData::UseItem(UseItemTransactionData {
                        action_type: VarInt(0),
                        trigger_type: 0,
                        block_position: position,
                        block_face: BlockDirection::Up as u8,
                        hot_bar_slot: VarInt(i32::from(
                            client.player.inventory().get_selected_slot(),
                        )),
                        hand: HandSlot::Mainhand,
                        item_in_hand: NetworkItemDescriptor::from(
                            &client.player.inventory().held_item(),
                        ),
                        player_position: client.player.position().to_f32_lossy(),
                        click_position: Vector3::new(0.5, 1.0, 0.5),
                        block_runtime_id: VarUInt(0),
                        client_prediction: 0,
                        client_cooldown_state: 0,
                    }),
                },
            ),
        }
    }

    async fn close(&self) {
        if let Self::Bedrock(client) = self {
            client.close().await;
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Threat {
    None,
    Ocelot,
    Zombie,
    Parched,
    Phantom,
    CalmPiglin,
    OwnAnger,
    OtherAnger,
}

fn spawn_threat(world: &Arc<World>, player: &Player, threat: Threat) {
    let kind = match threat {
        Threat::None => return,
        Threat::Ocelot => &EntityType::OCELOT,
        Threat::Zombie => &EntityType::ZOMBIE,
        Threat::Parched => &EntityType::PARCHED,
        Threat::Phantom => &EntityType::PHANTOM,
        Threat::CalmPiglin | Threat::OwnAnger | Threat::OtherAnger => &EntityType::ZOMBIFIED_PIGLIN,
    };
    let entity = from_type(
        kind,
        Vector3::new(11.5, 64.0, 8.5),
        world,
        uuid::Uuid::new_v4(),
    );
    if matches!(threat, Threat::OwnAnger | Threat::OtherAnger) {
        let neutral = entity.get_mob().unwrap().as_neutral().unwrap();
        neutral.set_time_to_remain_angry(400);
        neutral.set_persistent_anger_target(Some(if matches!(threat, Threat::OwnAnger) {
            player.gameprofile.id
        } else {
            uuid::Uuid::new_v4()
        }));
    }
    assert!(world.spawn_entity(entity.clone()));
    assert!(
        world
            .get_entity_by_uuid(entity.get_entity().entity_uuid)
            .is_some()
    );
}

async fn exercise(bedrock: bool, straw: bool, threat: Threat, mode: GameMode, sleeping: bool) {
    let fixture = PlayerFixture::new();
    let world = &fixture.world;
    publish(world, proto(&Biome::PLAINS, &Block::STONE));
    world.set_time_of_day(18000);
    assert!(world.is_dark_outside());
    let client = Client::new(world, bedrock).await;
    let player = client.player();
    player.gamemode.store(mode);
    player.get_entity().set_pos(Vector3::new(8.5, 64.0, 6.5));
    player.get_entity().set_rotation(-90.0, 0.0);
    let item = if straw {
        &Item::STRAW_BED
    } else {
        &Item::WHITE_BED
    };
    player
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::new(1, item));
    client.use_on(world, FOOT.down());
    for (position, part) in [(FOOT, BedPart::Foot), (HEAD, BedPart::Head)] {
        assert_eq!(
            world.get_block(&position).item_id,
            item.id,
            "real placement must succeed"
        );
        assert_eq!(
            WhiteBedLikeProperties::from_state_id(world.get_block_state_id(&position)).part,
            part
        );
    }
    player
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::EMPTY.clone());
    spawn_threat(world, player, threat);

    client.use_on(world, FOOT);

    assert_eq!(
        player.sleeping_since.load().is_some(),
        sleeping,
        "bedrock={bedrock}, straw={straw}, threat={threat:?}, mode={mode:?}"
    );
    assert_eq!(player.sleeping_bed_pos.load(), sleeping.then_some(HEAD));
    assert!(
        player.get_entity().pose.load()
            == if sleeping {
                EntityPose::Sleeping
            } else {
                EntityPose::Standing
            }
    );
    for position in [FOOT, HEAD] {
        assert_eq!(
            WhiteBedLikeProperties::from_state_id(world.get_block_state_id(&position)).occupied,
            sleeping
        );
    }
    client.close().await;
    fixture.finish().await;
}

async fn routes(threat: Threat, mode: GameMode, sleeping: bool) {
    for bedrock in [false, true] {
        for straw in [false, true] {
            tokio::time::timeout(
                Duration::from_secs(30),
                exercise(bedrock, straw, threat, mode, sleeping),
            )
            .await
            .unwrap();
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_bed_threats_modes() {
    routes(Threat::None, GameMode::Survival, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_bed_threats_modes_ocelot() {
    routes(Threat::Ocelot, GameMode::Survival, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_bed_threats_modes_zombie_survival() {
    routes(Threat::Zombie, GameMode::Survival, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_bed_threats_modes_zombie_creative() {
    routes(Threat::Zombie, GameMode::Creative, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_bed_threats_modes_parched_survival() {
    routes(Threat::Parched, GameMode::Survival, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_bed_threats_modes_parched_creative() {
    routes(Threat::Parched, GameMode::Creative, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_bed_threats_modes_phantom() {
    routes(Threat::Phantom, GameMode::Survival, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_bed_threats_modes_piglin_calm() {
    routes(Threat::CalmPiglin, GameMode::Survival, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_bed_threats_modes_piglin_own_anger() {
    routes(Threat::OwnAnger, GameMode::Survival, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_bed_threats_modes_piglin_other_anger() {
    routes(Threat::OtherAnger, GameMode::Survival, true).await;
}
