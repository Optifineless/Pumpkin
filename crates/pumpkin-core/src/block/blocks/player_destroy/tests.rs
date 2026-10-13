use crate::{
    block::{PlayerWillDestroyArgs, blocks::player_destroy::remove_partner},
    net::java::combat_test_support::TestPlayer,
    world::spawn_test_support::{proto, publish},
};
use pumpkin_data::{Block, biome::Biome};
use pumpkin_util::math::position::BlockPos;

#[tokio::test]
async fn paired_removal_applies_replacement_without_drops() {
    let fixture = crate::block::blocks::player_destroy::test_support::PlayerFixture::new();
    let chunk = publish(&fixture.world, proto(&Biome::PLAINS, &Block::DIRT));
    let source = BlockPos::new(5, 65, 5);
    let partner = source.down();
    chunk.set_block_absolute_y(5, 64, 5, Block::SUNFLOWER.default_state.id);
    let player = TestPlayer::new(&fixture.world);

    remove_partner(
        PlayerWillDestroyArgs {
            world: &fixture.world,
            block: &Block::SUNFLOWER,
            state: Block::SUNFLOWER.default_state,
            position: &source,
            player: &player.player,
        },
        partner,
        Block::WATER.default_state.id,
    );

    assert_eq!(fixture.world.get_block(&partner), &Block::WATER);
    assert!(
        !fixture
            .world
            .entities
            .load()
            .iter()
            .any(|entity| entity.get_item_entity().is_some())
    );
    fixture.finish().await;
}
