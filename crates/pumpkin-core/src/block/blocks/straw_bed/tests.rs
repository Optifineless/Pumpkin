use super::*;
use crate::{
    block::blocks::bed::test_support::PlayerFixture,
    net::java::combat_test_support::TestPlayer,
    world::spawn_test_support::{proto, publish},
};
use pumpkin_data::{biome::Biome, block_properties::HorizontalFacing};

#[tokio::test]
async fn straw_foot_sleep_uses_foot_shape_height_and_breaks_on_wake() {
    let fixture = PlayerFixture::new();
    publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    let foot = BlockPos::new(5, 64, 5);
    let mut props = BedProperties::default(&Block::STRAW_BED);
    props.part = BedPart::Foot;
    props.facing = HorizontalFacing::East;
    fixture.world.set_block_state(
        &foot,
        props.to_state_id(&Block::STRAW_BED),
        BlockFlags::NOTIFY_ALL,
    );
    let head = foot.offset(props.facing.to_offset());
    fixture.world.level_time.lock().unwrap().time_of_day = 13_000;
    let client = TestPlayer::new(&fixture.world);
    client.player.get_entity().set_pos(foot.to_centered_f64());
    StrawBedBlock::use_bed(&fixture.world, &client.player, &Block::STRAW_BED, &foot);
    assert_eq!(client.player.sleeping_bed_pos.load(), Some(head));
    // StrawBedBlock.getSleepHeight: foot top 4/16, LivingEntity.setPosToBed adds 2/16.
    assert_eq!(client.player.position().y, 64.375);
    client.player.wake_up();
    assert!(fixture.world.get_block(&foot).is_air());
    assert!(fixture.world.get_block(&head).is_air());
    fixture.finish().await;
}
