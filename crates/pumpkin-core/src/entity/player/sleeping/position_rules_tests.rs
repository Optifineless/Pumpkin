use std::{sync::Arc, time::Duration};

use pumpkin_data::{
    Advancement, Block, BlockStateId,
    block_properties::{BedPart, HorizontalFacing, WhiteBedLikeProperties},
    dimension::{BedRuleOption, Dimension},
    item::Item,
    item_stack::ItemStack,
    packet::CURRENT_MC_VERSION,
};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::{
    VarInt,
    java::{
        client::play::{CEntityAnimation, CEntityPositionSync, CPlayerPosition},
        server::play::SUseItemOn,
    },
    packet::MultiVersionJavaPacket,
};
use pumpkin_util::{GameMode, PermissionLvl, math::vector3::Vector3};
use pumpkin_world::{level::Level, world::BlockFlags};

use super::{EntityBase, EntityPose, FOOT, HEAD, TestPlayer};
use crate::{
    server::{Server, combat_test_support},
    world::{
        World,
        spawn_test_support::{proto, publish},
    },
};
use pumpkin_data::biome::Biome;
use pumpkin_util::math::position::BlockPos;

pub(super) struct Scenario {
    pub(super) server: Arc<Server>,
    pub(super) world: Arc<World>,
    pub(super) client: TestPlayer,
    pub(super) observer: TestPlayer,
    _directory: tempfile::TempDir,
}

impl Scenario {
    pub(super) fn new(dimension: Dimension, item: &'static Item) -> Self {
        Self::with_sleep(dimension, item, false)
    }

    pub(super) fn with_native_sleep(dimension: Dimension, item: &'static Item) -> Self {
        Self::with_sleep(dimension, item, true)
    }

    fn with_sleep(dimension: Dimension, item: &'static Item, native: bool) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        let world = Arc::new(World::load(
            Level::from_root_folder(
                &pumpkin_config::world::LevelConfig::default(),
                directory.path().to_path_buf(),
                0,
                dimension.clone(),
            ),
            server.level_info.clone(),
            dimension,
            server.block_registry.clone(),
            Arc::downgrade(&server),
        ));
        publish(&world, proto(&Biome::PLAINS, &Block::STONE));
        let client = TestPlayer::new(&world);
        client
            .player
            .advancements
            .lock()
            .unwrap()
            .set_player(&client.player);
        let observer = TestPlayer::new(&world);
        world.players.store(Arc::new(vec![
            client.player.clone(),
            observer.player.clone(),
        ]));
        observer.player.gamemode.store(GameMode::Spectator);
        client.player.permission_lvl.store(PermissionLvl::Four);
        client
            .player
            .get_entity()
            .set_pos(Vector3::new(5.5, 64.0, 6.5));
        client.player.get_entity().set_rotation(-90.0, 0.0);
        client
            .player
            .inventory()
            .set_stack(0, ItemStack::new(1, item));
        let scenario = Self {
            server,
            world,
            client,
            observer,
            _directory: directory,
        };
        scenario.use_on(FOOT.down());
        assert_eq!(scenario.world.get_block(&FOOT).item_id, item.id);
        assert_eq!(scenario.world.get_block(&HEAD).item_id, item.id);
        scenario.world.level_time.lock().unwrap().time_of_day = 13_000;
        if native {
            scenario.client.player.sleep(HEAD);
        } else {
            scenario.use_on(FOOT);
        }
        assert_eq!(scenario.client.player.sleeping_bed_pos.load(), Some(HEAD));
        assert!(scenario.client.player.get_entity().pose.load() == EntityPose::Sleeping);
        scenario
    }

    pub(super) fn use_on(&self, position: BlockPos) {
        self.client
            .client()
            .handle_use_item_on(
                &self.client.player,
                &SUseItemOn {
                    hand: VarInt(0),
                    position,
                    face: VarInt(1),
                    cursor_pos: Vector3::new(0.5, 1.0, 0.5),
                    inside_block: false,
                    is_against_world_border: false,
                    sequence: VarInt(1),
                },
                &self.server,
            )
            .unwrap();
    }

    pub(super) async fn finish(self) {
        tokio::time::timeout(Duration::from_secs(20), self.world.level.shutdown())
            .await
            .unwrap()
            .unwrap();
    }

    pub(super) fn drain(&mut self) {
        self.client.take_packets();
        self.observer.take_packets();
    }

    pub(super) fn assert_awake(&self) {
        assert!(self.client.player.get_entity().pose.load() == EntityPose::Standing);
        assert_eq!(self.client.player.sleeping_bed_pos.load(), None);
        assert_eq!(self.client.player.sleeping_since.load(), None);
        assert!(!self.world.should_skip_night());
    }
}

