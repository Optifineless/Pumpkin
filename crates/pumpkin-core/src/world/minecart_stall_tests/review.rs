use super::*;
use crate::{
    command::CommandSender,
    entity::{
        mob::slime::SlimeEntity, projectile::firework_rocket::FireworkRocketEntity, tnt::TNTEntity,
    },
};
use pumpkin_data::block_properties::{RailLikeProperties, RailShape};
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world_info::{LevelData, WorldInfoWriter, anvil::AnvilLevelInfo};

fn experimental(server: &Server, speed: i64) {
    server.level_info.rcu(|info| {
        let mut info = (**info).clone();
        info.data_packs
            .enabled
            .push("minecart_improvements".to_owned());
        info.game_rules.max_minecart_speed = speed;
        info
    });
}

fn set_block(world: &World, pos: BlockPos, state: pumpkin_data::BlockStateId) {
    world
        .level
        .loaded_chunks
        .get(&Vector2::new(0, 0))
        .unwrap()
        .set_block_absolute_y(pos.0.x as usize, pos.0.y, pos.0.z as usize, state);
}

#[tokio::test]
async fn minecart_review_command_rejects_speed_outside_vanilla_bounds() {
    let (_dir, server, _world) = fixture();
    let source = CommandSender::Console.into_source(&server);
    let dispatcher = server.command_dispatcher.load();
    for valid in [1, 1000] {
        assert_eq!(
            dispatcher
                .execute_input(&format!("gamerule max_minecart_speed {valid}"), &source)
                .unwrap(),
            valid
        );
        for invalid in [-1, 0, 1001] {
            assert!(
                dispatcher
                    .execute_input(&format!("gamerule max_minecart_speed {invalid}"), &source)
                    .is_err()
            );
            assert_eq!(
                server.level_info.load().game_rules.max_minecart_speed,
                i64::from(valid)
            );
        }
    }
}

#[tokio::test]
async fn minecart_review_bad_saved_speed_survives_restart_and_chunk_activation() {
    let dir = tempfile::tempdir().unwrap();
    let mut info = LevelData::default(pumpkin_util::world_seed::Seed(0));
    info.data_packs
        .enabled
        .push("minecart_improvements".to_owned());
    info.game_rules.max_minecart_speed = -1;
    // Older fork builds could write this value; vanilla uses the same game_rules.dat integer tag.
    AnvilLevelInfo.write_world_info(&info, dir.path()).unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    combat_test_support::publish_empty_chunk(&world, Vector2::new(0, 0));
    server.worlds.store(Arc::new(vec![world.clone()]));
    assert_eq!(server.level_info.load().game_rules.max_minecart_speed, -1);
    let source = cart(&world, Vector3::new(8.5, 64.0, 8.5), &EntityType::MINECART);
    source
        .get_entity()
        .velocity
        .store(Vector3::new(1.0, 0.0, 0.0));
    let mut nbt = NbtCompound::new();
    source.write_nbt(&mut nbt);
    let chunk = world
        .level
        .get_entity_chunk(Vector2::new(0, 0))
        .await
        .unwrap();
    chunk.data.lock().unwrap().push(nbt);
    world.make_chunk_entities_live(&chunk, None);
    let loaded = world
        .get_entity_by_uuid(source.get_entity().entity_uuid)
        .unwrap();
    within_deadline(move || {
        loaded.tick(loaded.as_ref(), &server);
        let moved = loaded.get_entity().pos.load().x - 8.5;
        assert!((moved - 0.05).abs() < 1.0e-9, "{moved}");
    });
}

#[tokio::test]
async fn minecart_review_rider_nudge_moves_a_stationary_cart_on_plain_rail() {
    for new_behavior in [false, true] {
        let (_dir, server, world) = fixture();
        if new_behavior {
            experimental(&server, 8);
        }
        let properties = RailLikeProperties {
            shape: RailShape::EastWest,
            waterlogged: false,
        };
        set_block(
            &world,
            BlockPos::new(8, 64, 8),
            properties.to_state_id(&Block::RAIL),
        );
        let cart = cart(
            &world,
            Vector3::new(8.5, 64.0625, 8.5),
            &EntityType::MINECART,
        );
        world.add_entity_silent(cart.clone());
        let owner = TestPlayer::new(&world);
        owner
            .player
            .get_entity()
            .set_pos(cart.get_entity().pos.load());
        owner.player.get_entity().yaw.store(-90.0);
        owner.player.last_input.store(
            pumpkin_protocol::java::server::play::SPlayerInput::FORWARD,
            Relaxed,
        );
        assert!(cart.interact(&owner.player, &mut ItemStack::EMPTY.clone()));
        cart.tick(cart.as_ref(), &server);
        let expected = if new_behavior { 0.000997 } else { 0.00075 };
        assert!((cart.get_entity().pos.load().x - 8.5 - expected).abs() < 1.0e-9);
        assert!(cart.get_entity().velocity.load().x > 0.0);
    }
}

