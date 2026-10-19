use super::*;
use crate::block::entities::{BlockEntity, decorated_pot::DecoratedPotBlockEntity};
use pumpkin_inventory::crafting::crafting_screen_handler::CraftingTableScreenHandler;
use pumpkin_inventory::screen_handler::ScreenHandler;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::java::server::play::SlotActionType;

fn crafted_pot(fixture: &Fixture, names: [&str; 4]) -> ItemStack {
    let mut menu = CraftingTableScreenHandler::new(1, fixture.player.player.inventory(), None);
    for (slot, name) in [1, 3, 5, 7].into_iter().zip(names) {
        let mut brick = ItemStack::new(1, &Item::BRICK);
        brick.set_custom_name(name.to_owned());
        menu.get_behaviour().slots[slot + 1].set_stack(brick);
    }
    menu.send_content_updates();
    menu.on_slot_click(0, 0, SlotActionType::Pickup, fixture.player.player.as_ref());
    for slot in [2, 4, 6, 8] {
        assert!(menu.get_behaviour().slots[slot].get_stack().is_empty());
    }
    let mut cursor = menu
        .get_behaviour()
        .cursor_stack
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    std::mem::replace(&mut *cursor, ItemStack::EMPTY.clone())
}

fn load_pot_from_nbt(
    fixture: &Fixture,
    position: BlockPos,
    mut saved: NbtCompound,
) -> Arc<dyn BlockEntity> {
    let relative = position.chunk_relative_position();
    fixture
        .world
        .level
        .read_chunk_sync(&position.chunk_position(), |chunk| {
            chunk
                .set_block_absolute_y_if(
                    relative.x as usize,
                    relative.y,
                    relative.z as usize,
                    Block::DECORATED_POT.default_state.id,
                    |_| true,
                )
                .unwrap();
        })
        .unwrap();

    saved.put_string("id", DecoratedPotBlockEntity::ID.to_owned());
    saved.put_int("x", position.0.x);
    saved.put_int("y", position.0.y);
    saved.put_int("z", position.0.z);
    fixture.world.add_block_entity_nbt(position, &saved);
    let loaded = fixture
        .world
        .get_block_entity(&position)
        .expect("pending decorated-pot NBT loads into the chunk");
    assert!(
        loaded
            .as_any()
            .downcast_ref::<DecoratedPotBlockEntity>()
            .is_some()
    );
    loaded
}

