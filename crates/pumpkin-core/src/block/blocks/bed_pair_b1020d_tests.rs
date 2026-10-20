use super::*;
use crate::block::entities::bed::BedBlockEntity;
use pumpkin_data::{BlockStateId, block_properties::HorizontalFacing};

fn raw_half(
    world: &Arc<World>,
    position: BlockPos,
    block: &'static Block,
    part: BedPart,
    facing: HorizontalFacing,
) -> BlockStateId {
    let mut properties = WhiteBedLikeProperties::default(block);
    properties.part = part;
    properties.facing = facing;
    properties.occupied = true;
    let state = properties.to_state_id(block);
    world.set_block_state(
        &position,
        state,
        BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
    );
    world.add_block_entity(Arc::new(BedBlockEntity::new(position)));
    state
}

async fn mine_orphan(partner_block: &'static Block, partner_facing: HorizontalFacing) {
    let fixture = PlayerFixture::new();
    publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    let client = TestPlayer::new(&fixture.world);
    client.player.permission_lvl.store(PermissionLvl::Four);
    client.player.gamemode.store(GameMode::Survival);
    client.player.get_entity().set_pos(FOOT.to_f64());
    raw_half(
        &fixture.world,
        FOOT,
        &Block::RED_BED,
        BedPart::Foot,
        HorizontalFacing::East,
    );
    let partner_foot = HEAD.offset(partner_facing.opposite().to_offset());
    if partner_foot != FOOT {
        raw_half(
            &fixture.world,
            partner_foot,
            partner_block,
            BedPart::Foot,
            partner_facing,
        );
    }
    let expected = raw_half(
        &fixture.world,
        HEAD,
        partner_block,
        BedPart::Head,
        partner_facing,
    );
    let entity = fixture.world.get_block_entity(&HEAD).unwrap();
    let actor_data = entity.bedrock_block_actor_data(expected);
    let mut saved = pumpkin_nbt::NbtCompound::new();
    entity.write_internal(&mut saved);
    let foot_state = fixture.world.get_block_state_id(&partner_foot);

    destroy(&client, &fixture.world, FOOT);

    assert!(fixture.world.get_block(&FOOT).is_air());
    let reciprocal = partner_block == &Block::RED_BED && partner_facing == HorizontalFacing::East;
    if reciprocal {
        assert!(fixture.world.get_block(&HEAD).is_air());
        assert!(fixture.world.get_block_entity(&HEAD).is_none());
    } else {
        assert_eq!(
            fixture.world.get_block_state_id(&HEAD),
            expected,
            "mining an orphan must preserve an unrelated head"
        );
        assert_eq!(fixture.world.get_block_state_id(&partner_foot), foot_state);
        let retained = fixture.world.get_block_entity(&HEAD).unwrap();
        assert!(Arc::ptr_eq(&retained, &entity));
        let mut retained_saved = pumpkin_nbt::NbtCompound::new();
        retained.write_internal(&mut retained_saved);
        assert_eq!(retained_saved, saved);
        assert_eq!(retained.bedrock_block_actor_data(expected), actor_data);
    }
    finish(fixture).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_orphan_bed_keeps_unrelated_pair() {
    mine_orphan(&Block::RED_BED, HorizontalFacing::North).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_orphan_bed_keeps_unrelated_pair_reciprocal_control() {
    mine_orphan(&Block::RED_BED, HorizontalFacing::East).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_orphan_bed_keeps_unrelated_pair_color_control() {
    mine_orphan(&Block::BLUE_BED, HorizontalFacing::North).await;
}
