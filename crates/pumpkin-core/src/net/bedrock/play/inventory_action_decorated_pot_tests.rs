use super::*;
use crate::{
    block::entities::{BlockEntity, bed::BedBlockEntity, decorated_pot::DecoratedPotBlockEntity},
    net::bedrock::play::inventory_exploit_tests::packet_payload,
    world::World,
};
use pumpkin_inventory::Inventory;
use pumpkin_nbt::{Nbt, compound::NbtCompound, deserializer::NbtReadHelperBedrock, tag::NbtTag};
use pumpkin_protocol::{bedrock::client::update_block::CUpdateBlock, serial::PacketRead};
use pumpkin_util::permission::PermissionLvl;
use std::io::Cursor;

async fn new_fixture() -> HandUseFixture {
    tokio::time::timeout(std::time::Duration::from_secs(20), HandUseFixture::new())
        .await
        .expect("Bedrock decorated pot fixture setup did not finish")
}

async fn verify_fixture(
    fixture: HandUseFixture,
    verify: impl FnOnce(&mut HandUseFixture) -> TestResult,
) -> TestResult {
    tokio::time::timeout(
        std::time::Duration::from_secs(20),
        fixture.verify_and_close(verify),
    )
    .await
    .expect("Bedrock decorated pot fixture cleanup did not finish")
}

fn configure_protected_use(
    fixture: &HandUseFixture,
    position: BlockPos,
    permission: PermissionLvl,
) {
    let player = &fixture.owner.player;
    player.permission_lvl.store(permission);
    player.get_entity().set_pos(Vector3::new(8.5, 64.0, 10.5));
    fixture.world.server.level_info.rcu(|info| {
        let mut next = (**info).clone();
        next.spawn_x = position.0.x;
        next.spawn_y = position.0.y;
        next.spawn_z = position.0.z;
        next
    });
    fixture
        .world
        .server
        .data
        .operator_config
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .ops
        .push(pumpkin_config::op::Op::new(
            if matches!(permission, PermissionLvl::Zero) {
                uuid::Uuid::new_v4()
            } else {
                player.gameprofile.id
            },
            "decorated-pot-test-op".to_owned(),
            permission,
            false,
        ));
}

fn mixed_saved_pot(world: &Arc<World>, position: BlockPos) -> Arc<DecoratedPotBlockEntity> {
    world.set_block_state(
        &position,
        Block::DECORATED_POT.default_state.id,
        pumpkin_world::world::BlockFlags::FORCE_STATE,
    );
    let mut sherds = NbtCompound::new();
    for (face, item) in [
        ("back", "minecraft:angler_pottery_sherd"),
        ("left", "minecraft:archer_pottery_sherd"),
        ("front", "minecraft:heart_pottery_sherd"),
    ] {
        let mut template = NbtCompound::new();
        template.put_string("id", item.to_owned());
        sherds.put_compound(face, template);
    }
    let mut item = NbtCompound::new();
    item.put_string("id", "minecraft:diamond".to_owned());
    item.put_int("count", 3);
    let mut saved = NbtCompound::new();
    saved.put_compound("sherds", sherds);
    saved.put_compound("item", item);
    let pot = Arc::new(DecoratedPotBlockEntity::from_nbt(&saved, position));
    world.add_block_entity(pot.clone());
    pot
}

fn predicted_insertion(
    player: &Player,
    position: BlockPos,
    diamond: &ItemStack,
) -> SInventoryTransaction {
    click_block_packet(
        player,
        HandSlot::Mainhand,
        position,
        diamond,
        vec![action(
            Some(WINDOW_ID_INVENTORY),
            4,
            NetworkItemDescriptor::from(diamond),
            NetworkItemDescriptor::from(ItemStack::EMPTY),
        )],
    )
}

fn assert_block_corrections(
    fixture: &HandUseFixture,
    packets: &[bytes::Bytes],
    position: BlockPos,
) -> TestResult {
    let world = fixture.world.world();
    for corrected in [position, position.up()] {
        let update = CUpdateBlock::new(
            corrected,
            pumpkin_data::BlockState::to_be_network_id(world.get_block_state_id(&corrected)),
        );
        assert!(packets.contains(&fixture.owner.client().serialize_packet(&update)?));
    }
    Ok(())
}