fn item_stacks(world: &World, item: &Item) -> Vec<ItemStack> {
    world
        .entities
        .load()
        .iter()
        .filter_map(|entity| entity.get_item_entity())
        .filter_map(|entity| {
            let stack = entity
                .get_item_stack()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (stack.item.id == item.id).then(|| stack.clone())
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g03_crafting_shatters_four_bricks_with_or_without_contents() {
    let fixture = Fixture::new();
    for (round, with_contents) in [false, true].into_iter().enumerate() {
        let names = [
            format!("craft-{round}-back"),
            format!("craft-{round}-left"),
            format!("craft-{round}-right"),
            format!("craft-{round}-front"),
        ];
        let crafted = crafted_pot(&fixture, std::array::from_fn(|index| names[index].as_str()));
        assert_eq!(crafted.item, &Item::DECORATED_POT);

        let support = BlockPos::new(8 + round as i32 * 3, 64, 8);
        let position = support.up();
        fixture.set_block(support, &Block::STONE);
        fixture.set_block(position, &Block::AIR);
        fixture.give(crafted);
        fixture.use_on(support);
        assert_eq!(fixture.world.get_block(&position), &Block::DECORATED_POT);

        if with_contents {
            fixture.insert(position, &Item::DIAMOND);
        }
        fixture.give(ItemStack::new(1, &Item::DIAMOND_PICKAXE));
        fixture.dig(position);

        assert_eq!(fixture.world.get_block(&position), &Block::AIR);
        assert_eq!(item_count(&fixture.world, &Item::DECORATED_POT), 0);
        assert_eq!(
            item_count(&fixture.world, &Item::BRICK),
            (round as u32 + 1) * 4
        );
        assert_eq!(
            item_count(&fixture.world, &Item::DIAMOND),
            u32::from(with_contents)
        );
        for name in names {
            assert!(
                item_stacks(&fixture.world, &Item::BRICK)
                    .iter()
                    .any(|stack| stack.get_hover_name() == name),
                "the {name} brick template survives crafting and shattering"
            );
        }
    }
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g03_pre_upgrade_pot_without_decorations_keeps_four_bricks() {
    let fixture = Fixture::new();
    let position = BlockPos::new(8, 65, 8);
    fixture.set_block(BlockPos::new(8, 64, 8), &Block::STONE);
    let mut old_save = NbtCompound::new();
    let mut item = NbtCompound::new();
    ItemStack::new(1, &Item::DIAMOND).write_item_stack(&mut item);
    old_save.put_compound("item", item);
    let _loaded = load_pot_from_nbt(&fixture, position, old_save);

    fixture.give(ItemStack::new(1, &Item::DIAMOND_PICKAXE));
    fixture.dig(position);

    assert_eq!(fixture.world.get_block(&position), &Block::AIR);
    assert_eq!(item_count(&fixture.world, &Item::DECORATED_POT), 0);
    assert_eq!(item_count(&fixture.world, &Item::BRICK), 4);
    assert_eq!(item_count(&fixture.world, &Item::DIAMOND), 1);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g03_pre_upgrade_intact_pot_keeps_bricks_in_returned_pot() {
    let fixture = Fixture::new();
    let position = BlockPos::new(8, 65, 8);
    fixture.set_block(BlockPos::new(8, 64, 8), &Block::STONE);
    let _loaded = load_pot_from_nbt(&fixture, position, NbtCompound::new());

    fixture.give(ItemStack::EMPTY.clone());
    fixture.dig(position);

    assert_eq!(fixture.world.get_block(&position), &Block::AIR);
    assert_eq!(item_count(&fixture.world, &Item::DECORATED_POT), 1);
    assert_eq!(item_count(&fixture.world, &Item::BRICK), 0);
    let returned = take_pot_drop(&fixture.world).expect("intact decorated pot item");
    let decorations = returned
        .get_data_component::<PotDecorationsImpl>()
        .expect("legacy materials remain on the returned pot");
    assert!(decorations.decorations.iter().all(|face| {
        face.as_ref().is_some_and(|face| {
            face.item.id == Item::BRICK.id && face.count == 1 && face.patch.is_empty()
        })
    }));
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g03_explicitly_empty_pot_component_does_not_invent_bricks() {
    let fixture = Fixture::new();
    let position = fixture.place_pot(BlockPos::new(8, 64, 8), PotDecorationsImpl::EMPTY);
    fixture.give(ItemStack::new(1, &Item::DIAMOND_PICKAXE));
    fixture.dig(position);

    assert_eq!(fixture.world.get_block(&position), &Block::AIR);
    assert_eq!(item_count(&fixture.world, &Item::DECORATED_POT), 0);
    assert_eq!(item_count(&fixture.world, &Item::BRICK), 0);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g03_vanilla_sherds_compound_survives_loading_and_shattering() {
    let fixture = Fixture::new();
    let position = BlockPos::new(8, 65, 8);
    fixture.set_block(BlockPos::new(8, 64, 8), &Block::STONE);
    let decorations = PotDecorationsImpl {
        decorations: [
            Some(decoration(&Item::BRICK, 1)),
            None,
            Some(decoration(&Item::ARCHER_POTTERY_SHERD, 1)),
            None,
        ],
    };
    let raw_sherds = decorations.write_data();
    let mut vanilla_save = NbtCompound::new();
    vanilla_save.put("sherds", raw_sherds.clone());
    let block_entity = load_pot_from_nbt(&fixture, position, vanilla_save);
    let mut round_trip = NbtCompound::new();
    block_entity.write_nbt(&mut round_trip);
    assert_eq!(round_trip.get("sherds"), Some(&raw_sherds));

    fixture.give(ItemStack::new(1, &Item::DIAMOND_PICKAXE));
    fixture.dig(position);

    assert_eq!(fixture.world.get_block(&position), &Block::AIR);
    assert_eq!(item_count(&fixture.world, &Item::DECORATED_POT), 0);
    assert_eq!(item_count(&fixture.world, &Item::BRICK), 1);
    assert_eq!(item_count(&fixture.world, &Item::ARCHER_POTTERY_SHERD), 1);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g03_legacy_sherds_list_loads_and_shatters() {
    use pumpkin_nbt::tag::NbtTag;

    let fixture = Fixture::new();
    let support = BlockPos::new(8, 64, 8);
    let position = support.up();
    fixture.set_block(support, &Block::STONE);
    let mut old_save = NbtCompound::new();
    old_save.put_list(
        "sherds",
        [
            "minecraft:brick",
            "minecraft:angler_pottery_sherd",
            "minecraft:archer_pottery_sherd",
            "minecraft:brick",
        ]
        .map(|name| NbtTag::String(name.into()))
        .to_vec(),
    );
    let _loaded = load_pot_from_nbt(&fixture, position, old_save);

    fixture.give(ItemStack::new(1, &Item::DIAMOND_PICKAXE));
    fixture.dig(position);

    assert_eq!(fixture.world.get_block(&position), &Block::AIR);
    assert_eq!(item_count(&fixture.world, &Item::DECORATED_POT), 0);
    assert_eq!(item_count(&fixture.world, &Item::BRICK), 2);
    assert_eq!(item_count(&fixture.world, &Item::ANGLER_POTTERY_SHERD), 1);
    assert_eq!(item_count(&fixture.world, &Item::ARCHER_POTTERY_SHERD), 1);
    fixture.server.shutdown().await;
}
