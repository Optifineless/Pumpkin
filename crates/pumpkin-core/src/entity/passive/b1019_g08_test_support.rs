#![expect(clippy::unwrap_used, reason = "Valid bounded gameplay fixtures")]

use std::{sync::Arc, time::Duration};

use pumpkin_data::{entity::EntityType, item::Item, item_stack::ItemStack};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::{VarInt, java::server::play::SInteract};
use pumpkin_util::{Hand, math::vector3::Vector3, version::JavaMinecraftVersion};

use crate::{
    entity::{EntityBase, player::Player},
    net::java::{JavaClient, combat_test_support::TestPlayer},
    server::{Server, combat_test_support},
    world::World,
};

pub(super) struct Fixture {
    pub server: Arc<Server>,
    pub world: Arc<World>,
    pub player: Arc<Player>,
    observer: TestPlayer,
    _directory: tempfile::TempDir,
}

impl Fixture {
    pub fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        let world = combat_test_support::world(&server, directory.path());
        server.worlds.store(Arc::new(vec![world.clone()]));
        combat_test_support::publish_empty_chunk(
            &world,
            pumpkin_util::math::vector2::Vector2::new(0, 0),
        );
        let observer = TestPlayer::new(&world);
        observer.client().set_player(observer.player.clone());
        observer
            .client()
            .version
            .store(JavaMinecraftVersion::V_1_21_2);
        Self {
            server,
            world,
            player: observer.player.clone(),
            observer,
            _directory: directory,
        }
    }

    pub fn client(&self) -> &JavaClient {
        self.observer.client()
    }

    pub fn mob(&self, kind: &'static EntityType) -> Arc<dyn EntityBase> {
        let mob = crate::entity::r#type::from_type(
            kind,
            Vector3::new(0.0, 64.0, 0.0),
            &self.world,
            uuid::Uuid::new_v4(),
        );
        self.world.add_entity_silent(mob.clone());
        mob
    }

    pub fn interact(&self, target: &dyn EntityBase, hand: Hand) {
        self.client().handle_interact(
            &self.player,
            &SInteract {
                entity_id: VarInt(target.get_entity().entity_id),
                r#type: VarInt(2),
                target_position: Some(Vector3::new(0.0, 0.0, 0.0)),
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

    pub fn dropped_count(&self, item: &Item) -> u32 {
        self.world
            .entities
            .load()
            .iter()
            .filter_map(|entity| entity.get_item_entity())
            .map(|entity| entity.get_item_stack().lock().unwrap().clone())
            .filter(|stack| stack.get_item() == item)
            .map(|stack| u32::from(stack.item_count))
            .sum()
    }

    pub async fn finish(self) {
        self.client().close();
        tokio::time::timeout(Duration::from_secs(10), self.server.shutdown())
            .await
            .unwrap();
    }
}
