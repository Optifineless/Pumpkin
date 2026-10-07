use std::f32::consts::{PI, TAU};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use crossbeam::atomic::AtomicCell;
use pumpkin_data::attributes::Attributes;
use pumpkin_data::effect::StatusEffect;
use pumpkin_data::entity::EntityStatus;
use pumpkin_data::particle::Particle;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use rand::RngExt;

use crate::entity::{
    Entity, EntityBase,
    ai::{goal::Goal, util::goal_utils},
    living::LivingEntity,
    mob::{Mob, MobEntity},
};

/// Represents a Squid, a passive aquatic mob that swims by pulsing its tentacles.
///
/// Wiki: <https://minecraft.wiki/w/Squid>
pub struct SquidEntity {
    pub mob_entity: MobEntity,
    pub movement: Arc<SquidMovement>,
}

impl SquidEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        let movement = SquidMovement::init(&mob_entity);
        Arc::new(Self {
            mob_entity,
            movement,
        })
    }
}

impl Mob for SquidEntity {
    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn post_tick(&self) {
        self.movement.tick(self);
    }

    fn custom_travel(&self, caller: &dyn EntityBase) -> bool {
        SquidMovement::travel(self, caller)
    }

    fn mob_is_pushed_by_fluids(&self) -> bool {
        false
    }
}

/// Squid.aiStep applies the heading chosen by its goals once per tentacle stroke.
pub struct SquidMovement {
    movement_vector: AtomicCell<Vector3<f64>>,
    tentacle_movement: AtomicCell<f32>,
    tentacle_speed: AtomicCell<f32>,
}

impl Default for SquidMovement {
    fn default() -> Self {
        Self::new()
    }
}

impl SquidMovement {
    #[must_use]
    pub fn new() -> Self {
        Self {
            movement_vector: AtomicCell::new(Vector3::default()),
            tentacle_movement: AtomicCell::new(0.0),
            tentacle_speed: AtomicCell::new(random_tentacle_speed(&mut rand::rng())),
        }
    }

    /// Vanilla Squid.registerGoals; neither goal uses path navigation.
    pub fn init(mob_entity: &MobEntity) -> Arc<Self> {
        let movement = Arc::new(Self::new());
        mob_entity.add_goal(
            0,
            SquidRandomMovementGoal {
                movement: movement.clone(),
            },
        );
        mob_entity.add_goal(
            1,
            SquidFleeGoal {
                movement: movement.clone(),
                flee_ticks: 0,
            },
        );
        movement
    }

    /// Vanilla Squid.aiStep updates motion after super.aiStep has travelled.
    pub fn tick(&self, mob: &dyn Mob) {
        let living = &mob.get_mob_entity().living_entity;
        let entity = &living.entity;
        let in_water = entity.touching_water.load(Ordering::Relaxed);
        let mut rng = mob.get_random();

        let mut tentacle = self.tentacle_movement.load() + self.tentacle_speed.load();
        if tentacle > TAU {
            tentacle -= TAU;
            if rng.random_range(0..10) == 0 {
                self.tentacle_speed.store(random_tentacle_speed(&mut rng));
            }
            // Keeps the client's stroke animation in step with the shoves.
            entity
                .world
                .load()
                .send_entity_status(entity, EntityStatus::SquidAnimSynch, None);
        }
        self.tentacle_movement.store(tentacle);

        if in_water {
            // Plain stores throughout: the tracker syncs motion, and set_velocity
            // would send a packet every tick.
            if tentacle < PI {
                if tentacle / PI > 0.75 {
                    entity.velocity.store(self.movement_vector.load());
                }
            } else {
                entity.velocity.store(entity.velocity.load() * 0.9);
            }
            // The body swings round to face where the squid is going.
            let velocity = entity.velocity.load();
            let heading = -(velocity.x.atan2(velocity.z) as f32).to_degrees();
            let body_yaw = entity.body_yaw.load();
            let body_yaw = body_yaw + (heading - body_yaw) * 0.1;
            entity.body_yaw.store(body_yaw);
            entity.yaw.store(body_yaw);
        } else {
            let fall = living.get_effect(&StatusEffect::LEVITATION).map_or_else(
                || {
                    let gravity = if entity.has_no_gravity() {
                        0.0
                    } else {
                        mob.get_gravity()
                    };
                    entity.velocity.load().y - gravity
                },
                |levitation| 0.05 * (f64::from(levitation.amplifier) + 1.0),
            );
            // LivingEntity.getAirDrag / computeModifiedFriction, for a non-flying mob.
            let modifier = living.get_attribute_value(&Attributes::AIR_DRAG_MODIFIER) as f32;
            let drag = (1.0 - (1.0 - 0.98f32) * modifier).clamp(0.0, 1.0);
            entity
                .velocity
                .store(Vector3::new(0.0, fall * f64::from(drag), 0.0));
        }
    }