#[tokio::test]
async fn minecart_review_fast_cart_transfers_raw_momentum_during_tick() {
    let (_dir, server, world) = fixture();
    let properties = PoweredRailLikeProperties {
        powered: false,
        shape: RailShapeStraight::EastWest,
        waterlogged: false,
    };
    set_block(
        &world,
        BlockPos::new(8, 64, 8),
        properties.to_state_id(&Block::ACTIVATOR_RAIL),
    );
    let moving = cart(
        &world,
        Vector3::new(8.5, 64.0625, 8.5),
        &EntityType::MINECART,
    );
    let stopped = cart(
        &world,
        Vector3::new(9.3, 64.0625, 8.5),
        &EntityType::MINECART,
    );
    moving
        .get_entity()
        .velocity
        .store(Vector3::new(1.0, 0.0, 0.0));
    moving.get_entity().yaw.store(0.0);
    world.add_entity_silent(moving.clone());
    world.add_entity_silent(stopped.clone());
    within_deadline(move || {
        moving.tick(moving.as_ref(), &server);
        assert!((moving.get_entity().pos.load().x - 8.9).abs() < 1.0e-9);
        // AbstractMinecart.pushOtherMinecart: mean delta 0.5 plus the 0.05 separating push.
        assert!((stopped.get_entity().velocity.load().x - 0.55).abs() < 1.0e-8);
        // Keep the collision's change to our own momentum through natural slowdown too.
        assert!((moving.get_entity().velocity.load().x - 0.624).abs() < 1.0e-8);
    });
}

#[tokio::test]
async fn minecart_review_wall_collision_does_not_restore_raw_momentum() {
    let (_dir, server, world) = fixture();
    let properties = PoweredRailLikeProperties {
        powered: false,
        shape: RailShapeStraight::EastWest,
        waterlogged: false,
    };
    set_block(
        &world,
        BlockPos::new(8, 64, 8),
        properties.to_state_id(&Block::ACTIVATOR_RAIL),
    );
    set_block(
        &world,
        BlockPos::new(9, 64, 8),
        Block::STONE.default_state.id,
    );
    let cart = cart(
        &world,
        Vector3::new(8.5, 64.0625, 8.5),
        &EntityType::MINECART,
    );
    cart.get_entity()
        .velocity
        .store(Vector3::new(1.0, 0.0, 0.0));
    cart.tick(cart.as_ref(), &server);
    assert!(cart.get_entity().horizontal_collision.load(Relaxed));
    assert_eq!(cart.get_entity().velocity.load().x, 0.0);
}

#[tokio::test]
async fn minecart_review_high_experimental_speed_visits_curve_and_activator() {
    let (_dir, server, world) = fixture();
    experimental(&server, 1000);
    rail(&world, true);
    let curve = RailLikeProperties {
        shape: RailShape::SouthWest,
        waterlogged: false,
    };
    set_block(
        &world,
        BlockPos::new(9, 64, 8),
        curve.to_state_id(&Block::RAIL),
    );
    let activator = PoweredRailLikeProperties {
        shape: RailShapeStraight::NorthSouth,
        powered: true,
        waterlogged: false,
    };
    set_block(
        &world,
        BlockPos::new(9, 64, 9),
        activator.to_state_id(&Block::ACTIVATOR_RAIL),
    );
    let cart = cart(
        &world,
        Vector3::new(8.5, 64.0625, 8.5),
        &EntityType::MINECART,
    );
    world.add_entity_silent(cart.clone());
    let owner = TestPlayer::new(&world);
    owner
        .player
        .get_entity()
        .set_pos(cart.get_entity().pos.load());
    assert!(cart.interact(&owner.player, &mut ItemStack::EMPTY.clone()));
    cart.get_entity()
        .velocity
        .store(Vector3::new(20.0, 0.0, 0.0));
    within_deadline(move || {
        for _ in 0..12 {
            cart.tick(cart.as_ref(), &server);
            assert!(cart.get_entity().pos.load().x < 10.0);
            if !cart.get_entity().has_passengers() {
                break;
            }
        }
        assert!(!cart.get_entity().has_passengers());
        assert!(cart.get_entity().pos.load().z >= 9.0);
        assert_eq!(owner.player.get_entity().riding_cooldown.load(Relaxed), 60);
    });
}

