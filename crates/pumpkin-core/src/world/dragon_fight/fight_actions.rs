use super::{DRAGON_SPAWN_Y, DragonFight, DragonRespawnStage, EntityBase, RespawnAction, World};
use pumpkin_data::{Block, entity::EntityType};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use pumpkin_util::text::TextComponent;
use pumpkin_world::world::BlockFlags;
use std::sync::Arc;
use uuid::Uuid;

pub(super) struct DragonSpawn {
    pub(super) uuid: Uuid,
    origin: BlockPos,
    fight_origin: BlockPos,
}

impl DragonSpawn {
    // EnderDragonFight.createNewDragon is synchronous; reserve its deferred equivalent.
    pub(super) fn reserve(fight: &mut DragonFight) -> Option<Self> {
        if fight.pending_dragon_spawn.is_some() {
            return None;
        }
        let spawn = Self {
            uuid: Uuid::new_v4(),
            origin: fight.origin,
            fight_origin: fight.exit_portal_location.unwrap_or(fight.origin),
        };
        fight.pending_dragon_spawn = Some(spawn.uuid);
        Some(spawn)
    }

    // EntitySpawnEvent precedes validation; tracked identity precedes admission packets.
    fn spawn(&self, world: &Arc<World>, validate: impl FnOnce() -> bool) -> bool {
        let position = Vector3::new(
            self.origin.0.x as f64 + 0.5,
            (self.origin.0.y + DRAGON_SPAWN_Y) as f64,
            self.origin.0.z as f64 + 0.5,
        );
        let dragon =
            crate::entity::r#type::from_type(&EntityType::ENDER_DRAGON, position, world, self.uuid);
        if let Some(dragon_entity) = dragon
            .cast_any()
            .downcast_ref::<crate::entity::boss::ender_dragon::EnderDragonEntity>(
        ) {
            dragon_entity.set_fight_origin(self.fight_origin);
        }
        let spawned = world.admit_spawn_tree(&dragon, true, validate);
        if spawned {
            tracing::info!("Spawned ender dragon {:?}.", self.uuid);
        }
        spawned
    }
}

pub(super) enum FightAction {
    AddBossbar(Arc<super::BossbarRecipient>, super::Bossbar),
    RemoveBossbar(Arc<super::BossbarRecipient>, Uuid),
    BossbarHealth(Arc<super::BossbarRecipient>, Uuid, f32),
    BossbarDeathHealth(Arc<super::BossbarRecipient>, Uuid),
    BossbarTitle(Arc<super::BossbarRecipient>, Uuid, TextComponent),
    WorldEvent(pumpkin_data::world::WorldEvent, BlockPos, i32),
    PlacePortal(BlockPos, bool),
    ClearPortal(BlockPos),
    SpawnCrystals,
    SpawnGateway(BlockPos),
    PlaceEgg(BlockPos),
    RemoveEntity(Arc<dyn EntityBase>),
    TryRespawn,
    SpawnDragon(DragonSpawn),
    CompleteRespawn {
        spawn: DragonSpawn,
        crystals: Vec<Arc<dyn EntityBase>>,
        respawn_stage: DragonRespawnStage,
        respawn_crystals: Vec<Uuid>,
    },
    Respawn {
        action: RespawnAction,
        generation: u64,
    },
}

