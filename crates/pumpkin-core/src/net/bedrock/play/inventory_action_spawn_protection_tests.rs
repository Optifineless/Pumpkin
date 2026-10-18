use super::*;
use crate::block::entities::decorated_pot::DecoratedPotBlockEntity;
use pumpkin_data::{Block, item::Item};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::bedrock::client::update_block::CUpdateBlock;
use pumpkin_protocol::bedrock::network_item::NetworkItemDescriptor;
use pumpkin_util::{math::vector3::Vector3, permission::PermissionLvl};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bedrock_spawn_protection_denies_decorated_pot_use_before_callbacks() -> TestResult {
    let mut fixture = HandUseFixture::new().await;
    let world = fixture.world.world();
    let player = fixture.owner.player.clone();
    player.permission_lvl.store(PermissionLvl::Zero);
    player.get_entity().set_pos(Vector3::new(8.5, 64.0, 10.5));
    let position = BlockPos::new(8, 65, 8);
    world.set_block_state(
        &position,
        Block::DECORATED_POT.default_state.id,
        pumpkin_world::world::BlockFlags::FORCE_STATE,
    );
    let pot = Arc::new(DecoratedPotBlockEntity::new(position));
    world.add_block_entity(pot.clone());
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
            uuid::Uuid::new_v4(),
            "spawn-protection-test-op".to_owned(),
            PermissionLvl::Zero,
            false,
        ));

    let diamond = ItemStack::new(1, &Item::DIAMOND);
    player.inventory().set_selected_slot(4);
    player.inventory().set_stack(4, diamond.clone());
    fixture.drain_packets();
    let packet = click_block_packet(
        &player,
        HandSlot::Mainhand,
        position,
        &diamond,
        vec![action(
            Some(WINDOW_ID_INVENTORY),
            4,
            NetworkItemDescriptor::from(&diamond),
            NetworkItemDescriptor::from(ItemStack::EMPTY),
        )],
    );
    fixture
        .owner
        .client()
        .handle_inventory_action(&player, packet);

    fixture
        .verify_and_close(|fixture| {
            assert!(pot.get_stack(0).is_empty());
            assert!(
                fixture
                    .owner
                    .player
                    .inventory()
                    .get_stack(4)
                    .are_equal(&diamond)
            );
            let packets = fixture.owner.take_packets();
            let adjacent = BlockPos::new(position.0.x, position.0.y + 1, position.0.z);
            for corrected in [position, adjacent] {
                let state_id = world.get_block_state_id(&corrected);
                let update = CUpdateBlock::new(
                    corrected,
                    pumpkin_data::BlockState::to_be_network_id(state_id),
                );
                assert!(packets.contains(&fixture.owner.client().serialize_packet(&update)?));
            }
            assert_owner_inventory_slot(&packets, WINDOW_ID_INVENTORY, 4, &diamond, false)?;
            Ok(())
        })
        .await
}
