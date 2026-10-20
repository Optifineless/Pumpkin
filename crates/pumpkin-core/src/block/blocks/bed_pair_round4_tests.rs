use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b5_survival_java_head_break_drops_straw_bed_and_spore_blossom_once() {
    let fixture = PlayerFixture::new();
    publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    let client = TestPlayer::new(&fixture.world);
    client
        .player
        .get_entity()
        .set_pos(Vector3::new(5.5, 64.0, 2.5));
    client.player.get_entity().set_rotation(-90.0, 0.0);

    let foot = BlockPos::new(5, 65, 5);
    let supports = [
        foot.down(),
        BlockPos::new(6, 64, 5),
        BlockPos::new(4, 64, 5),
        BlockPos::new(5, 64, 6),
        BlockPos::new(5, 64, 4),
    ];
    for support in supports {
        fixture.world.set_block_state(
            &support,
            Block::STONE.default_state.id,
            BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
        );
    }
    client
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::new(1, &Item::STRAW_BED));
    client
        .client()
        .handle_use_item_on(
            &client.player,
            &SUseItemOn {
                hand: VarInt(0),
                position: foot.down(),
                face: VarInt(BlockDirection::Up as i32),
                cursor_pos: Vector3::new(0.5, 1.0, 0.5),
                inside_block: false,
                is_against_world_border: false,
                sequence: VarInt(1),
            },
            &fixture.world.server.upgrade().unwrap(),
        )
        .unwrap();
    assert_eq!(fixture.world.get_block(&foot), &Block::STRAW_BED);
    let target_props =
        WhiteBedLikeProperties::from_state_id(fixture.world.get_block_state_id(&foot));
    let (bed_foot, head) = if target_props.part == BedPart::Foot {
        (foot, foot.offset(target_props.facing.to_offset()))
    } else {
        (
            foot.offset(target_props.facing.opposite().to_offset()),
            foot,
        )
    };
    assert_eq!(fixture.world.get_block(&bed_foot), &Block::STRAW_BED);
    assert_eq!(fixture.world.get_block(&head), &Block::STRAW_BED);
    assert!(client.player.inventory().held_item().is_empty());

    for support in supports {
        fixture.world.set_block_state(
            &support,
            Block::AIR.default_state.id,
            BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
        );
    }
    assert_eq!(fixture.world.get_block(&bed_foot), &Block::STRAW_BED);
    assert_eq!(fixture.world.get_block(&head), &Block::STRAW_BED);

    client.player.get_entity().set_sneaking(true);
    client
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::new(1, &Item::SPORE_BLOSSOM));
    client
        .client()
        .handle_use_item_on(
            &client.player,
            &SUseItemOn {
                hand: VarInt(0),
                position: head,
                face: VarInt(BlockDirection::Down as i32),
                cursor_pos: Vector3::new(0.5, 0.0, 0.5),
                inside_block: false,
                is_against_world_border: false,
                sequence: VarInt(2),
            },
            &fixture.world.server.upgrade().unwrap(),
        )
        .unwrap();
    let blossom = head.down();
    assert_eq!(fixture.world.get_block(&blossom), &Block::SPORE_BLOSSOM);
    assert!(client.player.inventory().held_item().is_empty());

    destroy(&client, &fixture.world, head);

    assert_eq!(drops(&fixture.world, &Item::STRAW_BED), 1);
    assert_eq!(drops(&fixture.world, &Item::SPORE_BLOSSOM), 1);
    assert!(fixture.world.get_block(&bed_foot).is_air());
    assert!(fixture.world.get_block(&head).is_air());
    assert!(fixture.world.get_block(&blossom).is_air());
    finish(fixture).await;
}