impl FightAction {
    // EnderDragonFight world effects retain their order and tick, without the fight guard.
    pub(super) fn execute(self, world: &Arc<World>) {
        match self {
            Self::AddBossbar(recipient, bossbar) => {
                DragonFight::send_bossbar(world, &recipient, bossbar.uuid, |player| {
                    if recipient.publish() {
                        player.send_bossbar(&bossbar);
                    }
                });
            }
            Self::RemoveBossbar(recipient, uuid) => {
                if recipient.retire()
                    && let Some(player) = recipient.player()
                {
                    player.remove_bossbar(uuid);
                }
            }
            Self::BossbarHealth(recipient, uuid, health) => {
                DragonFight::send_bossbar(world, &recipient, uuid, |player| {
                    if recipient.is_published() {
                        player.update_bossbar_health(&uuid, health);
                    }
                });
            }
            Self::BossbarDeathHealth(recipient, uuid) => {
                DragonFight::send_bossbar_death_health(world, &recipient, uuid);
            }
            Self::BossbarTitle(recipient, uuid, title) => {
                DragonFight::send_bossbar(world, &recipient, uuid, |player| {
                    if recipient.is_published() {
                        player.update_bossbar_title(&uuid, title);
                    }
                });
            }
            Self::WorldEvent(event, pos, data) => world.sync_world_event(event, pos, data),
            Self::PlacePortal(pos, active) => super::super::end_podium::place(world, pos, active),
            Self::ClearPortal(pos) => Self::clear_portal(world, pos),
            Self::SpawnCrystals => DragonFight::spawn_initial_crystals(world),
            Self::SpawnGateway(pos) => DragonFight::spawn_new_gateway_at(world, pos),
            Self::PlaceEgg(pos) => {
                world.set_block_state(
                    &pos,
                    Block::DRAGON_EGG.default_state.id,
                    BlockFlags::NOTIFY_LISTENERS,
                );
            }
            Self::RemoveEntity(entity) => entity.get_entity().remove(),
            Self::TryRespawn => {
                if let Some(fight_mutex) = &world.dragon_fight {
                    DragonFight::with_fight(fight_mutex, world, |fight| {
                        fight.try_respawn(world);
                    });
                }
            }
            Self::SpawnDragon(spawn) => Self::spawn_dragon(world, &spawn),
            Self::CompleteRespawn {
                spawn,
                crystals,
                respawn_stage,
                respawn_crystals,
            } => {
                Self::complete_respawn(world, &spawn, crystals, respawn_stage, &respawn_crystals);
            }
            Self::Respawn { action, generation } => Self::respawn(world, action, generation),
        }
    }

    // DragonRespawnStage.SUMMONING_PILLARS runs synchronously in vanilla. A reentrant
    // tick may defer its effect past onCrystalDestroyed or a new respawn sequence here.
    fn respawn(world: &Arc<World>, action: RespawnAction, generation: u64) {
        // Every world effect releases state ownership before native callbacks can run.
        action.execute(world, || Self::respawn_is_current(world, generation));
    }

    fn respawn_is_current(world: &Arc<World>, generation: u64) -> bool {
        let Some(mutex) = &world.dragon_fight else {
            return false;
        };
        DragonFight::with_fight(mutex, world, |fight| {
            // Advancing animation stages preserves earlier work in the same active sequence.
            if fight.respawn_generation != generation || fight.respawn_stage.is_none() {
                return false;
            }
            // EndCrystal.hurtServer marks removal before notifying the fight. The existing
            // notification guard serializes this check with that mark, as at completion.
            let entities = world.entities.load();
            if fight.respawn_crystals.iter().any(|uuid| {
                !entities.iter().any(|entity| {
                    entity.get_entity().entity_uuid == *uuid && !entity.get_entity().is_removed()
                })
            }) {
                fight.abort_respawn_sequence(world);
                return false;
            }
            true
        })
    }

    fn spawn_dragon(world: &Arc<World>, spawn: &DragonSpawn) {
        let Some(fight_mutex) = &world.dragon_fight else {
            return;
        };
        if !DragonFight::with_fight(fight_mutex, world, |fight| {
            fight.pending_dragon_spawn == Some(spawn.uuid)
        }) {
            return;
        }
        spawn.spawn(world, || {
            DragonFight::with_fight(fight_mutex, world, |fight| {
                if fight.pending_dragon_spawn != Some(spawn.uuid) {
                    return false;
                }
                // Admit the tracked identity after spawn events, before world/packet callbacks.
                // Keep the reservation until publication finishes or that dragon dies.
                fight.dragon_uuid = Some(spawn.uuid);
                fight.dragon_killed = false;
                true
            })
        });
        DragonFight::with_fight(fight_mutex, world, |fight| {
            if fight.pending_dragon_spawn == Some(spawn.uuid) {
                fight.pending_dragon_spawn = None;
                // createNewDragon records failed/cancelled attempts for recovery as well.
                // A publication death clears this reservation and keeps its completed state.
                fight.dragon_uuid = Some(spawn.uuid);
                fight.dragon_killed = false;
            }
        });
    }

