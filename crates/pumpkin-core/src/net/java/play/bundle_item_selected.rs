#[allow(clippy::wildcard_imports)]
use super::*;

impl JavaClient {
    pub fn handle_bundle_item_selected(&self, player: &Arc<Player>, packet: &SBundleItemSelected) {
        if !player.has_client_loaded() {
            return;
        }
        player.update_last_action_time();

        let selected_item_index = packet.selected_item_index.0;
        if selected_item_index < 0 && selected_item_index != -1 {
            self.try_kick(&TextComponent::text("Invalid selected item index"));
            return;
        }

        // AbstractContainerMenu.setSelectedBundleItemIndex -> BundleItem.toggleSelectedItem.
        let menu = player
            .current_screen_handler
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let menu = menu
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Ok(index) = usize::try_from(packet.slot_id.0)
            && let Some(slot) = menu.get_behaviour().slots.get(index)
        {
            let mut stack = slot.get_stack();
            if let Some(contents) = stack
                .get_data_component_mut::<pumpkin_data::data_component_impl::BundleContentsImpl>()
            {
                contents.selected_item = if contents.selected_item != selected_item_index
                    && usize::try_from(selected_item_index).is_ok_and(|i| i < contents.items.len())
                {
                    selected_item_index
                } else {
                    -1
                };
                slot.set_stack(stack);
            }
        }
    }
}
