use super::{
    BedrockClient, CSetPlayerInventory, EquipmentSlot, Hand, ItemStack, ItemStackSerializer,
    Player, PlayerInventory, hand_inventory_slot, hand_screen_handler_slot,
};
use crate::net::java::play::hand_use_result::{HandMutation, write_back_hand_item};
use pumpkin_inventory::screen_handler::ScreenHandler;

struct LiveHands {
    selected: u8,
    main: ItemStack,
    off: ItemStack,
}

fn same_stack(left: &ItemStack, right: &ItemStack) -> bool {
    left.uid == right.uid && left.are_equal(right)
}

impl LiveHands {
    fn read(player: &Player) -> Self {
        let selected = player.inventory().get_selected_slot();
        Self {
            selected,
            main: player.inventory().get_slot(usize::from(selected)),
            off: player.inventory().get_slot(PlayerInventory::OFF_HAND_SLOT),
        }
    }
}

impl BedrockClient {
    /// Commits the server's block-use result before publishing the live hands.
    pub(super) fn finish_block_use(
        player: &Player,
        hand: Hand,
        selected: u8,
        before: &ItemStack,
        after: &ItemStack,
    ) {
        // ServerPlayerGameMode.useItemOn keeps the source stack; callback replacements win.
        write_back_hand_item(
            player,
            hand,
            hand_inventory_slot(hand, selected),
            before,
            after,
            HandMutation::ItemUse,
        );
        Self::sync_live_hands(player, hand, selected);
    }

    /// Admits only the pinned originating stack and selection after synchronous callbacks.
    pub(super) fn stack_for_hand_use(
        player: &Player,
        hand: Hand,
        selected: u8,
        before: &ItemStack,
    ) -> Option<ItemStack> {
        // ServerPlayerGameMode.useItem/useItemOn receives one server-held stack at admission.
        let live = player
            .inventory()
            .get_slot(hand_inventory_slot(hand, selected));
        if player.inventory().get_selected_slot() != selected || !same_stack(&live, before) {
            Self::sync_live_hands(player, hand, selected);
            return None;
        }
        Some(live)
    }

    fn sync_hand_inventory_slot(player: &Player, hand: Hand, selected: u8) {
        let inventory_slot = hand_inventory_slot(hand, selected);
        let stack = player.inventory().get_slot(inventory_slot);
        if let Some(screen_slot) = hand_screen_handler_slot(hand, selected) {
            player
                .player_screen_handler
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .set_received_stack(screen_slot, stack.clone());
        }
        player.try_send_slot_set_packet(&CSetPlayerInventory::new(
            (inventory_slot as i32).into(),
            &ItemStackSerializer::from(stack),
        ));
        player.sync_inventory_to_client();
    }

    /// Corrects the originating slot and publishes current hands with no guard across a send.
    pub(super) fn sync_live_hands(player: &Player, hand: Hand, selected: u8) {
        // LivingEntity.detectEquipmentUpdates/collectEquipmentChanges reads current equipment.
        // Always correct the origin: even denied use can disagree with client prediction.
        let origin = player
            .inventory()
            .get_slot(hand_inventory_slot(hand, selected));
        Self::sync_hand_inventory_slot(player, hand, selected);
        let published = LiveHands::read(player);
        let mut equipment = Vec::new();
        if hand == Hand::Right || published.selected != selected {
            equipment.push((EquipmentSlot::MAIN_HAND, published.main.clone()));
        }
        if hand == Hand::Left {
            equipment.push((EquipmentSlot::OFF_HAND, published.off.clone()));
        }
        player.living_entity.send_equipment_changes(&equipment);

        // A PacketSentEvent can replace a stack or select another slot during publication.
        // Re-read both hands after that boundary, like the next vanilla equipment detection.
        let current = LiveHands::read(player);
        let main_changed =
            current.selected != published.selected || !same_stack(&current.main, &published.main);
        let off_changed = !same_stack(&current.off, &published.off);
        if !main_changed && !off_changed {
            return;
        }
        equipment.clear();
        if main_changed {
            if hand != Hand::Right
                || current.selected != selected
                || !same_stack(&current.main, &origin)
            {
                Self::sync_hand_inventory_slot(player, Hand::Right, current.selected);
            }
            equipment.push((EquipmentSlot::MAIN_HAND, current.main));
        }
        if off_changed {
            Self::sync_hand_inventory_slot(player, Hand::Left, current.selected);
            equipment.push((EquipmentSlot::OFF_HAND, current.off));
        }
        player.living_entity.send_equipment_changes(&equipment);
    }
}