    // EnderDragonFight.respawnDragon clears the old podium before placing the inactive one.
    fn clear_portal(world: &Arc<World>, loc: BlockPos) {
        for dy in -1i32..=4 {
            for dx in -3i32..=3 {
                for dz in -3i32..=3 {
                    let p = BlockPos::new(loc.0.x + dx, loc.0.y + dy, loc.0.z + dz);
                    let b = world.get_block(&p);
                    if b == &Block::BEDROCK || b == &Block::END_PORTAL {
                        world.set_block_state(
                            &p,
                            Block::END_STONE.default_state.id,
                            BlockFlags::NOTIFY_ALL,
                        );
                    }
                }
            }
        }
    }

    fn can_complete(
        fight: &mut DragonFight,
        world: &Arc<World>,
        crystals: &[Arc<dyn EntityBase>],
        respawn_stage: DragonRespawnStage,
        respawn_crystals: &[Uuid],
    ) -> bool {
        let same_sequence = fight.respawn_stage == Some(respawn_stage)
            && fight.respawn_crystals == respawn_crystals;
        if !same_sequence {
            return false;
        }
        // EndCrystal.hurtServer removes before notifying; catch a notification still in flight.
        if crystals.len() != respawn_crystals.len()
            || crystals
                .iter()
                .any(|crystal| crystal.get_entity().is_removed())
        {
            fight.abort_respawn_sequence(world);
            return false;
        }
        true
    }

    // DragonRespawnStage.SUMMONING_DRAGON and EnderDragonFight.onCrystalDestroyed.
    fn complete_respawn(
        world: &Arc<World>,
        spawn: &DragonSpawn,
        crystals: Vec<Arc<dyn EntityBase>>,
        respawn_stage: DragonRespawnStage,
        respawn_crystals: &[Uuid],
    ) {
        let Some(fight_mutex) = &world.dragon_fight else {
            return;
        };
        let ready = DragonFight::with_fight(fight_mutex, world, |fight| {
            fight.pending_dragon_spawn == Some(spawn.uuid)
                && Self::can_complete(fight, world, &crystals, respawn_stage, respawn_crystals)
        });
        let mut completed = false;
        if ready {
            // Keep the sequence active through EntitySpawnEvent; validate before publication.
            spawn.spawn(world, || {
                completed = DragonFight::with_fight(fight_mutex, world, |fight| {
                    Self::commit_respawn(
                        fight,
                        world,
                        spawn,
                        &crystals,
                        respawn_stage,
                        respawn_crystals,
                    )
                });
                completed
            });
        }
        let finished = DragonFight::with_fight(fight_mutex, world, |fight| {
            // DragonRespawnStage.SUMMONING_DRAGON.tick consumes crystals after publication callbacks.
            if fight.pending_dragon_spawn != Some(spawn.uuid) && !completed {
                return false;
            }
            // setRespawnStage(END) and SUMMONING_DRAGON finish even if addFreshEntity fails.
            if ready && !completed {
                completed = Self::commit_respawn(
                    fight,
                    world,
                    spawn,
                    &crystals,
                    respawn_stage,
                    respawn_crystals,
                );
            }
            if fight.pending_dragon_spawn == Some(spawn.uuid) {
                fight.pending_dragon_spawn = None;
            }
            if completed {
                fight.reset_spike_crystals(world);
            }
            completed
        });
        if finished {
            RespawnAction::DestroyRespawnCrystals(crystals).execute(world, || true);
        }
    }

    fn commit_respawn(
        fight: &mut DragonFight,
        world: &Arc<World>,
        spawn: &DragonSpawn,
        crystals: &[Arc<dyn EntityBase>],
        respawn_stage: DragonRespawnStage,
        respawn_crystals: &[Uuid],
    ) -> bool {
        if fight.pending_dragon_spawn != Some(spawn.uuid)
            || !Self::can_complete(fight, world, crystals, respawn_stage, respawn_crystals)
        {
            return false;
        }
        #[cfg(test)]
        super::removal_lock_tests::observe_completion();
        fight.respawn_stage = None;
        fight.respawn_time = 0;
        fight.dragon_killed = false;
        fight.dragon_uuid = Some(spawn.uuid);
        // ServerBossEvent.setVisible re-adds existing recipients after a hidden respawn.
        // Recompute current arena recipients and queue packets outside both state guards.
        fight.bossbar_players.clear();
        fight.update_players(world);
        true
    }
}
