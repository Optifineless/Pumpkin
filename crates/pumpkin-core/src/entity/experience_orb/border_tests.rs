#![allow(clippy::unwrap_used)]

use super::ExperienceOrbEntity;
use crate::{
    entity::Entity,
    world::{BlockFlags, border::test_support::Fixture},
};
use pumpkin_data::{Block, entity::EntityType};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020_g04_orb_free_position_reads_the_moving_extent() {
    let fixture = Fixture::new();
    // A partial overlap has an escape on the east side inside the live 100-wide border.
    fixture.world.set_block_state(
        &BlockPos::new(30, 65, 3),
        Block::STONE.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let start = Vector3::new(31.05, 65.4, 3.5);
    for (from, to, early_moves) in [(100, 20, true), (20, 100, false)] {
        fixture.run(&format!("worldborder set {from}"));
        fixture.run(&format!("worldborder set {to} 200t"));
        let orb = ExperienceOrbEntity::new_empty(Entity::new(
            fixture.world.clone(),
            start,
            &EntityType::EXPERIENCE_ORB,
        ));
        orb.unstuck_if_possible(1.0);
        assert_eq!(
            orb.entity.pos.load() != start,
            early_moves,
            "orb collision filtering must use the early live extent"
        );
        fixture.ticks(150);
        orb.entity.set_pos(start);
        orb.unstuck_if_possible(1.0);
        assert_eq!(
            orb.entity.pos.load() != start,
            !early_moves,
            "orb collision filtering must follow progressed bounds"
        );
    }
    fixture.finish().await;
}
