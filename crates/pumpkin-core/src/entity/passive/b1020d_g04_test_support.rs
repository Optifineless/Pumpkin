#![expect(
    clippy::unwrap_used,
    reason = "Valid bounded animal interaction fixtures"
)]

use std::{sync::Arc, time::Duration};

use bytes::Bytes;
use pumpkin_data::{
    entity::{EntityStatus, EntityType},
    item::Item,
    item_stack::ItemStack,
    packet::clientbound::play::{ENTITY_EVENT, LEVEL_PARTICLES},
    particle::Particle,
};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::{VarInt, java::server::play::SInteract, ser::NetworkReadExt};
use pumpkin_util::{Hand, math::vector2::Vector2, math::vector3::Vector3};

use crate::{
    entity::{EntityBase, player::Player},
    net::java::combat_test_support::TestPlayer,
    server::{Server, combat_test_support},
    world::World,
};

pub(super) use crate::net::java::sound_test_support::decode_sounds;

pub(super) struct Fixture {
    pub server: Arc<Server>,
    pub world: Arc<World>,
    pub player: Arc<Player>,
    pub actor: TestPlayer,
    pub observers: Vec<TestPlayer>,
    _directory: tempfile::TempDir,
}

impl Fixture {
    pub fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        let world = combat_test_support::world(&server, directory.path());
        server.worlds.store(Arc::new(vec![world.clone()]));
        combat_test_support::publish_empty_chunk(&world, Vector2::new(0, 0));
        let actor = TestPlayer::new(&world);
        let observers = vec![TestPlayer::new(&world), TestPlayer::new(&world)];
        let players: Vec<_> = std::iter::once(&actor)
            .chain(observers.iter())
            .enumerate()
            .map(|(index, client)| {
                client.client().set_player(client.player.clone());
                client
                    .player
                    .get_entity()
                    .set_pos(Vector3::new(index as f64 + 1.0, 64.0, 1.0));
                client
                    .player
                    .chunk_sender
                    .lock()
                    .unwrap()
                    .mark_sent_out_of_band(Vector2::new(0, 0));
                client.player.clone()
            })
            .collect();
        world.players.store(Arc::new(players));
        Self {
            server,
            world,
            player: actor.player.clone(),
            actor,
            observers,
            _directory: directory,
        }
    }

    pub fn mob(&self, kind: &'static EntityType) -> Arc<dyn EntityBase> {
        let entity = crate::entity::r#type::from_type(
            kind,
            Vector3::new(0.0, 64.0, 0.0),
            &self.world,
            uuid::Uuid::new_v4(),
        );
        self.world.add_entity_silent(entity.clone());
        self.world
            .entity_tracker
            .update_entity_position(entity.as_ref(), &self.world);
        entity
    }

    pub fn interact(&self, target: &dyn EntityBase, hand: Hand, at: bool) {
        self.actor.client().handle_interact(
            &self.player,
            &SInteract {
                entity_id: VarInt(target.get_entity().entity_id),
                r#type: VarInt(if at { 2 } else { 0 }),
                target_position: at.then(|| Vector3::new(0.0, 0.0, 0.0)),
                hand: Some(VarInt(i32::from(hand == Hand::Left))),
                sneaking: false,
            },
            &self.server,
        );
    }

    pub fn set_hand(&self, hand: Hand, stack: ItemStack) {
        self.player.inventory().set_stack_in_hand(hand, stack);
    }

    pub fn hand(&self, hand: Hand) -> ItemStack {
        self.player.inventory().get_stack_in_hand(hand)
    }

    pub fn inventory_count(&self, item: &Item) -> u32 {
        let inventory = self.player.inventory();
        (0..inventory.size())
            .map(|slot| inventory.get_stack(slot))
            .filter(|stack| stack.get_item() == item)
            .map(|stack| u32::from(stack.item_count))
            .sum()
    }

    pub fn drain_packets(&mut self) {
        self.actor.take_packets();
        for observer in &mut self.observers {
            observer.take_packets();
        }
    }

    pub async fn finish(self) {
        self.actor.client().close();
        for observer in &self.observers {
            observer.client().close();
        }
        tokio::time::timeout(Duration::from_secs(10), self.server.shutdown())
            .await
            .unwrap();
    }
}

/// Decodes the real 26.3 status and Heart particle publications.
pub(super) fn feedback(packets: &[Bytes], entity_id: i32) -> (usize, Vec<i32>) {
    let mut statuses = 0;
    let mut hearts = Vec::new();
    for packet in packets {
        let mut bytes = packet.as_ref();
        let id = bytes.get_var_int().unwrap().0;
        if id == ENTITY_EVENT.0 {
            let target = bytes.get_i32_be().unwrap();
            let status = bytes.get_i8().unwrap();
            assert!(bytes.is_empty());
            if target == entity_id && status == EntityStatus::InLoveHearts as i8 {
                statuses += 1;
            }
        } else if id == LEVEL_PARTICLES.0
            && bytes.get_var_int().unwrap().0 == i32::from(Particle::Heart.to_id())
        {
            assert!(!bytes.get_bool().unwrap());
            assert!(!bytes.get_bool().unwrap());
            for _ in 0..3 {
                bytes.get_f64_be().unwrap();
            }
            for _ in 0..6 {
                bytes.get_f32_be().unwrap();
            }
            hearts.push(bytes.get_var_int().unwrap().0);
            assert_eq!(bytes.get_var_int().unwrap().0, 0);
            assert!(bytes.is_empty());
        }
    }
    (statuses, hearts)
}
