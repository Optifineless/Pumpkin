use super::{EntityBase, decoration::armor_stand::ArmorStandEntity};
use pumpkin_data::{
    entity::EntityType,
    tag::{self, Taggable},
};

// Entity.canBeHitByProjectile calls the LivingEntity.isAlive override before isPickable.
pub(super) fn can_be_hit_by_projectile(other: &(impl EntityBase + ?Sized)) -> bool {
    // Interaction.canBeHitByProjectile overrides the otherwise pickable entity.
    other.get_entity().entity_type != &EntityType::INTERACTION
        && other.get_entity().is_alive()
        && other
            .get_living_entity()
            .is_none_or(|living| living.health.load() > 0.0)
        && other.is_pickable()
}

// LivingEntity.isPickable, Player/ArmorStand overrides, and the nonliving overrides
// already used by Projectile.canHitEntity. Living pickability does not test health.
pub(super) fn is_pickable(other: &(impl EntityBase + ?Sized)) -> bool {
    let entity = other.get_entity();
    if other.is_spectator() || entity.entity_type == &EntityType::ENDER_DRAGON {
        return false;
    }
    if let Some(stand) = other.cast_any().downcast_ref::<ArmorStandEntity>()
        && stand.is_marker()
    {
        return false;
    }
    if other.get_living_entity().is_some() {
        return !entity.is_removed();
    }
    // FallingBlockEntity/PrimedTnt.isPickable also excludes removed entities.
    if [&EntityType::FALLING_BLOCK, &EntityType::TNT].contains(&entity.entity_type) {
        return !entity.is_removed();
    }

    // Interaction.isPickable is true even though it cannot be hit by projectiles.
    entity.entity_type == &EntityType::INTERACTION
        || other.can_hit()
        || [
            &EntityType::END_CRYSTAL,
            &EntityType::ITEM_FRAME,
            &EntityType::GLOW_ITEM_FRAME,
            &EntityType::SHULKER_BULLET,
        ]
        .contains(&entity.entity_type)
        || entity
            .entity_type
            .has_tag(&tag::EntityType::MINECRAFT_REDIRECTABLE_PROJECTILE)
}
