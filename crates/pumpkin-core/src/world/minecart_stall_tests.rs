use super::{BlockInteraction, Explosion, World};
use crate::{
    entity::{
        Entity, EntityBase, projectile::fishing_bobber::FishingBobberEntity,
        vehicle::minecart::MinecartEntity,
    },
    net::java::combat_test_support::TestPlayer,
    server::{Server, combat_test_support},
};
use pumpkin_data::{
    Block,
    block_properties::{PoweredRailLikeProperties, RailShapeStraight},
    entity::EntityType,
    item::Item,
    item_stack::ItemStack,
};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_util::{
    Hand,
    math::{vector2::Vector2, vector3::Vector3},
};
use std::{
    sync::{Arc, atomic::Ordering::Relaxed},
    time::Duration,
};

mod review;

fn fixture() -> (tempfile::TempDir, Arc<Server>, Arc<World>) {
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    combat_test_support::publish_empty_chunk(&world, Vector2::new(0, 0));
    server.worlds.store(Arc::new(vec![world.clone()]));
    (dir, server, world)
}

// A blocking synchronous tick needs a thread deadline; Tokio timeout cannot interrupt it.
fn within_deadline<T: Send + 'static>(action: impl FnOnce() -> T + Send + 'static) -> T {
    let (sender, receiver) = std::sync::mpsc::channel();
    let runtime = tokio::runtime::Handle::current();
    std::thread::spawn(move || {
        let _guard = runtime.enter();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(action));
        let _ = sender.send(result);
    });
    match receiver.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(result)) => result,
        Ok(Err(panic)) => std::panic::resume_unwind(panic),
        Err(error) => panic!("movement did not finish within ten seconds: {error}"),
    }
}

fn cart(world: &Arc<World>, pos: Vector3<f64>, kind: &'static EntityType) -> Arc<MinecartEntity> {
    Arc::new(MinecartEntity::new(Entity::new(world.clone(), pos, kind)))
}

fn fuse(cart: &MinecartEntity, ticks: i32) {
    let mut nbt = NbtCompound::new();
    nbt.put_int("fuse", ticks);
    cart.read_custom_nbt(&nbt);
}

fn rail(world: &World, powered: bool) {
    let properties = PoweredRailLikeProperties {
        powered,
        shape: RailShapeStraight::EastWest,
        waterlogged: false,
    };
    world
        .level
        .loaded_chunks
        .get(&Vector2::new(0, 0))
        .unwrap()
        .set_block_absolute_y(8, 64, 8, properties.to_state_id(&Block::POWERED_RAIL));
}

#[tokio::test]
async fn minecart_stall_stacked_explosion_impulse_is_limited_on_and_off_rails() {
    for on_rails in [false, true] {
        let (_dir, server, world) = fixture();
        if on_rails {
            // An unpowered activator rail does not brake, boost or ignite the cart.
            let properties = PoweredRailLikeProperties {
                powered: false,
                shape: RailShapeStraight::EastWest,
                waterlogged: false,
            };
            world
                .level
                .loaded_chunks
                .get(&Vector2::new(0, 0))
                .unwrap()
                .set_block_absolute_y(8, 64, 8, properties.to_state_id(&Block::ACTIVATOR_RAIL));
        }
        let cart = cart(
            &world,
            Vector3::new(8.5, 64.0625, 8.5),
            &EntityType::TNT_MINECART,
        );
        fuse(&cart, 200);
        world.add_entity_silent(cart.clone());
        let moved = within_deadline(move || {
            // Drive the actual explosion entry point, rather than copying its impulse formula.
            for _ in 0..21 {
                world.run_explosion(&Explosion::new(
                    4.0,
                    Vector3::new(7.5, 64.0625, 8.5),
                    BlockInteraction::Keep,
                ));
            }
            assert!(cart.get_entity().velocity.load().x > 0.4);
            let start = cart.get_entity().pos.load();
            cart.tick(cart.as_ref(), &server);
            cart.get_entity().pos.load() - start
        });
        assert!(moved.x > 0.0 && moved.x <= 0.4 + 1.0e-9, "{moved:?}");
        assert!(moved.z.abs() <= 0.4 + 1.0e-9);
        if on_rails {
            assert_eq!(moved.y, 0.0);
        }
    }
}

