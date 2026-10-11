use std::sync::Arc;

use pumpkin_data::{entity::EntityType, item_stack::ItemStack};
use pumpkin_util::math::{cos, sin, vector3::Vector3};
use rand::RngExt;

use crate::{
    entity::{Entity, item::ItemEntity},
    plugin::api::events::entity::item_spawn::ItemSpawnEvent,
    world::World,
};

/// Spawns a removed inventory stack at the captured death position.
pub(super) fn spawn_death_inventory_stack(
    world: &Arc<World>,
    position: Vector3<f64>,
    stack: ItemStack,
) {
    // Inventory.dropAll -> LivingEntity.createItemStackToDrop(randomly=true, thrownFromHand=false).
    // Java sets delay 40 here; setDefaultPickUpDelay's 10 is for ordinary world drops.
    const PICKUP_DELAY: u8 = 40;
    let mut random = rand::rng();
    let power = random.random::<f32>() * 0.5;
    let direction = random.random::<f32>() * std::f32::consts::TAU;
    let velocity = Vector3::new(
        f64::from(-sin(direction) * power),
        f64::from(0.2f32),
        f64::from(cos(direction) * power),
    );
    let entity = Entity::new(world.clone(), position, &EntityType::ITEM);
    let mut event = ItemSpawnEvent::new(
        entity.entity_id,
        position,
        stack.item.registry_key.to_owned(),
    );
    if let Some(server) = world.server.upgrade() {
        server.plugin_manager.fire_blocking(&server, &mut event);
    }
    if !event.cancelled {
        world.spawn_entity(Arc::new(ItemEntity::new_with_velocity(
            entity,
            stack,
            velocity,
            PICKUP_DELAY,
        )));
    }
}
