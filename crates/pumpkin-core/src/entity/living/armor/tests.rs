use super::super::test_support::armor_test_world;
use super::*;
use crate::entity::Entity;
use crate::entity::attributes::{Modifier, ModifierOperation};
use pumpkin_data::Enchantment;
use pumpkin_nbt::compound::NbtCompound;

#[test]
fn equipment_wear_requires_positive_damage() {
    let armor = ItemStack::new(1, &Item::IRON_CHESTPLATE);
    for damage in [-4.0, 0.0] {
        assert_eq!(
            equipment_damage_amount(&armor, &DamageType::MOB_ATTACK, damage),
            None
        );
    }
    for (damage, expected) in [(0.5, 1), (7.99, 1), (8.0, 2)] {
        assert_eq!(
            equipment_damage_amount(&armor, &DamageType::MOB_ATTACK, damage),
            Some(expected)
        );
    }
}

#[test]
fn equipment_wear_requires_equippable_damage_on_hurt() {
    let sword = ItemStack::new(1, &Item::IRON_SWORD);
    assert_eq!(
        equipment_damage_amount(&sword, &DamageType::MOB_ATTACK, 8.0),
        None
    );
    let mut missing_damage = ItemStack::new(1, &Item::IRON_CHESTPLATE);
    missing_damage.remove_data_component(pumpkin_data::data_component::DataComponent::Damage);
    assert_eq!(
        equipment_damage_amount(&missing_damage, &DamageType::MOB_ATTACK, 8.0),
        None
    );
    let mut unbreakable = ItemStack::new(1, &Item::IRON_CHESTPLATE);
    unbreakable.set_data_component(pumpkin_data::data_component_impl::UnbreakableImpl);
    assert_eq!(
        equipment_damage_amount(&unbreakable, &DamageType::MOB_ATTACK, 8.0),
        None
    );
    let mut armor = ItemStack::new(1, &Item::IRON_CHESTPLATE);
    let mut equippable = armor
        .get_data_component::<EquippableImpl>()
        .unwrap()
        .clone();
    equippable.damage_on_hurt = false;
    armor.set_data_component(equippable);
    assert_eq!(
        equipment_damage_amount(&armor, &DamageType::MOB_ATTACK, 8.0),
        None
    );
}

#[test]
fn equipment_wear_respects_damage_resistant_tags() {
    let armor = ItemStack::new(1, &Item::NETHERITE_CHESTPLATE);
    assert_eq!(
        equipment_damage_amount(&armor, &DamageType::LAVA, 8.0),
        None
    );
    assert_eq!(
        equipment_damage_amount(&armor, &DamageType::MOB_ATTACK, 8.0),
        Some(2)
    );
}