#[tokio::test]
async fn minecart_stall_simultaneous_tnt_carts_finish_world_tick() {
    let (_dir, server, world) = fixture();
    world
        .forced_chunks
        .lock()
        .unwrap()
        .insert(Vector2::new(0, 0));
    let mut exploding = Vec::new();
    for _ in 0..4 {
        let cart = cart(
            &world,
            Vector3::new(7.5, 64.0, 8.5),
            &EntityType::TNT_MINECART,
        );
        fuse(&cart, 0);
        world.add_entity_silent(cart.clone());
        exploding.push(cart);
    }
    let survivor = cart(
        &world,
        Vector3::new(8.5, 64.0, 8.5),
        &EntityType::TNT_MINECART,
    );
    fuse(&survivor, 200);
    world.add_entity_silent(survivor.clone());
    within_deadline(move || {
        world.tick(&server);
        assert!(exploding.iter().all(|cart| cart.get_entity().is_removed()));
        let moved = survivor.get_entity().pos.load().x - 8.5;
        assert!(moved > 0.0 && moved <= 0.4 + 1.0e-9, "{moved}");
        assert!(!survivor.get_entity().is_removed());
    });
}

#[tokio::test]
async fn minecart_stall_powered_rail_reaches_max_without_allowing_overspeed() {
    let (_dir, server, world) = fixture();
    rail(&world, true);
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
    owner.player.get_entity().yaw.store(-90.0);
    owner.player.last_input.store(
        pumpkin_protocol::java::server::play::SPlayerInput::FORWARD,
        Relaxed,
    );
    within_deadline(move || {
        let _owner = owner;
        // Keep the cart on the same powered cell while exercising real successive ticks.
        cart.get_entity()
            .velocity
            .store(Vector3::new(0.1, 0.0, 0.0));
        let mut fastest: f64 = 0.0;
        for _ in 0..20 {
            cart.get_entity().set_pos(Vector3::new(8.5, 64.0625, 8.5));
            cart.tick(cart.as_ref(), &server);
            fastest = fastest.max(cart.get_entity().pos.load().x - 8.5);
        }
        assert!((fastest - 0.4).abs() < 1.0e-9, "{fastest}");
        // Water halves the limit even when the powered rail has already accelerated it.
        cart.get_entity().set_pos(Vector3::new(8.5, 64.0625, 8.5));
        let chunk = world.level.loaded_chunks.get(&Vector2::new(0, 0)).unwrap();
        let mut properties = PoweredRailLikeProperties {
            powered: true,
            shape: RailShapeStraight::EastWest,
            waterlogged: true,
        };
        chunk.set_block_absolute_y(8, 64, 8, properties.to_state_id(&Block::POWERED_RAIL));
        drop(chunk);
        cart.tick(cart.as_ref(), &server);
        assert!(cart.get_entity().touching_water.load(Relaxed));
        cart.get_entity().set_pos(Vector3::new(8.5, 64.0625, 8.5));
        cart.tick(cart.as_ref(), &server);
        assert!((cart.get_entity().pos.load().x - 8.5 - 0.2).abs() < 1.0e-9);
        properties.waterlogged = false;
        world
            .level
            .loaded_chunks
            .get(&Vector2::new(0, 0))
            .unwrap()
            .set_block_absolute_y(8, 64, 8, properties.to_state_id(&Block::POWERED_RAIL));
        cart.get_entity().set_pos(Vector3::new(8.5, 64.0625, 8.5));
        cart.tick(cart.as_ref(), &server);
        assert!(!cart.get_entity().touching_water.load(Relaxed));
        cart.get_entity().set_pos(Vector3::new(8.5, 64.0625, 8.5));
        cart.get_entity()
            .velocity
            .store(Vector3::new(0.6, 0.0, 0.0));
        cart.tick(cart.as_ref(), &server);
        assert!((cart.get_entity().pos.load().x - 8.5 - 0.4).abs() < 1.0e-9);
    });
}

