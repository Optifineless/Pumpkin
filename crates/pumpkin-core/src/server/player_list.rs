use super::Server;
use crate::{
    entity::player::Player,
    world::{
        World,
        guard_effects::{self, GuardEffects},
    },
};
use pumpkin_protocol::{
    ClientPacket, VarInt,
    bedrock::client::player_list::CPlayerList,
    java::client::play::{
        CPlayerInfoUpdate, CRemovePlayerInfo, InitChat, PlayerAction, PlayerInfoFlags,
    },
};
use std::sync::{Arc, atomic::Ordering};

impl World {
    pub(crate) fn add_pending_player(&self, player: &Arc<Player>) {
        self.players.rcu(|players| {
            let mut next = (**players).clone();
            next.push(player.clone());
            next
        });
    }
}

impl Server {
    pub(crate) fn can_track_player(&self, player: &Player) -> bool {
        self.admission_reservations.can_track(player)
    }

    pub(crate) fn broadcast_player_info<P: ClientPacket>(&self, subject: &Player, packet: &P) {
        let _effects = GuardEffects::new();
        self.admission_reservations
            .publish_info(subject, |players| {
                for player in players {
                    if let Some(java) = player.client.java()
                        && let Ok(bytes) = java.serialize_packet(packet)
                    {
                        guard_effects::send(player.client.clone(), bytes);
                    }
                }
            });
    }

    // PlayerList.placeNewPlayer initializes the connection before any level entity spawns.
    // Roster routing adapts JulesB40's upstream PR #3432; tracking remains world-local.
    pub(crate) fn initialize_player_list(&self, joining: &Arc<Player>) {
        let _effects = GuardEffects::new();
        self.admission_reservations.activate(joining, |online| {
            for subject in &online {
                if joining.client.java().is_none()
                    && subject.gameprofile.id == joining.gameprofile.id
                {
                    continue;
                }
                self.with_player_list_entry(subject, |java, bedrock| {
                    guard_effects::send_editioned(joining.client.clone(), java, bedrock);
                });
            }
            self.with_player_list_entry(joining, |java, bedrock| {
                for observer in online
                    .iter()
                    .filter(|p| p.gameprofile.id != joining.gameprofile.id)
                {
                    guard_effects::send_editioned(observer.client.clone(), java, bedrock);
                }
            });
            let admissions = self.admission_reservations.clone();
            let joining = joining.clone();
            let _ = guard_effects::defer(move || admissions.finish_initialization(&joining));
        });
    }

    fn with_player_list_entry(
        &self,
        player: &Player,
        send: impl FnOnce(&CPlayerInfoUpdate<'_>, &CPlayerList),
    ) {
        let profile = &player.gameprofile;
        let properties = profile.properties.load_full();
        let name = player.player_list_display_name();
        let chat = {
            let chat = player
                .chat_session
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (chat.session_id != uuid::Uuid::nil()
                && !chat.public_key.is_empty()
                && !chat.signature.is_empty())
            .then(|| InitChat {
                session_id: chat.session_id,
                expires_at: chat.expires_at,
                public_key: chat.public_key.clone(),
                signature: chat.signature.clone(),
            })
        };
        let mut flags = PlayerInfoFlags::all();
        let mut actions = vec![PlayerAction::AddPlayer {
            name: &profile.name,
            properties: &properties,
        }];
        if self.basic_config.allow_chat_reports {
            actions.push(PlayerAction::InitializeChat(chat));
        } else {
            flags.remove(PlayerInfoFlags::INITIALIZE_CHAT);
        }
        actions.extend([
            PlayerAction::UpdateGameMode(VarInt(player.gamemode.load() as i32)),
            PlayerAction::UpdateListed(player.tab_list_listed.load(Ordering::Relaxed)),
            PlayerAction::UpdateLatency(VarInt(player.tab_list_latency.load(Ordering::Relaxed))),
            PlayerAction::UpdateDisplayName(name.as_ref()),
            PlayerAction::UpdateListOrder(VarInt(player.tab_list_order.load(Ordering::Relaxed))),
            PlayerAction::UpdateHat(true),
        ]);
        let entries = [pumpkin_protocol::java::client::play::Player {
            uuid: profile.id,
            actions: &actions,
        }];
        send(
            &CPlayerInfoUpdate::new(flags.bits(), &entries),
            &player.bedrock_player_list(),
        );
    }

    pub(crate) fn remove_player_list_entry(&self, player: &Player) {
        let _effects = GuardEffects::new();
        self.admission_reservations.retire(player, |online| {
            let mut bedrock = player.bedrock_player_list();
            bedrock.action = CPlayerList::ACTION_REMOVE;
            let ids = [player.gameprofile.id];
            let java = CRemovePlayerInfo::new(&ids);
            for observer in online {
                guard_effects::send_editioned(observer.client.clone(), &java, &bedrock);
            }
        });
    }
}
