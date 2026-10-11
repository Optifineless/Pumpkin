use std::sync::Arc;

use crate::{
    entity::EntityBase, net::java::combat_test_support::TestPlayer, server::combat_test_support,
    world::spawn_test_support,
};
use pumpkin_data::{
    Block, BlockDirection, biome::Biome, item::Item, item_stack::ItemStack,
    statistic::StatisticCategory,
};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::{codec::var_int::VarInt, java::server::play::SUseItem};
use pumpkin_util::{
    Hand,
    math::{position::BlockPos, vector3::Vector3},
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review_offhand_xp_bottle_preserves_mainhand_sword() {
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    spawn_test_support::publish(
        &world,
        spawn_test_support::proto(&Biome::PLAINS, &Block::STONE),
    );
    let fixture = TestPlayer::new(&world);
    fixture
        .player
        .get_entity()
        .set_pos(Vector3::new(8.5, 64.0, 8.5));
    let sword = ItemStack::new(1, &Item::DIAMOND_SWORD);
    fixture.player.inventory().set_stack(0, sword.clone());
    fixture
        .player
        .inventory()
        .set_stack(40, ItemStack::new(2, &Item::EXPERIENCE_BOTTLE));
    fixture.client().handle_use_item(
        &fixture.player,
        &SUseItem {
            hand: VarInt(1),
            sequence: VarInt(1),
            yaw: 0.0,
            pitch: 0.0,
        },
        &server,
    );
    assert!(fixture.player.inventory().held_item().are_equal(&sword));
    assert_eq!(fixture.player.inventory().off_hand_item().item_count, 1);
    assert!(world.level.shutdown().await.is_ok());
    crate::server::fixture_lifecycle::finish().await;
}

async fn check_both_hands(item: &'static Item) {
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    let mut chunk = spawn_test_support::proto(&Biome::PLAINS, &Block::STONE);
    chunk.set_block_state(8, 65, 11, Block::WATER.default_state);
    spawn_test_support::publish(&world, chunk);
    let fixture = TestPlayer::new(&world);
    fixture
        .player
        .get_entity()
        .set_pos(Vector3::new(8.5, 64.0, 8.5));
    fixture.player.get_entity().set_rotation(0.0, 0.0);
    fixture.player.get_entity().set_fall_flying(true);
    let main = ItemStack::new(2, item);
    fixture.player.inventory().set_stack(0, main.clone());
    fixture
        .player
        .inventory()
        .set_stack(40, ItemStack::new(2, item));
    let stack = fixture.player.inventory().off_hand_item();
    server
        .item_registry
        .on_use_with_rotation(&stack, &fixture.player, 0.0, 0.0, Hand::Left);
    assert!(
        fixture.player.inventory().held_item().are_equal(&main),
        "{} main hand changed",
        item.registry_key
    );
    assert_eq!(
        fixture.player.inventory().off_hand_item().item_count,
        1,
        "{} must actually succeed",
        item.registry_key
    );
    assert!(world.level.shutdown().await.is_ok());
}

macro_rules! offhand_test {
    ($name:ident, $item:ident) => {
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn $name() {
            check_both_hands(&Item::$item).await;
        }
    };
}
offhand_test!(review_offhand_egg_with_eggs_in_both_hands, BLUE_EGG);
offhand_test!(review_offhand_pearl_with_pearls_in_both_hands, ENDER_PEARL);
offhand_test!(review_offhand_boat_with_boats_in_both_hands, OAK_BOAT);
offhand_test!(
    review_offhand_rocket_with_rockets_in_both_hands,
    FIREWORK_ROCKET
);
offhand_test!(review_offhand_book_with_books_in_both_hands, KNOWLEDGE_BOOK);
offhand_test!(review_offhand_eye_with_eyes_in_both_hands, ENDER_EYE);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review_raising_shield_does_not_increment_used_stat() {
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    spawn_test_support::publish(
        &world,
        spawn_test_support::proto(&Biome::PLAINS, &Block::STONE),
    );
    let fixture = TestPlayer::new(&world);
    fixture
        .player
        .get_entity()
        .set_pos(Vector3::new(8.5, 64.0, 8.5));
    fixture
        .player
        .inventory()
        .set_stack(40, ItemStack::new(1, &Item::SHIELD));
    fixture.client().handle_use_item(
        &fixture.player,
        &SUseItem {
            hand: VarInt(1),
            sequence: VarInt(1),
            yaw: 0.0,
            pitch: 0.0,
        },
        &server,
    );
    assert_eq!(
        *fixture.player.living_entity.active_hand.lock().unwrap(),
        Some(Hand::Left)
    );
    assert_eq!(
        fixture
            .player
            .stats
            .lock()
            .unwrap()
            .get(StatisticCategory::Used, i32::from(Item::SHIELD.id)),
        0
    );
    assert!(world.level.shutdown().await.is_ok());
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review_one_click_fills_one_bottle() {
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    let mut chunk = spawn_test_support::proto(&Biome::PLAINS, &Block::STONE);
    let target = BlockPos::new(8, 65, 11);
    chunk.set_block_state(8, 65, 11, Block::WATER.default_state);
    spawn_test_support::publish(&world, chunk);
    let fixture = TestPlayer::new(&world);
    fixture
        .player
        .get_entity()
        .set_pos(Vector3::new(8.5, 64.0, 8.5));
    let bottles = ItemStack::new(3, &Item::GLASS_BOTTLE);
    fixture.player.inventory().set_stack(40, bottles.clone());
    let mut stack = bottles.clone();
    let result = server.item_registry.use_on_block(
        &mut stack,
        &fixture.player,
        target,
        BlockDirection::Up,
        Vector3::new(0.5, 1.0, 0.5),
        &Block::WATER,
        &server,
    );
    // A block click followed by UseItem has just one BottleItem.use fill.
    fixture.player.inventory().set_stack(40, stack);
    fixture.client().handle_use_item(
        &fixture.player,
        &SUseItem {
            hand: VarInt(1),
            sequence: VarInt(2),
            yaw: 0.0,
            pitch: 0.0,
        },
        &server,
    );
    let potions: u32 = (0..41)
        .map(|slot| fixture.player.inventory().get_stack(slot))
        .filter(|stack| stack.item == &Item::POTION)
        .map(|stack| u32::from(stack.item_count))
        .sum();
    assert_eq!(potions, 1);
    assert_eq!(fixture.player.inventory().off_hand_item().item_count, 2);
    assert!(matches!(
        result,
        crate::block::registry::BlockActionResult::Pass
    ));
    assert!(world.level.shutdown().await.is_ok());
    crate::server::fixture_lifecycle::finish().await;
}

#[test]
fn review_plugin_debug_includes_remainder_template_and_block_item() {
    use crate::entity::player::advancement::trigger::AdvancementTrigger;
    use pumpkin_data::data_component_impl::{CustomNameImpl, UseRemainderImpl};
    let mut template = ItemStack::new(2, &Item::BOWL);
    template.set_data_component(CustomNameImpl {
        name: pumpkin_util::text::TextComponent::text("Review template"),
    });
    let remainder = UseRemainderImpl {
        remainder: None,
        template: Some(Box::new(template.clone())),
    };
    let trigger = AdvancementTrigger::ItemUsedOnBlock {
        position: BlockPos::new(1, 2, 3),
        item: template,
        state: Block::STONE.default_state.id,
    };
    assert!(format!("{remainder:?}").contains("Review template"));
    assert!(format!("{trigger:?}").contains("Review template"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review_shearing_unsmoked_hive_releases_angry_bees() {
    use crate::block::entities::beehive::BeehiveBlockEntity;
    use pumpkin_data::{block_properties::BeeNestLikeProperties, entity::EntityType};
    use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    let pos = BlockPos::new(8, 65, 11);
    let mut chunk = spawn_test_support::proto(&Biome::PLAINS, &Block::STONE);
    let mut props = BeeNestLikeProperties::default(&Block::BEEHIVE);
    props.honey_level = 5;
    chunk.set_block_state(8, 65, 11, props.to_state_id(&Block::BEEHIVE).to_state());
    spawn_test_support::publish(&world, chunk);
    let fixture = TestPlayer::new(&world);
    fixture
        .player
        .get_entity()
        .set_pos(Vector3::new(8.5, 64.0, 8.5));
    let hive = Arc::new(BeehiveBlockEntity::new(pos));
    let mut data = NbtCompound::new();
    data.put_string("id", "minecraft:bee".into());
    let mut occupant = NbtCompound::new();
    occupant.put_compound("entity_data", data);
    *hive.bees.lock().unwrap() = Some(vec![NbtTag::Compound(occupant)]);
    world.add_block_entity(hive.clone());
    let mut shears = ItemStack::new(1, &Item::SHEARS);
    fixture.player.inventory().set_stack(0, shears.clone());
    let result = server.item_registry.use_on_block(
        &mut shears,
        &fixture.player,
        pos,
        BlockDirection::Up,
        Vector3::new(0.5, 1.0, 0.5),
        &Block::BEEHIVE,
        &server,
    );
    assert!(result.consumes_action());
    assert!(hive.bees.lock().unwrap().as_ref().unwrap().is_empty());
    let bees = world.get_entities_at_box(
        &pumpkin_util::math::boundingbox::BoundingBox::from_block(&pos).expand_all(3.0),
    );
    let bee = bees
        .iter()
        .find(|entity| entity.get_entity().entity_type == &EntityType::BEE)
        .unwrap();
    assert!(
        bee.get_mob()
            .unwrap()
            .get_mob_entity()
            .get_target()
            .is_some()
    );
    assert!(world.level.shutdown().await.is_ok());
    crate::server::fixture_lifecycle::finish().await;
}
