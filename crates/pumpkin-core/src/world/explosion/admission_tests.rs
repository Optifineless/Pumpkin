use super::{BlockInteraction, Explosion, ExplosionInteraction};
use crate::server::combat_test_support;
use pumpkin_data::Block;
use pumpkin_util::math::{position::BlockPos, vector2::Vector2, vector3::Vector3};
use pumpkin_world::world::BlockFlags;

#[tokio::test]
async fn explosion_inflight_capture_keeps_loot_and_terrain_unchanged() {
    for direct in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        let world = combat_test_support::world(&server, directory.path());
        world.level_info.rcu(|info| {
            let mut info = (**info).clone();
            info.game_rules.block_explosion_drop_decay = false;
            info
        });
        let chunk = Vector2::new(0, 0);
        combat_test_support::publish_empty_chunk(&world, chunk);
        let chest = BlockPos::new(8, 64, 8);
        world.set_block_state(
            &chest,
            Block::CHEST.default_state.id,
            BlockFlags::FORCE_STATE,
        );
        assert!(world.begin_block_entity_unload(chunk));

        // A legacy serializer calling either public explosion entry must not wait on itself.
        let center = Vector3::new(8.5, 64.5, 8.5);
        if direct {
            let result = Explosion::new(4.0, center, BlockInteraction::Destroy).explode(&world);
            assert_eq!(result.block_count, 0);
        } else {
            world.explode(center, 4.0, ExplosionInteraction::Block);
        }
        world.finish_block_entity_unload(chunk);
        assert_eq!(world.get_block(&chest), &Block::CHEST);
        assert!(
            world.entities.load().is_empty(),
            "an in-flight capture refused the terrain write after explosion loot spawned"
        );
        world.level.shutdown().await.unwrap();
    }
}
