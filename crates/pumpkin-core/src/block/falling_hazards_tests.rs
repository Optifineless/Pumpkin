use super::*;
use crate::{
    block::blocks::{
        dirt_path::DirtPathBlock, dripstone::DripstoneBlock, plant::wither_rose::WitherRoseBlock,
    },
    entity::{EntityBase, death_test_world::DeathTestWorld},
    item::{ItemBehaviour, items::ender_eye::EnderEyeItem},
    world::spawn_test_support::{proto, publish},
};
use pumpkin_data::{
    biome::Biome,
    block_properties::{
        EndPortalFrameLikeProperties, PointedDripstoneLikeProperties, SpeleothemThickness,
        VerticalDirection,
    },
    effect::StatusEffect,
    entity::EntityType,
    item::Item,
};
use pumpkin_util::GameMode;
use pumpkin_world::world::BlockFlags;

fn put(world: &Arc<World>, pos: BlockPos, state: BlockStateId) {
    world.set_block_state(&pos, state, BlockFlags::FORCE_STATE);
}

fn dripstone(tip: VerticalDirection, thickness: SpeleothemThickness) -> BlockStateId {
    let mut props = PointedDripstoneLikeProperties::default(&Block::POINTED_DRIPSTONE);
    props.vertical_direction = tip;
    props.thickness = thickness;
    props.to_state_id(&Block::POINTED_DRIPSTONE)
}