#[tokio::test]
async fn minecart_stall_experimental_speed_rule_limits_both_movement_paths() {
    for on_rails in [false, true] {
        let (_dir, server, world) = fixture();
        server.level_info.rcu(|info| {
            let mut info = (**info).clone();
            info.data_packs
                .enabled
                .push("minecart_improvements".to_owned());
            info.game_rules.max_minecart_speed = 20;
            info
        });
        if on_rails {
            rail(&world, true);
        }
        let cart = cart(
            &world,
            Vector3::new(8.5, 64.0625, 8.5),
            &EntityType::MINECART,
        );
        world.add_entity_silent(cart.clone());
        cart.get_entity()
            .velocity
            .store(Vector3::new(20.0, 0.0, 0.0));
        within_deadline(move || {
            cart.tick(cart.as_ref(), &server);
            // High experimental rail speeds wait for the vanilla stepAlongTrack loop.
            let expected = if on_rails { 0.46 } else { 1.0 };
            assert!((cart.get_entity().pos.load().x - 8.5 - expected).abs() < 1.0e-9);
        });
    }
}

#[tokio::test]
async fn minecart_stall_generic_movement_rejects_nonfinite_and_oversized_sweeps() {
    let (_dir, _server, world) = fixture();
    within_deadline(move || {
        let entity = Entity::new(world, Vector3::new(8.5, 64.0, 8.5), &EntityType::TNT);
        for motion in [
            Vector3::new(1.0e12, 1.0e12, 1.0e12),
            Vector3::new(f64::NAN, 0.0, 0.0),
            Vector3::new(0.0, f64::INFINITY, 0.0),
            Vector3::new(100.0, 100.0, 100.0),
        ] {
            let start = entity.pos.load();
            entity.velocity.store(motion);
            entity.move_entity(&entity, motion);
            assert_eq!(entity.pos.load(), start);
            assert_eq!(entity.velocity.load(), Vector3::default());
        }
        // Normal movement remains accepted after a rejection.
        entity.move_entity(&entity, Vector3::new(0.1, -0.04, 0.0));
        assert!(entity.pos.load().x > 8.5);
    });
}

#[tokio::test]
async fn minecart_stall_saved_motion_is_limited_after_real_chunk_activation() {
    let (_dir, server, world) = fixture();
    let cart = cart(&world, Vector3::new(8.5, 64.0, 8.5), &EntityType::MINECART);
    let mut nbt = NbtCompound::new();
    cart.write_nbt(&mut nbt);
    // Both older fork builds and vanilla write Motion as three doubles. Values <= 10 load intact.
    nbt.put_list(
        "Motion",
        vec![
            NbtTag::Double(9.0),
            NbtTag::Double(0.0),
            NbtTag::Double(-9.0),
        ],
    );
    let chunk = world
        .level
        .get_entity_chunk(Vector2::new(0, 0))
        .await
        .unwrap();
    chunk.data.lock().unwrap().push(nbt);
    world.make_chunk_entities_live(&chunk, None);
    let loaded = world
        .get_entity_by_uuid(cart.get_entity().entity_uuid)
        .unwrap();
    assert_eq!(
        loaded.get_entity().velocity.load(),
        Vector3::new(9.0, 0.0, -9.0)
    );
    within_deadline(move || {
        loaded.tick(loaded.as_ref(), &server);
        let moved = loaded.get_entity().pos.load() - Vector3::new(8.5, 64.0, 8.5);
        assert!((moved.x - 0.4).abs() < 1.0e-9 && (moved.z + 0.4).abs() < 1.0e-9);
    });
}

