use super::LivingEntity;
use crate::enchantment::EnchantmentHelper;
use crate::entity::attributes::{AttributeInstance, Modifier, ModifierOperation};
use pumpkin_data::AttributeModifierSlot;
use pumpkin_data::attributes::Attributes;
use pumpkin_data::data_component_impl::{
    AttributeModifiersImpl, DamageImpl, EquipmentSlot, Operation,
};
use pumpkin_data::entity::EntityType;
use pumpkin_data::item_stack::ItemStack;
use rustc_hash::FxHashMap;

impl LivingEntity {
    /// Applies item `attribute_modifiers` for the given slots and notifies clients.
    ///
    /// The local HUD armor bar is driven by `minecraft:armor` / `minecraft:armor_toughness`
    /// on `UPDATE_ATTRIBUTES`, not by `SET_EQUIPMENT`.
    pub fn apply_and_send_equipment_attribute_modifiers(
        &self,
        equipment: &[(EquipmentSlot, ItemStack)],
    ) {
        let touched = self.apply_equipment_attribute_modifiers(equipment);
        if !touched.is_empty() {
            crate::entity::attributes::send_attribute_updates_for_living(self, touched);
        }
    }

    /// Re-applies modifiers from every currently equipped stack without notifying clients.
    pub fn apply_current_equipment_attribute_modifiers(&self) {
        // LivingEntity.collectEquipmentChanges rebuilds transient modifiers after loading equipment.
        let equipment = self.snapshot_equipped_stacks();
        self.apply_equipment_attribute_modifiers(&equipment);
    }

    /// Re-applies modifiers from every currently equipped stack and sends updates.
    pub fn send_current_equipment_attribute_modifiers(&self) {
        self.apply_and_send_equipment_attribute_modifiers(&self.snapshot_equipped_stacks());
    }

    fn snapshot_equipped_stacks(&self) -> Vec<(EquipmentSlot, ItemStack)> {
        // Player.getItemBySlot reads the selected hotbar stack from PlayerInventory.
        let main_hand = (self.entity.entity_type == &EntityType::PLAYER)
            .then(|| {
                self.entity
                    .world
                    .load()
                    .get_player_by_uuid(self.entity.entity_uuid)
            })
            .flatten()
            .map(|player| player.inventory.held_item());
        let guard = self
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut equipment: Vec<_> = guard
            .equipment
            .iter()
            .map(|(slot, stack)| (slot.clone(), stack.clone()))
            .collect();
        if let Some(stack) = main_hand {
            equipment.retain(|(slot, _)| *slot != EquipmentSlot::MAIN_HAND);
            equipment.push((EquipmentSlot::MAIN_HAND, stack));
        }
        equipment.sort_unstable_by_key(|(slot, _)| slot.discriminant());
        equipment
    }

    fn apply_equipment_attribute_modifiers(
        &self,
        equipment: &[(EquipmentSlot, ItemStack)],
    ) -> Vec<Attributes> {
        let world = self.entity.world.load();
        let batches: Vec<_> = equipment
            .iter()
            .map(|(slot, stack)| {
                (
                    slot,
                    equipment_slot_attribute_modifiers(&world, stack, slot),
                )
            })
            .collect();
        // LivingEntity.collectEquipmentChanges removes all old modifiers before adding new ones.
        // Otherwise a hand swap can remove the modifier just installed in the other hand.
        let mut ids = self
            .equipment_attribute_modifier_ids
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut attributes = self
            .attributes
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut touched = Vec::new();
        for (slot, _) in equipment {
            if let Some(previous) = ids.remove(slot) {
                for (attr_id, modifier_id) in previous {
                    if let Some(attr) = attributes_by_id(attr_id)
                        && let Some(instance) = attributes.get_mut(&attr_id)
                    {
                        instance.remove_modifier(&modifier_id);
                        push_unique_attribute(&mut touched, attr);
                    }
                }
            }
        }
        for (slot, modifiers) in batches {
            self.apply_equipment_slot_attribute_modifiers(
                slot,
                modifiers,
                &mut attributes,
                &mut ids,
                &mut touched,
            );
        }
        touched
    }

