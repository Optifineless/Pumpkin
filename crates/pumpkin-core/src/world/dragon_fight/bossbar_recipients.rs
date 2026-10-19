use super::{ARENA_RADIUS, BossbarRecipient, DRAGON_SPAWN_Y, DragonFight, FightAction, World};
use crate::entity::player::Player;
use pumpkin_util::text::TextComponent;
use std::sync::Arc;
use uuid::Uuid;

impl DragonFight {
    // ServerBossEvent.broadcast retains the exact session for deferred delivery.
    pub(super) fn update_bossbar_health(&mut self, _world: &Arc<World>, health: f32) {
        for recipient in &self.bossbar_players {
            if self.dragon_killed {
                self.actions.push(FightAction::BossbarDeathHealth(
                    recipient.clone(),
                    self.bossbar_uuid,
                ));
                continue;
            }
            self.actions.push(FightAction::BossbarHealth(
                recipient.clone(),
                self.bossbar_uuid,
                health,
            ));
        }
    }

    // Preserve setDragonKilled's final zero progress before REMOVE for a still-present viewer.
    // This token is retained by that terminal batch after active membership has retired.
    pub(super) fn send_bossbar_death_health(
        world: &Arc<World>,
        recipient: &Arc<BossbarRecipient>,
        uuid: Uuid,
    ) {
        let Some(player) = recipient.player() else {
            return;
        };
        let Some(mutex) = &world.dragon_fight else {
            return;
        };
        let current = Self::with_fight(mutex, world, |fight| {
            fight.bossbar_uuid == uuid
                && fight.dragon_killed
                && recipient.is_published()
                && BossbarRecipient::is_current(&player, world)
        });
        if current {
            player.update_bossbar_health(&uuid, 0.0);
        }
    }

    pub(super) fn update_bossbar_title(&mut self, _world: &Arc<World>, title: &TextComponent) {
        for recipient in &self.bossbar_players {
            self.actions.push(FightAction::BossbarTitle(
                recipient.clone(),
                self.bossbar_uuid,
                title.clone(),
            ));
        }
    }

    // ServerBossEvent.removeAllPlayers sends through retained recipients, even after detach.
    pub(super) fn remove_all_bossbar(&mut self, _world: &Arc<World>) {
        for recipient in self.bossbar_players.drain(..) {
            self.actions
                .push(FightAction::RemoveBossbar(recipient, self.bossbar_uuid));
        }
    }

    pub(super) fn update_players(&mut self, world: &Arc<World>) {
        let current: Vec<_> = world
            .players
            .load()
            .iter()
            .filter(|player| {
                let pos = player.living_entity.entity.pos.load();
                let dx = pos.x - self.origin.0.x as f64;
                let dy = pos.y - (self.origin.0.y as f64 + DRAGON_SPAWN_Y as f64);
                let dz = pos.z - self.origin.0.z as f64;
                BossbarRecipient::is_current(player, world)
                    && dx * dx + dy * dy + dz * dz < ARENA_RADIUS * ARENA_RADIUS
            })
            .cloned()
            .collect();

        for player in &current {
            if !self.bossbar_players.iter().any(|r| r.matches(player)) {
                let recipient = BossbarRecipient::new(player);
                if !self.dragon_killed {
                    self.actions.push(FightAction::AddBossbar(
                        recipient.clone(),
                        self.make_bossbar(),
                    ));
                }
                self.bossbar_players.push(recipient);
            }
        }
        self.bossbar_players.retain(|recipient| {
            if current.iter().any(|player| recipient.matches(player)) {
                return true;
            }
            self.actions.push(FightAction::RemoveBossbar(
                recipient.clone(),
                self.bossbar_uuid,
            ));
            false
        });
    }

    fn bossbar_recipient(
        world: &Arc<World>,
        recipient: &Arc<BossbarRecipient>,
        uuid: Uuid,
    ) -> Option<Arc<Player>> {
        let mutex = world.dragon_fight.as_ref()?;
        Self::with_fight(mutex, world, |fight| {
            let index = fight
                .bossbar_players
                .iter()
                .position(|r| Arc::ptr_eq(r, recipient))?;
            let player = recipient.player();
            if fight.bossbar_uuid == uuid
                && !fight.dragon_killed
                && player
                    .as_ref()
                    .is_some_and(|p| BossbarRecipient::is_current(p, world))
            {
                return player;
            }
            fight.bossbar_players.remove(index);
            fight
                .actions
                .push(FightAction::RemoveBossbar(recipient.clone(), uuid));
            None
        })
    }

    pub(super) fn send_bossbar(
        world: &Arc<World>,
        recipient: &Arc<BossbarRecipient>,
        uuid: Uuid,
        send: impl FnOnce(&Player),
    ) {
        if let Some(player) = Self::bossbar_recipient(world, recipient, uuid) {
            send(&player);
            // Packet plugins may detach this session; queue compensation after publication.
            Self::bossbar_recipient(world, recipient, uuid);
        }
    }
}