#[tokio::test]
async fn minecart_stall_limits_follow_behavior_water_furnace_and_ground() {
    for new_behavior in [false, true] {
        let (_dir, server, world) = fixture();
        if new_behavior {
            server.level_info.rcu(|info| {
                let mut info = (**info).clone();
                info.data_packs
                    .enabled
                    .push("minecart_improvements".to_owned());
                info.game_rules.max_minecart_speed = 20;
                info
            });
        }
        within_deadline(move || {
            for on_rails in [false, true] {
                let properties = PoweredRailLikeProperties {
                    powered: false,
                    shape: RailShapeStraight::EastWest,
                    waterlogged: false,
                };
                let chunk = world.level.loaded_chunks.get(&Vector2::new(0, 0)).unwrap();
                chunk.set_block_absolute_y(
                    8,
                    64,
                    8,
                    if on_rails {
                        properties.to_state_id(&Block::ACTIVATOR_RAIL)
                    } else {
                        Block::AIR.default_state.id
                    },
                );
                chunk.set_block_absolute_y(8, 63, 8, Block::STONE.default_state.id);
                drop(chunk);
                for in_water in [false, true] {
                    for furnace in [false, true] {
                        for on_ground in [false, true] {
                            let kind = if furnace {
                                &EntityType::FURNACE_MINECART
                            } else {
                                &EntityType::MINECART
                            };
                            let cart = cart(&world, Vector3::new(8.5, 64.0625, 8.5), kind);
                            let entity = cart.get_entity();
                            entity.touching_water.store(in_water, Relaxed);
                            entity.on_ground.store(on_ground, Relaxed);
                            // Two blocks/tick also catches excessive water drag before the clamp.
                            entity.velocity.store(Vector3::new(
                                if furnace { 2.0 } else { 20.0 },
                                0.0,
                                0.0,
                            ));
                            let mut expected = match (new_behavior, in_water, furnace) {
                                (false, false, false) => 0.4,
                                (false, true, false) | (false, false, true) => 0.2,
                                (false, true, true) => 0.15,
                                (true, false, false) => 1.0,
                                (true, true, false) | (true, false, true) => 0.5,
                                (true, true, true) => 0.375,
                            };
                            if new_behavior && on_rails {
                                expected *= 0.4;
                            }
                            if on_ground && !on_rails {
                                expected *= 0.5;
                            }
                            cart.tick(cart.as_ref(), &server);
                            let moved = entity.pos.load().x - 8.5;
                            assert!(
                                (moved - expected).abs() < 1.0e-9,
                                "new={new_behavior} rail={on_rails} water={in_water} furnace={furnace} ground={on_ground}: {moved} != {expected}"
                            );
                        }
                    }
                }
            }
        });
    }
}

#[tokio::test]
async fn minecart_stall_fishing_tick_bounds_its_separate_block_query() {
    let (_dir, server, world) = fixture();
    let owner = TestPlayer::new(&world);
    owner
        .player
        .get_entity()
        .set_pos(Vector3::new(8.5, 64.0, 8.5));
    owner
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::new(1, &Item::FISHING_ROD));
    let hook = FishingBobberEntity::new_with_rotation(
        Entity::new(world, owner.player.position(), &EntityType::FISHING_BOBBER),
        &owner.player,
        0.0,
        0.0,
        0,
        0,
        Hand::Right,
    );
    let start = hook.entity.pos.load();
    hook.entity
        .velocity
        .store(Vector3::new(1.0e12, 1.0e12, 1.0e12));
    within_deadline(move || {
        let _owner = owner;
        hook.tick(&hook, &server);
        assert_eq!(hook.entity.pos.load().x, start.x);
        assert_eq!(hook.entity.pos.load().z, start.z);
        assert!(hook.entity.velocity.load().length() < 0.04);
        assert!(!hook.entity.is_removed());
    });
}

#[tokio::test]
async fn minecart_stall_ordinary_cart_collision_still_transfers_motion() {
    let (_dir, server, world) = fixture();
    rail(&world, true);
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
    moving.get_entity().yaw.store(0.0);
    moving
        .get_entity()
        .velocity
        .store(Vector3::new(0.2, 0.0, 0.0));
    world.add_entity_silent(moving.clone());
    world.add_entity_silent(stopped.clone());
    within_deadline(move || {
        moving.tick(moving.as_ref(), &server);
        assert!(stopped.get_entity().velocity.load().x > 0.0);
        assert!(moving.get_entity().velocity.load().x > 0.0);
    });
}