    fn apply_equipment_slot_attribute_modifiers(
        &self,
        slot: &EquipmentSlot,
        modifiers: Vec<(&'static Attributes, Modifier)>,
        attributes: &mut FxHashMap<u8, AttributeInstance>,
        ids: &mut FxHashMap<EquipmentSlot, Vec<(u8, String)>>,
        touched: &mut Vec<Attributes>,
    ) {
        let mut applied = Vec::new();
        for (attribute, modifier) in modifiers {
            let instance = attributes.entry(attribute.id).or_insert_with(|| {
                let base = self
                    .entity
                    .entity_type
                    .attributes
                    .iter()
                    .find(|(candidate, _)| candidate.id == attribute.id)
                    .map_or(attribute.default_value, |(_, base)| *base);
                AttributeInstance::new(base)
            });
            applied.push((attribute.id, modifier.id.clone()));
            instance.add_or_replace_modifier(modifier);
            push_unique_attribute(touched, attribute);
        }

        if !applied.is_empty() {
            ids.insert(slot.clone(), applied);
        }
    }
}

// ItemStack.forEachModifier applies ItemAttributeModifiers.forEach, then enchantment modifiers.
fn equipment_slot_attribute_modifiers(
    world: &crate::world::World,
    stack: &ItemStack,
    slot: &EquipmentSlot,
) -> Vec<(&'static Attributes, Modifier)> {
    if stack.is_empty()
        || (stack.is_damageable()
            && !stack.is_unbreakable()
            && stack.get_data_component::<DamageImpl>().is_some()
            && stack.get_damage() >= stack.get_max_damage().unwrap_or(0))
    {
        return Vec::new();
    }
    let mut modifiers: Vec<_> = stack
        .get_data_component::<AttributeModifiersImpl>()
        .into_iter()
        .flat_map(|component| component.attribute_modifiers.iter())
        .filter(|modifier| attribute_modifier_slot_matches(&modifier.slot, slot))
        .map(|modifier| {
            (
                modifier.r#type,
                Modifier {
                    id: modifier.id.to_string(),
                    amount: modifier.amount,
                    operation: match modifier.operation {
                        Operation::AddValue => ModifierOperation::Add,
                        Operation::AddMultipliedBase => ModifierOperation::MultiplyBase,
                        Operation::AddMultipliedTotal => ModifierOperation::MultiplyTotal,
                    },
                    permanent: false,
                },
            )
        })
        .collect();
    modifiers.extend(EnchantmentHelper::equipment_attribute_modifiers(
        world, stack, slot,
    ));
    modifiers
}

fn attributes_by_id(id: u8) -> Option<&'static Attributes> {
    Attributes::ALL.iter().find(|attr| attr.id == id)
}

fn push_unique_attribute(touched: &mut Vec<Attributes>, attr: &Attributes) {
    if !touched.iter().any(|existing| existing.id == attr.id) {
        touched.push(attr.clone());
    }
}

