use crate::entity::equipment_break_status;
use crate::entity::{EntityBase, player::Player};
use pumpkin_data::{
    BlockStateId, data_component_impl::EquipmentSlot, item::Item, item_stack::ItemStack,
    statistic::StatisticCategory, tag::Taggable,
};
use pumpkin_inventory::{Inventory, player::player_inventory::PlayerInventory};
use pumpkin_util::Hand;

/// Returns the inventory slot to retain while an interaction mutates a hand clone.
pub fn hand_slot(player: &Player, hand: Hand) -> usize {
    if hand == Hand::Right {
        player.inventory().get_selected_slot() as usize
    } else {
        PlayerInventory::OFF_HAND_SLOT
    }
}

/// Distinguishes item use from equipment transfer when a hand becomes empty.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HandMutation {
    ItemUse,
    EquipmentTransfer,
}

/// Persists a mutated hand clone unless an interaction already replaced the real hand.
/// ServerPlayerGameMode.useItemOn passes the real stack; direct inventory writers take precedence here.
pub(super) fn write_back_used_item(
    player: &Player,
    hand: Hand,
    source_slot: usize,
    before: &ItemStack,
    after: &ItemStack,
) {
    write_back_hand_item(
        player,
        hand,
        source_slot,
        before,
        after,
        HandMutation::ItemUse,
    );
}

/// Writes a hand mutation, counting depletion as breakage only for item use.
pub fn write_back_hand_item(
    player: &Player,
    hand: Hand,
    source_slot: usize,
    before: &ItemStack,
    after: &ItemStack,
    mutation: HandMutation,
) {
    if after.are_equal(before) {
        return;
    }
    let stored = if after.is_empty() {
        ItemStack::EMPTY.clone()
    } else {
        after.clone()
    };
    let mut committed = false;
    // Player.setItemInHand runs on vanilla's server thread; validate and write under one lock.
    player.inventory().update_slot(source_slot, &mut |current| {
        if current.uid == before.uid && current.are_equal(before) {
            *current = stored.clone();
            committed = true;
        }
    });
    if !committed {
        return;
    }
    // LivingEntity.onEquippedItemBroken broadcasts while the client still has the old item.
    // ArmorStand.swapItem transfers equipment without ItemStack.hurtAndBreak.
    if mutation == HandMutation::ItemUse
        && !before.is_empty()
        && before.is_damageable()
        && after.is_empty()
    {
        player.increment_stat(StatisticCategory::Broken, i32::from(before.item.id), 1);
        let slot = if hand == Hand::Right {
            EquipmentSlot::MAIN_HAND
        } else {
            EquipmentSlot::OFF_HAND
        };
        player
            .world()
            .send_entity_status(player.get_entity(), equipment_break_status(&slot), None);
    }
    player.sync_hand_slot(source_slot, stored);
}

// HoneycombItem.useOn and AxeItem.useOn trigger before replacing the clicked block.
// ServerPlayerGameMode.useItemOn otherwise supplies its state after interaction.
pub(super) fn trigger_item_used_on_block(
    player: &Player,
    position: pumpkin_util::math::position::BlockPos,
    item: &ItemStack,
    state_before: BlockStateId,
) {
    let state = if item.item.id == Item::HONEYCOMB.id
        || item.item.has_tag(&pumpkin_data::tag::Item::MINECRAFT_AXES)
    {
        state_before
    } else {
        player.world().get_block_state_id(&position)
    };
    player.trigger_advancement(
        crate::entity::player::advancement::trigger::AdvancementTrigger::ItemUsedOnBlock {
            position,
            item: item.clone(),
            state,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{net::java::combat_test_support::TestPlayer, server::combat_test_support};

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn hand_use_break_animation_precedes_empty_hand_sync() {
        use pumpkin_protocol::{
            codec::{item_stack_seralizer::ItemStackSerializer, var_int::VarInt},
            java::client::play::{CEntityStatus, CSetPlayerInventory},
        };
        let dir = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(dir.path());
        let world = combat_test_support::world(&server, dir.path());
        let mut fixture = TestPlayer::new(&world);
        let before = ItemStack::new(1, &Item::SHEARS);
        fixture.player.inventory().set_stack(40, before.clone());
        fixture.take_packets();
        write_back_used_item(&fixture.player, Hand::Left, 40, &before, ItemStack::EMPTY);
        let status = fixture
            .client()
            .serialize_packet(&CEntityStatus::new(
                fixture.player.entity_id(),
                equipment_break_status(&EquipmentSlot::OFF_HAND) as i8,
            ))
            .unwrap();
        let slot = fixture
            .client()
            .serialize_packet(&CSetPlayerInventory::new(
                VarInt(40),
                &ItemStackSerializer::from(ItemStack::EMPTY.clone()),
            ))
            .unwrap();
        let packets = fixture.take_packets();
        assert!(
            packets.iter().position(|packet| *packet == status).unwrap()
                < packets.iter().position(|packet| *packet == slot).unwrap()
        );
        assert_eq!(
            fixture
                .player
                .stats
                .lock()
                .unwrap()
                .get(StatisticCategory::Broken, i32::from(Item::SHEARS.id),),
            1
        );
        assert!(world.level.shutdown().await.is_ok());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn hand_use_write_back_preserves_a_direct_inventory_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(dir.path());
        let world = combat_test_support::world(&server, dir.path());
        let fixture = TestPlayer::new(&world);
        let before = ItemStack::new(2, &Item::WATER_BUCKET);
        fixture
            .player
            .inventory()
            .set_stack_in_hand(Hand::Left, before.clone());
        let after = ItemStack::new(1, &Item::WATER_BUCKET);
        let replacement = ItemStack::new(1, &Item::COD_BUCKET);
        fixture
            .player
            .inventory()
            .set_stack_in_hand(Hand::Left, replacement.clone());
        write_back_used_item(&fixture.player, Hand::Left, 40, &before, &after);
        assert!(
            fixture
                .player
                .inventory()
                .off_hand_item()
                .are_equal(&replacement)
        );
        let identical_replacement = ItemStack::new(2, &Item::WATER_BUCKET);
        fixture
            .player
            .inventory()
            .set_stack_in_hand(Hand::Left, identical_replacement.clone());
        write_back_used_item(&fixture.player, Hand::Left, 40, &before, &after);
        assert!(
            fixture
                .player
                .inventory()
                .off_hand_item()
                .are_equal(&identical_replacement)
        );
        fixture.player.inventory().set_stack(0, before.clone());
        fixture.player.inventory().set_stack(1, before.clone());
        fixture.player.inventory().set_selected_slot(1);
        write_back_used_item(&fixture.player, Hand::Right, 0, &before, &after);
        assert!(fixture.player.inventory().get_stack(0).are_equal(&after));
        assert!(fixture.player.inventory().get_stack(1).are_equal(&before));
        let empty = after.copy_with_count(0);
        write_back_used_item(&fixture.player, Hand::Right, 0, &after, &empty);
        assert_eq!(fixture.player.inventory().get_stack(0).item, &Item::AIR);
        assert!(world.level.shutdown().await.is_ok());
    }
}
