use super::{LootContextParameters, context::snapshot_entity};
use crate::entity::{EntityBase, player::Player, projectile::fishing_bobber::FishingBobberEntity};
use pumpkin_data::{attributes::Attributes, item_stack::ItemStack};
use std::{collections::HashSet, sync::atomic::Ordering::Relaxed};

/// Builds FISHING parameters with the live hook snapshot, used rod, registry and combined luck.
#[must_use]
pub fn build_fishing_loot_context(
    hook: &FishingBobberEntity,
    owner: &Player,
    rod: &ItemStack,
    luck: i32,
) -> LootContextParameters {
    // FishingHook.retrieve supplies ORIGIN, TOOL, THIS_ENTITY and hook luck + player luck.
    let world = hook.get_entity().world.load_full();
    let time = world
        .level_time
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .world_age;
    let weather = world
        .weather
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (raining, thundering) = (weather.raining, weather.thundering);
    drop(weather);
    LootContextParameters {
        registry: world
            .server
            .upgrade()
            .map(|server| server.datapack_manager.clone()),
        world: Some(world),
        position: Some(hook.get_entity().pos.load()),
        tool: Some(rod.clone()),
        this_entity: Some(hook.get_entity().entity_type),
        this_entity_state: snapshot_entity(hook, &mut HashSet::new(), 0),
        luck: luck as f32 + owner.living_entity.get_attribute_value(&Attributes::LUCK) as f32,
        world_time: time as u64,
        is_raining: Some(raining),
        is_thundering: Some(thundering),
        is_on_fire: Some(hook.get_entity().fire_ticks.load(Relaxed) > 0),
        ..Default::default()
    }
}
