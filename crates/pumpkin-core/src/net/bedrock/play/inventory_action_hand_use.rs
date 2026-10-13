use super::{
    BedrockClient, CSetPlayerInventory, EquipmentSlot, Hand, ItemStack, ItemStackSerializer,
    Player, PlayerInventory, hand_inventory_slot, hand_screen_handler_slot,
};
use crate::net::java::play::hand_use_result::{HandMutation, write_back_hand_item};
use pumpkin_data::BlockDirection;
use pumpkin_inventory::screen_handler::ScreenHandler;
use pumpkin_protocol::bedrock::{
    client::{player_hotbar::CPlayerHotbar, update_block::CUpdateBlock},
    server::inventory_transaction::{HandSlot, UseItemTransactionData},
};
use pumpkin_protocol::codec::var_uint::VarUInt;
use pumpkin_util::math::position::BlockPos;
use std::sync::Arc;

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
    pub(super) fn reconcile_use_item_selection(
        &self,
        player: &Arc<Player>,
        data: &UseItemTransactionData,
    ) -> Option<u8> {
        let requested = usize::try_from(data.hot_bar_slot.0)
            .ok()
            .filter(|slot| PlayerInventory::is_valid_hotbar_index(*slot));
        let Some(requested) = requested else {
            self.correct_use_item_selection(player, data.hand);
            return None;
        };
        let requested = requested as u8;
        if player.inventory().get_selected_slot() != requested {
            let Some(server) = player.world().server.upgrade() else {
                self.correct_use_item_selection(player, data.hand);
                return None;
            };
            // Geyser BedrockInventoryTransactionTranslator.translate switches before use admission.
            if !self.select_hotbar_slot(&server, player, requested) {
                self.correct_use_item_selection(player, data.hand);
                return None;
            }
            self.send_live_hotbar_selection(player);
            if player.inventory().get_selected_slot() != requested {
                self.correct_use_item_selection(player, data.hand);
                return None;
            }
        }
        Some(requested)
    }

    fn send_live_hotbar_selection(&self, player: &Player) {
        self.try_enqueue_client_packet(&CPlayerHotbar {
            selected_slot: VarUInt(u32::from(player.inventory().get_selected_slot())),
            container_id: 0,
            should_select_slot: true,
        });
    }

    fn correct_use_item_selection(&self, player: &Player, hand: HandSlot) {
        self.send_player_inventory_content(player);
        Self::sync_live_hands(player, Hand::Right, player.inventory().get_selected_slot());
        if hand == HandSlot::Offhand {
            Self::sync_live_hands(player, Hand::Left, player.inventory().get_selected_slot());
        }
        self.send_live_hotbar_selection(player);
    }

    pub(super) fn correct_denied_placement(
        &self,
        player: &Player,
        clicked: BlockPos,
        face: BlockDirection,
    ) {
        // ServerGamePacketListenerImpl.handleUseItemOn corrects pos and pos.relative(direction).
        let world = player.world();
        let offset = face.to_offset();
        // BlockPos.relative uses Java's wrapping int addition, even for rejected positions.
        let adjacent = BlockPos::new(
            clicked.0.x.wrapping_add(offset.x),
            clicked.0.y.wrapping_add(offset.y),
            clicked.0.z.wrapping_add(offset.z),
        );
        for position in [clicked, adjacent] {
            let state = world.get_block_state_id(&position);
            self.try_enqueue_client_packet(&CUpdateBlock::new(
                position,
                pumpkin_data::BlockState::to_be_network_id(state),
            ));
        }
    }

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
