//! `Drowned.finalizeSpawn`'s 26.3 trident-rider branch.
use crate::{
    entity::{
        EntityBase,
        mob::{
            Mob,
            spawn::{SpawnReason, finalize_spawn_in_view},
        },
    },
    world::{World, spawn_view::SpawnView},
};
use pumpkin_data::{
    data_component_impl::EquipmentSlot, entity::EntityType, item::Item, tag::Taggable,
};
use std::sync::Arc;

pub(super) fn try_nautilus(
    mob: &dyn Mob,
    entity: &Arc<dyn EntityBase>,
    world: &Arc<World>,
    view: &SpawnView<'_>,
    reason: SpawnReason,
    roll: f32,
) {
    let base = mob.get_entity();
    if !matches!(reason, SpawnReason::Natural | SpawnReason::Structure)
        || mob
            .get_mob_entity()
            .living_entity
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&EquipmentSlot::MAIN_HAND)
            .item
            != &Item::TRIDENT
        || roll >= 0.5
        || base.age.load(std::sync::atomic::Ordering::Relaxed) < 0
        || view
            .get_biome(&base.block_pos.load())
            .has_tag(&pumpkin_data::tag::WorldgenBiome::MINECRAFT_MORE_FREQUENT_DROWNED_SPAWNS)
    {
        return;
    }
    let mount = crate::entity::r#type::from_type(
        &EntityType::ZOMBIE_NAUTILUS,
        base.pos.load(),
        world,
        uuid::Uuid::new_v4(),
    );
    if reason == SpawnReason::Structure
        && let Some(mob) = mount.get_mob()
    {
        mob.get_mob_entity()
            .persistence_required
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
    mount.get_entity().set_rotation(base.yaw.load(), 0.0);
    // Java creates with JOCKEY but finalizes with the original NATURAL/STRUCTURE reason.
    finalize_spawn_in_view(&mount, world, view, reason, None);
    crate::entity::spawn_mount::queue_spawn_mount(entity, mount);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::spawn_test_support::{Fixture, proto};
    use pumpkin_data::{Block, biome::Biome, item_stack::ItemStack};
    use pumpkin_util::math::vector3::Vector3;

    #[tokio::test]
    async fn drowned_mount_uses_generation_biome_and_restores_passenger_equipment() {
        let fixture = Fixture::new();
        let mut region = proto(&Biome::WARM_OCEAN, &Block::SAND);
        let drowned = crate::entity::r#type::from_type(
            &EntityType::DROWNED,
            Vector3::new(8.5, 64.0, 8.5),
            &fixture.world,
            uuid::Uuid::new_v4(),
        );
        let mob = drowned.get_mob().unwrap();
        mob.get_mob_entity()
            .set_item_slot(&EquipmentSlot::MAIN_HAND, ItemStack::new(1, &Item::TRIDENT));
        let view = SpawnView::generation(&fixture.world, &region);
        try_nautilus(
            mob,
            &drowned,
            &fixture.world,
            &view,
            SpawnReason::Structure,
            0.0,
        );
        let mount = crate::entity::spawn_mount::spawn_root(&drowned);
        assert_eq!(mount.get_entity().entity_type, &EntityType::ZOMBIE_NAUTILUS);
        assert!(
            mount
                .get_mob()
                .unwrap()
                .get_mob_entity()
                .persistence_required
                .load(std::sync::atomic::Ordering::Relaxed)
        );
        let cooldown = *mount
            .get_mob()
            .unwrap()
            .get_mob_entity()
            .brain
            .lock()
            .unwrap()
            .get(crate::entity::ai::brain::memory::types::ATTACK_TARGET_COOLDOWN)
            .unwrap();
        assert!((2400..=3600).contains(&cooldown));
        assert!(fixture.world.entities.load().is_empty());
        crate::world::generation_spawning::retain_entity(&mut region, &mount);
        let nbt = &region.pending_entities[0];
        assert_eq!(nbt.get_string("variant"), Some("minecraft:warm"));
        let restored = crate::entity::mob::spawn::load_spawn_entity(
            &fixture.world,
            nbt,
            Vector3::new(8.5, 64.0, 8.5),
        )
        .unwrap();
        let _guard = crate::entity::spawn_mount::UnpublishedRidingTree::new(&restored);
        assert_eq!(
            restored
                .get_mob()
                .unwrap()
                .get_mob_entity()
                .brain
                .lock()
                .unwrap()
                .get(crate::entity::ai::brain::memory::types::ATTACK_TARGET_COOLDOWN),
            Some(&cooldown)
        );
        let passenger = restored.get_entity().passengers.lock().unwrap()[0].clone();
        assert_eq!(passenger.get_entity().entity_type, &EntityType::DROWNED);
        assert_eq!(
            passenger
                .get_living_entity()
                .unwrap()
                .entity_equipment
                .lock()
                .unwrap()
                .get(&EquipmentSlot::MAIN_HAND)
                .item,
            &Item::TRIDENT
        );
        fixture.finish().await;
    }
}
