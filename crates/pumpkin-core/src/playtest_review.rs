use std::sync::Arc;

use pumpkin_data::{Block, entity::EntityType, item::Item, item_stack::ItemStack};
use pumpkin_inventory::Inventory;
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_util::math::{position::BlockPos, vector2::Vector2, vector3::Vector3};
use pumpkin_world::{chunk::ChunkData, world::BlockFlags};

use crate::{
    entity::EntityBase,
    server::{Server, combat_test_support},
    world::World,
};

mod boats;
mod death;
mod doors;
mod followup2;
mod followup3;
mod trading;
mod transformations;

struct Fixture {
    _dir: tempfile::TempDir,
    server: Arc<Server>,
    world: Arc<World>,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(dir.path());
        let world = combat_test_support::world(&server, dir.path());
        server.worlds.store(Arc::new(vec![world.clone()]));
        world
            .level
            .loaded_chunks
            .insert(Vector2::new(0, 0), ChunkData::empty_sync(0, 0));
        Self {
            _dir: dir,
            server,
            world,
        }
    }

    fn entity(&self, kind: &'static EntityType) -> Arc<dyn EntityBase> {
        let entity = crate::entity::r#type::from_type(
            kind,
            Vector3::new(8.0, 64.0, 8.0),
            &self.world,
            uuid::Uuid::new_v4(),
        );
        self.world.add_entity_silent(entity.clone());
        entity
    }

    fn drops(&self, item: &'static Item) -> u32 {
        self.world
            .entities
            .load()
            .iter()
            .filter_map(|e| e.get_item_entity())
            .map(|e| e.get_item_stack().lock().unwrap().clone())
            .filter(|s| s.item == item)
            .map(|s| u32::from(s.item_count))
            .sum()
    }

    async fn shutdown(&self) {
        self.world.level.shutdown().await.unwrap();
    }
}

fn fill_boat(boat: &dyn EntityBase, item: &'static Item) {
    let mut stack = NbtCompound::new();
    ItemStack::new(3, item).write_item_stack(&mut stack);
    stack.put_byte("Slot", 0);
    let mut nbt = NbtCompound::new();
    nbt.put("Items", NbtTag::List(vec![NbtTag::Compound(stack)]));
    boat.read_custom_nbt(&nbt);
}