fn packet_ids(packets: &[bytes::Bytes]) -> Vec<i32> {
    packets
        .iter()
        .map(|packet| {
            let mut data = packet.as_ref();
            VarInt::decode(&mut data).unwrap().0
        })
        .collect()
}

fn assert_corrections(scenario: &mut Scenario) {
    let owner_packets = scenario.client.take_packets();
    let observer_packets = scenario.observer.take_packets();
    let owner = packet_ids(&owner_packets);
    let observer = packet_ids(&observer_packets);
    assert!(
        owner.contains(&CPlayerPosition::to_id(CURRENT_MC_VERSION)),
        "owner receives position correction"
    );
    assert!(
        !owner.contains(&CEntityPositionSync::to_id(CURRENT_MC_VERSION)),
        "owner never receives observer position packet"
    );
    assert!(
        observer.contains(&CEntityPositionSync::to_id(CURRENT_MC_VERSION)),
        "observer receives stand-up position"
    );
    assert!(
        observer.contains(&CEntityAnimation::to_id(CURRENT_MC_VERSION)),
        "observer sees leave animation"
    );
    let (teleport_id, position) = scenario
        .client
        .player
        .awaiting_teleport
        .lock()
        .unwrap()
        .unwrap();
    assert_eq!(position, scenario.client.player.position());
    let entity = scenario.client.player.get_entity();
    let correction = scenario
        .client
        .client()
        .serialize_packet(&CPlayerPosition::new(
            teleport_id,
            position,
            Vector3::default(),
            entity.yaw.load(),
            entity.pitch.load(),
            Vec::new(),
        ))
        .unwrap();
    assert!(
        owner_packets.contains(&correction),
        "owner correction carries the accepted position and rotation"
    );
    let tracked = scenario
        .observer
        .client()
        .serialize_packet(&CEntityPositionSync::new(
            entity.entity_id.into(),
            position,
            entity.velocity.load(),
            entity.yaw.load(),
            entity.pitch.load(),
            entity.on_ground.load(std::sync::atomic::Ordering::SeqCst),
        ))
        .unwrap();
    assert!(
        observer_packets.contains(&tracked),
        "observer correction carries the stand-up position and rotation"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_sleep_wake_position_rules() {
    for (item, height) in [(&Item::WHITE_BED, 64.6875), (&Item::STRAW_BED, 64.375)] {
        let mut scenario = Scenario::new(Dimension::OVERWORLD, item);
        assert_eq!(
            scenario.client.player.position(),
            Vector3::new(6.5, height, 5.5),
            "sleep uses this bed's surface"
        );
        scenario.drain();
        scenario.client.player.wake_up();
        scenario.assert_awake();
        assert_eq!(
            scenario.client.player.position(),
            Vector3::new(6.5, 64.0, 6.5),
            "open exit follows yaw-selected candidate order"
        );
        assert_eq!(scenario.client.player.get_entity().yaw.load(), -180.0);
        assert_eq!(scenario.client.player.get_entity().pitch.load(), 0.0);
        assert_corrections(&mut scenario);
        scenario.finish().await;
    }
}

fn set(scenario: &Scenario, position: BlockPos, block: &Block) {
    scenario
        .world
        .set_block_state(&position, block.default_state.id, BlockFlags::FORCE_STATE);
}

fn enclose(scenario: &Scenario, hole: Option<BlockPos>) {
    for x in 3..=8 {
        for z in 3..=7 {
            let position = BlockPos::new(x, 64, z);
            if position == HEAD || position == FOOT || Some(position) == hole {
                continue;
            }
            set(scenario, position, &Block::STONE);
            set(scenario, position.up(), &Block::STONE);
        }
    }
}

fn bed_half(part: BedPart) -> BlockStateId {
    let mut properties = WhiteBedLikeProperties::default(&Block::WHITE_BED);
    properties.facing = HorizontalFacing::East;
    properties.part = part;
    properties.to_state_id(&Block::WHITE_BED)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_sleep_wake_position_rules_layouts() {
    for layout in ["enclosed", "bunk", "no-safe-position"] {
        let scenario = Scenario::new(Dimension::OVERWORLD, &Item::WHITE_BED);
        let hole = HEAD.add(0, 0, 1);
        enclose(&scenario, (layout == "bunk").then_some(hole));
        let expected = match layout {
            "enclosed" => Vector3::new(6.5, 64.5625, 5.5),
            "bunk" => {
                for x in 3..=8 {
                    for z in 3..=7 {
                        set(&scenario, BlockPos::new(x, 62, z), &Block::STONE);
                    }
                }
                for (position, part) in [(HEAD.down(), BedPart::Head), (FOOT.down(), BedPart::Foot)]
                {
                    scenario.world.set_block_state(
                        &position,
                        bed_half(part),
                        BlockFlags::FORCE_STATE,
                    );
                }
                set(&scenario, hole.down(), &Block::AIR);
                Vector3::new(6.5, 63.0, 6.5)
            }
            _ => {
                set(&scenario, HEAD.up(), &Block::STONE);
                set(&scenario, FOOT.up(), &Block::STONE);
                Vector3::new(6.5, 65.1, 5.5)
            }
        };
        scenario.client.player.wake_up();
        scenario.assert_awake();
        assert_eq!(
            scenario.client.player.position(),
            expected,
            "{layout} stand-up"
        );
        assert_eq!(scenario.client.player.get_entity().pitch.load(), 0.0);
        scenario.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_sleep_wake_position_rules_straw_rules() {
    for can_spawn in [false, true] {
        for destroy in [false, true] {
            let mut dimension = Dimension::OVERWORLD;
            dimension.straw_bed_rule.can_set_spawn = if can_spawn {
                BedRuleOption::Always
            } else {
                BedRuleOption::Never
            };
            dimension.straw_bed_rule.destroy_on_leave = destroy;
            let scenario = Scenario::new(dimension, &Item::STRAW_BED);
            let spawn = scenario.client.player.respawn_point.lock().unwrap().clone();
            assert_eq!(
                spawn.as_ref().map(|point| point.position),
                can_spawn.then_some(HEAD),
                "straw rule controls spawn mutation"
            );
            assert_eq!(
                scenario
                    .client
                    .player
                    .has_advancement(Advancement::ADVENTURE_SLEEP_IN_BED),
                can_spawn,
                "slept advancement follows spawn rule"
            );
            scenario.client.player.wake_up();
            scenario.assert_awake();
            for position in [HEAD, FOOT] {
                assert_eq!(
                    scenario.world.get_block(&position),
                    if destroy {
                        &Block::AIR
                    } else {
                        &Block::STRAW_BED
                    },
                    "destroy_on_leave={destroy}"
                );
                if !destroy {
                    assert!(
                        !WhiteBedLikeProperties::from_state_id(
                            scenario.world.get_block_state_id(&position)
                        )
                        .occupied
                    );
                }
            }
            scenario.finish().await;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_sleep_wake_position_rules_missing_bed() {
    let mut scenario = Scenario::new(Dimension::OVERWORLD, &Item::WHITE_BED);
    scenario.client.player.get_entity().set_rotation(27.0, 14.0);
    scenario.client.player.gamemode.store(GameMode::Creative);
    scenario.client.client().handle_player_action(
        &scenario.client.player,
        &pumpkin_protocol::java::server::play::SPlayerAction {
            status: VarInt(0),
            position: HEAD,
            face: 1,
            sequence: VarInt(2),
        },
        &scenario.server,
    );
    assert_eq!(scenario.world.get_block(&HEAD), &Block::AIR);
    let position = scenario.client.player.position();
    scenario.drain();
    scenario.client.player.wake_up();
    scenario.assert_awake();
    assert_eq!(
        scenario.client.player.position(),
        position,
        "missing bed retains position"
    );
    assert_eq!(scenario.client.player.get_entity().yaw.load(), 27.0);
    assert_eq!(scenario.client.player.get_entity().pitch.load(), 14.0);
    assert_corrections(&mut scenario);
    scenario.finish().await;
}