const fn attribute_modifier_slot_matches(
    modifier_slot: &AttributeModifierSlot,
    equipment_slot: &EquipmentSlot,
) -> bool {
    match modifier_slot {
        AttributeModifierSlot::Any => true,
        AttributeModifierSlot::MainHand => matches!(equipment_slot, EquipmentSlot::MainHand(_)),
        AttributeModifierSlot::OffHand => matches!(equipment_slot, EquipmentSlot::OffHand(_)),
        AttributeModifierSlot::Hand => {
            matches!(
                equipment_slot,
                EquipmentSlot::MainHand(_) | EquipmentSlot::OffHand(_)
            )
        }
        AttributeModifierSlot::Feet => matches!(equipment_slot, EquipmentSlot::Feet(_)),
        AttributeModifierSlot::Legs => matches!(equipment_slot, EquipmentSlot::Legs(_)),
        AttributeModifierSlot::Chest => matches!(equipment_slot, EquipmentSlot::Chest(_)),
        AttributeModifierSlot::Head => matches!(equipment_slot, EquipmentSlot::Head(_)),
        // EquipmentSlotGroup.ARMOR includes both humanoid and animal armor.
        AttributeModifierSlot::Armor => equipment_slot.is_armor_slot(),
        AttributeModifierSlot::Body => matches!(equipment_slot, EquipmentSlot::Body(_)),
        AttributeModifierSlot::Saddle => matches!(equipment_slot, EquipmentSlot::Saddle(_)),
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::armor_test_world;
    use super::*;
    use crate::entity::Entity;
    use pumpkin_data::damage::DamageType;
    use pumpkin_data::item::Item;
    use pumpkin_nbt::compound::NbtCompound;
    use pumpkin_util::math::vector3::Vector3;

    #[tokio::test]
    async fn loaded_equipment_rebuilds_transient_armor_modifiers() {
        let temp = tempfile::tempdir().unwrap();
        let world = armor_test_world(temp.path());
        let living = LivingEntity::new(Entity::new(
            world.clone(),
            Vector3::default(),
            &EntityType::ZOMBIE,
        ));
        living.entity_equipment.lock().unwrap().put(
            &EquipmentSlot::CHEST,
            ItemStack::new(1, &Item::IRON_CHESTPLATE),
        );
        living.apply_current_equipment_attribute_modifiers();
        let mut saved = NbtCompound::new();
        living.write_living_nbt(&mut saved);
        let loaded = LivingEntity::new(Entity::new(world, Vector3::default(), &EntityType::ZOMBIE));
        loaded.read_living_nbt_non_mut(&saved);
        assert_eq!(loaded.get_attribute_value(&Attributes::ARMOR), 8.0);
        assert!(
            loaded.attributes.read().unwrap()[&Attributes::ARMOR.id]
                .pack()
                .is_empty()
        );
        loaded.apply_current_equipment_attribute_modifiers();
        assert_eq!(loaded.get_attribute_value(&Attributes::ARMOR), 8.0);
        loaded.get_damage_after_armor_absorb_with_weapon(
            &loaded,
            8.0,
            &DamageType::MOB_ATTACK,
            None,
        );
        assert_eq!(
            loaded
                .entity_equipment
                .lock()
                .unwrap()
                .get(&EquipmentSlot::CHEST)
                .get_damage(),
            0
        );
        loaded.apply_and_send_equipment_attribute_modifiers(&[(
            EquipmentSlot::CHEST,
            ItemStack::EMPTY.clone(),
        )]);
        assert_eq!(loaded.get_attribute_value(&Attributes::ARMOR), 2.0);
    }

    #[tokio::test]
    async fn concurrent_equipment_modifier_batches_leave_no_untracked_bonus() {
        use std::borrow::Cow;
        let temp = tempfile::tempdir().unwrap();
        let living = LivingEntity::new(Entity::new(
            armor_test_world(temp.path()),
            Vector3::default(),
            &EntityType::ZOMBIE,
        ));
        let barrier = std::sync::Barrier::new(4);
        let ids = [
            "test:armor_a",
            "test:armor_b",
            "test:armor_c",
            "test:armor_d",
        ];
        std::thread::scope(|scope| {
            for id in ids {
                let living = &living;
                let barrier = &barrier;
                scope.spawn(move || {
                    let mut stack = ItemStack::new(1, &Item::IRON_CHESTPLATE);
                    stack.set_data_component(AttributeModifiersImpl {
                        attribute_modifiers: Cow::Owned(vec![
                            pumpkin_data::data_component_impl::Modifier {
                                r#type: &Attributes::ARMOR,
                                id,
                                amount: 6.0,
                                operation: Operation::AddValue,
                                slot: AttributeModifierSlot::Chest,
                            },
                        ]),
                    });
                    for _ in 0..64 {
                        barrier.wait();
                        living.apply_equipment_attribute_modifiers(&[(
                            EquipmentSlot::CHEST,
                            stack.clone(),
                        )]);
                    }
                });
            }
        });
        let attributes = living.attributes.read().unwrap();
        let instance = &attributes[&Attributes::ARMOR.id];
        assert_eq!(instance.modifiers.len(), 1);
        assert_eq!(instance.value(), 8.0);
        let tracked = living.equipment_attribute_modifier_ids.lock().unwrap();
        assert_eq!(
            tracked[&EquipmentSlot::CHEST],
            vec![(Attributes::ARMOR.id, instance.modifiers[0].id.clone())]
        );
    }

    #[tokio::test]
    async fn mainhand_attack_modifiers_respect_their_equipment_slot() {
        use std::borrow::Cow;
        let temp = tempfile::tempdir().unwrap();
        let living = LivingEntity::new(Entity::new(
            armor_test_world(temp.path()),
            Vector3::default(),
            &EntityType::ZOMBIE,
        ));
        let mut stack = ItemStack::new(1, &Item::IRON_SWORD);
        stack.set_data_component(AttributeModifiersImpl {
            attribute_modifiers: Cow::Owned(vec![pumpkin_data::data_component_impl::Modifier {
                r#type: &Attributes::ATTACK_DAMAGE,
                id: "test:offhand_attack",
                amount: 9.0,
                operation: Operation::AddValue,
                slot: AttributeModifierSlot::OffHand,
            }]),
        });
        living.send_equipment_changes(&[(EquipmentSlot::MAIN_HAND, stack.clone())]);
        assert_eq!(living.get_attribute_value(&Attributes::ATTACK_DAMAGE), 3.0);
        living.send_equipment_changes(&[(EquipmentSlot::OFF_HAND, stack)]);
        assert_eq!(living.get_attribute_value(&Attributes::ATTACK_DAMAGE), 12.0);
        living.send_equipment_changes(&[(EquipmentSlot::MAIN_HAND, ItemStack::EMPTY.clone())]);
        assert_eq!(living.get_attribute_value(&Attributes::ATTACK_DAMAGE), 12.0);
        let mut hand_item = ItemStack::new(1, &Item::IRON_SWORD);
        hand_item.set_data_component(AttributeModifiersImpl {
            attribute_modifiers: Cow::Owned(vec![pumpkin_data::data_component_impl::Modifier {
                r#type: &Attributes::ATTACK_DAMAGE,
                id: "test:offhand_attack",
                amount: 9.0,
                operation: Operation::AddValue,
                slot: AttributeModifierSlot::Hand,
            }]),
        });
        living.send_equipment_changes(&[
            (EquipmentSlot::MAIN_HAND, hand_item),
            (EquipmentSlot::OFF_HAND, ItemStack::EMPTY.clone()),
        ]);
        assert_eq!(living.get_attribute_value(&Attributes::ATTACK_DAMAGE), 12.0);
    }

    #[tokio::test]
    async fn animal_armor_modifiers_match_the_armor_slot_group() {
        use std::borrow::Cow;
        let temp = tempfile::tempdir().unwrap();
        let living = LivingEntity::new(Entity::new(
            armor_test_world(temp.path()),
            Vector3::default(),
            &EntityType::HORSE,
        ));
        living.set_attribute_base(&Attributes::ARMOR, 12.0);
        let mut armor = ItemStack::new(1, &Item::IRON_CHESTPLATE);
        armor.set_data_component(AttributeModifiersImpl {
            attribute_modifiers: Cow::Owned(vec![pumpkin_data::data_component_impl::Modifier {
                r#type: &Attributes::ARMOR,
                id: "test:body_armor",
                amount: 0.5,
                operation: Operation::AddMultipliedTotal,
                slot: AttributeModifierSlot::Armor,
            }]),
        });
        living
            .entity_equipment
            .lock()
            .unwrap()
            .put(&EquipmentSlot::BODY, armor.clone());
        living.apply_current_equipment_attribute_modifiers();
        assert_eq!(living.get_attribute_value(&Attributes::ARMOR), 18.0);
        let damage = living.get_damage_after_armor_absorb_with_weapon(
            &living,
            20.0,
            &DamageType::MOB_ATTACK,
            None,
        );
        assert!((damage - 13.6).abs() < 1.0e-5);
        armor.set_damage(armor.get_max_damage().unwrap());
        living.apply_and_send_equipment_attribute_modifiers(&[(EquipmentSlot::BODY, armor)]);
        assert_eq!(living.get_attribute_value(&Attributes::ARMOR), 12.0);
    }
    #[tokio::test]
    async fn melee_equipment_installs_replaces_restores_and_removes_sweeping_edge() {
        use crate::entity::player::Player;
        use pumpkin_data::{Enchantment, data_component_impl::EnchantmentsImpl};
        use std::borrow::Cow;
        let temp = tempfile::tempdir().unwrap();
        let world = armor_test_world(temp.path());
        let living = LivingEntity::new(Entity::new(
            world.clone(),
            Vector3::default(),
            &EntityType::ZOMBIE,
        ));
        let mut sword = ItemStack::new(1, &Item::DIAMOND_SWORD);
        sword.set_data_component(EnchantmentsImpl {
            enchantment: Cow::Owned(vec![(&Enchantment::SWEEPING_EDGE, 3)]),
        });
        living
            .entity_equipment
            .lock()
            .unwrap()
            .put(&EquipmentSlot::MAIN_HAND, sword.clone());
        living.send_equipment_changes(&[(EquipmentSlot::MAIN_HAND, sword.clone())]);
        assert_eq!(
            living.get_attribute_value(&Attributes::SWEEPING_DAMAGE_RATIO),
            0.75
        );
        assert_eq!(living.get_attribute_value(&Attributes::ATTACK_DAMAGE), 9.0);
        assert_eq!(
            Player::sweep_damage(
                &EntityType::ZOMBIE,
                &sword,
                7.0,
                living.get_attribute_value(&Attributes::SWEEPING_DAMAGE_RATIO) as f32,
                1.0
            ),
            6.25
        );

        let mut saved = NbtCompound::new();
        living.write_living_nbt(&mut saved);
        let loaded = LivingEntity::new(Entity::new(world, Vector3::default(), &EntityType::ZOMBIE));
        loaded.read_living_nbt_non_mut(&saved);
        assert_eq!(
            loaded.get_attribute_value(&Attributes::SWEEPING_DAMAGE_RATIO),
            0.75
        );
        loaded.apply_current_equipment_attribute_modifiers();
        assert_eq!(
            loaded.get_attribute_value(&Attributes::SWEEPING_DAMAGE_RATIO),
            0.75
        );

        sword.set_data_component(EnchantmentsImpl {
            enchantment: Cow::Owned(vec![(&Enchantment::SWEEPING_EDGE, 1)]),
        });
        let touched = loaded
            .apply_equipment_attribute_modifiers(&[(EquipmentSlot::MAIN_HAND, sword.clone())]);
        assert!(touched.contains(&Attributes::SWEEPING_DAMAGE_RATIO));
        assert_eq!(
            loaded.get_attribute_value(&Attributes::SWEEPING_DAMAGE_RATIO),
            0.5
        );
        sword.set_damage(sword.get_max_damage().unwrap());
        loaded.send_equipment_changes(&[(EquipmentSlot::MAIN_HAND, sword)]);
        assert_eq!(
            loaded.get_attribute_value(&Attributes::SWEEPING_DAMAGE_RATIO),
            0.0
        );
        assert_eq!(loaded.get_attribute_value(&Attributes::ATTACK_DAMAGE), 3.0);
        let sword = living
            .entity_equipment
            .lock()
            .unwrap()
            .get(&EquipmentSlot::MAIN_HAND);
        living.send_equipment_changes(&[
            (EquipmentSlot::OFF_HAND, sword),
            (EquipmentSlot::MAIN_HAND, ItemStack::EMPTY.clone()),
        ]);
        assert_eq!(
            living.get_attribute_value(&Attributes::SWEEPING_DAMAGE_RATIO),
            0.0
        );
    }

    #[tokio::test]
    async fn melee_enchantment_only_equipment_does_not_require_attribute_component() {
        use pumpkin_data::{Enchantment, data_component_impl::EnchantmentsImpl};
        use std::borrow::Cow;
        let temp = tempfile::tempdir().unwrap();
        let living = LivingEntity::new(Entity::new(
            armor_test_world(temp.path()),
            Vector3::default(),
            &EntityType::ZOMBIE,
        ));
        let mut sword = ItemStack::new(1, &Item::DIAMOND_SWORD);
        sword
            .remove_data_component(pumpkin_data::data_component::DataComponent::AttributeModifiers);
        sword.set_data_component(EnchantmentsImpl {
            enchantment: Cow::Owned(vec![(&Enchantment::SWEEPING_EDGE, 1)]),
        });
        living.send_equipment_changes(&[(EquipmentSlot::MAIN_HAND, sword)]);
        assert_eq!(
            living.get_attribute_value(&Attributes::SWEEPING_DAMAGE_RATIO),
            0.5
        );
        living.send_equipment_changes(&[(EquipmentSlot::MAIN_HAND, ItemStack::EMPTY.clone())]);
        assert_eq!(
            living.get_attribute_value(&Attributes::SWEEPING_DAMAGE_RATIO),
            0.0
        );
    }
}