fn assert_actor_correction(
    packets: &[bytes::Bytes],
    position: BlockPos,
    expected: &NbtCompound,
) -> TestResult {
    let mut corrections = 0;
    for packet in packets {
        let Some(mut payload) = packet_payload(packet, 56) else {
            continue;
        };
        if BlockPos::read(&mut payload)? != position {
            continue;
        }
        let mut reader = NbtReadHelperBedrock::new(Cursor::new(payload));
        let actual = Nbt::read(&mut reader)?.root_tag;
        assert_eq!(&actual, expected, "authoritative Bedrock actor fields");
        corrections += 1;
    }
    assert_eq!(corrections, 1, "one authoritative block-actor correction");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_bedrock_denied_pot_resends_authoritative_actor() -> TestResult {
    let mut fixture = new_fixture().await;
    let world = fixture.world.world();
    let player = fixture.owner.player.clone();
    let position = BlockPos::new(8, 65, 8);
    configure_protected_use(&fixture, position, PermissionLvl::Zero);
    let pot = mixed_saved_pot(&world, position);
    let contents = pot.get_stack(0);
    assert_eq!(contents.item, &Item::DIAMOND);
    assert_eq!(contents.item_count, 3);
    assert!(world.is_in_spawn_protection(&player, &position));
    let diamond = ItemStack::new(1, &Item::DIAMOND);
    player.inventory().set_selected_slot(4);
    player.inventory().set_stack(4, diamond.clone());
    fixture.drain_packets();
    fixture
        .owner
        .client()
        .handle_inventory_action(&player, predicted_insertion(&player, position, &diamond));

    verify_fixture(fixture, |fixture| {
        assert!(pot.get_stack(0).are_equal(&contents));
        assert!(player.inventory().get_stack(4).are_equal(&diamond));
        assert_eq!(world.get_block(&position), &Block::DECORATED_POT);
        let packets = fixture.owner.take_packets();
        assert_block_corrections(fixture, &packets, position)?;
        assert_owner_inventory_slot(&packets, WINDOW_ID_INVENTORY, 4, &diamond, false)?;

        // Geyser be2f5da, DecoratedPotBlockEntityTranslator.translateTag: back/left/right/front.
        // This protocol-2193 authority sends no stored-item fields; server contents stay intact.
        let mut expected = NbtCompound::new();
        expected.put_string("id", "DecoratedPot".to_owned());
        expected.put_int("x", 8);
        expected.put_int("y", 65);
        expected.put_int("z", 8);
        expected.put_list(
            "sherds",
            vec![
                NbtTag::String("minecraft:angler_pottery_sherd".into()),
                NbtTag::String("minecraft:archer_pottery_sherd".into()),
                NbtTag::String("minecraft:brick".into()),
                NbtTag::String("minecraft:heart_pottery_sherd".into()),
            ],
        );
        assert_actor_correction(&packets, position, &expected)
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_bedrock_denied_bed_retains_actor_correction() -> TestResult {
    let mut fixture = new_fixture().await;
    let world = fixture.world.world();
    let player = fixture.owner.player.clone();
    let position = BlockPos::new(8, 65, 8);
    configure_protected_use(&fixture, position, PermissionLvl::Zero);
    world.set_block_state(
        &position,
        Block::BLUE_BED.default_state.id,
        pumpkin_world::world::BlockFlags::FORCE_STATE,
    );
    world.add_block_entity(Arc::new(BedBlockEntity::new(position)));
    assert!(world.is_in_spawn_protection(&player, &position));
    let diamond = ItemStack::new(1, &Item::DIAMOND);
    player.inventory().set_selected_slot(4);
    player.inventory().set_stack(4, diamond.clone());
    fixture.drain_packets();
    fixture
        .owner
        .client()
        .handle_inventory_action(&player, predicted_insertion(&player, position, &diamond));

    verify_fixture(fixture, |fixture| {
        assert!(player.inventory().get_stack(4).are_equal(&diamond));
        assert_eq!(world.get_block(&position), &Block::BLUE_BED);
        let packets = fixture.owner.take_packets();
        assert_block_corrections(fixture, &packets, position)?;
        assert_owner_inventory_slot(&packets, WINDOW_ID_INVENTORY, 4, &diamond, false)?;
        let mut expected = NbtCompound::new();
        expected.put_string("id", "Bed".to_owned());
        expected.put_int("x", 8);
        expected.put_int("y", 65);
        expected.put_int("z", 8);
        expected.put_byte("color", 11);
        expected.put_bool("isMovable", true);
        assert_actor_correction(&packets, position, &expected)
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_bedrock_allowed_pot_insertion_conserves_contents() -> TestResult {
    let mut fixture = new_fixture().await;
    let world = fixture.world.world();
    let player = fixture.owner.player.clone();
    let position = BlockPos::new(8, 65, 8);
    configure_protected_use(&fixture, position, PermissionLvl::Four);
    let pot = mixed_saved_pot(&world, position);
    assert!(!world.is_in_spawn_protection(&player, &position));
    assert_eq!(pot.get_stack(0).item_count, 3);
    let diamond = ItemStack::new(1, &Item::DIAMOND);
    player.inventory().set_selected_slot(4);
    player.inventory().set_stack(4, diamond.clone());
    fixture.drain_packets();
    fixture
        .owner
        .client()
        .handle_inventory_action(&player, predicted_insertion(&player, position, &diamond));

    verify_fixture(fixture, |fixture| {
        assert!(player.inventory().get_stack(4).is_empty());
        assert!(
            pot.get_stack(0)
                .are_equal(&ItemStack::new(4, &Item::DIAMOND))
        );
        assert_eq!(world.get_block(&position), &Block::DECORATED_POT);
        let packets = fixture.owner.take_packets();
        assert_owner_inventory_slot(&packets, WINDOW_ID_INVENTORY, 4, ItemStack::EMPTY, false)
    })
    .await
}
