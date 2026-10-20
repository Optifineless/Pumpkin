use super::*;
use crate::{
    entity::player::Player,
    plugin::{
        BoxFuture, EventHandler, EventPriority, api::events::server::packet::PacketSentEvent,
    },
};
use std::sync::atomic::{AtomicBool, Ordering};

struct GapMutation {
    moving: Arc<Player>,
    fired: Arc<AtomicBool>,
}

impl EventHandler<PacketSentEvent> for GapMutation {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        event: &'a mut PacketSentEvent,
    ) -> BoxFuture<'a, ()> {
        if event.packet_id == pumpkin_data::packet::clientbound::play::REMOVE_ENTITIES.0
            && !self.fired.swap(true, Ordering::Relaxed)
        {
            let world = self.moving.world();
            assert!(
                world
                    .get_player_by_uuid(self.moving.gameprofile.id)
                    .is_none()
            );
            assert!(world.scoreboard.try_lock().is_ok());
            command(server, &world, "scoreboard players set Gap tally 99");
        }
        Box::pin(async {})
    }
}

pub(super) fn mutate_during_transfer(server: &Arc<Server>, moving: Arc<Player>) -> Arc<AtomicBool> {
    let fired = Arc::new(AtomicBool::new(false));
    server.plugin_manager.register::<PacketSentEvent, _>(
        Arc::new(GapMutation {
            moving,
            fired: fired.clone(),
        }),
        EventPriority::Normal,
        true,
    );
    fired
}
