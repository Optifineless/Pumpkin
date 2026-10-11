use super::*;
use pumpkin_inventory::Inventory;
use pumpkin_protocol::{
    codec::item_stack_seralizer::ItemStackSerializer, java::client::play::CSetPlayerInventory,
};
use pumpkin_util::PermissionLvl;

#[path = "glass_bottle_review_support.rs"]
mod support;
use support::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review116_bottle_outlines_use_the_main_hand_collision_context() {
    // EntityCollisionContext stores the main hand, irrespective of the hand using the bottle.
    for (block, held) in [
        (&Block::SCAFFOLDING, &Item::SCAFFOLDING),
        (&Block::LIGHT, &Item::LIGHT),
    ] {
        let fixture = BottleFixture::new();
        fixture.put(OBSTACLE.down(), Block::STONE.default_state.id);
        let state = if block == &Block::SCAFFOLDING {
            block
                .from_properties(&[
                    ("distance", "0"),
                    ("bottom", "false"),
                    ("waterlogged", "false"),
                ])
                .to_state_id(block)
        } else {
            block.default_state.id
        };
        fixture.put(OBSTACLE, state);
        fixture
            .user
            .player
            .inventory()
            .set_stack_in_hand(Hand::Right, ItemStack::new(1, held));
        fixture.hold_bottle(Hand::Left);
        fixture.use_bottle(Hand::Left);
        fixture.assert_empty_bottle(Hand::Left);
        assert_eq!(fixture.user.player.inventory().held_item().item, held);

        // The unheld outline leaves the middle of the block open to the source behind it.
        fixture
            .user
            .player
            .inventory()
            .set_stack_in_hand(Hand::Right, ItemStack::new(1, &Item::DIAMOND_SWORD));
        fixture.use_bottle(Hand::Left);
        fixture.assert_water(Hand::Left);

        // Holding the special item in the offhand does not make it the collision-context item.
        fixture
            .user
            .player
            .inventory()
            .set_stack_in_hand(Hand::Left, ItemStack::new(1, held));
        fixture.hold_bottle(Hand::Right);
        fixture.use_bottle(Hand::Right);
        fixture.assert_water(Hand::Right);
        fixture.finish().await;
    }

    // Even a context-selected solid outline can yield water from that waterlogged hit cell.
    let fixture = BottleFixture::new();
    fixture.put(OBSTACLE.down(), Block::STONE.default_state.id);
    let state = Block::SCAFFOLDING
        .from_properties(&[
            ("distance", "0"),
            ("bottom", "false"),
            ("waterlogged", "true"),
        ])
        .to_state_id(&Block::SCAFFOLDING);
    fixture.put(OBSTACLE, state);
    fixture
        .user
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::new(1, &Item::SCAFFOLDING));
    fixture.hold_bottle(Hand::Left);
    fixture.use_bottle(Hand::Left);
    fixture.assert_water(Hand::Left);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review116_bottle_spawn_protection_checks_ops_radius_and_dimension() {
    // DedicatedServer.isUnderSpawnProtection: the target is 11 blocks from shared spawn.
    for (radius, operator_is_actor, permission, allowed) in [
        (11, Some(false), PermissionLvl::Zero, false),
        (10, Some(false), PermissionLvl::Zero, true),
        (0, Some(false), PermissionLvl::Zero, true),
        (11, None, PermissionLvl::Zero, true),
        (11, Some(true), PermissionLvl::One, true),
        (11, Some(false), PermissionLvl::Four, false),
    ] {
        let mut fixture =
            BottleFixture::configured(|server| server.basic_config.spawn_protection = radius);
        fixture.user.player.permission_lvl.store(permission);
        if let Some(is_actor) = operator_is_actor {
            let id = if is_actor {
                fixture.user.player.gameprofile.id
            } else {
                uuid::Uuid::from_u128(1)
            };
            add_operator(&fixture, id);
        }
        let effects = Effects::watch(&mut fixture, Hand::Left);
        fixture.hold_bottle(Hand::Left);
        fixture.use_bottle(Hand::Left);
        if allowed {
            fixture.assert_water(Hand::Left);
        } else {
            fixture.assert_empty_bottle(Hand::Left);
            assert!(effects.observations.lock().unwrap().is_empty());
            assert!(take_sounds(&mut fixture.user).is_empty());
            assert!(effects.observer_sounds().is_empty());
        }
        assert_eq!(
            fixture
                .user
                .player
                .get_stat(StatisticCategory::Used, i32::from(Item::GLASS_BOTTLE.id)),
            i32::from(allowed)
        );
        fixture.finish().await;
    }

    // The fork's shared respawn data is in the Overworld; another dimension is unprotected.
    let fixture = nether_fixture();
    add_operator(&fixture, uuid::Uuid::from_u128(1));
    fixture.hold_bottle(Hand::Right);
    fixture.use_bottle(Hand::Right);
    fixture.assert_water(Hand::Right);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review116_bottle_world_border_checks_the_hit_block_origin() {
    // 26.3 WorldBorder.isWithinBounds(BlockPos) tests integer X/Z, not the whole cube.
    for (center_z, diameter, allowed) in [
        (8.5, 5.0, false),
        (8.5, 5.25, true),
        (13.5, 5.0, true),
        (13.625, 5.0, false),
    ] {
        let mut fixture = BottleFixture::new();
        add_operator(&fixture, fixture.user.player.gameprofile.id);
        {
            let mut border = fixture.world.worldborder.lock().unwrap();
            border.center_x = 8.5;
            border.center_z = center_z;
            border.old_diameter = diameter;
            border.new_diameter = diameter;
        };
        let effects = Effects::watch(&mut fixture, Hand::Right);
        fixture.hold_bottle(Hand::Right);
        fixture.use_bottle(Hand::Right);
        if allowed {
            fixture.assert_water(Hand::Right);
        } else {
            fixture.assert_empty_bottle(Hand::Right);
            assert!(effects.observations.lock().unwrap().is_empty());
            assert!(take_sounds(&mut fixture.user).is_empty());
            assert!(effects.observer_sounds().is_empty());
        }
        assert_eq!(
            fixture
                .user
                .player
                .get_stat(StatisticCategory::Used, i32::from(Item::GLASS_BOTTLE.id)),
            i32::from(allowed)
        );
        fixture.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review116_bottle_fluid_pickup_precedes_stat_and_hand_exchange() {
    let mut fixture = BottleFixture::new();
    let effects = Effects::watch(&mut fixture, Hand::Left);
    fixture.hold_bottle(Hand::Left);
    fixture.use_bottle(Hand::Left);
    fixture.assert_water(Hand::Left);
    {
        let seen = effects.observations.lock().unwrap();
        assert_eq!(
            seen.iter().map(|event| event.stage).collect::<Vec<_>>(),
            vec![Stage::FluidPickup, Stage::Statistic]
        );
        assert_eq!(seen[0].position, Some(Vector3::new(8.5, 65.5, 11.5)));
        assert_eq!(
            seen[0].sounds.len(),
            1,
            "the fill sound precedes FLUID_PICKUP"
        );
        assert_eq!(seen[0].sounds[0].sound, Sound::ItemBottleFill as i32);
        assert!(seen[1].sounds.is_empty());
        for event in seen.iter() {
            assert_eq!(event.held.item, &Item::GLASS_BOTTLE);
            assert_eq!(event.held.item_count, 1);
            assert_eq!(event.used, 0);
        }
    }
    assert_eq!(
        fixture
            .user
            .player
            .get_stat(StatisticCategory::Used, i32::from(Item::GLASS_BOTTLE.id)),
        1
    );
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review116_bottle_fill_sound_excludes_actor_at_neutral_player_position() {
    let mut fixture = BottleFixture::new();
    let effects = Effects::watch(&mut fixture, Hand::Right);
    fixture.hold_bottle(Hand::Right);
    fixture.use_bottle(Hand::Right);
    fixture.assert_water(Hand::Right);
    assert!(take_sounds(&mut fixture.user).is_empty());
    assert_eq!(
        effects.observer_sounds(),
        vec![SoundPacket {
            sound: Sound::ItemBottleFill as i32,
            category: SoundCategory::Neutral as i32,
            position: Vector3::new(68, 512, 68),
            volume: 1.0,
            pitch: 1.0,
        }]
    );
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review116_bottle_callbacks_preserve_direct_hand_replacements() {
    for stage in [Stage::FluidPickup, Stage::Statistic] {
        let mut fixture = BottleFixture::new();
        let effects = Effects::watch(&mut fixture, Hand::Left);
        let replacement = ItemStack::new(1, &Item::DIAMOND_SWORD);
        *effects.replacement.lock().unwrap() = Some((stage, replacement.clone()));
        fixture.hold_bottle(Hand::Left);
        fixture.use_bottle(Hand::Left);
        assert!(
            fixture
                .user
                .player
                .inventory()
                .off_hand_item()
                .are_equal(&replacement)
        );
        assert!(!fixture.user.player.inventory().contains_item(&Item::POTION));
        assert_eq!(
            fixture
                .user
                .player
                .get_stat(StatisticCategory::Used, i32::from(Item::GLASS_BOTTLE.id)),
            i32::from(stage == Stage::Statistic)
        );
        fixture.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review116_bottle_callbacks_preserve_fresh_identical_replacements() {
    for stage in [Stage::FluidPickup, Stage::Statistic] {
        let mut fixture = BottleFixture::new();
        let effects = Effects::watch(&mut fixture, Hand::Left);
        fixture.hold_bottle(Hand::Left);
        let original = fixture.user.player.inventory().off_hand_item();
        let replacement = ItemStack::new(1, &Item::GLASS_BOTTLE);
        assert!(original.are_equal(&replacement));
        assert_ne!(original.uid, replacement.uid);
        *effects.replacement.lock().unwrap() = Some((stage, replacement.clone()));
        fixture.use_bottle(Hand::Left);
        let held = fixture.user.player.inventory().off_hand_item();
        assert_eq!(
            held.uid, replacement.uid,
            "a direct writer owns its fresh stack"
        );
        assert!(held.are_equal(&replacement));
        assert!(!fixture.user.player.inventory().contains_item(&Item::POTION));
        assert_eq!(
            fixture
                .user
                .player
                .get_stat(StatisticCategory::Used, i32::from(Item::GLASS_BOTTLE.id),),
            i32::from(stage == Stage::Statistic)
        );
        fixture.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review116_bottle_callbacks_keep_the_original_hotbar_slot_and_sync_it() {
    for stage in [Stage::FluidPickup, Stage::Statistic] {
        let mut fixture = BottleFixture::new();
        let effects = Effects::watch(&mut fixture, Hand::Right);
        fixture.hold_bottle(Hand::Right);
        let inventory = fixture.user.player.inventory();
        let other = ItemStack::new(1, &Item::GLASS_BOTTLE);
        inventory.set_stack(1, other.clone());
        *effects.selection.lock().unwrap() = Some((stage, 1));
        fixture.use_bottle(Hand::Right);
        assert_eq!(inventory.get_selected_slot(), 1);
        let selected = inventory.get_stack(1);
        assert_eq!(
            selected.uid, other.uid,
            "the newly selected slot is not the input"
        );
        assert!(selected.are_equal(&other));
        let output = inventory.get_stack(0);
        assert!(output.are_equal(&water_bottle()));
        let expected = fixture
            .user
            .client()
            .serialize_packet(&CSetPlayerInventory::new(
                VarInt(0),
                &ItemStackSerializer::from(output),
            ))
            .unwrap();
        assert!(fixture.user.take_packets().contains(&expected));
        assert_eq!(
            fixture
                .user
                .player
                .get_stat(StatisticCategory::Used, i32::from(Item::GLASS_BOTTLE.id),),
            1
        );
        fixture.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review116_bottle_dragon_breath_bypasses_water_permission_and_broadcasts() {
    let mut fixture = BottleFixture::new();
    add_operator(&fixture, uuid::Uuid::from_u128(1));
    fixture.world.worldborder.lock().unwrap().new_diameter = 1.0;
    fixture.put(OBSTACLE, Block::STONE.default_state.id);
    let dragon: Arc<dyn EntityBase> = Arc::new(LivingEntity::new(Entity::new(
        fixture.world.clone(),
        Vector3::new(12.5, 64.0, 8.5),
        &EntityType::ENDER_DRAGON,
    )));
    let cloud = AreaEffectCloudEntity::create(
        Entity::new(
            fixture.world.clone(),
            Vector3::new(8.5, 65.0, 9.5),
            &EntityType::AREA_EFFECT_CLOUD,
        ),
        ItemStack::EMPTY.clone(),
        Vec::new(),
        600,
        1.5,
        20,
        0,
        0.0,
        0,
    );
    cloud.set_owner(Some(dragon.as_ref()));
    fixture
        .world
        .entities
        .store(Arc::new(vec![dragon, cloud.clone()]));
    let effects = Effects::watch(&mut fixture, Hand::Left);
    fixture.hold_bottle(Hand::Left);
    fixture.use_bottle(Hand::Left);
    assert_eq!(
        fixture.user.player.inventory().off_hand_item().item,
        &Item::DRAGON_BREATH
    );
    assert_eq!(cloud.radius(), 1.0);
    let expected = vec![SoundPacket {
        sound: Sound::ItemBottleFillDragonbreath as i32,
        category: SoundCategory::Neutral as i32,
        position: Vector3::new(68, 512, 68),
        volume: 1.0,
        pitch: 1.0,
    }];
    assert_eq!(take_sounds(&mut fixture.user), expected);
    assert_eq!(effects.observer_sounds(), expected);
    {
        let seen = effects.observations.lock().unwrap();
        assert_eq!(seen[0].stage, Stage::FluidPickup);
        assert_eq!(seen[0].position, Some(Vector3::new(8.5, 64.0, 8.5)));
        assert_eq!(seen[0].held.item, &Item::GLASS_BOTTLE);
        assert_eq!(seen[0].used, 0);
    };
    fixture.finish().await;
}
