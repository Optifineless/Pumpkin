use std::sync::RwLock;
use std::sync::atomic::AtomicBool;

use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::world::WorldEvent;
use pumpkin_protocol::codec::item_stack_seralizer::ItemStackSerializer;
use rand::RngExt;

use crate::entity::experience_orb::ExperienceOrbEntity;
use crate::entity::projectile::{ProjectileHit, ThrownItemEntity};
use crate::entity::{Entity, EntityBase};
use crate::server::Server;

const GRAVITY: f64 = 0.07;
const SPLASH_COLOR: i32 = -13_083_194;
// ThrownExperienceBottle.onHit hardcodes two nextInt(5) XP rolls.
const EXPERIENCE_RANDOM_BOUND: u32 = 5;

pub struct ExperienceBottleEntity {
    pub thrown: ThrownItemEntity,
    item_stack: RwLock<ItemStack>,
}

impl ExperienceBottleEntity {
    pub fn new(entity: Entity) -> Self {
        Self {
            thrown: ThrownItemEntity {
                entity,
                projectile: super::ownership::ProjectileState::new(None),
                has_hit: AtomicBool::new(false),
                gravity: GRAVITY,
            },
            item_stack: RwLock::new(ItemStack::new(1, &Item::EXPERIENCE_BOTTLE)),
        }
    }

    pub fn new_shot(entity: Entity, shooter: &Entity) -> Self {
        let bottle = Self {
            thrown: ThrownItemEntity::new(entity, shooter, GRAVITY),
            item_stack: RwLock::new(ItemStack::new(1, &Item::EXPERIENCE_BOTTLE)),
        };
        // ThrowableItemProjectile's shooter constructor uses setPos, synchronizing its bounds.
        let mut origin = shooter.pos.load();
        origin.y += shooter.get_eye_height() - f64::from(0.1f32);
        bottle.get_entity().set_pos(origin);
        bottle
    }

    /// Stores a fresh one-item copy of the supplied stack.
    pub fn set_item_stack(&self, stack: &ItemStack) {
        // ThrowableItemProjectile.setItem copies the source stack.
        *self
            .item_stack
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = stack.copy_with_count(1);
    }
}

impl EntityBase for ExperienceBottleEntity {
    fn get_entity(&self) -> &Entity {
        &self.thrown.entity
    }

    fn get_living_entity(&self) -> Option<&crate::entity::living::LivingEntity> {
        None
    }

    fn projectile_state(&self) -> Option<&super::ownership::ProjectileState> {
        Some(&self.thrown.projectile)
    }

    // ThrowableItemProjectile.addAdditionalSaveData / readAdditionalSaveData.
    fn write_custom_nbt(&self, nbt: &mut pumpkin_nbt::NbtCompound) {
        let mut item = pumpkin_nbt::NbtCompound::new();
        self.item_stack
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .write_item_stack(&mut item);
        nbt.put("Item", item);
    }

    fn read_custom_nbt(&self, nbt: &pumpkin_nbt::NbtCompound) {
        let stack = nbt
            .get_compound("Item")
            .and_then(ItemStack::read_item_stack)
            .unwrap_or_else(|| ItemStack::new(1, &Item::EXPERIENCE_BOTTLE));
        self.set_item_stack(&stack);
    }

    fn init_data_tracker(&self) {
        let stack = self
            .item_stack
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.get_entity().set_synced_data(
            pumpkin_data::tracked_data::experience_bottle::ITEM_STACK,
            ItemStackSerializer::from(stack.clone()),
        );
    }

    fn tick(&self, caller: &dyn EntityBase, server: &Server) {
        self.thrown.process_tick(caller, server);
    }

    // ThrownExperienceBottle.onHit; the shared ThrownItemEntity hit path discards it.
    fn on_hit(&self, hit: ProjectileHit) {
        let entity = self.get_entity();
        entity.set_pos(hit.hit_pos());
        let world = entity.world.load();
        let position = entity.block_pos.load();
        world.sync_world_event(
            WorldEvent::ParticlesSpellPotionSplash,
            position,
            SPLASH_COLOR,
        );
        if !entity.is_silent() {
            world.sync_world_event(WorldEvent::SoundSpellPotionSplash, position, 0);
        }
        let mut random = rand::rng();
        let amount = 3
            + random.random_range(0..EXPERIENCE_RANDOM_BOUND)
            + random.random_range(0..EXPERIENCE_RANDOM_BOUND);
        let direction = match &hit {
            ProjectileHit::Block { face, .. } => {
                let offset = face.to_offset();
                pumpkin_util::math::vector3::Vector3::new(
                    f64::from(offset.x),
                    f64::from(offset.y),
                    f64::from(offset.z),
                )
            }
            ProjectileHit::Entity { .. } => entity.velocity.load().multiply(-1.0, -1.0, -1.0),
        };
        ExperienceOrbEntity::award_with_direction(&world, hit.hit_pos(), direction, amount);
    }

    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
#[path = "experience_bottle_tests.rs"]
mod tests;
