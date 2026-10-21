use super::*;
use crate::{
    block::entities::{BlockEntity, decorated_pot::DecoratedPotBlockEntity},
    command::CommandSender,
};
use pumpkin_nbt::compound::NbtCompound;
use serde_json::json;

fn load_dynamic_sherds_pack(fixture: &Fixture) {
    let pack = fixture.directory.path().join("datapacks/dynamic_sherds");
    std::fs::create_dir_all(pack.join("data/minecraft/loot_table/blocks")).unwrap();
    std::fs::write(
        pack.join("pack.mcmeta"),
        r#"{"pack":{"min_format":94,"max_format":94,"description":"dynamic sherds regression"}}"#,
    )
    .unwrap();
    let table = json!({"pools":[{"rolls":1,"entries":[{
        "type":"minecraft:dynamic","name":"minecraft:sherds"
    }]}]})
    .to_string();
    for path in [
        "data/minecraft/loot_table/b1020b_g09_sherds.json",
        "data/minecraft/loot_table/blocks/stone.json",
        "data/minecraft/loot_table/blocks/decorated_pot.json",
    ] {
        std::fs::write(pack.join(path), &table).unwrap();
    }
    fixture.server.datapack_manager.load_all(
        fixture.directory.path(),
        &["file/dynamic_sherds".to_owned()],
        &fixture.server.recipe_manager,
    );
    fixture
        .server
        .worlds
        .store(Arc::new(vec![fixture.world.clone()]));
    assert!(
        fixture
            .server
            .datapack_manager
            .get_loot_table("minecraft:b1020b_g09_sherds")
            .is_some()
    );
}

fn execute_loot(fixture: &Fixture, source: &str) -> i32 {
    let command = format!("loot spawn 8 65 8 {source}");
    let sender = CommandSender::Console.into_source(&fixture.server);
    fixture
        .server
        .command_dispatcher
        .load()
        .execute_input(&command, &sender)
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_dynamic_sherds_require_a_real_pot() {
    let fixture = Fixture::new();
    load_dynamic_sherds_pack(&fixture);

    assert_eq!(execute_loot(&fixture, "loot b1020b_g09_sherds"), 0);
    assert_eq!(item_count(&fixture.world, &Item::BRICK), 0);

    let stone = BlockPos::new(8, 65, 8);
    fixture.set_block(stone, &Block::STONE);
    assert!(fixture.world.get_block_entity(&stone).is_none());
    assert_eq!(
        execute_loot(&fixture, "mine 8 65 8 minecraft:diamond_pickaxe"),
        0
    );
    fixture.give(ItemStack::new(1, &Item::DIAMOND_PICKAXE));
    fixture.dig(stone);
    assert_eq!(fixture.world.get_block(&stone), &Block::AIR);
    assert_eq!(item_count(&fixture.world, &Item::BRICK), 0);
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_legacy_pot_break_keeps_four_bricks() {
    let fixture = Fixture::new();
    let legacy = BlockPos::new(11, 65, 8);
    fixture.set_block(legacy.down(), &Block::STONE);
    fixture.set_block(legacy, &Block::DECORATED_POT);
    fixture
        .world
        .add_block_entity(Arc::new(DecoratedPotBlockEntity::from_nbt(
            &NbtCompound::new(),
            legacy,
        )));
    fixture.give(ItemStack::new(1, &Item::DIAMOND_PICKAXE));
    fixture.dig(legacy);
    assert_eq!(fixture.world.get_block(&legacy), &Block::AIR);
    assert!(fixture.world.get_block_entity(&legacy).is_none());
    assert_eq!(item_count(&fixture.world, &Item::DECORATED_POT), 0);
    assert_eq!(item_count(&fixture.world, &Item::BRICK), 4);
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_empty_pot_provider_emits_no_dynamic_sherds() {
    let fixture = Fixture::new();
    load_dynamic_sherds_pack(&fixture);
    let empty = fixture.place_pot(BlockPos::new(14, 64, 8), PotDecorationsImpl::EMPTY);
    assert_eq!(
        execute_loot(&fixture, "mine 14 65 8 minecraft:diamond_pickaxe"),
        0
    );
    fixture.give(ItemStack::new(1, &Item::DIAMOND_PICKAXE));
    fixture.dig(empty);
    assert_eq!(fixture.world.get_block(&empty), &Block::AIR);
    assert_eq!(item_count(&fixture.world, &Item::BRICK), 0);
    fixture.shutdown().await;
}
