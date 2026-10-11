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
    pub(super) fn new(fight: &DragonFight) -> Self {
        Self {
            uuid: Uuid::new_v4(),
            origin: fight.origin,
            fight_origin: fight.exit_portal_location.unwrap_or(fight.origin),
        }
    }

    // EnderDragonFight.createNewDragon: callbacks precede publication and UUID assignment.
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
    AddBossbar(Arc<crate::entity::player::Player>, super::Bossbar),
    RemoveBossbar(Arc<crate::entity::player::Player>, Uuid),
    BossbarHealth(Arc<crate::entity::player::Player>, Uuid, f32),
    BossbarTitle(Arc<crate::entity::player::Player>, Uuid, TextComponent),
    WorldEvent(pumpkin_data::world::WorldEvent, BlockPos, i32),
    PlacePortal(BlockPos, bool),
    ClearPortal(BlockPos),
    SpawnCrystals,
    SpawnGateway(BlockPos),
    PlaceEgg(BlockPos),
    RemoveEntity(Arc<dyn EntityBase>),
    ReconcileState(bool),
    FinishDragonKill,
    TryRespawn,
    SpawnDragon(DragonSpawn),
    CompleteRespawn {
        spawn: DragonSpawn,
        crystals: Vec<Arc<dyn EntityBase>>,
        respawn_stage: DragonRespawnStage,
        respawn_crystals: Vec<Uuid>,
    },
    Respawn(RespawnAction),
}

impl FightAction {
    // EnderDragonFight world effects retain their order and tick, without the fight guard.
    pub(super) fn execute(self, world: &Arc<World>) {
        match self {
            Self::AddBossbar(player, bossbar) => player.send_bossbar(&bossbar),
            Self::RemoveBossbar(player, uuid) => player.remove_bossbar(uuid),
            Self::BossbarHealth(player, uuid, health) => {
                player.update_bossbar_health(&uuid, health);
            }
            Self::BossbarTitle(player, uuid, title) => player.update_bossbar_title(&uuid, title),
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
            Self::ReconcileState(active_portal_exists) => {
                if let Some(fight_mutex) = &world.dragon_fight {
                    DragonFight::with_fight(fight_mutex, world, |fight| {
                        fight.reconcile_state(world, active_portal_exists);
                    });
                }
            }
            Self::FinishDragonKill => {
                // EnderDragonFight.setDragonKilled sets these after portal, gateway and egg placement.
                if let Some(fight_mutex) = &world.dragon_fight {
                    DragonFight::with_fight(fight_mutex, world, |fight| {
                        fight.previously_killed = true;
                        fight.dragon_killed = true;
                        fight.dragon_uuid = None;
                    });
                }
            }
            Self::TryRespawn => {
                if let Some(fight_mutex) = &world.dragon_fight {
                    DragonFight::with_fight(fight_mutex, world, |fight| {
                        fight.try_respawn(world);
                    });
                }
            }
            Self::SpawnDragon(spawn) => {
                if spawn.spawn(world, || true)
                    && let Some(fight_mutex) = &world.dragon_fight
                {
                    DragonFight::with_fight(fight_mutex, world, |fight| {
                        fight.dragon_uuid = Some(spawn.uuid);
                        fight.dragon_killed = false;
                    });
                }
            }
            Self::CompleteRespawn {
                spawn,
                crystals,
                respawn_stage,
                respawn_crystals,
            } => {
                Self::complete_respawn(world, &spawn, crystals, respawn_stage, &respawn_crystals);
            }
            Self::Respawn(action) => action.execute(world),
        }
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
        if !DragonFight::with_fight(fight_mutex, world, |fight| {
            Self::can_complete(fight, world, &crystals, respawn_stage, respawn_crystals)
        }) {
            return;
        }
        // Keep the sequence active through EntitySpawnEvent. Reconcile again before publication.
        if !spawn.spawn(world, || {
            DragonFight::with_fight(fight_mutex, world, |fight| {
                if !Self::can_complete(fight, world, &crystals, respawn_stage, respawn_crystals) {
                    return false;
                }
                #[cfg(test)]
                super::removal_lock_tests::observe_completion();
                fight.respawn_stage = None;
                fight.respawn_time = 0;
                fight.dragon_killed = false;
                fight.dragon_uuid = Some(spawn.uuid);
                true
            })
        }) {
            return;
        }
        DragonFight::with_fight(fight_mutex, world, |fight| {
            fight.reset_spike_crystals(world);
        });
        RespawnAction::DestroyRespawnCrystals(crystals).execute(world);
    }
}
