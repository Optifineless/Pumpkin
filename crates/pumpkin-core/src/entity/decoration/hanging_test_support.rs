#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Real-entry regression fixtures require successful bounded setup"
)]

use crate::{
    entity::EntityBase,
    net::java::combat_test_support::TestPlayer,
    server::{Server, combat_test_support},
    world::{World, spawn_test_support},
};
use pumpkin_data::{
    Block, BlockDirection, biome::Biome, data_component_impl::PaintingVariantImpl, item::Item,
    item_stack::ItemStack, painting_variant::PaintingVariant,
};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::{VarInt, java::server::play::SUseItemOn};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use pumpkin_world::world::BlockFlags;
use std::{sync::Arc, time::Duration};

pub struct Fixture {
    pub server: Arc<Server>,
    pub world: Arc<World>,
    pub player: TestPlayer,
    _directory: tempfile::TempDir,
}

impl Fixture {
    pub fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        server.level_info.rcu(|info| {
            let mut info = (**info).clone();
            info.game_rules.random_tick_speed = 0;
            info.game_rules.spawn_mobs = false;
            info
        });
        let world = combat_test_support::world(&server, directory.path());
        spawn_test_support::publish(
            &world,
            spawn_test_support::proto(&Biome::PLAINS, &Block::STONE),
        );
        let player = TestPlayer::new(&world);
        player
            .player
            .get_entity()
            .set_pos(Vector3::new(8.5, 64.0, 12.5));
        Self {
            server,
            world,
            player,
            _directory: directory,
        }
    }

    pub fn set_block(&self, pos: BlockPos, block: &'static Block) {
        self.world.set_block_state(
            &pos,
            block.default_state.id,
            BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
        );
    }

    pub fn place(
        &self,
        item: &'static Item,
        support: BlockPos,
        face: BlockDirection,
        variant: Option<PaintingVariant>,
    ) -> Arc<dyn EntityBase> {
        let before = self.world.entities.load_full();
        let mut stack = ItemStack::new(1, item);
        if let Some(variant) = variant {
            stack.set_data_component(PaintingVariantImpl {
                value: variant.asset_id().into(),
            });
        }
        self.player.player.inventory().set_stack(0, stack);
        self.player
            .client()
            .handle_use_item_on(
                &self.player.player,
                &SUseItemOn {
                    hand: VarInt(0),
                    position: support,
                    face: VarInt(face as i32),
                    cursor_pos: Vector3::new(0.5, 0.5, 0.5),
                    inside_block: false,
                    is_against_world_border: false,
                    sequence: VarInt(1),
                },
                &self.server,
            )
            .unwrap();
        let placed = self
            .world
            .entities
            .load()
            .iter()
            .find(|entity| {
                !before.iter().any(|old| Arc::ptr_eq(old, entity))
                    && entity.get_item_entity().is_none()
            })
            .cloned()
            .expect("normal item interaction must place a decoration");
        assert!(self.player.player.inventory().get_stack(0).is_empty());
        placed
    }

    pub fn tick(&self, count: usize) {
        assert!(count <= 404, "bounded hanging check intervals");
        for _ in 0..count {
            self.world.tick(&self.server);
        }
    }

    pub fn drops(&self, item: &Item) -> u32 {
        let world_count = self
            .world
            .entities
            .load()
            .iter()
            .filter_map(|entity| entity.get_item_entity())
            .map(|entity| {
                let stack = entity.get_item_stack().lock().unwrap();
                if stack.item == item {
                    u32::from(stack.item_count)
                } else {
                    0
                }
            })
            .sum::<u32>();
        let inventory = self.player.player.inventory();
        let carried_count = (0..inventory.size())
            .map(|slot| inventory.get_stack(slot))
            .filter(|stack| stack.item == item)
            .map(|stack| u32::from(stack.item_count))
            .sum::<u32>();

        world_count + carried_count
    }

    pub async fn finish(self) {
        tokio::time::timeout(Duration::from_secs(30), self.world.level.shutdown())
            .await
            .unwrap()
            .unwrap();
    }
}