#[tokio::test]
async fn minecart_review_experimental_start_requires_a_conducting_end() {
    for (shape, end, expected) in [
        (RailShapeStraight::EastWest, None, Vector3::default()),
        (
            RailShapeStraight::EastWest,
            Some(BlockPos::new(7, 64, 8)),
            Vector3::new(0.2, 0.0, 0.0),
        ),
        (
            RailShapeStraight::EastWest,
            Some(BlockPos::new(9, 64, 8)),
            Vector3::new(-0.2, 0.0, 0.0),
        ),
        (
            RailShapeStraight::NorthSouth,
            Some(BlockPos::new(8, 64, 7)),
            Vector3::new(0.0, 0.0, 0.2),
        ),
        (
            RailShapeStraight::NorthSouth,
            Some(BlockPos::new(8, 64, 9)),
            Vector3::new(0.0, 0.0, -0.2),
        ),
    ] {
        let (_dir, server, world) = fixture();
        experimental(&server, 8);
        let properties = PoweredRailLikeProperties {
            shape,
            powered: true,
            waterlogged: false,
        };
        set_block(
            &world,
            BlockPos::new(8, 64, 8),
            properties.to_state_id(&Block::POWERED_RAIL),
        );
        if let Some(end) = end {
            set_block(&world, end, Block::STONE.default_state.id);
        }
        let cart = cart(
            &world,
            Vector3::new(8.5, 64.0625, 8.5),
            &EntityType::MINECART,
        );
        cart.tick(cart.as_ref(), &server);
        let moved = cart.get_entity().pos.load() - Vector3::new(8.5, 64.0625, 8.5);
        assert!(
            (moved - expected).length() < 1.0e-9,
            "{moved:?} != {expected:?}"
        );
    }
}

#[tokio::test]
async fn minecart_review_size_63_slime_falls_during_mob_tick() {
    let (_dir, server, world) = fixture();
    let slime = SlimeEntity::new(Entity::new(
        world.clone(),
        Vector3::new(8.5, 100.0, 8.5),
        &EntityType::SLIME,
    ));
    // Both vanilla and older fork builds persist Size one less than the actual size.
    let mut nbt = NbtCompound::new();
    slime.write_nbt(&mut nbt);
    nbt.put_int("Size", 63);
    nbt.put_bool("PersistenceRequired", true);
    let chunk = world
        .level
        .get_entity_chunk(Vector2::new(0, 0))
        .await
        .unwrap();
    chunk.data.lock().unwrap().push(nbt);
    world.make_chunk_entities_live(&chunk, None);
    let slime = world
        .get_entity_by_uuid(slime.get_entity().entity_uuid)
        .unwrap();
    assert_eq!(slime.get_entity().data.load(Relaxed), 64);
    within_deadline(move || {
        for _ in 0..3 {
            slime.tick(slime.as_ref(), &server);
        }
        assert!(slime.get_entity().pos.load().y < 100.0);
        assert!(slime.get_entity().velocity.load().y < 0.0);
    });
}

#[tokio::test]
async fn minecart_review_finite_diagonal_explosion_launch_is_applied_by_tnt_tick() {
    let (_dir, server, world) = fixture();
    let tnt = TNTEntity::new(
        Entity::new(world, Vector3::new(8.5, 64.0, 8.5), &EntityType::TNT),
        TNTEntity::DEFAULT_POWER,
        TNTEntity::DEFAULT_FUSE,
    );
    tnt.get_entity()
        .velocity
        .store(Vector3::new(40.0, 40.0, 40.0));
    within_deadline(move || {
        tnt.tick(&tnt, &server);
        let moved = tnt.get_entity().pos.load() - Vector3::new(8.5, 64.0, 8.5);
        assert!((moved.x - 40.0).abs() < 1.0e-9 && (moved.z - 40.0).abs() < 1.0e-9);
        assert!(moved.y > 39.0);
    });
}

#[tokio::test]
async fn minecart_review_firework_clears_rejected_motion_then_resumes_flight() {
    let (_dir, server, world) = fixture();
    let rocket = FireworkRocketEntity::new(Entity::new(
        world,
        Vector3::new(8.5, 64.0, 8.5),
        &EntityType::FIREWORK_ROCKET,
    ));
    rocket
        .get_entity()
        .velocity
        .store(Vector3::new(1000.0, 0.0, 0.0));
    within_deadline(move || {
        rocket.tick(&rocket, &server);
        assert_eq!(rocket.get_entity().velocity.load(), Vector3::default());
        assert_eq!(rocket.get_entity().pos.load(), Vector3::new(8.5, 64.0, 8.5));
        rocket.tick(&rocket, &server);
        assert!(rocket.get_entity().pos.load().y > 64.0);
        assert!(!rocket.get_entity().is_removed());
    });
}
