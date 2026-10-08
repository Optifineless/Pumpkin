use std::sync::RwLock;
use std::sync::atomic::AtomicBool;

use crate::{
    entity::{Entity, EntityBase, projectile::ThrownItemEntity},
    server::Server,
};
use pumpkin_data::item_stack::ItemStack;
use pumpkin_util::math::vector3::Vector3;

const GRAVITY: f64 = 0.05;

pub struct LingeringPotionEntity {
    pub thrown: ThrownItemEntity,
    pub item_stack: RwLock<ItemStack>,
}

impl LingeringPotionEntity {
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
            item_stack: RwLock::new(ItemStack::new(
                1,
                &pumpkin_data::item::Item::LINGERING_POTION,
            )),
        }
    }

    pub fn new_shot(entity: Entity, shooter: &Entity) -> Self {
        let thrown = ThrownItemEntity::new(entity, shooter, GRAVITY);
        thrown.entity.set_velocity(Vector3::new(0.0, 0.1, 0.0));
        Self {
            thrown,
            item_stack: RwLock::new(ItemStack::new(
                1,
                &pumpkin_data::item::Item::LINGERING_POTION,
            )),
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

impl EntityBase for LingeringPotionEntity {
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

        // Sync the item stack so the client renders the correct potion type
        entity.set_synced_data(
            pumpkin_data::tracked_data::lingering_potion::ITEM_STACK,
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
        let effects = crate::item::potion::PotionContents::read_potion_effects(&stack);
        if !effects.is_empty() {
            self.make_cloud(stack.clone(), effects, &hit);
        }
        super::potion_effects::splash_event(self.get_entity(), &stack);
    }
}

impl LingeringPotionEntity {
    // ThrownLingeringPotion.onHitAsPotion: entity hits center the cloud on the victim's feet.
    fn make_cloud(
        &self,
        stack: ItemStack,
        effects: Vec<super::potion_effects::EffectEntry>,
        hit: &super::ProjectileHit,
    ) {
        let world = self.get_entity().world.load();
        let position = match hit {
            super::ProjectileHit::Entity { entity, .. } => entity.get_entity().pos.load(),
            super::ProjectileHit::Block { hit_pos, .. } => *hit_pos,
        };
        if let Some(server) = world.server.upgrade() {
            let mut event = crate::plugin::api::events::entity::lingering_potion_splash::LingeringPotionSplashEvent::new(
                self.get_entity().entity_id, position.to_block_pos(), stack.item.registry_key.to_string());
            server.plugin_manager.fire_blocking(&server, &mut event);
            if event.cancelled {
                return;
            }
        }
        let entity = Entity::new(
            world.clone(),
            position,
            &pumpkin_data::entity::EntityType::AREA_EFFECT_CLOUD,
        );
        let cloud = crate::entity::area_effect_cloud::AreaEffectCloudEntity::create(
            entity, stack, effects, 600, 3.0, 20, 10, -0.5, 0,
        );
        cloud.set_radius_per_tick(-3.0 / 600.0);
        let owner = self.thrown.projectile.owner(self.get_entity());
        cloud.set_owner(owner.as_deref());
        world.spawn_entity(cloud);
    }
}
