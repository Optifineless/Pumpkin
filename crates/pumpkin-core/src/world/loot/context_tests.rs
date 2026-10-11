use super::*;
use crate::entity::{Entity, EntityBase};
use pumpkin_data::entity::EntityType;
use pumpkin_util::math::vector3::Vector3;
use std::sync::Arc;

fn test_world(path: &std::path::Path) -> Arc<crate::world::World> {
    use arc_swap::ArcSwap;
    use pumpkin_config::world::LevelConfig;
    use pumpkin_data::dimension::Dimension;
    use pumpkin_util::world_seed::Seed;
    use pumpkin_world::level::Level;
    let world = Arc::new(crate::world::World::load(
        Level::from_root_folder(
            &LevelConfig::default(),
            path.to_path_buf(),
            0,
            Dimension::OVERWORLD,
        ),
        Arc::new(ArcSwap::from_pointee(crate::world::LevelData::default(
            Seed(0),
        ))),
        Dimension::OVERWORLD,
        Arc::new(crate::block::registry::BlockRegistry::default()),
        std::sync::Weak::new(),
    ));
    crate::server::fixture_lifecycle::track_world(&world);
    world
}
#[tokio::test]
async fn live_death_context_retains_jockey_mount_and_sheared_sheep() {
    let dir = tempfile::tempdir().unwrap();
    let world = test_world(dir.path());
    let zombie = crate::entity::mob::zombie::zombie::ZombieEntity::new(Entity::new(
        world.clone(),
        Vector3::new(0.0, 64.0, 0.0),
        &EntityType::ZOMBIE,
    ));
    let chicken = crate::entity::passive::chicken::ChickenEntity::new(Entity::new(
        world.clone(),
        Vector3::new(0.0, 64.0, 0.0),
        &EntityType::CHICKEN,
    ));
    zombie
        .get_entity()
        .age
        .store(-1, std::sync::atomic::Ordering::Relaxed);
    chicken
        .get_entity()
        .add_passenger(chicken.clone(), zombie.clone());
    let mut params = build_entity_death_loot_context(
        zombie.as_ref(),
        None,
        None,
        None,
        &LootContextParameters::default(),
    );
    let state = params.this_entity_state.as_ref().unwrap();
    assert_eq!(
        state.vehicle.as_ref().unwrap().entity_type,
        Some(&EntityType::CHICKEN)
    );
    assert_eq!(state.flags.get("is_baby"), Some(&true));
    params.last_damage_player_state = Some(EntityLootState {
        entity_type: Some(&EntityType::PLAYER),
        ..Default::default()
    });
    let table = pumpkin_data::loot_table::get_loot_table("entities/zombie").unwrap();
    assert_eq!(
        super::tests::count(
            &generate_loot_with_context(table, 1, &params),
            &Item::MUSIC_DISC_LAVA_CHICKEN
        ),
        1
    );
    let sheep = crate::entity::passive::sheep::SheepEntity::new(Entity::new(
        world,
        Vector3::new(0.0, 64.0, 0.0),
        &EntityType::SHEEP,
    ));
    let table = pumpkin_data::loot_table::get_loot_table("entities/sheep").unwrap();
    let params = build_entity_death_loot_context(
        sheep.as_ref(),
        None,
        None,
        None,
        &LootContextParameters::default(),
    );
    assert_eq!(
        super::tests::count(
            &generate_loot_with_context(table, 1, &params),
            &Item::WHITE_WOOL
        ),
        1
    );
    sheep.set_sheared(true);
    let params = build_entity_death_loot_context(
        sheep.as_ref(),
        None,
        None,
        None,
        &LootContextParameters::default(),
    );
    assert_eq!(
        super::tests::count(
            &generate_loot_with_context(table, 1, &params),
            &Item::WHITE_WOOL
        ),
        0
    );
    crate::server::fixture_lifecycle::finish().await;
}
#[tokio::test]
async fn command_kill_context_never_inherits_remembered_player_credit() {
    let dir = tempfile::tempdir().unwrap();
    let world = test_world(dir.path());
    let sheep = crate::entity::passive::sheep::SheepEntity::new(Entity::new(
        world,
        Vector3::new(0.0, 64.0, 0.0),
        &EntityType::SHEEP,
    ));
    let base = LootContextParameters {
        killed_by_player: Some(true),
        last_damage_player_state: Some(EntityLootState {
            entity_type: Some(&EntityType::PLAYER),
            ..Default::default()
        }),
        ..Default::default()
    };
    let context = build_command_kill_loot_context(sheep.as_ref(), None, &base);
    assert!(context.last_damage_player_state.is_none());
    assert_eq!(
        context.this_entity_state.unwrap().components["minecraft:sheep/color"],
        "white"
    );
    let context = build_command_kill_loot_context(sheep.as_ref(), Some(sheep.as_ref()), &base);
    assert!(context.attacking_entity_state.is_some());
    assert!(context.direct_attacking_entity_state.is_some());
    assert!(context.last_damage_player_state.is_none());
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn block_context_supplies_banner_patterns_and_preserves_modifier_order() {
    use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
    use pumpkin_util::math::position::BlockPos;
    let dir = tempfile::tempdir().unwrap();
    let world = test_world(dir.path());
    let pos = BlockPos::new(0, 64, 0);
    let banner = crate::block::entities::banner::BannerBlockEntity::new(pos);
    let mut pattern = NbtCompound::new();
    pattern.put_string("pattern", "minecraft:stripe_bottom".to_owned());
    pattern.put_string("color", "red".to_owned());
    *banner.patterns.lock().unwrap() = Some(vec![NbtTag::Compound(pattern)]);
    *banner.custom_name.lock().unwrap() = Some(r#"{"text":"Original"}"#.to_owned());
    world.add_block_entity(Arc::new(banner));
    let params = build_block_loot_context(&world, &pos, &LootContextParameters::default());
    let table = super::tests::parse(
        &serde_json::json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:white_banner","modifier":[{"type":"minecraft:copy_components","source":"block_entity","include":["minecraft:banner_patterns","minecraft:custom_name"]},{"type":"minecraft:set_components","components":{"minecraft:custom_name":{"text":"After"}}}]}]}]}),
    );
    let drops = generate_dynamic_loot_with_context(&table, 1, &params);
    let patterns = drops[0]
        .get_data_component::<pumpkin_data::data_component_impl::BannerPatternsImpl>()
        .unwrap();
    assert_eq!(patterns.layers.len(), 1);
    assert_eq!(patterns.layers[0].pattern, "minecraft:stripe_bottom");
    assert_eq!(
        drops[0]
            .get_data_component::<pumpkin_data::data_component_impl::CustomNameImpl>()
            .unwrap()
            .name
            .clone()
            .get_text(),
        "After"
    );
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn shulker_components_remain_typed_and_modifiers_can_remove_them() {
    use crate::block::entities::shulker_box::ShulkerBoxBlockEntity;
    use pumpkin_data::data_component_impl::{
        ContainerImpl, FireworkExplosionImpl, FireworkExplosionShape,
    };
    use pumpkin_util::math::position::BlockPos;
    let dir = tempfile::tempdir().unwrap();
    let world = test_world(dir.path());
    let pos = BlockPos::new(0, 64, 0);
    let shulker = ShulkerBoxBlockEntity::new(pos);
    let mut firework = ItemStack::new(1, &Item::FIREWORK_STAR);
    firework.set_data_component(FireworkExplosionImpl::new(
        FireworkExplosionShape::SmallBall,
        vec![16711680],
        vec![255],
        true,
        true,
    ));
    shulker.items.write().unwrap()[7] = firework;
    world.add_block_entity(Arc::new(shulker));
    let params = build_block_loot_context(&world, &pos, &LootContextParameters::default());
    for remove in [false, true] {
        let mut modifiers =
            vec![serde_json::json!({"type":"minecraft:copy_components","source":"block_entity"})];
        if remove {
            modifiers.push(serde_json::json!({"type":"minecraft:set_components","components":{"!minecraft:container":{}}}));
        }
        let table = super::tests::parse(
            &serde_json::json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:shulker_box","modifier":modifiers}]}]}),
        );
        let drops = generate_dynamic_loot_with_context(&table, 1, &params);
        let container = drops[0].get_data_component::<ContainerImpl>();
        if remove {
            assert!(container.is_none());
        } else {
            let contents = &container.unwrap().items;
            assert_eq!(contents[0].0, 7);
            let explosion = contents[0]
                .1
                .get_data_component::<FireworkExplosionImpl>()
                .unwrap();
            assert_eq!(explosion.colors, [16711680]);
            assert_eq!(explosion.fade_colors, [255]);
        }
    }
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn seed_zero_without_a_sequence_continues_the_world_source() {
    let dir = tempfile::tempdir().unwrap();
    let world = test_world(dir.path());
    *world.loot_random.lock().unwrap() =
        pumpkin_util::random::legacy_rand::LegacyRand::from_seed(123);
    let table = super::tests::parse(
        &serde_json::json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:apple","modifier":{"type":"minecraft:set_count","count":{"type":"minecraft:uniform","min":1,"max":10}}}]}]}),
    );
    let params = LootContextParameters {
        world: Some(world),
        ..Default::default()
    };
    // java.util.Random(123).nextInt(10) + 1: 3, 1.
    for expected in [3, 1] {
        assert_eq!(
            super::tests::count(
                &generate_dynamic_loot_with_context(&table, 0, &params),
                &Item::APPLE
            ),
            expected
        );
    }
    crate::server::fixture_lifecycle::finish().await;
}
