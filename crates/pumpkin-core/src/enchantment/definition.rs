use crate::entity::attributes::{Modifier, ModifierOperation};
use crate::world::World;
use pumpkin_data::{
    attributes::Attributes, data_component_impl::EquipmentSlot, enchantment::Enchantment,
    item_stack::ItemStack,
};
use pumpkin_nbt::{Nbt, NbtCompound, deserializer::NbtReadHelperJava};
use rustc_hash::FxHashMap;
use std::{borrow::Cow, sync::OnceLock};

use super::conditions::number_provider;
use super::helper::EnchantmentHelper;

impl EnchantmentHelper {
    /// Reads transient enchantment attribute modifiers for an equipped stack.
    /// Equipment updates must apply these and remove their previous IDs when the
    /// stack changes. IDs include the slot, as in EnchantmentAttributeEffect.getModifier.
    pub fn equipment_attribute_modifiers(
        world: &World,
        stack: &ItemStack,
        slot: &EquipmentSlot,
    ) -> Vec<(&'static Attributes, Modifier)> {
        equipment_attribute_modifiers_from_registry(Some(world), stack, slot)
    }
}

// The generated effect wrappers omit requirements. The synced vanilla registry
// retains them; cache its decoded entries rather than copying game-data values.
pub(super) fn vanilla_enchantment_definitions() -> &'static FxHashMap<&'static str, NbtCompound> {
    static DEFINITIONS: OnceLock<FxHashMap<&'static str, NbtCompound>> = OnceLock::new();
    DEFINITIONS.get_or_init(|| {
        pumpkin_data::registry::REGISTRY_V_26_3
            .iter()
            .find(|registry| registry.registry_id == "enchantment")
            .into_iter()
            .flat_map(|registry| registry.entries)
            .filter_map(|entry| {
                Nbt::read_unnamed(&mut NbtReadHelperJava::new(std::io::Cursor::new(
                    entry.data,
                )))
                .ok()
                .map(|nbt| (entry.name, nbt.root_tag))
            })
            .collect()
    })
}

/// Reads an enchantment definition from the world registry, falling back to bundled game data.
pub fn enchantment_definition<'a>(
    world: Option<&World>,
    enchantment: &Enchantment,
) -> Option<Cow<'a, NbtCompound>> {
    if let Some(server) = world.and_then(|world| world.server.upgrade())
        && let Some(entry) = server
            .datapack_manager
            .get_custom_registry_entry("enchantment", enchantment.name)
        && let Some(bytes) = entry.data
    {
        return Nbt::read_unnamed(&mut NbtReadHelperJava::new(std::io::Cursor::new(
            bytes.as_ref(),
        )))
        .ok()
        .map(|nbt| Cow::Owned(nbt.root_tag));
    }
    vanilla_enchantment_definitions()
        .get(enchantment.registry_key)
        .map(Cow::Borrowed)
}

// EnchantmentAttributeEffect.getModifier; amounts and operations come from game data.
pub(super) fn attribute_modifiers_from_definition(
    definition: &NbtCompound,
    level: i32,
    slot: &EquipmentSlot,
) -> Vec<(&'static Attributes, Modifier)> {
    definition
        .get_compound("effects")
        .and_then(|effects| effects.get_list("minecraft:attributes"))
        .into_iter()
        .flatten()
        .filter_map(|effect| {
            let effect = effect.extract_compound()?;
            let attribute_name = effect.get_string("attribute")?;
            let attribute = Attributes::ALL
                .iter()
                .find(|attribute| attribute.name == attribute_name)?;
            let id = format!("{}/{}", effect.get_string("id")?, slot.to_name());
            let amount = f64::from(number_provider(effect.get("amount")?, level)?);
            let operation = ModifierOperation::from_name(effect.get_string("operation")?)?;
            Some((
                attribute,
                Modifier {
                    id,
                    amount,
                    operation,
                    permanent: false,
                },
            ))
        })
        .collect()
}

// Enchantment.matchingSlot evaluates the same definition as its effect payloads.
/// Tests an equipment slot against the enchantment definition's slot groups.
pub fn definition_matches_slot(definition: &NbtCompound, slot: &EquipmentSlot) -> bool {
    definition.get_list("slots").is_some_and(|slots| {
        slots.iter().any(|group| match group.extract_string() {
            Some("any") => true,
            Some("hand") => matches!(slot, EquipmentSlot::MainHand(_) | EquipmentSlot::OffHand(_)),
            Some("armor") => slot.is_armor_slot(),
            Some(name) => name == slot.to_name(),
            None => false,
        })
    })
}

// Also used when rebuilding equipment without a server-backed registry.
pub fn equipment_attribute_modifiers_from_registry(
    world: Option<&World>,
    stack: &ItemStack,
    slot: &EquipmentSlot,
) -> Vec<(&'static Attributes, Modifier)> {
    let mut modifiers = Vec::new();
    if stack.is_empty() {
        return modifiers;
    }
    EnchantmentHelper::run_iteration_on_item(stack, |enchantment, level| {
        if let Some(definition) = enchantment_definition(world, enchantment)
            && definition_matches_slot(&definition, slot)
        {
            modifiers.extend(attribute_modifiers_from_definition(
                &definition,
                level,
                slot,
            ));
        }
    });
    modifiers
}