#[tokio::test]
async fn armor_absorption_reads_live_attributes() {
    let temp = tempfile::tempdir().unwrap();
    let living = LivingEntity::new(Entity::new(
        armor_test_world(temp.path()),
        Vector3::default(),
        &EntityType::ZOMBIE,
    ));
    living.set_attribute_base(&Attributes::ARMOR, 12.9);
    living.set_attribute_base(&Attributes::ARMOR_TOUGHNESS, 8.0);
    let absorb = || {
        living.get_damage_after_armor_absorb_with_weapon(
            &living,
            20.0,
            &DamageType::MOB_ATTACK,
            None,
        )
    };
    assert!((absorb() - 14.4).abs() < 1.0e-5);
    living.update_attribute(&Attributes::ARMOR, |attribute| {
        attribute.add_or_replace_modifier(Modifier {
            id: "test:armor".to_string(),
            amount: 8.0,
            operation: ModifierOperation::Add,
            permanent: true,
        });
    });
    assert!((absorb() - 8.0).abs() < 1.0e-5);
    living.set_attribute_base(&Attributes::ARMOR_TOUGHNESS, 0.0);
    assert!((absorb() - 12.0).abs() < 1.0e-5);
    assert_eq!(
        living.get_damage_after_armor_absorb_with_weapon(&living, 20.0, &DamageType::FALL, None),
        20.0
    );
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn armor_mitigation_sanitizes_live_attribute_ranges() {
    let temp = tempfile::tempdir().unwrap();
    let living = LivingEntity::new(Entity::new(
        armor_test_world(temp.path()),
        Vector3::default(),
        &EntityType::ZOMBIE,
    ));
    let absorb = |damage| {
        living.get_damage_after_armor_absorb_with_weapon(
            &living,
            damage,
            &DamageType::MOB_ATTACK,
            None,
        )
    };
    living.set_attribute_base(&Attributes::ARMOR_TOUGHNESS, 0.0);
    // Attributes.java ranges and RangedAttribute.sanitizeValue, including command bases.
    for (base, expected) in [
        (-1.0, 100.0),
        (0.0, 100.0),
        (30.0, 76.0),
        (31.0, 76.0),
        (101.0, 76.0),
        (f64::NAN, 100.0),
        (f64::INFINITY, 76.0),
        (f64::NEG_INFINITY, 100.0),
    ] {
        living.set_attribute_base(&Attributes::ARMOR, base);
        assert!((absorb(100.0) - expected).abs() < 1e-5, "armor base {base}");
    }
    living.set_attribute_base(&Attributes::ARMOR, 20.0);
    for (base, expected) in [
        (-1.0, 12.0),
        (0.0, 12.0),
        (20.0, 6.285_714),
        (21.0, 6.285_714),
        (f64::NAN, 12.0),
        (f64::INFINITY, 6.285_714),
        (f64::NEG_INFINITY, 12.0),
    ] {
        living.set_attribute_base(&Attributes::ARMOR_TOUGHNESS, base);
        assert!(
            (absorb(20.0) - expected).abs() < 1e-5,
            "toughness base {base}"
        );
    }
    living.update_attribute(&Attributes::ARMOR, |attribute| {
        attribute.add_or_replace_modifier(Modifier {
            id: "test:nan_armor".to_string(),
            amount: f64::NAN,
            operation: ModifierOperation::Add,
            permanent: true,
        });
    });
    assert_eq!(absorb(100.0), 100.0);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn wolf_armor_absorbs_admitted_hits_and_respects_bypass() {
    let temp = tempfile::tempdir().unwrap();
    let living = LivingEntity::new(Entity::new(
        armor_test_world(temp.path()),
        Vector3::default(),
        &EntityType::WOLF,
    ));
    living
        .entity_equipment
        .lock()
        .unwrap()
        .put(&EquipmentSlot::BODY, ItemStack::new(1, &Item::WOLF_ARMOR));
    assert!(living.try_absorb_wolf_armor_damage(&DamageType::MOB_ATTACK, 8.1));
    assert_eq!(
        living
            .entity_equipment
            .lock()
            .unwrap()
            .get(&EquipmentSlot::BODY)
            .get_damage(),
        9
    );
    // FALL bypasses ordinary armor, but is absent from BYPASSES_WOLF_ARMOR.
    assert!(living.try_absorb_wolf_armor_damage(&DamageType::FALL, 8.0));
    assert!(!living.try_absorb_wolf_armor_damage(&DamageType::MAGIC, 8.0));
    assert_eq!(
        living
            .entity_equipment
            .lock()
            .unwrap()
            .get(&EquipmentSlot::BODY)
            .get_damage(),
        17
    );
    {
        let mut equipment = living.entity_equipment.lock().unwrap();
        let stack = equipment.equipment.get_mut(&EquipmentSlot::BODY).unwrap();
        stack.set_damage(stack.get_max_damage().unwrap() - 1);
        drop(equipment);
        assert!(living.try_absorb_wolf_armor_damage(&DamageType::MOB_ATTACK, 8.0));
        assert!(
            living
                .entity_equipment
                .lock()
                .unwrap()
                .get(&EquipmentSlot::BODY)
                .is_empty()
        );
        assert!(!living.try_absorb_wolf_armor_damage(&DamageType::MOB_ATTACK, 8.0));
        living.entity_equipment.lock().unwrap().put(
            &EquipmentSlot::BODY,
            ItemStack::new(1, &Item::IRON_CHESTPLATE),
        );
        assert!(!living.try_absorb_wolf_armor_damage(&DamageType::MOB_ATTACK, 8.0));
        living.get_damage_after_armor_absorb_with_weapon(
            &living,
            8.0,
            &DamageType::MOB_ATTACK,
            None,
        );
        assert_eq!(
            living
                .entity_equipment
                .lock()
                .unwrap()
                .get(&EquipmentSlot::BODY)
                .get_damage(),
            2
        );
    };
    crate::server::fixture_lifecycle::finish().await;
}

#[test]
fn wolf_armor_cracks_at_vanilla_strict_boundaries() {
    use pumpkin_data::data_component_impl::MaxDamageImpl;
    let mut stack = ItemStack::new(1, &Item::WOLF_ARMOR);
    stack.set_data_component(MaxDamageImpl { max_damage: 100 });
    for (damage, level) in [
        (5, WolfArmorCrackiness::None),
        (6, WolfArmorCrackiness::Low),
        (31, WolfArmorCrackiness::Low),
        (32, WolfArmorCrackiness::Medium),
        (68, WolfArmorCrackiness::Medium),
        (69, WolfArmorCrackiness::High),
    ] {
        stack.set_damage(damage);
        assert_eq!(wolf_armor_crackiness(&stack), level);
    }
    stack.set_data_component(DamageImpl { damage: i32::MIN });
    assert_eq!(wolf_armor_crackiness(&stack), WolfArmorCrackiness::None);
}

#[test]
fn wolf_armor_crack_particles_use_the_vanilla_item_template_layout() {
    // ItemParticleOption.streamCodec -> ItemStackTemplate.STREAM_CODEC:
    // vanilla registry item 1003 (scute), count 1, empty component patch.
    assert_eq!(
        wolf_armor_crack_particle_data().unwrap(),
        [0xeb, 0x07, 1, 0, 0]
    );
}

#[tokio::test]
async fn equipment_wear_mutates_and_breaks_the_stored_body_stack() {
    let temp = tempfile::tempdir().unwrap();
    let living = LivingEntity::new(Entity::new(
        armor_test_world(temp.path()),
        Vector3::default(),
        &EntityType::HORSE,
    ));
    living.entity_equipment.lock().unwrap().put(
        &EquipmentSlot::BODY,
        ItemStack::new(1, &Item::IRON_CHESTPLATE),
    );
    living.get_damage_after_armor_absorb_with_weapon(&living, 8.0, &DamageType::MOB_ATTACK, None);
    let mut saved = NbtCompound::new();
    living.write_living_nbt(&mut saved);
    let stack = ItemStack::read_item_stack(
        saved
            .get_compound("equipment")
            .unwrap()
            .get_compound("body")
            .unwrap(),
    )
    .unwrap();
    assert_eq!(stack.get_damage(), 2);
    let mut worn = stack;
    worn.set_damage(worn.get_max_damage().unwrap() - 1);
    living
        .entity_equipment
        .lock()
        .unwrap()
        .put(&EquipmentSlot::BODY, worn);
    living.get_damage_after_armor_absorb_with_weapon(&living, 4.0, &DamageType::MOB_ATTACK, None);
    assert!(
        living
            .entity_equipment
            .lock()
            .unwrap()
            .get(&EquipmentSlot::BODY)
            .is_empty()
    );
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn armor_effects_use_the_direct_projectiles_stored_weapon() {
    use crate::entity::projectile::arrow::ArrowEntity;
    use crate::entity::projectile::trident::TridentEntity;
    use std::borrow::Cow;
    let temp = tempfile::tempdir().unwrap();
    let world = armor_test_world(temp.path());
    let shooter = LivingEntity::new(Entity::new(
        world.clone(),
        Vector3::default(),
        &EntityType::ZOMBIE,
    ));
    let mut mace = ItemStack::new(1, &Item::MACE);
    mace.set_data_component(EnchantmentsImpl {
        enchantment: Cow::Owned(vec![(&Enchantment::BREACH, 4)]),
    });
    shooter
        .entity_equipment
        .lock()
        .unwrap()
        .put(&EquipmentSlot::MAIN_HAND, mace);
    let arrow = ArrowEntity::new(
        Entity::new(world.clone(), Vector3::default(), &EntityType::ARROW),
        Some(shooter.entity.entity_id),
    );
    assert!(LivingEntity::damage_source_weapon(Some(&arrow)).is_none());
    *arrow.weapon.write().unwrap() = Some(ItemStack::new(1, &Item::BOW));
    let weapon = LivingEntity::damage_source_weapon(Some(&arrow)).unwrap();
    assert_eq!(weapon.item.id, Item::BOW.id);
    assert_eq!(weapon.get_enchantment_level(&Enchantment::BREACH), 0);
    assert_eq!(
        LivingEntity::damage_source_weapon(Some(&shooter))
            .unwrap()
            .get_enchantment_level(&Enchantment::BREACH),
        4
    );
    shooter.set_attribute_base(&Attributes::ARMOR, 20.0);
    shooter.set_attribute_base(&Attributes::ARMOR_TOUGHNESS, 8.0);
    assert!(
        (shooter.get_damage_after_armor_absorb(20.0, &DamageType::ARROW, Some(&shooter)) - 8.0)
            .abs()
            < 1.0e-5
    );
    let trident = TridentEntity::new(
        Entity::new(world, Vector3::default(), &EntityType::TRIDENT),
        Some(shooter.entity.entity_id),
    );
    assert_eq!(
        LivingEntity::damage_source_weapon(Some(&trident))
            .unwrap()
            .item
            .id,
        Item::TRIDENT.id
    );
    crate::server::fixture_lifecycle::finish().await;
}
