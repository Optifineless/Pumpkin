use super::*;
use pumpkin_data::game_event::GameEvent;
use std::sync::atomic::Ordering::Relaxed;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s3_support_climb_wool_and_fluid_branches() {
    for block in [&Block::STONE, &Block::WHITE_WOOL, &Block::POWDER_SNOW] {
        let s = scenario(true, true);
        for x in 0..16 {
            for z in 0..16 {
                place(&s, &BlockPos::new(x, 64, z), block.default_state.id);
            }
        }
        position(&s, false, at(1.75), true);
        let sink = s.sink.lock().unwrap();
        assert_eq!(sink.forwarded.len(), 1, "eligible support including wool");
        assert_eq!(sink.forwarded[0].event, GameEvent::Step);
        assert_eq!(
            sink.forwarded[0].affected_state,
            Some(block.default_state.id)
        );
    }
    for policy in 0..3 {
        let s = scenario(true, true);
        match policy {
            0 => s.owner.player.get_entity().sneaking.store(true, Relaxed),
            1 => s.owner.player.abilities.lock().unwrap().flying = true,
            _ => s.owner.player.get_entity().no_physics.store(true, Relaxed),
        }
        position(&s, false, at(2.0), true);
        assert_eq!(counts(&s), (0, 0), "player emission NONE/no physics");
        assert_eq!(s.owner.player.position(), at(2.0));
    }
    for ground in [false, true] {
        let s = scenario(true, true);
        for x in 0..16 {
            for z in 0..16 {
                for y in 65..69 {
                    place(&s, &BlockPos::new(x, y, z), Block::WATER.default_state.id);
                }
            }
        }
        s.owner.player.get_entity().set_sprinting(true);
        for _ in 0..2 {
            s.world.tick(&s.server);
        }
        assert!(
            s.owner.player.get_entity().touching_water.load(Relaxed),
            "real water sampler"
        );
        s.owner.player.update_swimming();
        assert!(
            s.owner.player.get_entity().swimming.load(Relaxed),
            "real swimming state"
        );
        position(&s, false, at(1.75), ground);
        let sink = s.sink.lock().unwrap();
        assert_eq!(
            sink.forwarded.len(),
            1,
            "failed support effects select water fallback"
        );
        assert_eq!(sink.forwarded[0].event, GameEvent::Swim);
        assert_eq!(sink.forwarded[0].affected_state, None);
    }
    let s = scenario(true, true);
    s.owner
        .player
        .get_entity()
        .touching_water
        .store(true, Relaxed);
    position(&s, false, Vector3::new(9.75, 70.0, 8.0), false);
    assert_eq!(counts(&s), (0, 0), "air support is FLAP branch, not SWIM");
    let s = scenario(true, true);
    s.world
        .level
        .loaded_chunks
        .remove(&pumpkin_util::math::vector2::Vector2::new(0, 0));
    position(&s, false, at(2.0), true);
    assert_eq!(counts(&s), (0, 0), "unloaded support is unavailable");
    assert!(
        !s.world
            .level
            .is_chunk_loaded(&pumpkin_util::math::vector2::Vector2::new(0, 0)),
        "emission must not load terrain"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s3_cached_tall_support_and_vertical_climbing() {
    for block in [
        &Block::OAK_FENCE,
        &Block::COBBLESTONE_WALL,
        &Block::OAK_FENCE_GATE,
    ] {
        let s = scenario(true, true);
        for x in 0..16 {
            for z in 0..16 {
                place(&s, &BlockPos::new(x, 64, z), block.default_state.id);
            }
        }
        s.owner
            .player
            .get_entity()
            .set_pos(Vector3::new(8.5, 65.5, 8.5));
        position(&s, false, Vector3::new(10.25, 65.5, 8.5), true);
        let sink = s.sink.lock().unwrap();
        assert_eq!(
            sink.forwarded.len(),
            1,
            "cached tall supporting collision block"
        );
        assert_eq!(
            sink.forwarded[0].affected_state,
            Some(block.default_state.id)
        );
        assert_eq!(sink.forwarded[0].event, GameEvent::Step);
    }
    for block in [&Block::LADDER, &Block::POWDER_SNOW] {
        let s = scenario(true, true);
        place(&s, &BlockPos::new(9, 65, 8), Block::STONE.default_state.id);
        place(&s, &BlockPos::new(9, 66, 8), block.default_state.id);
        s.owner
            .player
            .get_entity()
            .set_pos(Vector3::new(9.5, 64.35, 8.5));
        position(&s, false, Vector3::new(9.5, 66.1, 8.5), false);
        let sink = s.sink.lock().unwrap();
        assert_eq!(
            sink.forwarded.len(),
            1,
            "climbable vertical distance crosses threshold"
        );
        assert_eq!(sink.forwarded[0].event, GameEvent::Step);
        assert_eq!(
            sink.forwarded[0].affected_state,
            Some(block.default_state.id),
            "support differs from legacy effect state"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s3_chunk_boundary_uses_loaded_support() {
    use pumpkin_util::math::vector2::Vector2;
    let s = scenario(true, true);
    crate::server::combat_test_support::publish_empty_chunk(&s.world, Vector2::new(1, 0));
    for x in 16..20 {
        place(
            &s,
            &BlockPos::new(x, 64, 8),
            Block::WHITE_WOOL.default_state.id,
        );
    }
    s.owner
        .player
        .get_entity()
        .set_pos(Vector3::new(15.0, 65.0, 8.5));
    position(&s, true, Vector3::new(16.75, 65.0, 8.5), true);
    let sink = s.sink.lock().unwrap();
    assert_eq!(sink.forwarded.len(), 1);
    assert_eq!(sink.forwarded[0].event, GameEvent::Step);
    assert_eq!(
        sink.forwarded[0].affected_state,
        Some(Block::WHITE_WOOL.default_state.id)
    );
    drop(sink);
    s.world.level.loaded_chunks.remove(&Vector2::new(1, 0));
    position(&s, false, Vector3::new(19.0, 65.0, 8.5), true);
    assert_eq!(
        counts(&s),
        (1, 1),
        "retired destination terrain cannot emit"
    );
    assert!(!s.world.level.is_chunk_loaded(&Vector2::new(1, 0)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s3_crouching_pose_differs_from_discrete_flag() {
    for crouching in [true, false] {
        let s = scenario(true, true);
        let entity = s.owner.player.get_entity();
        entity.sneaking.store(!crouching, Relaxed);
        if crouching {
            entity.set_pose(pumpkin_data::entity::EntityPose::Crouching);
        }
        position(&s, false, at(1.75), false);
        assert_eq!(
            counts(&s),
            if crouching { (1, 1) } else { (0, 0) },
            "airborne helper uses pose; Player discrete suppression is grounded only"
        );
    }
}
