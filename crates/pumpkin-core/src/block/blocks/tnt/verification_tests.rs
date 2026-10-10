use super::*;
use crate::{
    net::java::combat_test_support::TestPlayer,
    server::combat_test_support::{server, world},
};
use pumpkin_util::math::vector2::Vector2;

#[tokio::test]
async fn verification_breaking_unstable_tnt_primes_without_owner() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let player = TestPlayer::new(&world).player;
    player
        .permission_lvl
        .store(pumpkin_util::permission::PermissionLvl::Four);
    let mut props = TntLikeProperties::from_state_id(Block::TNT.default_state.id);
    props.r#unstable = true;
    let state = props.to_state_id(&Block::TNT);
    let chunk = pumpkin_world::chunk::ChunkData::empty_sync(0, 0);
    chunk.set_block_absolute_y(8, 64, 8, state);
    world.level.loaded_chunks.insert(Vector2::new(0, 0), chunk);
    let position = BlockPos::new(8, 64, 8);
    // Player breaking invokes the block callback after World.break_block removes the TNT.
    assert!(
        world
            .break_block(&position, Some(&player), BlockFlags::SKIP_DROPS)
            .is_some()
    );
    world.block_registry.broken(
        &world,
        &Block::TNT,
        &player,
        &position,
        &server,
        state.to_state(),
    );
    let entities = world.entities.load();
    let tnt = entities
        .iter()
        .find_map(|entity| entity.cast_any().downcast_ref::<TNTEntity>())
        .unwrap();
    assert!(tnt.owner().is_none());
    crate::server::fixture_lifecycle::finish().await;
}