    /// Vanilla's `Squid.travel`: a squid only ever moves by the velocity `tick`
    /// gave it.
    pub fn travel(mob: &dyn Mob, caller: &dyn EntityBase) -> bool {
        let entity = mob.get_entity();
        entity.move_entity(caller, entity.velocity.load());
        true
    }
}

struct SquidRandomMovementGoal {
    movement: Arc<SquidMovement>,
}

impl Goal for SquidRandomMovementGoal {
    fn can_start(&mut self, _mob: &dyn Mob) -> bool {
        true
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let mut rng = mob.get_random();
        // TODO: SquidRandomMovementGoal also stops at noActionTime > 100;
        // Pumpkin does not yet keep that despawn counter.
        if rng.random_range(0..self.get_tick_count(50)) == 0
            || !mob.get_entity().touching_water.load(Ordering::Relaxed)
            || self.movement.movement_vector.load().length_squared() <= f64::from(1.0e-5f32)
        {
            let angle = rng.random::<f32>() * TAU;
            self.movement.movement_vector.store(Vector3::new(
                f64::from(angle.cos() * 0.2),
                f64::from(-0.1 + rng.random::<f32>() * 0.2),
                f64::from(angle.sin() * 0.2),
            ));
        }
    }
}

struct SquidFleeGoal {
    movement: Arc<SquidMovement>,
    flee_ticks: u32,
}

const SQUID_FLEE_SPEED: f64 = 3.0;
const SQUID_FLEE_MIN_DISTANCE: f64 = 5.0;
const SQUID_FLEE_MAX_DISTANCE: f64 = 10.0;

impl SquidFleeGoal {
    fn attacker(living: &LivingEntity) -> Option<Arc<dyn EntityBase>> {
        let entity = &living.entity;
        if !entity.touching_water.load(Ordering::Relaxed) {
            return None;
        }
        let attacker_id = living.last_attacker_id.load(Ordering::Relaxed);
        // LivingEntity.baseTick forgets a dead attacker or a hit older than 100 ticks.
        let since_hit =
            entity.age.load(Ordering::Relaxed) - living.last_attacked_time.load(Ordering::Relaxed);
        if attacker_id == 0 || since_hit > 100 {
            return None;
        }
        let attacker = entity.world.load().get_entity_by_id(attacker_id)?;
        let attacker_living = attacker.get_living_entity()?;
        if attacker_living.health.load() <= 0.0
            || (entity.pos.load() - attacker.get_entity().pos.load()).length_squared()
                >= SQUID_FLEE_MAX_DISTANCE * SQUID_FLEE_MAX_DISTANCE
        {
            return None;
        }
        Some(attacker)
    }
}

impl Goal for SquidFleeGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        Self::attacker(&mob.get_mob_entity().living_entity).is_some()
    }

    fn start(&mut self, _mob: &dyn Mob) {
        self.flee_ticks = 0;
    }

    fn should_run_every_tick(&self) -> bool {
        true
    }

    fn tick(&mut self, mob: &dyn Mob) {
        self.flee_ticks += 1;
        let Some(attacker) = Self::attacker(&mob.get_mob_entity().living_entity) else {
            return;
        };
        let entity = mob.get_entity();
        let world = entity.world.load();
        let pos = entity.pos.load();
        let mut away = pos - attacker.get_entity().pos.load();

        let probe = BlockPos::floored(pos.x + away.x, pos.y + away.y, pos.z + away.z);
        let open_air = world.get_block_state(&probe).is_air();
        if goal_utils::is_water(&world, &probe) || open_air {
            let length = away.length();
            if length > 0.0 {
                // Vanilla normalizes a copy of the offset here and discards it,
                // so the scale applies to the raw offset.
                let mut scale = SQUID_FLEE_SPEED;
                if length > SQUID_FLEE_MIN_DISTANCE {
                    scale -= (length - SQUID_FLEE_MIN_DISTANCE) / SQUID_FLEE_MIN_DISTANCE;
                }
                if scale > 0.0 {
                    away = away * scale;
                }
            }
            if open_air {
                away.y = 0.0;
            }
            self.movement.movement_vector.store(away / 20.0);
        }
        if self.flee_ticks % 10 == 5 {
            world.spawn_particle(pos, Vector3::default(), 0.0, 1, Particle::Bubble);
        }
    }
}

fn random_tentacle_speed(rng: &mut impl RngExt) -> f32 {
    1.0 / (rng.random::<f32>() + 1.0) * 0.2
}
