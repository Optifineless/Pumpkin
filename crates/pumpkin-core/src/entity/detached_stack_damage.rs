use super::player::Player;

impl Player {
    /// Fires [`PlayerItemDamageEvent`], and [`PlayerItemBreakEvent`] when `broken`, for an item that took damage.
    pub fn fire_item_damage_events(
        &self,
        item: &pumpkin_data::item::Item,
        amount: i32,
        broken: bool,
    ) {
        let world = self.world();
        let Some(server) = world.server.upgrade() else {
            return;
        };
        let Some(player_arc) = world.get_player_by_uuid(self.gameprofile.id) else {
            return;
        };

        let mut event =
            crate::plugin::api::events::player::player_item_damage::PlayerItemDamageEvent::new(
                player_arc.clone(),
                item.registry_key.to_string(),
                amount,
            );
        server.plugin_manager.fire_blocking(&server, &mut event);
        if broken {
            let mut event =
                crate::plugin::api::events::player::player_item_break::PlayerItemBreakEvent::new(
                    player_arc,
                    item.registry_key.to_string(),
                );
            server.plugin_manager.fire_blocking(&server, &mut event);
        }
    }

}
