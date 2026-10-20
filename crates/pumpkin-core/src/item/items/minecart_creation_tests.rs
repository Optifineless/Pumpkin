use crate::{
    entity::{
        Entity, EntityBase, death_test_world::DeathTestWorld, vehicle::minecart::MinecartEntity,
    },
    net::java::combat_test_support::TestPlayer,
    server::combat_test_support,
};
use pumpkin_data::{
    Block, BlockDirection,
    block_properties::{EnumVariants, PoweredRailLikeProperties, RailLikeProperties, RailShape},
    entity::EntityType,
    item::Item,
    item_stack::ItemStack,
    packet::CURRENT_MC_VERSION,
};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::{VarInt, java::server::play::SUseItemOn};
use pumpkin_util::{
    GameMode, Hand,
    math::{boundingbox::BoundingBox, position::BlockPos, vector3::Vector3},
};
use pumpkin_world::world::BlockFlags;
use std::sync::Arc;

#[path = "minecart_creation_test_support.rs"]
mod support;
use support::*;

const RAIL: BlockPos = BlockPos::new(10, 64, 20);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g12_experimental_cart_placement_aligns_and_rejects_overlap() {
    let mut f = Fixture::new(true).await;
    f.rail(RAIL, RailShape::NorthSouth, &Block::RAIL);
    f.equip(RAIL, &Item::MINECART, Hand::Right);
    f.use_on(RAIL, Hand::Right);
    assert_eq!(f.carts().len(), 1);
    assert_geometry(&f.carts()[0], (10.5, 64.1, 20.5, 270.0, 0.0));
    assert_eq!(f.hand_count(Hand::Right), 1);
    let (spawns, hands) = f.packets();
    assert_eq!(spawns.len(), 1, "one decoded cart spawn");
    assert_eq!(spawns[0].position, Vector3::new(10.5, 64.1, 20.5));
    assert!(
        hands
            .iter()
            .any(|(slot, stack)| *slot == 0 && stack.item_count == 1)
    );
    f.use_on(RAIL, Hand::Right);
    assert_eq!(
        f.carts().len(),
        1,
        "overlapping placement must not publish a second cart"
    );
    assert_eq!(
        f.hand_count(Hand::Right),
        1,
        "refused placement must retain the second item"
    );
    let (spawns, hands) = f.packets();
    assert!(spawns.is_empty(), "refusal sends no spawn packet");
    assert!(
        hands
            .iter()
            .any(|(slot, stack)| *slot == 0 && stack.item_count == 1),
        "refusal corrects the live hand"
    );
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g12_exact_aligned_aabb_boundaries_and_noncart_control() {
    // The adjacent box starts just inside, at, and outside the literal aligned cart face.
    for (left, accepted) in [
        (10.989999, false),
        (10.990000009536743, true),
        (10.990001, true),
    ] {
        let mut f = Fixture::new(true).await;
        f.rail(RAIL, RailShape::NorthSouth, &Block::RAIL);
        let other = f.cart(Vector3::new(11.48, 64.1, 20.5), &EntityType::CHEST_MINECART);
        other.get_entity().bounding_box.store(BoundingBox::new(
            Vector3::new(left, 64.1, 20.01),
            Vector3::new(11.97, 64.8, 20.99),
        ));
        f.equip(RAIL, &Item::MINECART, Hand::Right);
        f.use_on(RAIL, Hand::Right);
        assert_eq!(
            f.carts().len(),
            if accepted { 2 } else { 1 },
            "AABB minX={left}"
        );
        assert_eq!(f.hand_count(Hand::Right), if accepted { 1 } else { 2 });
        f.close().await;
    }
    let mut f = Fixture::new(true).await;
    f.rail(RAIL, RailShape::NorthSouth, &Block::RAIL);
    let pig = crate::entity::r#type::from_type(
        &EntityType::PIG,
        Vector3::new(10.5, 64.1, 20.5),
        &f.world,
        uuid::Uuid::new_v4(),
    );
    f.world.add_entity_silent(pig);
    f.cart(Vector3::new(100.5, 64.1, 20.5), &EntityType::MINECART);
    f.equip(RAIL, &Item::MINECART, Hand::Right);
    f.use_on(RAIL, Hand::Right);
    assert_eq!(
        f.carts().len(),
        2,
        "only overlapping minecarts obstruct creation"
    );
    assert_eq!(f.hand_count(Hand::Right), 1);
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g12_minecart_feature_legacy_slopes_and_negative_controls() {
    let mut f = Fixture::new(true).await;
    // Literal NewMinecartBehavior.adjustToRails results for zero initial velocity.
    let cases = [
        (RailShape::NorthSouth, 10.5, 64.1, 20.5, 270.0, 0.0),
        (RailShape::EastWest, 10.5, 64.1, 20.5, 0.0, 0.0),
        (RailShape::AscendingEast, 10.5, 64.6, 20.5, 0.0, -45.0),
        (RailShape::AscendingWest, 10.5, 64.6, 20.5, 180.0, -45.0),
        (RailShape::AscendingNorth, 10.5, 64.6, 20.5, 90.0, -45.0),
        (RailShape::AscendingSouth, 10.5, 64.6, 20.5, 270.0, -45.0),
        (RailShape::SouthEast, 10.75, 64.1, 20.75, 225.0, 0.0),
        (RailShape::SouthWest, 10.25, 64.1, 20.75, 315.0, 0.0),
        (RailShape::NorthWest, 10.25, 64.1, 20.25, 45.0, 0.0),
        (RailShape::NorthEast, 10.75, 64.1, 20.25, 135.0, 0.0),
    ];
    for (shape, x, y, z, yaw, pitch) in cases {
        f.world.entities.store(Arc::new(Vec::new()));
        f.rail(RAIL, shape, &Block::RAIL);
        f.equip(RAIL, &Item::MINECART, Hand::Left);
        f.use_on(RAIL, Hand::Left);
        assert_eq!(f.carts().len(), 1, "shape {shape:?}");
        assert_geometry(&f.carts()[0], (x, y, z, yaw, pitch));
        assert_eq!(f.hand_count(Hand::Left), 1);
    }
    f.close().await;
    let mut f = Fixture::new(false).await;
    for (shape, y) in [
        (RailShape::NorthSouth, 64.0625),
        (RailShape::AscendingEast, 64.5625),
        (RailShape::NorthEast, 64.0625),
    ] {
        f.world.entities.store(Arc::new(Vec::new()));
        f.rail(RAIL, shape, &Block::RAIL);
        f.equip(RAIL, &Item::MINECART, Hand::Right);
        f.use_on(RAIL, Hand::Right);
        f.use_on(RAIL, Hand::Right);
        assert_eq!(f.carts().len(), 2, "legacy mode permits overlapping carts");
        assert_geometry(&f.carts()[0], (10.5, y, 20.5, 0.0, 0.0));
        assert_eq!(f.hand_count(Hand::Right), 0);
    }
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g12_variants_powered_rails_negative_coordinates_and_creative() {
    let mut f = Fixture::new(true).await;
    let variants = [
        (&Item::MINECART, &EntityType::MINECART),
        (&Item::CHEST_MINECART, &EntityType::CHEST_MINECART),
        (&Item::HOPPER_MINECART, &EntityType::HOPPER_MINECART),
        (&Item::FURNACE_MINECART, &EntityType::FURNACE_MINECART),
        (&Item::TNT_MINECART, &EntityType::TNT_MINECART),
        (
            &Item::COMMAND_BLOCK_MINECART,
            &EntityType::COMMAND_BLOCK_MINECART,
        ),
    ];
    let negative = BlockPos::new(-10, 64, -20);
    for (item, kind) in variants {
        f.world.entities.store(Arc::new(Vec::new()));
        f.rail(negative, RailShape::EastWest, &Block::POWERED_RAIL);
        f.equip(negative, item, Hand::Right);
        f.use_on(negative, Hand::Right);
        assert_geometry(&f.carts()[0], (-9.5, 64.1, -19.5, 0.0, 0.0));
        assert_eq!(f.carts()[0].get_entity().entity_type, kind);
        assert_eq!(f.hand_count(Hand::Right), 1);
    }
    f.world.entities.store(Arc::new(Vec::new()));
    f.rail(RAIL, RailShape::AscendingWest, &Block::ACTIVATOR_RAIL);
    f.equip(RAIL, &Item::MINECART, Hand::Right);
    f.actor.player.gamemode.store(GameMode::Creative);
    f.use_on(RAIL, Hand::Right);
    assert_geometry(&f.carts()[0], (10.5, 64.6, 20.5, 180.0, -45.0));
    assert_eq!(f.hand_count(Hand::Right), 2);
    f.close().await;
}

#[path = "minecart_creation_bedrock_tests.rs"]
mod bedrock;
#[path = "minecart_creation_callback_tests.rs"]
mod callbacks;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g12_initial_rail_below_and_aligned_vertical_overlap() {
    let mut f = Fixture::new(true).await;
    f.rail(RAIL, RailShape::NorthSouth, &Block::RAIL);
    f.rail(RAIL.down(), RailShape::EastWest, &Block::RAIL);
    f.equip(RAIL, &Item::MINECART, Hand::Right);
    f.use_on(RAIL, Hand::Right);
    assert_geometry(&f.carts()[0], (10.5, 63.1, 20.5, 0.0, 0.0));
    f.close().await;
    let mut f = Fixture::new(true).await;
    f.rail(RAIL, RailShape::NorthSouth, &Block::RAIL);
    let other = f.cart(Vector3::new(10.5, 64.79, 20.5), &EntityType::MINECART);
    // Above the legacy box (maxY 64.7625), inside the aligned box (maxY 64.8).
    other.get_entity().bounding_box.store(BoundingBox::new(
        Vector3::new(10.01, 64.79, 20.01),
        Vector3::new(10.99, 65.49, 20.99),
    ));
    f.equip(RAIL, &Item::MINECART, Hand::Right);
    f.use_on(RAIL, Hand::Right);
    assert_eq!(
        f.carts().len(),
        1,
        "overlap must query the aligned bounding box"
    );
    assert_eq!(f.hand_count(Hand::Right), 2);
    f.close().await;
}
