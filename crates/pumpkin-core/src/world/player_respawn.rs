use super::World;
use crate::entity::{EntityBase, player::Player};
use pumpkin_util::math::{boundingbox::BoundingBox, position::BlockPos, vector3::Vector3};
use std::sync::Arc;

#[cfg(test)]
pub mod test_support;

impl World {
    /// PlayerList.remove must address the current player instance, not a same-UUID rejoin.
    pub(crate) fn contains_player_instance(&self, player: &Player) -> bool {
        self.players
            .load()
            .iter()
            .any(|current| std::ptr::eq(current.as_ref(), player))
    }

    /// PlayerList.respawn may continue only while its connection and world instance are current.
    pub(crate) fn can_respawn_player(&self, player: &Arc<Player>) -> bool {
        !player.client.closed()
            && !player.get_entity().is_removed()
            && std::ptr::eq(player.world().as_ref(), self)
            && self.contains_player_instance(player)
    }

    pub(super) async fn transfer_respawning_player(
        self: &Arc<Self>,
        player: &Arc<Player>,
        destination: &Arc<Self>,
        position: Vector3<f64>,
    ) -> bool {
        if !self.can_respawn_player(player) {
            return false;
        }
        let destination_has_session = destination.players.load().iter().any(|current| {
            current.gameprofile.id == player.gameprofile.id && !Arc::ptr_eq(current, player)
        });
        if destination_has_session {
            return false;
        }

        #[cfg(test)]
        test_support::pause(player, test_support::Phase::BeforeAdmission).await;
        // Admit first: rejection must not disturb source membership or chunk streaming.
        let published = loop {
            let previous = destination.players.load_full();
            if !self.can_respawn_player(player)
                || previous
                    .iter()
                    .any(|current| current.gameprofile.id == player.gameprofile.id)
            {
                break false;
            }
            let mut next = (*previous).clone();
            next.push(player.clone());
            if destination.commit_game_event_player(player, &previous, Arc::new(next)) {
                break true;
            }
        };
        if !published {
            #[cfg(test)]
            test_support::pause(player, test_support::Phase::AdmissionRejected).await;
            return false;
        }

        if self.remove_player(player, false).await.is_none() {
            destination.forget_respawn_admission(player);
            return false;
        }
        player.unload_watched_chunks(self).await;
        if !owns_unpublished_session(self, player) {
            destination.forget_respawn_admission(player);
            return false;
        }
        player.change_world_chunks(&self.level, destination);
        if !self.commit_respawn_world(player, destination, position)
            || !destination.can_respawn_player(player)
        {
            destination.forget_respawn_admission(player);
            return false;
        }
        true
    }

    fn forget_respawn_admission(&self, player: &Arc<Player>) {
        self.withdraw_game_event_player(player);
    }

    // PlayerList.respawn commits the resolved location before the respawn hook.
    fn commit_respawn_world(
        &self,
        player: &Arc<Player>,
        destination: &Arc<Self>,
        position: Vector3<f64>,
    ) -> bool {
        if !owns_unpublished_session(self, player) {
            return false;
        }
        let entity = player.get_entity();
        let block_pos = BlockPos::new(
            position.x.floor() as i32,
            position.y.floor() as i32,
            position.z.floor() as i32,
        );
        let biome = Arc::new(destination.level.get_rough_biome(&block_pos));
        let bounds = BoundingBox::new_from_pos(
            position.x,
            position.y,
            position.z,
            &entity.entity_dimension.load(),
        );
        let _session = player
            .storage_session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if entity.is_removed() || !std::ptr::eq(entity.world.load().as_ref(), self) {
            return false;
        }
        // Every player save holds this existing boundary. Only atomic stores occur
        // inside it; the ordinary setters perform biome lookups under chunk guards.
        entity.world.store(destination.clone());
        entity.pos.store(position);
        entity.last_pos.store(position);
        entity.block_pos.store(block_pos);
        entity
            .chunk_pos
            .store(block_pos.chunk_and_chunk_relative_position().0);
        entity.bounding_box.store(bounds);
        entity.current_biome.store(biome);
        entity.last_biome_update_pos.store(block_pos);
        true
    }
}

fn owns_unpublished_session(world: &World, player: &Arc<Player>) -> bool {
    !player.client.closed()
        && !player.get_entity().is_removed()
        && std::ptr::eq(player.world().as_ref(), world)
}
