use super::{World, entity_tracker::TrackedEntity};
use crate::entity::{EntityBase, player::Player};
use std::{any::Any, sync::Arc};

impl World {
    // Pearl arrivals are tracked for feedback before joining the destination tick list.
    pub(super) fn player_from_tracking(&self, tracked: &TrackedEntity) -> Option<Arc<Player>> {
        let entity: Arc<dyn Any + Send + Sync> = tracked.entity.clone();
        let player = entity.downcast::<Player>().ok()?;
        (!player.get_entity().is_removed() && std::ptr::eq(player.world().as_ref(), self))
            .then_some(player)
    }

    pub(super) fn find_tracked_player(
        &self,
        matches: impl Fn(&Player) -> bool,
    ) -> Option<Arc<Player>> {
        // Clone tracking entries before entity access; no shard guard reaches caller code.
        self.entity_tracker
            .snapshot()
            .iter()
            .filter_map(|tracked| self.player_from_tracking(tracked))
            .find(|player| matches(player))
    }
}
