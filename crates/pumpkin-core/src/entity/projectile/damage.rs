use crate::entity::EntityBase;
use pumpkin_data::damage::DamageType;

// DamageSources' entity-based constructors leave sourcePositionRaw null.
pub(super) fn hurt_entity(
    target: &dyn EntityBase,
    amount: f32,
    damage_type: DamageType,
    projectile: &dyn EntityBase,
    owner: Option<&dyn EntityBase>,
) -> bool {
    target.damage_with_context(target, amount, damage_type, None, Some(projectile), owner)
}

// EnchantmentHelper.doPostAttackEffects: retain separate attacker and damaging entity.
pub(super) fn post_attack(
    target: &dyn EntityBase,
    damage_type: DamageType,
    projectile: &dyn EntityBase,
    owner: Option<&dyn EntityBase>,
) {
    crate::enchantment::EnchantmentHelper::on_post_attack(
        target,
        crate::enchantment::post_attack::AttackEffectContext {
            attacker: owner,
            damaging_entity: Some(projectile),
            damage_type,
        },
        None,
    );
}
