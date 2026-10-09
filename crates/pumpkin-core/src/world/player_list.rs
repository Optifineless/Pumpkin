use super::{
    BuildPlatform, CPlayerList, CRemoveActor, CRemoveEntities, CRemovePlayerInfo, EntityBase,
    Player, PlayerListEntry, Skin, VarLong, World,
};
use std::sync::Arc;

impl World {
    // PlayerList.respawn / ServerLevel.removePlayerImmediately always detach tracking.
    pub(super) fn detach_player(
        &self,
        player: &Arc<Player>,
        disconnect: bool,
    ) -> (Option<Arc<Self>>, Option<Arc<Player>>) {
        let _owner = player.living_entity.own_damage();
        // PlayerList.remove reads the current level before removePlayerImmediately.
        let current_world = disconnect.then(|| player.world());
        let world = current_world.as_deref().unwrap_or(self);
        let first_disconnect = disconnect && player.living_entity.abort_respawn();
        let mut removed_player = None;
        world.players.rcu(|current_list| {
            let mut new_list = (**current_list).clone();
            // Find the player before we filter them out
            let pos = new_list
                .iter()
                .position(|p| p.gameprofile.id == player.gameprofile.id);
            removed_player = pos.map(|pos| new_list.remove(pos));
            new_list
        });
        // begin_respawn hides tick/pickup membership, but leaves tracking as the teardown claim.
        if removed_player.is_none()
            && player.living_entity.is_respawning()
            && world.entity_tracker.has_entity_with_id(player.entity_id())
        {
            removed_player = Some(player.clone());
        }
        if let Some(ref player) = removed_player {
            world
                .entity_tracker
                .remove_entity(player.as_ref() as &dyn EntityBase, world);
            let uuid = player.gameprofile.id;
            let entity_id = player.entity_id();

            let bedrock_remove_player = CPlayerList {
                action: CPlayerList::ACTION_REMOVE,
                entries: vec![PlayerListEntry {
                    uuid,
                    entity_unique_id: VarLong(entity_id as i64),
                    username: player.gameprofile.name.clone(),
                    xuid: String::new(),
                    platform_chat_id: String::new(),
                    build_platform: BuildPlatform::Unknown,
                    skin: Skin::steve(),
                    is_teacher: false,
                    is_host: false,
                    is_sub_client: false,
                    player_color: [0, 0, 0, 0],
                }],
            };

            world.broadcast_editioned(&CRemovePlayerInfo::new(&[uuid]), &bedrock_remove_player);

            world.broadcast_editioned(
                &CRemoveEntities::new(&[entity_id.into()]),
                &CRemoveActor::new(VarLong(entity_id as i64)),
            );
        }
        // PlayerList.remove still announces disconnect after the source level was detached.
        let removed_player = removed_player.or_else(|| {
            (first_disconnect && player.living_entity.is_respawning()).then(|| player.clone())
        });
        (current_world, removed_player)
    }
}
