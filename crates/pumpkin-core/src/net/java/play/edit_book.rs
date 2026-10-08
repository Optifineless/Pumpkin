#[allow(clippy::wildcard_imports)]
use super::*;
use pumpkin_inventory::Inventory;

impl JavaClient {
    pub fn handle_edit_book(&self, player: &Player, packet: &SEditBook<'_>) {
        // ServerGamePacketListenerImpl.handleEditBook accepts hotbar slots or the off hand.
        let slot = packet.slot.0;
        if !(0..=8).contains(&slot) && slot != PlayerInventory::OFF_HAND_SLOT as i32 {
            return;
        }
        let held_stack = player.inventory().get_stack(slot as usize);
        if held_stack
            .get_data_component::<WritableBookContentImpl>()
            .is_none()
        {
            return;
        }

        let mut pages: Vec<String> = packet.pages.iter().map(|p| (*p).to_string()).collect();
        let mut title = packet.title.map(std::string::ToString::to_string);
        let signing = title.is_some();
        let slot = slot as u32;

        if let Some(player_arc) = player.world().get_player_by_uuid(player.gameprofile.id)
            && let Some(server) = player.world().server.upgrade()
        {
            let mut event =
                crate::plugin::api::events::player::player_edit_book::PlayerEditBookEvent {
                    player: player_arc,
                    slot,
                    pages: pages.clone(),
                    title: title.clone(),
                    signing,
                    cancelled: false,
                };
            server.plugin_manager.fire_blocking(&server, &mut event);
            if event.cancelled {
                return;
            }
            pages = event.pages;
            title = event.title;
        }

        let held_stack = player.inventory().get_stack(slot as usize);
        if held_stack
            .get_data_component::<WritableBookContentImpl>()
            .is_none()
        {
            return;
        }
        if let Some(title) = title {
            // signBook uses ItemStack.transmuteCopy to retain unrelated components.
            let mut written_book = held_stack;
            written_book.item = &Item::WRITTEN_BOOK;
            written_book.remove_data_component(DataComponent::WritableBookContent);
            let content = WrittenBookContentImpl {
                title,
                author: player.gameprofile.name.clone(),
                pages: pages.into_iter().map(TextComponent::text).collect(),
                // ServerGamePacketListenerImpl.signBook writes generation 0 and a resolved book.
                generation: 0,
                resolved: true,
            };
            written_book.set_data_component(content);
            player.inventory().set_stack(slot as usize, written_book);
        } else {
            let mut writable_book = held_stack;
            let content = WritableBookContentImpl { pages };
            writable_book.set_data_component(content);
            player.inventory().set_stack(slot as usize, writable_book);
        }
    }
}
