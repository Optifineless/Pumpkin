#![allow(clippy::unwrap_used)]

use super::{enable_spawn_protection, support::*};
use crate::entity::{area_effect_cloud::AreaEffectCloudEntity, living::LivingEntity};
use pumpkin_data::{
    Block, block_properties::WaterLikeProperties, entity::EntityType, fluid::Fluid, item::Item,
    item_stack::ItemStack,
};
use pumpkin_util::{Hand, PermissionLvl, math::position::BlockPos};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g12_bottle_waterlogged_source_use() {
    let fixture = Fixture::new();
    let position = WATER_POSITION;
    let waterlogged_slab = Block::OAK_SLAB.default_state.set_waterlogged(true).unwrap();
    let interact = register_interact(&fixture, Hand::Right, false, None);

    let mut uses = Vec::new();
    for (hand, expected_potions, expected_stat) in [(Hand::Right, 1u32, 1i32), (Hand::Left, 2, 2)] {
        fixture.put_state(position, waterlogged_slab);
        fixture.hold_bottle(hand, 2);

        let (fluid, fluid_state) = fixture.world.get_fluid_and_fluid_state(&position);
        assert!(fluid.matches_type(&Fluid::WATER));
        assert!(fluid_state.is_source);
        let eye_y = fixture.actor.player.eye_position().y;
        let slab_top = f64::from(position.0.y) + 0.5;
        let water_top = f64::from(position.0.y)
            + f64::from(
                fixture
                    .world
                    .get_fluid_height(&position, fluid, &fluid_state),
            );
        assert!(eye_y > slab_top && eye_y < water_top);

        fixture.use_item(hand);

        let held = fixture.actor.player.inventory().get_stack_in_hand(hand);
        let potions = fixture.potion_count();
        let stat = fixture.used_bottle_stat();
        let state = fixture.world.get_block_state_id(&position);
        uses.push((
            hand,
            held,
            expected_potions,
            expected_stat,
            potions,
            stat,
            state,
        ));
    }

    // Exposed source water remains the control for the same use-item packet path.
    let potions_before_exposed_source = fixture.potion_count();
    let stat_before_exposed_source = fixture.used_bottle_stat();
    fixture.put_source_water(position);
    fixture.hold_bottle(Hand::Right, 1);
    fixture.use_item(Hand::Right);
    assert_eq!(
        fixture.actor.player.inventory().held_item().item,
        &Item::POTION
    );
    assert_eq!(fixture.actor.player.inventory().held_item().item_count, 1);
    assert_eq!(fixture.potion_count(), potions_before_exposed_source + 1);
    assert_eq!(fixture.used_bottle_stat(), stat_before_exposed_source + 1);

    for (hand, held, expected_potions, expected_stat, potions, stat, state) in uses {
        assert_eq!(held.item, &Item::GLASS_BOTTLE);
        assert_eq!(potions, expected_potions, "waterlogged source use {hand:?}");
        assert_eq!(stat, expected_stat, "waterlogged source use {hand:?}");
        assert_eq!(held.item_count, 1);
        assert_eq!(state, waterlogged_slab.id);
        assert!(state.is_waterlogged());
    }
    let interactions = interact.events.lock().unwrap();
    assert_eq!(interactions.len(), 3);
    assert!(interactions.iter().all(|event| {
        matches!(event.action, crate::plugin::api::events::player::player_interact_event::InteractAction::RightClickBlock)
            && event.clicked_pos == Some(position)
            && (event.block == &Block::OAK_SLAB || event.block == &Block::WATER)
    }));
    fixture.finish().await;
}

