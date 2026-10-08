use std::sync::RwLock;
use std::sync::atomic::AtomicBool;

use crate::{
    entity::{Entity, EntityBase, projectile::ThrownItemEntity},
    server::Server,
};
use pumpkin_data::item_stack::ItemStack;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

const GRAVITY: f64 = 0.05;

pub struct SplashPotionEntity {
    pub thrown: ThrownItemEntity,
    pub item_stack: RwLock<ItemStack>,
}

impl SplashPotionEntity {
    pub fn new(entity: Entity) -> Self {
        entity.set_velocity(Vector3::new(0.0, 0.1, 0.0));
        let thrown = ThrownItemEntity {
            entity,
            projectile: crate::entity::projectile::ownership::ProjectileState::new(None),
            has_hit: AtomicBool::new(false),
            gravity: GRAVITY,
        };

        Self {
            thrown,
            item_stack: RwLock::new(ItemStack::new(1, &pumpkin_data::item::Item::SPLASH_POTION)),
        }
    }

    pub fn new_shot(entity: Entity, shooter: &Entity) -> Self {
        let thrown = ThrownItemEntity::new(entity, shooter, GRAVITY);
        thrown.entity.set_velocity(Vector3::new(0.0, 0.1, 0.0));
        Self {
            thrown,
            item_stack: RwLock::new(ItemStack::new(1, &pumpkin_data::item::Item::SPLASH_POTION)),
        }
    }

    pub fn set_item_stack(&self, item_stack: ItemStack) {
        let mut write = self
            .item_stack
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *write = item_stack;
    }
}

impl EntityBase for SplashPotionEntity {
    fn projectile_state(&self) -> Option<&super::ownership::ProjectileState> {
        Some(&self.thrown.projectile)
    }

    fn write_custom_nbt(&self, nbt: &mut pumpkin_nbt::compound::NbtCompound) {
        let mut item = pumpkin_nbt::compound::NbtCompound::new();
        self.item_stack
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .write_item_stack(&mut item);
        nbt.put_compound("Item", item);
    }
    fn read_custom_nbt(&self, nbt: &pumpkin_nbt::compound::NbtCompound) {
        if let Some(item) = nbt
            .get_compound("Item")
            .and_then(ItemStack::read_item_stack)
        {
            self.set_item_stack(item);
        }
    }
    fn init_data_tracker(&self) {
        let entity = self.get_entity();
        let stack = self
            .item_stack
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        // Sync the item stack
        entity.set_synced_data(
            pumpkin_data::tracked_data::splash_potion::ITEM_STACK,
            pumpkin_protocol::codec::item_stack_seralizer::ItemStackSerializer::from(stack.clone()),
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

    fn on_hit(&self, hit: crate::entity::projectile::ProjectileHit) {
        let stack = self
            .item_stack
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        super::potion_water::on_hit(self, &stack, &hit);
        self.apply_splash(&stack, &hit);
        super::potion_effects::splash_event(self.get_entity(), &stack);
    }
}

impl SplashPotionEntity {
    // ThrownSplashPotion.onHitAsPotion: impact-shifted boxes and the age-dependent projectile margin.
    fn apply_splash(&self, stack: &ItemStack, hit: &super::ProjectileHit) {
        use super::potion_effects::{affected_by_potions, apply_effect, box_distance_squared};
        let entity = self.get_entity();
        let world = entity.world.load();
        let effects = crate::item::potion::PotionContents::read_potion_effects(stack);
        if effects.is_empty() {
            return;
        }
        let potion_box = entity
            .bounding_box
            .load()
            .shift(hit.hit_pos() - entity.pos.load());
        let margin = super::collision::compute_margin(entity);
        let mut affected = Vec::new();
        for target in world.get_all_at_box(&potion_box.expand(4.0, 2.0, 4.0)) {
            if !affected_by_potions(target.as_ref()) {
                continue;
            }
            let distance = box_distance_squared(
                potion_box,
                target.get_entity().bounding_box.load().expand_all(margin),
            );
            if distance < 16.0 {
                affected.push((target, 1.0 - distance.sqrt() / 4.0));
            }
        }
        if let Some(server) = world.server.upgrade() {
            let pos = hit.hit_pos();
            let mut event =
                crate::plugin::api::events::entity::potion_splash::PotionSplashEvent::new(
                    entity.entity_id,
                    BlockPos::floored(pos.x, pos.y, pos.z),
                    stack.item.registry_key.to_string(),
                    affected
                        .iter()
                        .map(|(target, _)| target.get_entity().entity_id)
                        .collect(),
                );
            server.plugin_manager.fire_blocking(&server, &mut event);
            if event.cancelled {
                return;
            }
            affected.retain(|(target, _)| {
                event
                    .affected_entities
                    .contains(&target.get_entity().entity_id)
            });
        }
        let duration_scale = stack
            .get_data_component::<pumpkin_data::data_component_impl::PotionDurationScaleImpl>()
            .map_or(1.0, |scale| scale.scale);
        let owner = self.thrown.projectile.owner(entity);
        for (target, scale) in affected {
            for effect in &effects {
                apply_effect(
                    target.as_ref(),
                    *effect,
                    scale * f64::from(duration_scale),
                    scale,
                    self,
                    owner.as_deref(),
                    true,
                );
            }
        }
    }
}