fn unsupported(world: &Arc<World>, pos: BlockPos, direction: BlockDirection) {
    let neighbor = pos.offset(direction.to_offset());
    let state = world.get_block_state_id(&pos);
    assert_eq!(
        DripstoneBlock.get_state_for_neighbor_update(GetStateForNeighborUpdateArgs {
            world,
            block: &Block::POINTED_DRIPSTONE,
            state_id: state,
            position: &pos,
            direction,
            neighbor_position: &neighbor,
            neighbor_state_id: world.get_block_state_id(&neighbor),
        }),
        state
    );
    assert!(
        world
            .level
            .is_block_tick_scheduled(&pos, &Block::POINTED_DRIPSTONE)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unsupported_stalactite_falls_and_damages_an_entity_below() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let root = BlockPos::new(4, 73, 4);
    for (offset, thickness) in [
        (0, SpeleothemThickness::Base),
        (1, SpeleothemThickness::Frustum),
        (2, SpeleothemThickness::Tip),
    ] {
        put(
            &world,
            root.down_height(offset),
            dripstone(VerticalDirection::Down, thickness),
        );
    }
    unsupported(&world, root, BlockDirection::Up);
    DripstoneBlock.on_scheduled_tick(OnScheduledTickArgs {
        world: &world,
        block: &Block::POINTED_DRIPSTONE,
        position: &root,
    });
    for offset in 0..3 {
        assert!(world.get_block_state(&root.down_height(offset)).is_air());
    }
    let falling: Vec<_> = world
        .entities
        .load()
        .iter()
        .filter(|entity| entity.get_entity().entity_type == &EntityType::FALLING_BLOCK)
        .cloned()
        .collect();
    assert_eq!(falling.len(), 3);
    let cow = fixture.mob(&EntityType::COW);
    cow.get_entity().set_pos(Vector3::new(4.5, 64.0, 4.5));
    cow.get_living_entity().unwrap().set_max_health(100.0);
    cow.get_living_entity().unwrap().set_health(100.0);
    for _ in 0..100 {
        for entity in &falling {
            if !entity.get_entity().is_removed() {
                entity.tick(entity.as_ref(), &fixture.server);
            }
        }
    }
    assert_eq!(cow.get_living_entity().unwrap().health.load(), 64.0);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn falling_stalactite_column_includes_sulfur_spikes() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let root = BlockPos::new(4, 73, 4);
    // Keep the supplied column thickness instead of running pointed-dripstone placement logic.
    world.set_block_state(
        &root,
        dripstone(VerticalDirection::Down, SpeleothemThickness::Base),
        BlockFlags::FORCE_STATE | BlockFlags::SKIP_BLOCK_ADDED_CALLBACK,
    );
    let mut tip = PointedDripstoneLikeProperties::default(&Block::SULFUR_SPIKE);
    tip.vertical_direction = VerticalDirection::Down;
    tip.thickness = SpeleothemThickness::Tip;
    put(&world, root.down(), tip.to_state_id(&Block::SULFUR_SPIKE));
    DripstoneBlock.on_scheduled_tick(OnScheduledTickArgs {
        world: &world,
        block: &Block::POINTED_DRIPSTONE,
        position: &root,
    });
    let falling: Vec<_> = world
        .entities
        .load()
        .iter()
        .filter(|entity| entity.get_entity().entity_type == &EntityType::FALLING_BLOCK)
        .cloned()
        .collect();
    assert_eq!(falling.len(), 2);
    assert!(world.get_block_state(&root.down()).is_air());
    let cow = fixture.mob(&EntityType::COW);
    cow.get_entity().set_pos(Vector3::new(4.5, 64.0, 4.5));
    cow.get_entity()
        .set_damage_immunity(pumpkin_data::damage::DamageType::FALLING_BLOCK, true);
    cow.get_living_entity().unwrap().set_max_health(100.0);
    cow.get_living_entity().unwrap().set_health(100.0);
    for _ in 0..100 {
        for entity in &falling {
            if !entity.get_entity().is_removed() {
                entity.tick(entity.as_ref(), &fixture.server);
            }
        }
    }
    assert_eq!(cow.get_living_entity().unwrap().health.load(), 60.0);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unsupported_stalagmite_breaks_with_drops_after_its_tick() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::AIR));
    let pos = BlockPos::new(5, 64, 5);
    put(
        &world,
        pos,
        dripstone(VerticalDirection::Up, SpeleothemThickness::Tip),
    );
    unsupported(&world, pos, BlockDirection::Down);
    assert_eq!(world.get_block(&pos), &Block::POINTED_DRIPSTONE);
    DripstoneBlock.on_scheduled_tick(OnScheduledTickArgs {
        world: &world,
        block: &Block::POINTED_DRIPSTONE,
        position: &pos,
    });
    assert!(world.get_block_state(&pos).is_air());
    assert!(
        world
            .entities
            .load()
            .iter()
            .any(
                |entity| entity.get_item_entity().is_some_and(|entity| entity
                    .get_item_stack()
                    .lock()
                    .unwrap()
                    .item
                    == &Item::POINTED_DRIPSTONE)
            )
    );
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn falling_onto_a_stalagmite_tip_deals_stalagmite_damage() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let pos = BlockPos::new(5, 64, 5);
    for (tip, thickness, expected_health) in [
        (VerticalDirection::Up, SpeleothemThickness::Tip, 15.0),
        (VerticalDirection::Up, SpeleothemThickness::Frustum, 20.0),
        (VerticalDirection::Down, SpeleothemThickness::Tip, 20.0),
    ] {
        put(&world, pos, dripstone(tip, thickness));
        let cow = fixture.mob(&EntityType::COW);
        cow.get_living_entity().unwrap().set_max_health(20.0);
        cow.get_living_entity().unwrap().set_health(20.0);
        cow.get_entity().set_pos(Vector3::new(5.5, 65.0, 5.5));
        // A FALL immunity distinguishes the tip's STALAGMITE source from ordinary fall damage.
        cow.get_entity()
            .set_damage_immunity(pumpkin_data::damage::DamageType::FALL, true);
        DripstoneBlock.on_landed_upon(OnLandedUponArgs {
            world: &world,
            position: &pos,
            fall_distance: 3.0,
            entity: cow.as_ref(),
        });
        assert_eq!(
            cow.get_living_entity().unwrap().health.load(),
            expected_health
        );
    }
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sliding_down_a_honey_side_resets_fall_distance() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let pos = BlockPos::new(5, 66, 5);
    put(&world, pos, Block::HONEY_BLOCK.default_state.id);
    let player = fixture.player("slider");
    player
        .get_entity()
        .on_ground
        .store(false, std::sync::atomic::Ordering::Relaxed);
    // Feet below the top, body intersects the full inside cube while outside its inset collider.
    player.get_entity().set_pos(Vector3::new(6.25, 65.5, 5.5));
    player.known_movement.record(Vector3::new(0.2, -0.8, 0.1));
    player.living_entity.fall_distance.store(12.0);
    player.get_entity().tick_block_collisions(player.as_ref());
    assert_eq!(player.living_entity.fall_distance.load(), 0.0);
    assert!((player.get_entity().velocity.load().y - (-0.12740000247955322)).abs() < 1.0E-10);
    player.living_entity.fall_distance.store(12.0);
    player
        .living_entity
        .fall(player.as_ref(), -0.1274, false, false);
    assert_eq!(player.living_entity.fall_distance.load(), 0.0);
    player
        .living_entity
        .fall(player.as_ref(), -0.2, true, false);
    assert_eq!(player.living_entity.health.load(), 20.0);
    player.get_entity().set_pos(Vector3::new(6.5, 65.5, 5.5));
    player.living_entity.fall_distance.store(12.0);
    player.get_entity().tick_block_collisions(player.as_ref());
    assert_eq!(player.living_entity.fall_distance.load(), 12.0);
    player
        .living_entity
        .fall(player.as_ref(), -0.2, true, false);
    assert_eq!(player.living_entity.health.load(), 11.0);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dirt_path_reverting_to_dirt_lifts_a_standing_entity() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let pos = BlockPos::new(5, 64, 5);
    put(&world, pos, Block::DIRT_PATH.default_state.id);
    let cow = fixture.mob(&EntityType::COW);
    cow.get_entity().set_pos(Vector3::new(5.5, 64.9375, 5.5));
    let spectator = fixture.player("path-spectator");
    spectator.set_gamemode(GameMode::Spectator);
    spectator
        .get_entity()
        .set_pos(Vector3::new(5.5, 64.9375, 5.5));
    DirtPathBlock.on_scheduled_tick(OnScheduledTickArgs {
        world: &world,
        block: &Block::DIRT_PATH,
        position: &pos,
    });
    assert_eq!(world.get_block(&pos), &Block::DIRT);
    assert_eq!(cow.get_entity().pos.load().y, 65.0);
    assert_eq!(spectator.get_entity().pos.load().y, 64.9375);

    cow.get_entity().set_pos(Vector3::new(5.5, 64.9375, 5.5));
    put(&world, pos.up(), Block::STONE.default_state.id);
    let player = fixture.player("placer");
    let packet = SUseItemOn {
        hand: 0.into(),
        position: pos,
        face: 1.into(),
        cursor_pos: Vector3::new(0.5, 1.0, 0.5),
        inside_block: false,
        is_against_world_border: false,
        sequence: 0.into(),
    };
    let state = DirtPathBlock.on_place(OnPlaceArgs {
        server: &fixture.server,
        world: &world,
        block: &Block::DIRT_PATH,
        position: &pos,
        direction: BlockDirection::Up,
        player: &player,
        replacing: BlockIsReplacing::None,
        use_item_on: &packet,
    });
    assert_eq!(state, Block::DIRT.default_state.id);
    assert_eq!(cow.get_entity().pos.load().y, 65.0);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn eye_insertion_lifts_an_entity_standing_on_the_frame() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let pos = BlockPos::new(5, 64, 5);
    put(&world, pos, Block::END_PORTAL_FRAME.default_state.id);
    let cow = fixture.mob(&EntityType::COW);
    cow.get_entity().set_pos(Vector3::new(5.5, 64.8125, 5.5));
    let outside = fixture.mob(&EntityType::COW);
    outside
        .get_entity()
        .set_pos(Vector3::new(6.8, 64.8125, 5.5));
    let player = fixture.player("eye-user");
    let mut eye = ItemStack::new(1, &Item::ENDER_EYE);
    assert!(matches!(
        EnderEyeItem.use_on_block(
            &mut eye,
            &player,
            pos,
            BlockDirection::Up,
            Vector3::new(0.5, 1.0, 0.5),
            &Block::END_PORTAL_FRAME,
            &fixture.server
        ),
        BlockActionResult::Success
    ));
    assert!(EndPortalFrameLikeProperties::from_state_id(world.get_block_state_id(&pos)).eye);
    assert_eq!(cow.get_entity().pos.load().y, 65.0);
    assert_eq!(outside.get_entity().pos.load().y, 64.8125);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn end_portal_activation_drops_blocks_in_its_interior() {
    use crate::block::entities::chest::ChestBlockEntity;
    use pumpkin_inventory::Inventory;
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let center = BlockPos::new(8, 64, 8);
    let last = center.offset_dir(BlockDirection::North.to_offset(), 2);
    for dir in BlockDirection::horizontal() {
        for side in -1..=1 {
            let pos = center
                .offset_dir(dir.to_offset(), 2)
                .offset_dir(dir.rotate_clockwise().to_offset(), side);
            let mut props = EndPortalFrameLikeProperties::default(&Block::END_PORTAL_FRAME);
            props.facing = dir.to_facing().opposite();
            props.eye = pos != last;
            put(&world, pos, props.to_state_id(&Block::END_PORTAL_FRAME));
        }
    }
    put(
        &world,
        center.offset(Vector3::new(-1, 0, -1)),
        Block::TORCH.default_state.id,
    );
    put(&world, center, Block::CHEST.default_state.id);
    let chest = Arc::new(ChestBlockEntity::new(center));
    chest.set_stack(0, ItemStack::new(7, &Item::DIAMOND));
    world.add_block_entity(chest);
    let player = fixture.player("portal-user");
    let mut eye = ItemStack::new(1, &Item::ENDER_EYE);
    EnderEyeItem.use_on_block(
        &mut eye,
        &player,
        last,
        BlockDirection::Up,
        Vector3::new(0.5, 1.0, 0.5),
        &Block::END_PORTAL_FRAME,
        &fixture.server,
    );
    for x in -1..=1 {
        for z in -1..=1 {
            assert_eq!(
                world.get_block(&center.offset(Vector3::new(x, 0, z))),
                &Block::END_PORTAL
            );
        }
    }
    for (item, expected) in [(&Item::TORCH, 1), (&Item::CHEST, 1), (&Item::DIAMOND, 7)] {
        let count: u32 = world
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
            .sum();
        assert_eq!(count, expected, "{} drops", item.registry_key);
    }
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn creative_player_in_a_wither_rose_gets_the_wither_effect() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    fixture.server.level_info.rcu(|info| {
        let mut info = (**info).clone();
        info.difficulty = pumpkin_util::Difficulty::Normal;
        info
    });
    let pos = BlockPos::new(5, 64, 5);
    put(&world, pos, Block::WITHER_ROSE.default_state.id);
    let creative = fixture.player("creative");
    creative.set_gamemode(GameMode::Creative);
    let survival = fixture.player("survival");
    for player in [&creative, &survival] {
        player.get_entity().set_pos(Vector3::new(5.5, 64.0, 5.5));
        player.get_entity().tick_block_collisions(player.as_ref());
    }
    assert_eq!(
        creative
            .living_entity
            .get_effect(&StatusEffect::WITHER)
            .unwrap()
            .duration,
        40
    );
    assert_eq!(
        survival
            .living_entity
            .get_effect(&StatusEffect::WITHER)
            .unwrap()
            .duration,
        40
    );
    let immune = fixture.mob(&EntityType::COW);
    immune.get_entity().set_invulnerable(true);
    WitherRoseBlock.on_entity_collision(OnEntityCollisionArgs {
        server: &fixture.server,
        world: &world,
        block: &Block::WITHER_ROSE,
        state: Block::WITHER_ROSE.default_state,
        position: &pos,
        entity: immune.as_ref(),
    });
    assert!(
        immune
            .get_living_entity()
            .unwrap()
            .get_effect(&StatusEffect::WITHER)
            .is_none()
    );
    let immune_player = fixture.player("permanently-invulnerable");
    immune_player.get_entity().set_invulnerable(true);
    immune_player.set_gamemode(GameMode::Creative);
    immune_player
        .get_entity()
        .set_pos(Vector3::new(5.5, 64.0, 5.5));
    immune_player
        .get_entity()
        .tick_block_collisions(immune_player.as_ref());
    assert!(
        immune_player
            .living_entity
            .get_effect(&StatusEffect::WITHER)
            .is_none()
    );
    // Creative still blocks the damage caused by the admitted effect.
    creative.living_entity.damage(
        creative.as_ref(),
        1.0,
        pumpkin_data::damage::DamageType::WITHER,
    );
    assert_eq!(creative.living_entity.health.load(), 20.0);
    let mut nbt = pumpkin_nbt::compound::NbtCompound::new();
    creative.get_entity().write_nbt(&mut nbt);
    assert_eq!(nbt.get_bool("Invulnerable"), Some(false));
    immune_player.set_gamemode(GameMode::Survival);
    immune_player.get_entity().write_nbt(&mut nbt);
    assert_eq!(nbt.get_bool("Invulnerable"), Some(true));
    immune_player
        .get_entity()
        .tick_block_collisions(immune_player.as_ref());
    assert!(
        immune_player
            .living_entity
            .get_effect(&StatusEffect::WITHER)
            .is_none()
    );
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wither_rose_respects_equipped_enchantment_damage_immunity() {
    use pumpkin_data::{Enchantment, data_component_impl::EquipmentSlot};
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let pack = world
        .level
        .level_folder
        .root_folder
        .join("datapacks/rose-immunity");
    std::fs::create_dir_all(pack.join("data/minecraft/enchantment")).unwrap();
    std::fs::write(
        pack.join("pack.mcmeta"),
        r#"{"pack":{"min_format":94,"max_format":94,"description":"rose immunity regression"}}"#,
    )
    .unwrap();
    std::fs::write(
        pack.join("data/minecraft/enchantment/unbreaking.json"),
        r#"{"slots":["armor"],"effects":{"minecraft:damage_immunity":[{"effect":{}}]}}"#,
    )
    .unwrap();
    fixture.server.datapack_manager.load_all(
        &world.level.level_folder.root_folder,
        &["file/rose-immunity".to_owned()],
        &fixture.server.recipe_manager,
    );
    let cow = fixture.mob(&EntityType::COW);
    let living = cow.get_living_entity().unwrap();
    let mut helmet = ItemStack::new(1, &Item::IRON_HELMET);
    helmet.add_enchantment(&Enchantment::UNBREAKING, 1);
    living
        .entity_equipment
        .lock()
        .unwrap()
        .put(&EquipmentSlot::HEAD, helmet);
    let pos = BlockPos::new(5, 64, 5);
    let contact = || {
        WitherRoseBlock.on_entity_collision(OnEntityCollisionArgs {
            server: &fixture.server,
            world: &world,
            block: &Block::WITHER_ROSE,
            state: Block::WITHER_ROSE.default_state,
            position: &pos,
            entity: cow.as_ref(),
        });
    };
    contact();
    assert!(living.get_effect(&StatusEffect::WITHER).is_none());
    living
        .entity_equipment
        .lock()
        .unwrap()
        .put(&EquipmentSlot::HEAD, ItemStack::EMPTY.clone());
    contact();
    assert_eq!(
        living.get_effect(&StatusEffect::WITHER).unwrap().duration,
        40
    );
    fixture.server.shutdown().await;
}
