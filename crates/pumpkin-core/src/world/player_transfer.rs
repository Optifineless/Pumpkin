use std::sync::Arc;

use super::{
    BuildPlatform, CPlayerList, CRemoveActor, CRemoveEntities, CRemovePlayerInfo, PlayerListEntry,
    Skin, VarLong, World,
};
use crate::entity::{EntityBase, player::Player};

impl World {
    /// ServerLevel.removePlayerImmediately, without a leave callback during transfer.
    pub(crate) fn detach_transferring_player(&self, player: &Arc<Player>) -> Option<Arc<Player>> {
        let mut removed_player: Option<Arc<Player>> = None;

        self.players.rcu(|current_list| {
            removed_player = None;
            let mut new_list = (**current_list).clone();
            // Find the player before we filter them out
            let pos = new_list.iter().position(|p| Arc::ptr_eq(p, player));
            if let Some(pos) = pos {
                removed_player = Some(new_list.remove(pos));
            }
            new_list
        });
        if let Some(ref player) = removed_player {
            self.entity_tracker
                .remove_entity(player.as_ref() as &dyn EntityBase, self);
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

            self.broadcast_editioned(&CRemovePlayerInfo::new(&[uuid]), &bedrock_remove_player);

            self.broadcast_editioned(
                &CRemoveEntities::new(&[entity_id.into()]),
                &CRemoveActor::new(VarLong(entity_id as i64)),
            );
        }
        removed_player
    }
}
