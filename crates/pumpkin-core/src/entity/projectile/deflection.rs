use super::{ProjectileHit, wind_charge::WindChargeEntity};
use crate::entity::{EntityBase, projectile_deflection::ProjectileDeflectionType};
use pumpkin_data::{
    entity::EntityType,
    tag::{self, Taggable},
};
use pumpkin_util::math::vector3::Vector3;
use std::sync::atomic::Ordering;

/// `Projectile.deflect`: changes motion and owner and runs the projectile's deflection hook.
pub fn deflect(
    projectile: &dyn EntityBase,
    deflection: ProjectileDeflectionType,
    deflector: Option<&dyn EntityBase>,
    new_owner: Option<&dyn EntityBase>,
    by_attack: bool,
    power: Vector3<f64>,
) -> bool {
    let Some(state) = projectile.projectile_state() else {
        return false;
    };
    if let Some(wind) = projectile.cast_any().downcast_ref::<WindChargeEntity>()
        && wind
            .deflect_cooldown()
            .is_some_and(|cooldown| cooldown.load(Ordering::Relaxed) > 0)
    {
        return false;
    }
    deflection.apply(projectile, deflector, power);
    state.set_owner(new_owner.map(EntityBase::get_entity));
    if super::hurting::is_hurting(projectile.get_entity()) {
        state.set_acceleration_power(if by_attack {
            super::fireball::INITIAL_ACCELERATION_POWER
        } else {
            state.acceleration_power() * super::fireball::DEFLECTION_SCALE
        });
    }
    true
}

/// Returns true when `Projectile.hitTargetOrDeflectSelf` consumes the collision before hit effects.
pub(super) fn hit_target_or_deflect_self(projectile: &dyn EntityBase, hit: &ProjectileHit) -> bool {
    let Some(state) = projectile.projectile_state() else {
        return false;
    };
    let ProjectileHit::Entity { entity: target, .. } = hit else {
        return hit_block_or_deflect(projectile, hit);
    };
    let target_type = target.get_entity().entity_type;
    let is_wind = [
        EntityType::WIND_CHARGE.id,
        EntityType::BREEZE_WIND_CHARGE.id,
    ]
    .contains(&projectile.get_entity().entity_type.id);
    if target_type.has_tag(&tag::EntityType::MINECRAFT_DEFLECTS_PROJECTILES)
        && !(target_type == &EntityType::BREEZE && is_wind)
    {
        let last = *state
            .last_deflected_by
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if last != Some(target.get_entity().entity_uuid) {
            let owner_uuid = state.owner_uuid();
            let owner = state.owner(projectile.get_entity());
            if deflect(
                projectile,
                ProjectileDeflectionType::Simple,
                Some(target.as_ref()),
                owner.as_deref(),
                false,
                Vector3::new(1.0, 1.0, 1.0),
            ) {
                state.set_owner_uuid(owner_uuid);
                *state
                    .last_deflected_by
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) =
                    Some(target.get_entity().entity_uuid);
                if target_type == &EntityType::BREEZE {
                    target.get_entity().world.load().play_sound(
                        pumpkin_data::sound::Sound::EntityBreezeDeflect,
                        pumpkin_data::sound::SoundCategory::Hostile,
                        &target.get_entity().pos.load(),
                    );
                }
            }
        }
        return true;
    }
    // Projectile.onHit redirects the other projectile before invoking onHitEntity.
    if target_type.has_tag(&tag::EntityType::MINECRAFT_REDIRECTABLE_PROJECTILE) {
        let owner = state.owner(projectile.get_entity());
        // ShulkerBullet.onRedirectProjectile uses the bullet's momentum instead of its owner's aim.
        let momentum = projectile.get_entity().entity_type == &EntityType::SHULKER_BULLET;
        let redirected = deflect(
            target.as_ref(),
            if momentum {
                ProjectileDeflectionType::TransferVelocityDirection
            } else {
                ProjectileDeflectionType::Redirected
            },
            if momentum {
                Some(projectile)
            } else {
                owner.as_deref()
            },
            owner.as_deref(),
            true,
            Vector3::new(1.0, 1.0, 1.0),
        );
        if redirected && let Some(target_state) = target.projectile_state() {
            target_state.set_owner_uuid(state.owner_uuid());
        }
    }
    false
}

fn hit_block_or_deflect(projectile: &dyn EntityBase, hit: &ProjectileHit) -> bool {
    let Some(state) = projectile.projectile_state() else {
        return false;
    };
    if matches!(
        hit,
        ProjectileHit::Block {
            world_border: true,
            ..
        }
    ) {
        // AbstractArrow / FishingHook.shouldBounceOnWorldBorder.
        let kind = projectile.get_entity().entity_type;
        if [
            EntityType::ARROW.id,
            EntityType::SPECTRAL_ARROW.id,
            EntityType::TRIDENT.id,
            EntityType::FISHING_BOBBER.id,
        ]
        .contains(&kind.id)
        {
            let owner_uuid = state.owner_uuid();
            let owner = state.owner(projectile.get_entity());
            let bounced = deflect(
                projectile,
                ProjectileDeflectionType::Simple,
                None,
                owner.as_deref(),
                false,
                Vector3::new(0.2, 0.2, 0.2),
            );
            state.set_owner_uuid(owner_uuid);
            return bounced;
        }
        return false;
    }
    matches!(hit, ProjectileHit::Block { pos, .. } if projectile.get_entity().world.load().get_block_state(pos).is_air())
}
