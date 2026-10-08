use super::*;
#[test]
fn melee_sweep_damage_is_enchanted_for_each_victim_and_scaled_by_charge() {
    use super::Player;
    use pumpkin_data::{
        data_component_impl::EnchantmentsImpl, enchantment::Enchantment, entity::EntityType,
        item::Item, item_stack::ItemStack,
    };
    let mut weapon = ItemStack::new(1, &Item::DIAMOND_SWORD);
    weapon.set_data_component(EnchantmentsImpl {
        enchantment: std::borrow::Cow::Owned(vec![(&Enchantment::SMITE, 2)]),
    });
    let zombie_damage = Player::sweep_damage(&EntityType::ZOMBIE, &weapon, 7.0, 0.5, 0.95);
    let spider_damage = Player::sweep_damage(&EntityType::SPIDER, &weapon, 7.0, 0.5, 0.95);
    assert!((zombie_damage - 9.025).abs() < 0.000_001);
    assert!((spider_damage - 4.275).abs() < 0.000_001);
}

#[test]
fn melee_charge_has_an_unsaturated_live_high_speed_case() {
    use crate::entity::attributes::{AttributeInstance, Modifier, ModifierOperation};
    let mut attribute = AttributeInstance::new(4.0);
    attribute.add_or_replace_modifier(Modifier {
        id: "test:haste".into(),
        amount: 4.0,
        operation: ModifierOperation::Add,
        permanent: false,
    });
    assert_eq!(
        Player::attack_strength_scale(
            1,
            0.5,
            Player::current_item_attack_strength_delay(attribute.value())
        ),
        0.6
    );
    assert_eq!(Player::attack_strength_scale(0, 0.5, 5.0), 0.1);
}

#[test]
fn melee_crit_and_charge_keep_enchantment_damage_separate() {
    use super::{AttackType, Player};
    // Vanilla diamond sword (7) plus Sharpness V (3): 7 * 1.5 + 3.
    assert_eq!(
        Player::melee_attack_damage(7.0, 10.0, 0.0, 1.0, AttackType::Critical),
        Some((10.5, 13.5))
    );
    assert_eq!(
        Player::melee_attack_damage(7.0, 10.0, 0.0, 0.5, AttackType::Weak),
        Some((2.8, 4.3))
    );
    // Mace bonus is added after charge scaling, before the critical multiplier.
    assert_eq!(
        Player::melee_attack_damage(6.0, 9.0, 12.0, 1.0, AttackType::Critical),
        Some((27.0, 30.0))
    );
    assert_eq!(
        Player::melee_attack_damage(6.0, 9.0, 12.0, 0.5, AttackType::Weak),
        Some((14.4, 15.9))
    );
}

#[test]
fn melee_damage_composition_rejects_zero_base_and_magic() {
    use super::{AttackType, Player};
    assert_eq!(
        Player::melee_attack_damage(0.0, 0.0, 12.0, 1.0, AttackType::Strong),
        None
    );
    assert_eq!(
        Player::melee_attack_damage(0.0, 3.0, 0.0, 1.0, AttackType::Strong),
        Some((0.0, 3.0))
    );
}

#[test]
fn melee_sweep_uses_several_live_ratios() {
    let weapon = ItemStack::new(1, &pumpkin_data::item::Item::DIAMOND_SWORD);
    for (ratio, expected) in [(0.0, 1.0), (0.5, 4.5), (0.75, 6.25), (1.0, 8.0)] {
        assert_eq!(
            Player::sweep_damage(&EntityType::ZOMBIE, &weapon, 7.0, ratio, 1.0),
            expected
        );
    }
}

#[test]
fn melee_mace_effects_observe_fall_distance_before_post_hurt_reset() {
    use std::cell::Cell;
    let fall_distance = Cell::new(12.0);
    let braked = Cell::new(false);
    let protected = Cell::new(false);
    let exploded = Cell::new(false);
    Player::run_item_attack_interaction(
        || {
            braked.set(true);
            protected.set(true);
        },
        || {
            assert!(braked.get());
            assert!(protected.get());
            assert_eq!(fall_distance.get(), 12.0);
            exploded.set(true);
        },
        || {
            assert!(exploded.get());
            fall_distance.set(0.0);
        },
    );
    assert_eq!(fall_distance.get(), 0.0);
}
