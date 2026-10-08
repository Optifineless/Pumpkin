use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};

use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::codec::item_stack_seralizer::ItemStackSerializer;
use pumpkin_util::math::atomic_f32::AtomicF32;
use pumpkin_util::math::vector3::Vector3;

use crate::{
    entity::{
        Entity, EntityBase,
        projectile::{ProjectileHit, ThrownItemEntity},
    },
    server::Server,
};

pub const MIN_CAMERA_DISTANCE_SQUARED: f32 = 12.25;
pub const INITIAL_ACCELERATION_POWER: f64 = 0.1;
pub const DEFLECTION_SCALE: f64 = 0.5;
pub const DEFAULT_EXPLOSION_POWER: f32 = 1.0;
pub const AIR_INERTIA: f64 = 0.95;
pub const WATER_INERTIA: f64 = 0.8;

pub struct FireballEntity {
    pub thrown: ThrownItemEntity,
    pub item_stack: RwLock<ItemStack>,
    pub explosion_power: AtomicF32,
}

impl FireballEntity {
    #[must_use]
    pub fn new(entity: Entity) -> Self {
        let thrown = ThrownItemEntity {
            entity,
            projectile: crate::entity::projectile::ownership::ProjectileState::new(None),
            has_hit: AtomicBool::new(false),
            gravity: 0.0,
        };

        Self {
            thrown,
            item_stack: RwLock::new(Self::get_default_item()),
            explosion_power: AtomicF32::new(DEFAULT_EXPLOSION_POWER),
        }
    }

    #[must_use]
    pub fn new_shot(entity: Entity, shooter: &Entity, direction: Vector3<f64>) -> Self {
        let thrown = ThrownItemEntity::new(entity, shooter, 0.0);
        let accel = INITIAL_ACCELERATION_POWER;
        let vel = direction.normalize().multiply(accel, accel, accel);
        thrown.entity.velocity.store(vel);

        Self {
            thrown,
            item_stack: RwLock::new(Self::get_default_item()),
            explosion_power: AtomicF32::new(DEFAULT_EXPLOSION_POWER),
        }
    }

    #[must_use]
    pub fn new_directional(
        entity: Entity,
        direction: Vector3<f64>,
        acceleration_power: f64,
    ) -> Self {
        let thrown = ThrownItemEntity {
            entity,
            projectile: crate::entity::projectile::ownership::ProjectileState::new(None),
            has_hit: AtomicBool::new(false),
            gravity: 0.0,
        };
        thrown.projectile.set_acceleration_power(acceleration_power);
        let vel = direction.normalize().multiply(
            acceleration_power,
            acceleration_power,
            acceleration_power,
        );
        thrown.entity.velocity.store(vel);

        Self {
            thrown,
            item_stack: RwLock::new(Self::get_default_item()),
            explosion_power: AtomicF32::new(DEFAULT_EXPLOSION_POWER),
        }
    }

    #[must_use]
    pub fn get_default_item() -> ItemStack {
        ItemStack::new(1, &Item::FIRE_CHARGE)
    }

    pub fn get_item(&self) -> ItemStack {
        self.item_stack
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub fn set_item(&self, source: ItemStack) {
        let new_item = if source.item_count == 0 {
            Self::get_default_item()
        } else {
            let mut item = source;
            item.item_count = 1;
            item
        };
        *self
            .item_stack
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = new_item.clone();

        self.get_entity().set_synced_data(
            pumpkin_data::tracked_data::fireball::ITEM_STACK,
            ItemStackSerializer::from(new_item),
        );
    }

    pub fn get_acceleration_power(&self) -> f64 {
        self.thrown.projectile.acceleration_power()
    }

    pub fn set_acceleration_power(&self, power: f64) {
        self.thrown.projectile.set_acceleration_power(power);
    }

    pub fn get_explosion_power(&self) -> f32 {
        self.explosion_power.load(Ordering::Relaxed)
    }

    pub fn set_explosion_power(&self, power: f32) {
        self.explosion_power.store(power, Ordering::Relaxed);
    }

    pub fn should_render_at_sqr_distance(&self, distance_sqr: f64) -> bool {
        if self.get_entity().age.load(Ordering::Relaxed) < 2
            && distance_sqr < f64::from(MIN_CAMERA_DISTANCE_SQUARED)
        {
            false
        } else {
            let bb_size = self
                .get_entity()
                .bounding_box
                .load()
                .get_average_side_length()
                * 4.0;
            let size = if bb_size.is_nan() { 4.0 } else { bb_size } * 64.0;
            distance_sqr < size * size
        }
    }
}

impl EntityBase for FireballEntity {
    fn projectile_state(&self) -> Option<&super::ownership::ProjectileState> {
        Some(&self.thrown.projectile)
    }

    fn write_custom_nbt(&self, nbt: &mut NbtCompound) {
        // Fireball.addAdditionalSaveData / LargeFireball.addAdditionalSaveData.
        let mut item = NbtCompound::new();
        self.get_item().write_item_stack(&mut item);
        nbt.put_compound("Item", item);
        nbt.put_byte("ExplosionPower", self.get_explosion_power() as i8);
    }

    fn read_custom_nbt(&self, nbt: &NbtCompound) {
        if let Some(item) = nbt
            .get_compound("Item")
            .and_then(ItemStack::read_item_stack)
        {
            self.set_item(item);
        }
        self.set_explosion_power(nbt.get_byte("ExplosionPower").map_or_else(
            || {
                nbt.get_float("ExplosionPower")
                    .unwrap_or(DEFAULT_EXPLOSION_POWER)
            },
            f32::from,
        ));
    }

    fn init_data_tracker(&self) {
        let entity = self.get_entity();
        let stack = self
            .item_stack
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        entity.set_synced_data(
            pumpkin_data::tracked_data::fireball::ITEM_STACK,
            ItemStackSerializer::from(stack.clone()),
        );
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
        let world = self.get_entity().world.load();

        if let ProjectileHit::Entity { ref entity, .. } = hit {
            let owner = self.projectile_owner();
            // LargeFireball.onHitEntity / DamageSources.fireball.
            let damage_type = if owner.is_some() {
                pumpkin_data::damage::DamageType::FIREBALL
            } else {
                pumpkin_data::damage::DamageType::UNATTRIBUTED_FIREBALL
            };
            let _ = super::damage::hurt_entity(
                entity.as_ref(),
                6.0,
                damage_type,
                self,
                owner.as_deref().or(Some(self)),
            );
            super::damage::post_attack(
                entity.as_ref(),
                damage_type,
                self,
                owner.as_deref().or(Some(self)),
            );
        }

        let hit_pos = hit.hit_pos();
        let power = self.get_explosion_power();
        world.explode_from(
            self,
            hit_pos,
            power,
            crate::world::ExplosionInteraction::Mob,
            world.level_info.load().game_rules.mob_griefing,
        );
    }
}
