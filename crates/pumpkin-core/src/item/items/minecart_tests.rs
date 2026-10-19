use crate::{
    entity::{EntityBase, death_test_world::DeathTestWorld},
    net::java::combat_test_support::TestPlayer,
    server::combat_test_support,
};
use pumpkin_data::{
    Block, BlockDirection,
    block_properties::{RailLikeProperties, RailShape},
    entity::EntityType,
    item::Item,
    item_stack::ItemStack,
};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::{VarInt, java::server::play::SUseItemOn};
use pumpkin_util::{
    GameMode,
    math::{position::BlockPos, vector3::Vector3},
};
use pumpkin_world::world::BlockFlags;
use std::sync::Arc;

struct MinecartPlacementFixture {
    context: DeathTestWorld,
    world: Arc<crate::world::World>,
    player: TestPlayer,
}

impl MinecartPlacementFixture {
    async fn new() -> Self {
        let context = DeathTestWorld::new().await;
        let world = context.world();
        let player = TestPlayer::new(&world);
        Self {
            context,
            world,
            player,
        }
    }

    fn set_block(&self, position: BlockPos, block: &Block, state: pumpkin_data::BlockStateId) {
        let chunk = position.chunk_position();
        if !self.world.level.loaded_chunks.contains_key(&chunk) {
            combat_test_support::publish_empty_chunk(&self.world, chunk);
        }
        self.world.set_block_state(
            &position,
            state,
            BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
        );
        assert_eq!(self.world.get_block(&position), block);
    }

    fn set_rail(&self, position: BlockPos, state: pumpkin_data::BlockStateId) {
        self.set_block(position, &Block::RAIL, state);
    }

    fn use_item_on(&self, position: BlockPos) {
        self.player.player.get_entity().set_pos(Vector3::new(
            f64::from(position.0.x) + 0.5,
            f64::from(position.0.y) + 1.0,
            f64::from(position.0.z) + 0.5,
        ));
        self.player
            .client()
            .handle_use_item_on(
                &self.player.player,
                &SUseItemOn {
                    hand: VarInt(0),
                    position,
                    face: VarInt(BlockDirection::Up as i32),
                    cursor_pos: Vector3::new(0.5, 0.5, 0.5),
                    inside_block: false,
                    is_against_world_border: false,
                    sequence: VarInt(1),
                },
                &self.context.server,
            )
            .unwrap();
    }

    fn cart_position(&self, index: usize, expected_type: &EntityType) -> Vector3<f64> {
        let entities = self.world.entities.load();
        let entity = &entities[index];
        assert_eq!(entity.get_entity().entity_type.id, expected_type.id);
        entity.get_entity().pos.load()
    }

    async fn finish(self) {
        self.context.server.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g22_flat_positive_coordinates_center_every_minecart_variant() {
    let fixture = MinecartPlacementFixture::new().await;
    let variants = [
        (&Item::MINECART, &EntityType::MINECART),
        (&Item::CHEST_MINECART, &EntityType::CHEST_MINECART),
        (&Item::HOPPER_MINECART, &EntityType::HOPPER_MINECART),
        (&Item::TNT_MINECART, &EntityType::TNT_MINECART),
        (&Item::FURNACE_MINECART, &EntityType::FURNACE_MINECART),
        (
            &Item::COMMAND_BLOCK_MINECART,
            &EntityType::COMMAND_BLOCK_MINECART,
        ),
    ];

    for (index, (item, entity_type)) in variants.into_iter().enumerate() {
        let rail = BlockPos::new(10, 64, 20 + (index as i32 * 2));
        fixture.set_rail(rail, Block::RAIL.default_state.id);
        fixture
            .player
            .player
            .inventory()
            .set_stack(0, ItemStack::new(2, item));
        fixture.use_item_on(rail);

        let position = fixture.cart_position(index, entity_type);
        assert_eq!(
            (position.x, position.y, position.z),
            (10.5, 64.0625, f64::from(rail.0.z) + 0.5)
        );
        assert_eq!(fixture.player.player.inventory().held_item().item_count, 1);
    }

    assert_eq!(fixture.world.entities.load().len(), variants.len());
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g22_flat_negative_coordinates_center_on_the_clicked_rail() {
    let fixture = MinecartPlacementFixture::new().await;
    let rail = BlockPos::new(-10, 64, -20);
    fixture.set_rail(rail, Block::RAIL.default_state.id);
    fixture
        .player
        .player
        .inventory()
        .set_stack(0, ItemStack::new(1, &Item::MINECART));
    fixture.use_item_on(rail);

    let position = fixture.cart_position(0, &EntityType::MINECART);
    assert_eq!((position.x, position.y, position.z), (-9.5, 64.0625, -19.5));
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g22_ascending_rail_adds_only_the_vanilla_y_offset() {
    let fixture = MinecartPlacementFixture::new().await;
    let rail = BlockPos::new(10, 64, 20);
    let mut ascending_properties = RailLikeProperties::from_state_id(Block::RAIL.default_state.id);
    ascending_properties.shape = RailShape::AscendingEast;
    let ascending = ascending_properties.to_state_id(&Block::RAIL);
    fixture.set_rail(rail, ascending);
    fixture
        .player
        .player
        .inventory()
        .set_stack(0, ItemStack::new(1, &Item::MINECART));
    fixture.use_item_on(rail);

    let position = fixture.cart_position(0, &EntityType::MINECART);
    assert_eq!(position.y, 64.5625);
    assert_eq!((position.x, position.z), (10.5, 20.5));
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g22_nonrail_refusal_keeps_the_item_and_spawns_nothing() {
    let fixture = MinecartPlacementFixture::new().await;
    let block = BlockPos::new(10, 64, 20);
    fixture.set_block(block, &Block::STONE, Block::STONE.default_state.id);
    let stack = ItemStack::new(2, &Item::MINECART);
    fixture
        .player
        .player
        .inventory()
        .set_stack(0, stack.clone());
    fixture.use_item_on(block);

    assert!(fixture.world.entities.load().is_empty());
    assert!(
        fixture
            .player
            .player
            .inventory()
            .held_item()
            .are_equal(&stack)
    );
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g22_survival_pays_once_and_creative_keeps_the_stack() {
    let fixture = MinecartPlacementFixture::new().await;
    let survival_rail = BlockPos::new(10, 64, 20);
    let creative_rail = BlockPos::new(12, 64, 20);
    fixture.set_rail(survival_rail, Block::RAIL.default_state.id);
    fixture.set_rail(creative_rail, Block::RAIL.default_state.id);

    fixture
        .player
        .player
        .inventory()
        .set_stack(0, ItemStack::new(2, &Item::MINECART));
    fixture.use_item_on(survival_rail);
    assert_eq!(fixture.world.entities.load().len(), 1);
    assert_eq!(fixture.player.player.inventory().held_item().item_count, 1);

    fixture.player.player.gamemode.store(GameMode::Creative);
    fixture
        .player
        .player
        .inventory()
        .set_stack(0, ItemStack::new(2, &Item::MINECART));
    fixture.use_item_on(creative_rail);
    assert_eq!(fixture.world.entities.load().len(), 2);
    assert_eq!(fixture.player.player.inventory().held_item().item_count, 2);

    fixture.finish().await;
}
