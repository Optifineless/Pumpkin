use std::sync::atomic::AtomicBool;

use pumpkin_util::math::vector3::Vector3;

use crate::{
    entity::{
        Entity, EntityBase,
        projectile::{ProjectileHit, ThrownItemEntity, fireball::INITIAL_ACCELERATION_POWER},
    },
    server::Server,
};

const GRAVITY: f64 = 0.0;

pub struct SmallFireballEntity {
    pub thrown: ThrownItemEntity,
}

impl SmallFireballEntity {
    #[must_use]
    pub const fn new(entity: Entity) -> Self {
        let thrown = ThrownItemEntity {
            entity,
            projectile: crate::entity::projectile::ownership::ProjectileState::new(None),
            has_hit: AtomicBool::new(false),
            gravity: GRAVITY,
        };

        Self { thrown }
    }

    #[must_use]
    pub fn new_shot(entity: Entity, shooter: &Entity, direction: Vector3<f64>) -> Self {
        let thrown = ThrownItemEntity::new(entity, shooter, GRAVITY);
        let accel = INITIAL_ACCELERATION_POWER;
        thrown
            .entity
            .velocity
            .store(direction.normalize().multiply(accel, accel, accel));
        Self { thrown }
    }
}

impl EntityBase for SmallFireballEntity {
    fn projectile_state(&self) -> Option<&super::ownership::ProjectileState> {
        Some(&self.thrown.projectile)
    }

    fn tick(&self, caller: &dyn EntityBase, server: &Server) {
        self.thrown.process_tick(caller, server);
    }

    fn get_entity(&self) -> &Entity {
        self.thrown.get_entity()
    }

    fn get_living_entity(&self) -> Option<&crate::entity::living::LivingEntity> {
        None
    }
    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }

    fn on_hit(&self, hit: ProjectileHit) {
        match hit {
            ProjectileHit::Entity { ref entity, .. } => {
                let old_fire = entity
                    .get_entity()
                    .fire_ticks
                    .load(std::sync::atomic::Ordering::Relaxed);
                entity.get_entity().set_on_fire_for(5.0);
                let owner = self.projectile_owner();
                // SmallFireball.onHitEntity / DamageSources.fireball.
                let damage_type = if owner.is_some() {
                    pumpkin_data::damage::DamageType::FIREBALL
                } else {
                    pumpkin_data::damage::DamageType::UNATTRIBUTED_FIREBALL
                };
                let damaged = super::damage::hurt_entity(
                    entity.as_ref(),
                    5.0,
                    damage_type,
                    self,
                    owner.as_deref().or(Some(self)),
                );
                if damaged {
                    super::damage::post_attack(
                        entity.as_ref(),
                        damage_type,
                        self,
                        owner.as_deref().or(Some(self)),
                    );
                } else {
                    entity
                        .get_entity()
                        .fire_ticks
                        .store(old_fire, std::sync::atomic::Ordering::Relaxed);
                }
            }
            ProjectileHit::Block { pos, face, .. } => {
                // Try to place fire
                let block_to_place = match face {
                    pumpkin_data::BlockDirection::Up => pos.up(),
                    pumpkin_data::BlockDirection::Down => pos.down(),
                    pumpkin_data::BlockDirection::North => pos.north(),
                    pumpkin_data::BlockDirection::South => pos.south(),
                    pumpkin_data::BlockDirection::West => pos.west(),
                    pumpkin_data::BlockDirection::East => pos.east(),
                };
                let world = self.get_entity().world.load();
                // SmallFireball.onHitBlock only ignites empty space, respecting a mob owner's griefing rule.
                let owner = self.projectile_owner();
                if owner.as_ref().is_some_and(|owner| {
                    owner.get_living_entity().is_some() && owner.get_player().is_none()
                }) && !world.level_info.load().game_rules.mob_griefing
                    || !world.get_block_state(&block_to_place).is_air()
                {
                    return;
                }
                let block = crate::block::blocks::fire::FireBlockBase::get_fire_type(
                    &world,
                    &block_to_place,
                );
                let fire_state = if block == pumpkin_data::Block::FIRE {
                    crate::block::blocks::fire::fire::FireBlock.get_state_for_position(
                        &world,
                        &block,
                        &block_to_place,
                    )
                } else {
                    block.default_state.id
                };
                world.set_block_state(
                    &block_to_place,
                    fire_state,
                    pumpkin_world::world::BlockFlags::NOTIFY_ALL,
                );
            }
        }
    }
}