fn assert_no_bottle_output(fixture: &Fixture, hand: Hand) {
    let held = fixture.actor.player.inventory().get_stack_in_hand(hand);
    assert_eq!(held.item, &Item::GLASS_BOTTLE);
    assert_eq!(held.item_count, 1);
    assert_eq!(fixture.potion_count(), 0);
    assert_eq!(fixture.used_bottle_stat(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g12_bottle_ray_source_controls() {
    flowing_water_control().await;
    blocking_outline_control().await;
    miss_control().await;
    out_of_reach_control().await;
    protected_source_control().await;
    callback_replacement_control().await;
    dragon_breath_priority_control().await;
}

async fn flowing_water_control() {
    let fixture = Fixture::new();
    let flowing = WaterLikeProperties { level: 1 }
        .to_state_id(&Block::WATER)
        .to_state();
    fixture.put_state(WATER_POSITION, flowing);
    fixture.hold_bottle(Hand::Right, 1);
    fixture.use_item(Hand::Right);
    assert_no_bottle_output(&fixture, Hand::Right);
    fixture.finish().await;
}

async fn blocking_outline_control() {
    let fixture = Fixture::new();
    fixture.put_source_water(WATER_POSITION);
    fixture.put_state(BlockPos::new(8, 65, 10), Block::STONE.default_state);
    fixture.hold_bottle(Hand::Right, 1);
    fixture.use_item(Hand::Right);
    assert_no_bottle_output(&fixture, Hand::Right);
    fixture.finish().await;
}

async fn miss_control() {
    let fixture = Fixture::new();
    fixture.hold_bottle(Hand::Right, 1);
    let interact = register_interact(&fixture, Hand::Right, false, None);
    fixture.use_item(Hand::Right);
    assert_no_bottle_output(&fixture, Hand::Right);
    let events = interact.events.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0].action,
        crate::plugin::api::events::player::player_interact_event::InteractAction::RightClickAir
    ));
    assert_eq!(events[0].clicked_pos, None);
    drop(events);
    fixture.finish().await;
}

async fn out_of_reach_control() {
    let fixture = Fixture::new();
    fixture.put_source_water(BlockPos::new(8, 65, 14));
    fixture.hold_bottle(Hand::Right, 1);
    let interact = register_interact(&fixture, Hand::Right, false, None);
    fixture.use_item(Hand::Right);
    assert_no_bottle_output(&fixture, Hand::Right);
    assert!(interact.events.lock().unwrap()[0].clicked_pos.is_none());
    fixture.finish().await;
}

async fn protected_source_control() {
    let fixture = Fixture::new();
    fixture.put_source_water(WATER_POSITION);
    fixture.hold_bottle(Hand::Right, 1);
    fixture
        .actor
        .player
        .permission_lvl
        .store(PermissionLvl::Zero);
    enable_spawn_protection(&fixture, BlockPos::new(8, 64, 8));
    fixture.use_item(Hand::Right);
    assert_no_bottle_output(&fixture, Hand::Right);
    fixture.finish().await;
}

async fn callback_replacement_control() {
    let fixture = Fixture::new();
    fixture.put_source_water(WATER_POSITION);
    let mut replacement = ItemStack::new(1, &Item::GLASS_BOTTLE);
    replacement.set_custom_name("callback bottle".to_owned());
    let replacement_uid = replacement.uid;
    fixture.hold_bottle(Hand::Right, 1);
    register_interact(&fixture, Hand::Right, false, Some(replacement));
    fixture.use_item(Hand::Right);
    let held = fixture.actor.player.inventory().held_item();
    assert_eq!(held.uid, replacement_uid);
    assert_eq!(held.item, &Item::GLASS_BOTTLE);
    assert_eq!(held.item_count, 1);
    assert_eq!(
        held.get_custom_name().map(|name| name.clone().get_text()),
        Some("callback bottle".to_owned())
    );
    assert_eq!(fixture.potion_count(), 0);
    assert_eq!(fixture.used_bottle_stat(), 0);
    fixture.finish().await;
}

async fn dragon_breath_priority_control() {
    let fixture = Fixture::new();
    fixture.put_source_water(WATER_POSITION);
    fixture.hold_bottle(Hand::Right, 1);
    let player_position = fixture.actor.player.position();
    let dragon = std::sync::Arc::new(LivingEntity::new(crate::entity::Entity::new(
        fixture.world.clone(),
        player_position,
        &EntityType::ENDER_DRAGON,
    )));
    let cloud = AreaEffectCloudEntity::new(crate::entity::Entity::new(
        fixture.world.clone(),
        player_position,
        &EntityType::AREA_EFFECT_CLOUD,
    ));
    let cloud_entity = cloud
        .cast_any()
        .downcast_ref::<AreaEffectCloudEntity>()
        .unwrap();
    cloud_entity.set_owner(Some(dragon.as_ref()));
    fixture
        .world
        .entities
        .store(std::sync::Arc::new(vec![dragon, cloud.clone()]));
    fixture.use_item(Hand::Right);
    assert_eq!(
        fixture.actor.player.inventory().held_item().item,
        &Item::DRAGON_BREATH
    );
    assert_eq!(cloud_entity.radius(), 2.5);
    assert_eq!(fixture.world.get_block(&WATER_POSITION), &Block::WATER);
    assert_eq!(fixture.potion_count(), 0);
    assert_eq!(fixture.used_bottle_stat(), 1);
    fixture.finish().await;
}
