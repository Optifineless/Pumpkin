#![allow(clippy::unwrap_used)]

use std::{sync::Arc, time::Duration};

use pumpkin_data::{Block, item::Item, item_stack::ItemStack, statistic::StatisticCategory};
use pumpkin_protocol::{VarInt, java::server::play::SUseItem, ser::NetworkReadExt};
use pumpkin_util::{
    Hand, PermissionLvl,
    math::{position::BlockPos, vector2::Vector2, vector3::Vector3},
};

use crate::{
    entity::EntityBase,
    net::java::combat_test_support::TestPlayer,
    server::{Server, combat_test_support},
    world::{BlockFlags, World},
};

pub struct Fixture {
    _directory: tempfile::TempDir,
    pub server: Arc<Server>,
    pub world: Arc<World>,
    pub actor: TestPlayer,
}

impl Fixture {
    pub fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        let world = combat_test_support::world(&server, directory.path());
        server.worlds.store(Arc::new(vec![world.clone()]));
        for x in [0, 1, 2] {
            combat_test_support::publish_empty_chunk(&world, Vector2::new(x, 0));
        }
        let actor = TestPlayer::new(&world);
        actor.player.permission_lvl.store(PermissionLvl::Four);
        actor.client().set_player(actor.player.clone());
        actor
            .player
            .get_entity()
            .set_pos(Vector3::new(30.5, 64.0, 0.5));
        actor.player.get_entity().set_rotation(0.0, 0.0);
        align_watched_section(&actor);
        Self {
            _directory: directory,
            server,
            world,
            actor,
        }
    }

    pub fn run(&self, command: &str) -> i32 {
        self.server
            .command_dispatcher
            .load()
            .execute_input(command, &self.actor.player.get_command_source(&self.server))
            .unwrap()
    }

    pub fn resize(&self, from: u32, to: u32) {
        self.run("worldborder center 1 1");
        self.run("worldborder center 0 0");
        self.run(&format!("worldborder set {from}"));
        self.run(&format!("worldborder set {to} 200t"));
    }

    pub fn ticks(&self, ticks: u32) {
        for _ in 0..ticks {
            self.world.tick_environment();
        }
    }

    pub fn assert_bottle_use(&self, permitted: bool) {
        let position = BlockPos::new(30, 65, 3);
        self.world.set_block_state(
            &position,
            Block::WATER.default_state.id,
            BlockFlags::FORCE_STATE,
        );
        self.actor
            .player
            .inventory()
            .set_stack_in_hand(Hand::Right, ItemStack::new(1, &Item::GLASS_BOTTLE));
        let previous = self
            .actor
            .player
            .get_stat(StatisticCategory::Used, i32::from(Item::GLASS_BOTTLE.id));
        assert!(self.actor.player.item_cooldowns.lock().unwrap().is_empty());
        self.actor.client().handle_use_item(
            &self.actor.player,
            &SUseItem {
                hand: VarInt(0),
                sequence: VarInt(1),
                yaw: 0.0,
                pitch: 0.0,
            },
            &self.server,
        );
        let hand = self.actor.player.inventory().get_stack_in_hand(Hand::Right);
        assert_eq!(
            hand.item,
            if permitted {
                &Item::POTION
            } else {
                &Item::GLASS_BOTTLE
            },
            "bottle admission must use the live border"
        );
        assert_eq!(hand.item_count, 1);
        assert_eq!(
            self.actor
                .player
                .get_stat(StatisticCategory::Used, i32::from(Item::GLASS_BOTTLE.id)),
            previous + i32::from(permitted)
        );
        assert!(
            self.actor.player.item_cooldowns.lock().unwrap().is_empty(),
            "filling or refusing a bottle must not commit a cooldown"
        );
    }

    pub async fn finish(self) {
        self.actor.client().close();
        tokio::time::timeout(Duration::from_secs(10), self.world.level.shutdown())
            .await
            .unwrap()
            .unwrap();
    }
}

pub fn align_watched_section(player: &TestPlayer) {
    let mut config = (**player.player.config.load()).clone();
    config.view_distance = std::num::NonZeroU8::new(2).unwrap();
    player.player.config.store(Arc::new(config));
    player.player.watched_section.store(
        pumpkin_world::cylindrical_chunk_iterator::Cylindrical::new(
            player.player.get_entity().chunk_pos.load(),
            std::num::NonZeroU8::new(2).unwrap(),
        ),
    );
}

pub fn payloads(packets: Vec<bytes::Bytes>, id: i32) -> Vec<bytes::Bytes> {
    packets
        .into_iter()
        .map(|packet| {
            let mut reader = &packet[..];
            let packet_id = reader.get_var_int().unwrap().0;
            (packet_id, packet.slice(packet.len() - reader.len()..))
        })
        .filter(|(packet_id, _)| *packet_id == id)
        .map(|(_, payload)| payload)
        .collect()
}

pub fn lerps(player: &mut TestPlayer) -> Vec<(f64, f64, i64)> {
    payloads(
        player.take_packets(),
        pumpkin_data::packet::clientbound::play::SET_BORDER_LERP_SIZE.0,
    )
    .into_iter()
    .map(|packet| {
        let mut reader = &packet[..];
        let result = (
            reader.get_f64_be().unwrap(),
            reader.get_f64_be().unwrap(),
            reader.get_var_long().unwrap().0,
        );
        assert!(reader.is_empty());
        result
    })
    .collect()
}

#[derive(Debug, PartialEq)]
pub struct Initialization {
    pub center: (f64, f64),
    pub sizes: (f64, f64, i64),
    pub limit: i32,
    pub warning: (i32, i32),
}

pub fn initialization(player: &mut TestPlayer) -> Initialization {
    let packets = payloads(
        player.take_packets(),
        pumpkin_data::packet::clientbound::play::INITIALIZE_BORDER.0,
    );
    assert_eq!(packets.len(), 1);
    let mut reader = &packets[0][..];
    let result = Initialization {
        center: (reader.get_f64_be().unwrap(), reader.get_f64_be().unwrap()),
        sizes: (
            reader.get_f64_be().unwrap(),
            reader.get_f64_be().unwrap(),
            reader.get_var_long().unwrap().0,
        ),
        limit: reader.get_var_int().unwrap().0,
        warning: (
            reader.get_var_int().unwrap().0,
            reader.get_var_int().unwrap().0,
        ),
    };
    assert!(reader.is_empty());
    result
}
