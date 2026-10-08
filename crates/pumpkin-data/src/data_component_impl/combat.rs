use super::Digest;
use crate::Block;
use crate::Enchantment;
use crate::attributes::Attributes;
use crate::damage::DamageType;
use crate::data_component_impl::basic::SoundEvent;
use crate::data_component_impl::food::StatusEffectInstance;
use crate::data_component_impl::{
    DataComponentImpl, EquipmentSlot, IDSet, IDSetContent, IdOr, get_f32_hash, get_i32_hash,
    get_idor, get_idor_hash, get_idset_hash, get_str_hash, put_idor,
};
use crate::effect::StatusEffect;
use crate::entity_type::EntityType;
use crate::item::Item;
use crate::item_stack::ItemStack;
use crate::sound::Sound;
use crate::tag::Taggable;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::tag::NbtTag;
use std::borrow::Cow;
use std::hash::Hash;

/// Maximum accepted nesting of hidden death-protection status effects on disk and the wire.
pub const MAX_DEATH_STATUS_EFFECT_DEPTH: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Operation {
    AddValue,
    AddMultipliedBase,
    AddMultipliedTotal,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Modifier {
    pub r#type: &'static Attributes,
    pub id: &'static str,
    pub amount: f64,
    pub operation: Operation,
    pub slot: crate::AttributeModifierSlot,
}
impl Hash for Modifier {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.r#type.hash(state);
        self.id.hash(state);
        unsafe { (*(&raw const self.amount).cast::<u64>()).hash(state) };
        self.operation.hash(state);
        self.slot.hash(state);
    }
}

#[derive(Clone, Debug, Hash, PartialEq)]
pub struct AttributeModifiersImpl {
    pub attribute_modifiers: Cow<'static, [Modifier]>,
}
impl AttributeModifiersImpl {
    pub const fn read_data(_data: &NbtTag) -> Option<Self> {
        Some(Self {
            attribute_modifiers: Cow::Borrowed(&[]),
        })
    }
}
impl DataComponentImpl for AttributeModifiersImpl {
    default_impl!(AttributeModifiers);
}

#[derive(Clone, Hash, PartialEq, Eq, Default)]
pub struct EnchantmentsImpl {
    pub enchantment: Cow<'static, [(&'static Enchantment, i32)]>,
}
impl EnchantmentsImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let compound = data.extract_compound()?;
        let data = if let Some(NbtTag::Compound(levels)) = compound.child_tags.get("levels") {
            &levels.child_tags
        } else {
            &compound.child_tags
        };
        let mut enc = Vec::with_capacity(data.len());
        for (name, level) in data {
            let enchantment = Enchantment::from_name(name.as_ref())
                .or_else(|| Enchantment::from_name(&format!("minecraft:{name}")))?;
            enc.push((enchantment, level.extract_int()?));
        }
        Some(Self {
            enchantment: Cow::from(enc),
        })
    }
}
impl DataComponentImpl for EnchantmentsImpl {
    fn write_data(&self) -> NbtTag {
        let mut data = NbtCompound::new();
        for (enc, level) in self.enchantment.iter() {
            data.put_int(enc.name, *level);
        }
        NbtTag::Compound(data)
    }
    fn get_hash(&self) -> i32 {
        let mut digest = Digest::new();
        digest.update(&[2u8]);
        for (enc, level) in self.enchantment.iter() {
            digest.update(&get_str_hash(enc.name).to_le_bytes());
            digest.update(&get_i32_hash(*level).to_le_bytes());
        }
        digest.update(&[3u8]);
        digest.finalize() as i32
    }
    default_impl!(Enchantments);
}

/// An adventure-mode block predicate, kept as its raw NBT (a compound or list)
/// since Pumpkin does not yet model block predicates.
// TODO: replace `predicate` with a typed block predicate once block predicates are modelled.
#[derive(Clone, Debug, PartialEq)]
pub struct CanPlaceOnImpl {
    pub predicate: NbtTag,
}
impl CanPlaceOnImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        Some(Self {
            predicate: data.clone(),
        })
    }
}
impl DataComponentImpl for CanPlaceOnImpl {
    fn write_data(&self) -> NbtTag {
        self.predicate.clone()
    }
    default_impl!(CanPlaceOn);
}

/// An adventure-mode block predicate, kept as its raw NBT (a compound or list)
/// since Pumpkin does not yet model block predicates.
// TODO: replace `predicate` with a typed block predicate once block predicates are modelled.
#[derive(Clone, Debug, PartialEq)]
pub struct CanBreakImpl {
    pub predicate: NbtTag,
}
impl CanBreakImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        Some(Self {
            predicate: data.clone(),
        })
    }
}
impl DataComponentImpl for CanBreakImpl {
    fn write_data(&self) -> NbtTag {
        self.predicate.clone()
    }
    default_impl!(CanBreak);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, Default)]
pub struct RepairCostImpl {
    pub cost: i32,
}
impl RepairCostImpl {
    pub const DEFAULT: Self = Self { cost: 0 };

    pub fn read_data(data: &NbtTag) -> Option<Self> {
        Some(Self {
            cost: data.extract_int()?,
        })
    }
}
impl DataComponentImpl for RepairCostImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::Int(self.cost)
    }
    default_impl!(RepairCost);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct IntangibleProjectileImpl;
impl IntangibleProjectileImpl {
    pub const fn read_data(_data: &NbtTag) -> Option<Self> {
        Some(Self)
    }
}
impl DataComponentImpl for IntangibleProjectileImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::Compound(NbtCompound::new())
    }
    default_impl!(IntangibleProjectile);
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum DamageResistantType {
    AlwaysHurtsEnderDragons,
    AlwaysKillsArmorStands,
    AlwaysMostSignificantFall,
    AlwaysTriggersSilverfish,
    AvoidsGuardianThorns,
    BurnsArmorStands,
    BurnFromStepping,
    BypassesArmor,
    BypassesCooldown,
    BypassesEffects,
    BypassesEnchantments,
    BypassesInvulnerability,
    BypassesResistance,
    BypassesShield,
    BypassesWolfArmor,
    CanBreakArmorStands,
    DamagesHelmet,
    IgnitesArmorStands,
    Drowning,
    Explosion,
    Fall,
    Fire,
    Freezing,
    Lightning,
    PlayerAttack,
    Projectile,
    MaceSmash,
    NoAnger,
    NoImpact,
    NoKnockback,
    PanicCauses,
    PanicEnvironmentalCauses,
    WitchResistantTo,
    WitherImmuneTo,
    Generic,
}

impl DamageResistantType {
    pub fn from_tag(s: &str) -> Self {
        match s {
            "#minecraft:always_hurts_ender_dragons"
            | "minecraft:always_hurts_ender_dragons"
            | "always_hurts_ender_dragons" => Self::AlwaysHurtsEnderDragons,
            "#minecraft:always_kills_armor_stands"
            | "minecraft:always_kills_armor_stands"
            | "always_kills_armor_stands" => Self::AlwaysKillsArmorStands,
            "#minecraft:always_most_significant_fall"
            | "minecraft:always_most_significant_fall"
            | "always_most_significant_fall" => Self::AlwaysMostSignificantFall,
            "#minecraft:always_triggers_silverfish"
            | "minecraft:always_triggers_silverfish"
            | "always_triggers_silverfish" => Self::AlwaysTriggersSilverfish,
            "#minecraft:avoids_guardian_thorns"
            | "minecraft:avoids_guardian_thorns"
            | "avoids_guardian_thorns" => Self::AvoidsGuardianThorns,
            "#minecraft:burns_armor_stands"
            | "minecraft:burns_armor_stands"
            | "burns_armor_stands" => Self::BurnsArmorStands,
            "#minecraft:burn_from_stepping"
            | "minecraft:burn_from_stepping"
            | "burn_from_stepping" => Self::BurnFromStepping,
            "#minecraft:bypasses_armor" | "minecraft:bypasses_armor" | "bypasses_armor" => {
                Self::BypassesArmor
            }
            "#minecraft:bypasses_cooldown"
            | "minecraft:bypasses_cooldown"
            | "bypasses_cooldown" => Self::BypassesCooldown,
            "#minecraft:bypasses_effects" | "minecraft:bypasses_effects" | "bypasses_effects" => {
                Self::BypassesEffects
            }
            "#minecraft:bypasses_enchantments"
            | "minecraft:bypasses_enchantments"
            | "bypasses_enchantments" => Self::BypassesEnchantments,
            "#minecraft:bypasses_invulnerability"
            | "minecraft:bypasses_invulnerability"
            | "bypasses_invulnerability" => Self::BypassesInvulnerability,
            "#minecraft:bypasses_resistance"
            | "minecraft:bypasses_resistance"
            | "bypasses_resistance" => Self::BypassesResistance,
            "#minecraft:bypasses_shield" | "minecraft:bypasses_shield" | "bypasses_shield" => {
                Self::BypassesShield
            }
            "#minecraft:bypasses_wolf_armor"
            | "minecraft:bypasses_wolf_armor"
            | "bypasses_wolf_armor" => Self::BypassesWolfArmor,
            "#minecraft:can_break_armor_stand"
            | "minecraft:can_break_armor_stand"
            | "can_break_armor_stand" => Self::CanBreakArmorStands,
            "#minecraft:damages_helmet" | "minecraft:damages_helmet" | "damages_helmet" => {
                Self::DamagesHelmet
            }
            "#minecraft:ignites_armor_stands"
            | "minecraft:ignites_armor_stands"
            | "ignites_armor_stands" => Self::IgnitesArmorStands,
            "#minecraft:is_drowning" | "minecraft:is_drowning" | "is_drowning" => Self::Drowning,
            "#minecraft:is_explosion" | "minecraft:is_explosion" | "is_explosion" | "explosion" => {
                Self::Explosion
            }
            "#minecraft:is_fall" | "minecraft:is_fall" | "is_fall" | "fall" => Self::Fall,
            "#minecraft:is_fire" | "minecraft:is_fire" | "is_fire" | "fire" | "in_fire"
            | "minecraft:in_fire" => Self::Fire,
            "#minecraft:is_freezing" | "minecraft:is_freezing" | "is_freezing" => Self::Freezing,
            "#minecraft:is_lightning" | "minecraft:is_lightning" | "is_lightning" => {
                Self::Lightning
            }
            "#minecraft:is_player_attack" | "minecraft:is_player_attack" | "is_player_attack" => {
                Self::PlayerAttack
            }
            "#minecraft:is_projectile" | "minecraft:is_projectile" | "is_projectile" => {
                Self::Projectile
            }
            "#minecraft:mace_smash" | "minecraft:mace_smash" | "mace_smash" => Self::MaceSmash,
            "#minecraft:no_anger" | "minecraft:no_anger" | "no_anger" => Self::NoAnger,
            "#minecraft:no_impact" | "minecraft:no_impact" | "no_impact" => Self::NoImpact,
            "#minecraft:no_knockback" | "minecraft:no_knockback" | "no_knockback" => {
                Self::NoKnockback
            }
            "#minecraft:panic_causes" | "minecraft:panic_causes" | "panic_causes" => {
                Self::PanicCauses
            }
            "#minecraft:panic_environmental_causes"
            | "minecraft:panic_environmental_causes"
            | "panic_environmental_causes" => Self::PanicEnvironmentalCauses,
            "#minecraft:witch_resistant_to"
            | "minecraft:witch_resistant_to"
            | "witch_resistant_to" => Self::WitchResistantTo,
            "#minecraft:wither_immune_to" | "minecraft:wither_immune_to" | "wither_immune_to" => {
                Self::WitherImmuneTo
            }
            _ => Self::Generic,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AlwaysHurtsEnderDragons => "#minecraft:always_hurts_ender_dragons",
            Self::AlwaysKillsArmorStands => "#minecraft:always_kills_armor_stands",
            Self::AlwaysMostSignificantFall => "#minecraft:always_most_significant_fall",
            Self::AlwaysTriggersSilverfish => "#minecraft:always_triggers_silverfish",
            Self::AvoidsGuardianThorns => "#minecraft:avoids_guardian_thorns",
            Self::BurnsArmorStands => "#minecraft:burns_armor_stands",
            Self::BurnFromStepping => "#minecraft:burn_from_stepping",
            Self::BypassesArmor => "#minecraft:bypasses_armor",
            Self::BypassesCooldown => "#minecraft:bypasses_cooldown",
            Self::BypassesEffects => "#minecraft:bypasses_effects",
            Self::BypassesEnchantments => "#minecraft:bypasses_enchantments",
            Self::BypassesInvulnerability => "#minecraft:bypasses_invulnerability",
            Self::BypassesResistance => "#minecraft:bypasses_resistance",
            Self::BypassesShield => "#minecraft:bypasses_shield",
            Self::BypassesWolfArmor => "#minecraft:bypasses_wolf_armor",
            Self::CanBreakArmorStands => "#minecraft:can_break_armor_stand",
            Self::DamagesHelmet => "#minecraft:damages_helmet",
            Self::IgnitesArmorStands => "#minecraft:ignites_armor_stands",
            Self::Drowning => "#minecraft:is_drowning",
            Self::Explosion => "#minecraft:is_explosion",
            Self::Fall => "#minecraft:is_fall",
            Self::Fire => "#minecraft:is_fire",
            Self::Freezing => "#minecraft:is_freezing",
            Self::Lightning => "#minecraft:is_lightning",
            Self::PlayerAttack => "#minecraft:is_player_attack",
            Self::Projectile => "#minecraft:is_projectile",
            Self::MaceSmash => "#minecraft:mace_smash",
            Self::NoAnger => "#minecraft:no_anger",
            Self::NoImpact => "#minecraft:no_impact",
            Self::NoKnockback => "#minecraft:no_knockback",
            Self::PanicCauses => "#minecraft:panic_causes",
            Self::PanicEnvironmentalCauses => "#minecraft:panic_environmental_causes",
            Self::WitchResistantTo => "#minecraft:witch_resistant_to",
            Self::WitherImmuneTo => "#minecraft:wither_immune_to",
            Self::Generic => "minecraft:generic",
        }
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub struct DamageResistantImpl {
    pub res_type: DamageResistantType,
}
impl DamageResistantImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let compound = data.extract_compound()?;
        let type_str = compound.get_string("types")?;
        Some(Self {
            res_type: DamageResistantType::from_tag(type_str),
        })
    }
}
impl std::str::FromStr for DamageResistantType {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(DamageResistantType::from_tag(s))
    }
}
impl DataComponentImpl for DamageResistantImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        compound.put_string("types", self.res_type.as_str().to_string());
        NbtTag::Compound(compound)
    }
    fn get_hash(&self) -> i32 {
        get_str_hash(self.res_type.as_str()) as i32
    }
    default_impl!(DamageResistant);
}

#[derive(Clone, PartialEq)]
pub struct ToolRule {
    pub blocks: IDSet<Block>,
    pub speed: Option<f32>,
    pub correct_for_drops: Option<bool>,
}
impl Hash for ToolRule {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.blocks.hash(state);
        if let Some(val) = self.speed {
            true.hash(state);
            unsafe { (*(&raw const val).cast::<u32>()).hash(state) };
        } else {
            false.hash(state);
        }
        self.correct_for_drops.hash(state);
    }
}

#[derive(Clone, PartialEq)]
pub struct ToolImpl {
    pub rules: Cow<'static, [ToolRule]>,
    pub default_mining_speed: f32,
    pub damage_per_block: u32,
    pub can_destroy_blocks_in_creative: bool,
}
impl ToolImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let compound = data.extract_compound()?;
        let mut rules = Vec::new();
        if let Some(list) = compound.get_list("rules") {
            for rule_tag in list {
                if let Some(rule_compound) = rule_tag.extract_compound()
                    && let Some(blocks_tag) = rule_compound.get("blocks")
                    && let Some(blocks) = IDSet::<Block>::read(blocks_tag)
                {
                    rules.push(ToolRule {
                        blocks,
                        speed: rule_compound.get_float("speed"),
                        correct_for_drops: rule_compound.get_bool("correct_for_drops"),
                    });
                }
            }
        }
        let default_mining_speed = compound.get_float("default_mining_speed").unwrap_or(1.0);
        let damage_per_block = compound.get_int("damage_per_block").unwrap_or(1).max(0) as u32;
        let can_destroy_blocks_in_creative = compound
            .get_bool("can_destroy_blocks_in_creative")
            .unwrap_or(true);
        Some(Self {
            rules: Cow::Owned(rules),
            default_mining_speed,
            damage_per_block,
            can_destroy_blocks_in_creative,
        })
    }
}
impl DataComponentImpl for ToolImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        let mut rules_list = Vec::new();
        for rule in self.rules.iter() {
            let mut rule_compound = NbtCompound::new();
            rule.blocks.write(&mut rule_compound, "blocks");
            if let Some(speed) = rule.speed {
                rule_compound.put_float("speed", speed);
            }
            if let Some(correct_for_drops) = rule.correct_for_drops {
                rule_compound.put_bool("correct_for_drops", correct_for_drops);
            }
            rules_list.push(NbtTag::Compound(rule_compound));
        }
        compound.put_list("rules", rules_list);
        compound.put_float("default_mining_speed", self.default_mining_speed);
        compound.put_int("damage_per_block", self.damage_per_block as i32);
        compound.put_bool(
            "can_destroy_blocks_in_creative",
            self.can_destroy_blocks_in_creative,
        );
        NbtTag::Compound(compound)
    }
    default_impl!(Tool);
}
impl Hash for ToolImpl {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.rules.hash(state);
        unsafe { (*(&raw const self.default_mining_speed).cast::<u32>()).hash(state) };
        self.damage_per_block.hash(state);
        self.can_destroy_blocks_in_creative.hash(state);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WeaponImpl {
    pub item_damage_per_attack: u32,
    pub disable_blocking_for_seconds: f32,
}
impl WeaponImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let compound = data.extract_compound()?;
        // Weapon.CODEC rejects negative durability and disable durations.
        let item_damage_per_attack =
            u32::try_from(read_combat_int(compound, "item_damage_per_attack", 1)?).ok()?;
        Some(Self {
            item_damage_per_attack,
            disable_blocking_for_seconds: read_nonnegative_combat_float(
                compound,
                "disable_blocking_for_seconds",
                Some(0.0),
            )?,
        })
    }
}
impl DataComponentImpl for WeaponImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        compound.put_int("item_damage_per_attack", self.item_damage_per_attack as i32);
        compound.put_float(
            "disable_blocking_for_seconds",
            self.disable_blocking_for_seconds,
        );
        NbtTag::Compound(compound)
    }
    fn get_hash(&self) -> i32 {
        let mut digest = Digest::new();
        digest.update(&get_i32_hash(self.item_damage_per_attack as i32).to_le_bytes());
        digest.update(&get_f32_hash(self.disable_blocking_for_seconds).to_le_bytes());
        digest.finalize() as i32
    }
    default_impl!(Weapon);
}
impl Hash for WeaponImpl {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.item_damage_per_attack.hash(state);
        self.disable_blocking_for_seconds.to_bits().hash(state);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AttackRangeImpl {
    pub min_reach: f32,
    pub max_reach: f32,
    pub min_creative_reach: f32,
    pub max_creative_reach: f32,
    pub hitbox_margin: f32,
    pub mob_factor: f32,
}
impl AttackRangeImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let compound = data.extract_compound()?;
        Some(Self {
            min_reach: compound.get_float("min_reach").unwrap_or(0.0),
            max_reach: compound.get_float("max_reach").unwrap_or(3.0),
            min_creative_reach: compound.get_float("min_creative_reach").unwrap_or(0.0),
            max_creative_reach: compound.get_float("max_creative_reach").unwrap_or(5.0),
            hitbox_margin: compound.get_float("hitbox_margin").unwrap_or(0.3),
            mob_factor: compound.get_float("mob_factor").unwrap_or(1.0),
        })
    }
    fn values(&self) -> [f32; 6] {
        [
            self.min_reach,
            self.max_reach,
            self.min_creative_reach,
            self.max_creative_reach,
            self.hitbox_margin,
            self.mob_factor,
        ]
    }
}
impl DataComponentImpl for AttackRangeImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        compound.put_float("min_reach", self.min_reach);
        compound.put_float("max_reach", self.max_reach);
        compound.put_float("min_creative_reach", self.min_creative_reach);
        compound.put_float("max_creative_reach", self.max_creative_reach);
        compound.put_float("hitbox_margin", self.hitbox_margin);
        compound.put_float("mob_factor", self.mob_factor);
        NbtTag::Compound(compound)
    }
    fn get_hash(&self) -> i32 {
        let mut digest = Digest::new();
        for value in self.values() {
            digest.update(&get_f32_hash(value).to_le_bytes());
        }
        digest.finalize() as i32
    }
    default_impl!(AttackRange);
}
impl Hash for AttackRangeImpl {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        for value in self.values() {
            value.to_bits().hash(state);
        }
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct EnchantableImpl {
    pub value: i32,
}
impl EnchantableImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let compound = data.extract_compound()?;
        let value = compound.get_int("value")?;
        Some(Self { value })
    }
}
impl DataComponentImpl for EnchantableImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        compound.put_int("value", self.value);
        NbtTag::Compound(compound)
    }
    default_impl!(Enchantable);
}

#[derive(Clone, Hash, PartialEq)]
pub struct EquippableImpl {
    pub slot: &'static EquipmentSlot,
    pub equip_sound: IdOr<SoundEvent>,
    pub asset_id: Option<Cow<'static, str>>,
    pub camera_overlay: Option<Cow<'static, str>>,
    pub allowed_entities: Option<IDSet<EntityType>>,
    pub dispensable: bool,
    pub swappable: bool,
    pub damage_on_hurt: bool,
    pub equip_on_interact: bool,
    pub can_be_sheared: bool,
    pub shearing_sound: IdOr<SoundEvent>,
}
impl EquippableImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let compound = data.extract_compound()?;
        let slot = EquipmentSlot::get_from_name(compound.get_string("slot")?)?;
        let asset_id = compound
            .get_string("asset_id")
            .map(|str| Cow::Owned(str.to_owned()));
        let camera_overlay = compound
            .get_string("camera_overlay")
            .map(|str| Cow::Owned(str.to_owned()));
        let dispensable = compound.get_bool("dispensable").unwrap_or(true);
        let swappable = compound.get_bool("swappable").unwrap_or(true);
        let damage_on_hurt = compound.get_bool("damage_on_hurt").unwrap_or(true);
        let equip_on_interact = compound.get_bool("equip_on_interact").unwrap_or(false);
        let can_be_sheared = compound.get_bool("can_be_sheared").unwrap_or(false);
        let equip_sound = get_idor(compound, "equip_sound", Sound::ItemArmorEquipGeneric);
        let shearing_sound = get_idor(compound, "shearing_sound_sound", Sound::ItemShearsSnip);
        let allowed_entities = if let Some(nbt) = compound.get("allowed_entities") {
            IDSet::<EntityType>::read(nbt)
        } else {
            None
        };
        Some(Self {
            slot,
            equip_sound,
            asset_id,
            camera_overlay,
            allowed_entities,
            dispensable,
            swappable,
            damage_on_hurt,
            equip_on_interact,
            can_be_sheared,
            shearing_sound,
        })
    }
}
impl DataComponentImpl for EquippableImpl {
    fn get_hash(&self) -> i32 {
        let mut digest = Digest::new();
        digest.update(&[16u8]);
        digest.update(&get_i32_hash(self.slot.get_slot_index()).to_le_bytes());
        digest.update(&get_idor_hash(&self.equip_sound).to_le_bytes());
        if let Some(asset) = &self.asset_id {
            digest.update(&[1u8]);
            digest.update(&get_str_hash(asset).to_le_bytes());
        }
        if let Some(overlay) = &self.camera_overlay {
            digest.update(&[2u8]);
            digest.update(&get_str_hash(overlay).to_le_bytes());
        }
        if let Some(allowed_entities) = &self.allowed_entities {
            digest.update(&[3u8]);
            digest.update(&get_idset_hash(allowed_entities).to_le_bytes());
        }
        digest.update(&[self.dispensable as u8]);
        digest.update(&[self.swappable as u8]);
        digest.update(&[self.damage_on_hurt as u8]);
        digest.update(&[self.equip_on_interact as u8]);
        digest.update(&[self.can_be_sheared as u8]);
        digest.update(&get_idor_hash(&self.shearing_sound).to_le_bytes());
        digest.finalize() as i32
    }
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        compound.put_string("slot", self.slot.to_name().to_string());
        put_idor(&mut compound, "equip_sound", &self.equip_sound);
        put_idor(&mut compound, "shearing_sound", &self.shearing_sound);
        if let Some(asset_id) = &self.asset_id {
            compound.put_string("asset_id", asset_id.to_string());
        }
        if let Some(camera_overlay) = &self.camera_overlay {
            compound.put_string("camera_overlay", camera_overlay.to_string());
        }
        if let Some(allowed_entities) = &self.allowed_entities {
            allowed_entities.write(&mut compound, "allowed_entities");
        }
        compound.put_bool("dispensable", self.dispensable);
        compound.put_bool("swappable", self.swappable);
        compound.put_bool("damage_on_hurt", self.damage_on_hurt);
        compound.put_bool("equip_on_interact", self.equip_on_interact);
        compound.put_bool("can_be_sheared", self.can_be_sheared);
        NbtTag::Compound(compound)
    }
    default_impl!(Equippable);
}

#[derive(Clone, Debug, PartialEq)]
pub struct RepairableImpl {
    pub items: IDSet<Item>,
}

impl Hash for RepairableImpl {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        get_idset_hash(&self.items).hash(state);
    }
}

impl RepairableImpl {
    #[must_use]
    pub fn is_valid_repair_item(&self, repair_item: &ItemStack) -> bool {
        if repair_item.is_empty() {
            return false;
        }
        match &self.items {
            IDSet::Tag(tag) => repair_item.item.is_tagged_with(tag).unwrap_or(false),
            IDSet::IDs(items) => items.iter().any(|item| item.id == repair_item.item.id),
        }
    }

    pub fn read_data(data: &NbtTag) -> Option<Self> {
        if let NbtTag::Compound(c) = data {
            let items_tag = c.get("items")?;
            let items = IDSet::read(items_tag)?;
            Some(Self { items })
        } else if let Some(items) = IDSet::read(data) {
            Some(Self { items })
        } else {
            None
        }
    }
}

impl DataComponentImpl for RepairableImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        self.items.write(&mut compound, "items");
        NbtTag::Compound(compound)
    }

    default_impl!(Repairable);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct GliderImpl;
impl GliderImpl {
    pub const fn read_data(_data: &NbtTag) -> Option<Self> {
        Some(Self)
    }
}
impl DataComponentImpl for GliderImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::Compound(NbtCompound::new())
    }
    default_impl!(Glider);
}

#[derive(Clone, Debug, Hash, PartialEq)]
pub struct DeathProtectionImpl {
    pub death_effects: Cow<'static, [DeathEffect]>,
}
impl DeathProtectionImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let compound = data.extract_compound()?;
        let death_effects = match compound.get("death_effects") {
            Some(effects) => effects
                .extract_list()?
                .iter()
                .map(DeathEffect::read_data)
                .collect::<Option<Vec<_>>>()?,
            None => Vec::new(),
        };
        Some(Self {
            death_effects: Cow::Owned(death_effects),
        })
    }
}
impl DataComponentImpl for DeathProtectionImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        compound.put_list(
            "death_effects",
            self.death_effects.iter().map(DeathEffect::as_nbt).collect(),
        );
        NbtTag::Compound(compound)
    }
    default_impl!(DeathProtection);
}

/// Consume effects retained by DeathProtection, including teleport and hidden-effect parameters.
#[derive(Clone, Debug, PartialEq)]
pub enum DeathEffect {
    ApplyEffects(Cow<'static, [DeathStatusEffect]>, f32),
    RemoveEffects(IDSet<StatusEffect>),
    ClearAllEffects,
    TeleportRandomly {
        diameter: f32,
        directional_particles: bool,
    },
    PlaySound(IdOr<SoundEvent>),
}

#[derive(Clone, Debug, Hash, PartialEq)]
pub struct DeathStatusEffect {
    pub effect: StatusEffectInstance,
    pub hidden_effect: Option<HiddenDeathEffect>,
}
/// Hidden details can borrow generated constants or own runtime component data.
#[derive(Clone, Debug)]
pub enum HiddenDeathEffect {
    Static(&'static DeathStatusEffect),
    Owned(Box<DeathStatusEffect>),
}
impl std::ops::Deref for HiddenDeathEffect {
    type Target = DeathStatusEffect;
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Static(effect) => effect,
            Self::Owned(effect) => effect,
        }
    }
}
impl PartialEq for HiddenDeathEffect {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}
impl Hash for HiddenDeathEffect {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (**self).hash(state);
    }
}

impl DeathStatusEffect {
    /// Decodes vanilla effect details and their bounded hidden chain from NBT.
    #[must_use]
    pub fn from_nbt(data: &NbtTag) -> Option<Self> {
        Self::read_data(data, None, 0)
    }

    // MobEffectInstance.CODEC and Details.MAP_CODEC, bounded against hostile NBT nesting.
    fn read_data(data: &NbtTag, inherited_id: Option<&str>, depth: usize) -> Option<Self> {
        if depth > MAX_DEATH_STATUS_EFFECT_DEPTH {
            return None;
        }
        let data = data.extract_compound()?;
        let id = inherited_id.or_else(|| data.get_string("id"))?;
        let name = if id.contains(':') {
            id.to_string()
        } else {
            format!("minecraft:{id}")
        };
        let effect_type = StatusEffect::from_minecraft_name(&name)?;
        let id = effect_type.minecraft_name;
        let particles = read_combat_bool(data, "show_particles", true)?;
        let hidden_effect = match data.get("hidden_effect") {
            Some(hidden) => Some(HiddenDeathEffect::Owned(Box::new(Self::read_data(
                hidden,
                Some(id),
                depth + 1,
            )?))),
            None => None,
        };
        // MobEffectInstance.Details / ExtraCodecs.UNSIGNED_BYTE wraps Codec.BYTE.
        let amplifier = i32::from(read_combat_int(data, "amplifier", 0)? as u8);
        Some(Self {
            effect: StatusEffectInstance {
                effect_id: Cow::Borrowed(id),
                amplifier,
                duration: read_combat_int(data, "duration", 0)?,
                ambient: read_combat_bool(data, "ambient", false)?,
                show_particles: particles,
                show_icon: read_combat_bool(data, "show_icon", particles)?,
            },
            hidden_effect,
        })
    }

    pub fn as_nbt(&self) -> NbtTag {
        let NbtTag::Compound(mut data) = self.effect.as_nbt() else {
            return NbtTag::End;
        };
        data.put_byte(
            "amplifier",
            self.effect.amplifier.clamp(0, i32::from(u8::MAX)) as i8,
        );
        if let Some(hidden) = &self.hidden_effect {
            if let NbtTag::Compound(mut details) = hidden.as_nbt() {
                // Details.MAP_CODEC inherits the enclosing effect id.
                details.child_tags.remove("id");
                data.put_compound("hidden_effect", details);
            }
        }
        NbtTag::Compound(data)
    }
}
impl DeathEffect {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let data = data.extract_compound()?;
        let kind = data.get_string("type")?;
        match kind.strip_prefix("minecraft:").unwrap_or(kind) {
            "clear_all_effects" => Some(Self::ClearAllEffects),
            "apply_effects" => {
                let effects = data
                    .get_list("effects")?
                    .iter()
                    .map(|effect| DeathStatusEffect::read_data(effect, None, 0))
                    .collect::<Option<Vec<_>>>()?;
                Some(Self::ApplyEffects(Cow::Owned(effects), {
                    let probability = read_combat_float(data, "probability", Some(1.0))?;
                    (0.0..=1.0).contains(&probability).then_some(probability)?
                }))
            }
            "remove_effects" => Some(Self::RemoveEffects(read_combat_holder_set(
                data.get("effects")?,
            )?)),
            "teleport_randomly" => Some(Self::TeleportRandomly {
                diameter: {
                    // TeleportRandomlyConsumeEffect.CODEC uses POSITIVE_FLOAT.
                    let diameter = read_combat_float(data, "diameter", Some(16.0))?;
                    (diameter > 0.0).then_some(diameter)?
                },
                directional_particles: read_combat_bool(data, "directional_particles", true)?,
            }),
            "play_sound" => Some(Self::PlaySound(read_optional_combat_sound(data, "sound")??)),
            _ => None,
        }
    }

    pub fn as_nbt(&self) -> NbtTag {
        let mut data = NbtCompound::new();
        let kind = match self {
            Self::ClearAllEffects => "clear_all_effects",
            Self::ApplyEffects(effects, probability) => {
                data.put_list(
                    "effects",
                    effects.iter().map(DeathStatusEffect::as_nbt).collect(),
                );
                data.put_float("probability", *probability);
                "apply_effects"
            }
            Self::RemoveEffects(types) => {
                types.write(&mut data, "effects");
                "remove_effects"
            }
            Self::TeleportRandomly {
                diameter,
                directional_particles,
            } => {
                data.put_float("diameter", *diameter);
                data.put_bool("directional_particles", *directional_particles);
                "teleport_randomly"
            }
            Self::PlaySound(sound) => {
                put_idor(&mut data, "sound", sound);
                "play_sound"
            }
        };
        data.put_string("type", format!("minecraft:{kind}"));
        NbtTag::Compound(data)
    }
}
impl Hash for DeathEffect {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            Self::ClearAllEffects => 2.hash(state),
            Self::ApplyEffects(effects, probability) => {
                0.hash(state);
                effects.hash(state);
                probability.to_bits().hash(state);
            }
            Self::RemoveEffects(types) => {
                1.hash(state);
                types.hash(state);
            }
            Self::TeleportRandomly {
                diameter,
                directional_particles,
            } => {
                3.hash(state);
                diameter.to_bits().hash(state);
                directional_particles.hash(state);
            }
            Self::PlaySound(sound) => {
                4.hash(state);
                sound.hash(state);
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BlockingDamageReduction {
    pub horizontal_blocking_angle: f32,
    pub damage_type: Option<IDSet<DamageTypeImpl>>,
    pub base: f32,
    pub factor: f32,
}
impl BlockingDamageReduction {
    // BlocksAttacks.DamageReduction.resolve: the limit includes its boundary.
    pub fn resolve(&self, damage_type: &DamageType, damage: f32, angle: f64) -> f32 {
        if angle > f64::from((std::f64::consts::PI / 180.0) as f32 * self.horizontal_blocking_angle)
            || self
                .damage_type
                .as_ref()
                .is_some_and(|types| !damage_type_set_contains(types, damage_type))
        {
            return 0.0;
        }
        clamp_blocked_damage(self.base + self.factor * damage, damage)
    }
}

impl Hash for BlockingDamageReduction {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.horizontal_blocking_angle.to_bits().hash(state);
        self.damage_type.hash(state);
        self.base.to_bits().hash(state);
        self.factor.to_bits().hash(state);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockingItemDamage {
    pub threshold: f32,
    pub base: f32,
    pub factor: f32,
}
impl BlockingItemDamage {
    pub const DEFAULT: Self = Self {
        threshold: 1.0,
        base: 0.0,
        factor: 1.0,
    };

    // BlocksAttacks.ItemDamageFunction.apply.
    pub fn apply(&self, blocked_damage: f32) -> i32 {
        if blocked_damage < self.threshold {
            0
        } else {
            (self.base + self.factor * blocked_damage).floor() as i32
        }
    }
}

impl Hash for BlockingItemDamage {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.threshold.to_bits().hash(state);
        self.base.to_bits().hash(state);
        self.factor.to_bits().hash(state);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BlocksAttacksImpl {
    pub block_delay_seconds: f32,
    pub disable_cooldown_scale: f32,
    pub damage_reductions: Cow<'static, [BlockingDamageReduction]>,
    pub item_damage: BlockingItemDamage,
    pub bypassed_by: Option<IDSet<DamageTypeImpl>>,
    pub block_sound: Option<IdOr<SoundEvent>>,
    pub disable_sound: Option<IdOr<SoundEvent>>,
}
impl BlocksAttacksImpl {
    // BlocksAttacks.CODEC supplies these defaults even for an empty component.
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let compound = data.extract_compound()?;
        let damage_reductions = if let Some(list) = compound.get("damage_reductions") {
            list.extract_list()?
                .iter()
                .map(|data| {
                    let data = data.extract_compound()?;
                    Some(BlockingDamageReduction {
                        horizontal_blocking_angle: {
                            let angle =
                                read_combat_float(data, "horizontal_blocking_angle", Some(90.0))?;
                            (angle > 0.0).then_some(angle)?
                        },
                        damage_type: read_optional_damage_types(data, "type")?,
                        base: read_combat_float(data, "base", None)?,
                        factor: read_combat_float(data, "factor", None)?,
                    })
                })
                .collect::<Option<Vec<_>>>()?
        } else {
            vec![BlockingDamageReduction {
                horizontal_blocking_angle: 90.0,
                damage_type: None,
                base: 0.0,
                factor: 1.0,
            }]
        };
        let item_damage = if let Some(data) = compound.get("item_damage") {
            let data = data.extract_compound()?;
            BlockingItemDamage {
                threshold: read_nonnegative_combat_float(data, "threshold", None)?,
                base: read_combat_float(data, "base", None)?,
                factor: read_combat_float(data, "factor", None)?,
            }
        } else {
            BlockingItemDamage::DEFAULT
        };
        Some(Self {
            block_delay_seconds: read_nonnegative_combat_float(
                compound,
                "block_delay_seconds",
                Some(0.0),
            )?,
            disable_cooldown_scale: read_nonnegative_combat_float(
                compound,
                "disable_cooldown_scale",
                Some(1.0),
            )?,
            damage_reductions: Cow::Owned(damage_reductions),
            item_damage,
            bypassed_by: read_optional_damage_types(compound, "bypassed_by")?,
            block_sound: read_optional_combat_sound(compound, "block_sound")?,
            disable_sound: read_optional_combat_sound(compound, "disabled_sound")?,
        })
    }

    pub fn block_delay_ticks(&self) -> i32 {
        (self.block_delay_seconds * 20.0).round() as i32
    }

    // BlocksAttacks.disableBlockingForTicks.
    pub fn disable_blocking_for_ticks(&self, base_seconds: f32) -> i32 {
        let seconds = base_seconds * self.disable_cooldown_scale;
        if seconds > 0.0 {
            (seconds * 20.0).round() as i32
        } else {
            0
        }
    }

    pub fn resolve_blocked_damage(&self, damage_type: &DamageType, damage: f32, angle: f64) -> f32 {
        let blocked = self
            .damage_reductions
            .iter()
            .map(|reduction| reduction.resolve(damage_type, damage, angle))
            .sum::<f32>();
        clamp_blocked_damage(blocked, damage)
    }
}
impl DataComponentImpl for BlocksAttacksImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        compound.put_float("block_delay_seconds", self.block_delay_seconds);
        compound.put_float("disable_cooldown_scale", self.disable_cooldown_scale);
        compound.put_list(
            "damage_reductions",
            self.damage_reductions
                .iter()
                .map(|reduction| {
                    let mut data = NbtCompound::new();
                    data.put_float(
                        "horizontal_blocking_angle",
                        reduction.horizontal_blocking_angle,
                    );
                    if let Some(types) = &reduction.damage_type {
                        types.write(&mut data, "type");
                    }
                    data.put_float("base", reduction.base);
                    data.put_float("factor", reduction.factor);
                    NbtTag::Compound(data)
                })
                .collect(),
        );
        let mut item_damage = NbtCompound::new();
        item_damage.put_float("threshold", self.item_damage.threshold);
        item_damage.put_float("base", self.item_damage.base);
        item_damage.put_float("factor", self.item_damage.factor);
        compound.put_compound("item_damage", item_damage);
        if let Some(types) = &self.bypassed_by {
            types.write(&mut compound, "bypassed_by");
        }
        if let Some(sound) = &self.block_sound {
            put_idor(&mut compound, "block_sound", sound);
        }
        if let Some(sound) = &self.disable_sound {
            put_idor(&mut compound, "disabled_sound", sound);
        }
        NbtTag::Compound(compound)
    }
    default_impl!(BlocksAttacks);
}

// Mth.clamp propagates NaN instead of panicking on a NaN upper bound.
fn clamp_blocked_damage(value: f32, incoming: f32) -> f32 {
    if value < 0.0 {
        0.0
    } else if value.is_nan() || incoming.is_nan() {
        f32::NAN
    } else {
        value.min(incoming)
    }
}

impl Hash for BlocksAttacksImpl {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.block_delay_seconds.to_bits().hash(state);
        self.disable_cooldown_scale.to_bits().hash(state);
        self.damage_reductions.hash(state);
        self.item_damage.hash(state);
        self.bypassed_by.hash(state);
        self.block_sound.hash(state);
        self.disable_sound.hash(state);
    }
}

// NbtOps.getNumberValue / Codec.INT use NumericTag.intValue for every numeric tag.
fn read_combat_int(compound: &NbtCompound, key: &str, default: i32) -> Option<i32> {
    Some(match compound.get(key) {
        None => default,
        Some(NbtTag::Byte(value)) => i32::from(*value),
        Some(NbtTag::Short(value)) => i32::from(*value),
        Some(NbtTag::Int(value)) => *value,
        Some(NbtTag::Long(value)) => *value as i32,
        Some(NbtTag::Float(value)) => *value as i32,
        Some(NbtTag::Double(value)) => *value as i32,
        _ => return None,
    })
}

// NbtOps.getBooleanValue accepts numeric tags, including SNBT true/false bytes.
pub(super) fn read_combat_bool(compound: &NbtCompound, key: &str, default: bool) -> Option<bool> {
    match compound.get(key) {
        Some(value) => value.as_numeric_double().map(|value| value != 0.0),
        None => Some(default),
    }
}

// Vanilla Codec.FLOAT accepts every NBT numeric type, including integers in SNBT commands.
fn read_combat_float(compound: &NbtCompound, key: &str, default: Option<f32>) -> Option<f32> {
    match compound.get(key) {
        Some(value) => value.as_numeric_float(),
        None => default,
    }
}

fn read_nonnegative_combat_float(
    compound: &NbtCompound,
    key: &str,
    default: Option<f32>,
) -> Option<f32> {
    let value = read_combat_float(compound, key, default)?;
    (value >= 0.0).then_some(value)
}

fn read_optional_combat_sound(
    compound: &NbtCompound,
    key: &str,
) -> Option<Option<IdOr<SoundEvent>>> {
    let Some(data) = compound.get(key) else {
        return Some(None);
    };
    if let Some(name) = data.extract_string() {
        return Some(Some(IdOr::Id(Sound::from_name(
            name.strip_prefix("minecraft:").unwrap_or(name),
        )?)));
    }
    let data = data.extract_compound()?;
    Some(Some(IdOr::Value(SoundEvent {
        sound_name: Cow::Owned(data.get_string("sound_id")?.to_string()),
        range: data.get_numeric_float("range"),
    })))
}

fn read_optional_damage_types(
    compound: &NbtCompound,
    key: &str,
) -> Option<Option<IDSet<DamageTypeImpl>>> {
    match compound.get(key) {
        Some(data) => Some(Some(read_combat_holder_set(data)?)),
        None => Some(None),
    }
}

// RegistryCodecs.holderSet rejects the whole list if any holder is invalid.
fn read_combat_holder_set<T: IDSetContent>(data: &NbtTag) -> Option<IDSet<T>> {
    if let NbtTag::List(entries) = data {
        let entries = entries
            .iter()
            .map(|entry| T::from_str(entry.extract_string()?))
            .collect::<Option<Vec<_>>>()?;
        Some(IDSet::IDs(Cow::Owned(entries)))
    } else {
        IDSet::read(data)
    }
}

/// Tests a blocking component's holder set against the generated damage registry and tags.
pub fn damage_type_set_contains(types: &IDSet<DamageTypeImpl>, damage_type: &DamageType) -> bool {
    match types {
        IDSet::Tag(tag) => damage_type.is_tagged_with(tag).unwrap_or(false),
        IDSet::IDs(ids) => ids
            .iter()
            .any(|entry| entry.damage_type.id == damage_type.id),
    }
}

impl IDSetContent for DamageTypeImpl {
    fn registry_id(&self) -> u16 {
        u16::from(self.damage_type.id)
    }

    fn from_id(id: u16) -> Option<&'static Self> {
        static TYPES: std::sync::LazyLock<Vec<DamageTypeImpl>> = std::sync::LazyLock::new(|| {
            (0..=u8::MAX)
                .filter_map(DamageType::from_id)
                .map(|damage_type| DamageTypeImpl { damage_type })
                .collect()
        });
        TYPES
            .iter()
            .find(|entry| u16::from(entry.damage_type.id) == id)
    }

    fn from_str(name: &str) -> Option<&'static Self> {
        let damage_type = DamageType::from_name(name.strip_prefix("minecraft:").unwrap_or(name))?;
        <Self as IDSetContent>::from_id(u16::from(damage_type.id))
    }

    #[expect(
        clippy::expect_used,
        reason = "DamageType and the registry are generated from the same vanilla datapack"
    )]
    fn to_string(&self) -> String {
        // DamageType.message_id is a translation key, not the registry name.
        let name = crate::registry::REGISTRY_V_26_3
            .iter()
            .find(|registry| registry.registry_id == "damage_type")
            .and_then(|registry| registry.entries.get(usize::from(self.damage_type.id)))
            .expect("Generated damage registry contains every DamageType id")
            .name;
        format!("minecraft:{name}")
    }
}

fn get_optional_idor(compound: &NbtCompound, key: &str) -> Option<IdOr<SoundEvent>> {
    compound
        .get(key)
        .map(|_| get_idor(compound, key, Sound::IntentionallyEmpty))
}

fn put_optional_idor(compound: &mut NbtCompound, key: &str, sound: Option<&IdOr<SoundEvent>>) {
    if let Some(sound) = sound {
        put_idor(compound, key, sound);
    }
}

fn update_optional_idor(digest: &mut Digest, sound: Option<&IdOr<SoundEvent>>) {
    if let Some(sound) = sound {
        digest.update(&[1u8]);
        digest.update(&get_idor_hash(sound).to_le_bytes());
    } else {
        digest.update(&[0u8]);
    }
}

#[derive(Clone, Debug, Hash, PartialEq)]
pub struct PiercingWeaponImpl {
    pub deals_knockback: bool,
    pub dismounts: bool,
    pub sound: Option<IdOr<SoundEvent>>,
    pub hit_sound: Option<IdOr<SoundEvent>>,
}
impl PiercingWeaponImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let compound = data.extract_compound()?;
        Some(Self {
            deals_knockback: compound.get_bool("deals_knockback").unwrap_or(true),
            dismounts: compound.get_bool("dismounts").unwrap_or(false),
            sound: get_optional_idor(compound, "sound"),
            hit_sound: get_optional_idor(compound, "hit_sound"),
        })
    }
}
impl DataComponentImpl for PiercingWeaponImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        compound.put_bool("deals_knockback", self.deals_knockback);
        compound.put_bool("dismounts", self.dismounts);
        put_optional_idor(&mut compound, "sound", self.sound.as_ref());
        put_optional_idor(&mut compound, "hit_sound", self.hit_sound.as_ref());
        NbtTag::Compound(compound)
    }
    fn get_hash(&self) -> i32 {
        let mut digest = Digest::new();
        digest.update(&[self.deals_knockback as u8, self.dismounts as u8]);
        update_optional_idor(&mut digest, self.sound.as_ref());
        update_optional_idor(&mut digest, self.hit_sound.as_ref());
        digest.finalize() as i32
    }
    default_impl!(PiercingWeapon);
}

#[derive(Clone, Debug, PartialEq)]
pub struct KineticConditionImpl {
    pub max_duration_ticks: i32,
    pub min_speed: f32,
    pub min_relative_speed: f32,
}
impl KineticConditionImpl {
    pub fn test(&self, ticks_used: i32, attacker_speed: f64, relative_speed: f64) -> bool {
        ticks_used <= self.max_duration_ticks
            && attacker_speed >= f64::from(self.min_speed)
            && relative_speed >= f64::from(self.min_relative_speed)
    }
    fn read(compound: &NbtCompound, key: &str) -> Option<Self> {
        let condition = compound.get_compound(key)?;
        Some(Self {
            max_duration_ticks: condition.get_int("max_duration_ticks")?,
            min_speed: condition.get_float("min_speed").unwrap_or(0.0),
            min_relative_speed: condition.get_float("min_relative_speed").unwrap_or(0.0),
        })
    }
    fn write(&self, compound: &mut NbtCompound, key: &str) {
        let mut condition = NbtCompound::new();
        condition.put_int("max_duration_ticks", self.max_duration_ticks);
        condition.put_float("min_speed", self.min_speed);
        condition.put_float("min_relative_speed", self.min_relative_speed);
        compound.put(key, NbtTag::Compound(condition));
    }
    fn update_digest(&self, digest: &mut Digest) {
        digest.update(&get_i32_hash(self.max_duration_ticks).to_le_bytes());
        digest.update(&get_f32_hash(self.min_speed).to_le_bytes());
        digest.update(&get_f32_hash(self.min_relative_speed).to_le_bytes());
    }
}
impl Hash for KineticConditionImpl {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.max_duration_ticks.hash(state);
        self.min_speed.to_bits().hash(state);
        self.min_relative_speed.to_bits().hash(state);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct KineticWeaponImpl {
    pub contact_cooldown_ticks: i32,
    pub delay_ticks: i32,
    pub dismount_conditions: Option<KineticConditionImpl>,
    pub knockback_conditions: Option<KineticConditionImpl>,
    pub damage_conditions: Option<KineticConditionImpl>,
    pub forward_movement: f32,
    pub damage_multiplier: f32,
    pub sound: Option<IdOr<SoundEvent>>,
    pub hit_sound: Option<IdOr<SoundEvent>>,
}
impl KineticWeaponImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let compound = data.extract_compound()?;
        Some(Self {
            contact_cooldown_ticks: compound.get_int("contact_cooldown_ticks").unwrap_or(10),
            delay_ticks: compound.get_int("delay_ticks").unwrap_or(0),
            dismount_conditions: KineticConditionImpl::read(compound, "dismount_conditions"),
            knockback_conditions: KineticConditionImpl::read(compound, "knockback_conditions"),
            damage_conditions: KineticConditionImpl::read(compound, "damage_conditions"),
            forward_movement: compound.get_float("forward_movement").unwrap_or(0.0),
            damage_multiplier: compound.get_float("damage_multiplier").unwrap_or(1.0),
            sound: get_optional_idor(compound, "sound"),
            hit_sound: get_optional_idor(compound, "hit_sound"),
        })
    }
    fn conditions(&self) -> [(&str, Option<&KineticConditionImpl>); 3] {
        [
            ("dismount_conditions", self.dismount_conditions.as_ref()),
            ("knockback_conditions", self.knockback_conditions.as_ref()),
            ("damage_conditions", self.damage_conditions.as_ref()),
        ]
    }
}
impl DataComponentImpl for KineticWeaponImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        compound.put_int("contact_cooldown_ticks", self.contact_cooldown_ticks);
        compound.put_int("delay_ticks", self.delay_ticks);
        for (key, condition) in self.conditions() {
            if let Some(condition) = condition {
                condition.write(&mut compound, key);
            }
        }
        compound.put_float("forward_movement", self.forward_movement);
        compound.put_float("damage_multiplier", self.damage_multiplier);
        put_optional_idor(&mut compound, "sound", self.sound.as_ref());
        put_optional_idor(&mut compound, "hit_sound", self.hit_sound.as_ref());
        NbtTag::Compound(compound)
    }
    fn get_hash(&self) -> i32 {
        let mut digest = Digest::new();
        digest.update(&get_i32_hash(self.contact_cooldown_ticks).to_le_bytes());
        digest.update(&get_i32_hash(self.delay_ticks).to_le_bytes());
        for (_, condition) in self.conditions() {
            if let Some(condition) = condition {
                digest.update(&[1u8]);
                condition.update_digest(&mut digest);
            } else {
                digest.update(&[0u8]);
            }
        }
        digest.update(&get_f32_hash(self.forward_movement).to_le_bytes());
        digest.update(&get_f32_hash(self.damage_multiplier).to_le_bytes());
        update_optional_idor(&mut digest, self.sound.as_ref());
        update_optional_idor(&mut digest, self.hit_sound.as_ref());
        digest.finalize() as i32
    }
    default_impl!(KineticWeapon);
}
impl Hash for KineticWeaponImpl {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.contact_cooldown_ticks.hash(state);
        self.delay_ticks.hash(state);
        self.dismount_conditions.hash(state);
        self.knockback_conditions.hash(state);
        self.damage_conditions.hash(state);
        self.forward_movement.to_bits().hash(state);
        self.damage_multiplier.to_bits().hash(state);
        self.sound.hash(state);
        self.hit_sound.hash(state);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum SwingAnimationType {
    #[default]
    Whack = 0,
    Stab = 1,
    None = 2,
}

impl SwingAnimationType {
    #[must_use]
    pub fn from_id(id: i32) -> Option<Self> {
        match id {
            0 => Some(Self::Whack),
            1 => Some(Self::Stab),
            2 => Some(Self::None),
            _ => None,
        }
    }

    #[must_use]
    pub fn to_id(&self) -> i32 {
        *self as i32
    }

    #[must_use]
    pub fn to_name(&self) -> &'static str {
        match self {
            Self::Whack => "whack",
            Self::Stab => "stab",
            Self::None => "none",
        }
    }

    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "whack" => Some(Self::Whack),
            "stab" => Some(Self::Stab),
            "none" => Some(Self::None),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub struct SwingAnimationImpl {
    pub animation_type: SwingAnimationType,
    pub duration: i32,
}

impl Default for SwingAnimationImpl {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl SwingAnimationImpl {
    pub const DEFAULT: Self = Self {
        animation_type: SwingAnimationType::Whack,
        duration: 6,
    };

    pub fn read_data(data: &NbtTag) -> Option<Self> {
        match data {
            NbtTag::Compound(compound) => {
                let animation_type = compound
                    .get("type")
                    .and_then(|tag| match tag {
                        NbtTag::String(name) => SwingAnimationType::from_name(name),
                        NbtTag::Int(id) => SwingAnimationType::from_id(*id),
                        NbtTag::Byte(id) => SwingAnimationType::from_id(*id as i32),
                        _ => None,
                    })
                    .unwrap_or(Self::DEFAULT.animation_type);
                let duration = compound
                    .get("duration")
                    .and_then(|tag| tag.extract_int())
                    .unwrap_or(Self::DEFAULT.duration);
                Some(Self {
                    animation_type,
                    duration,
                })
            }
            _ => Some(Self::DEFAULT),
        }
    }
}

impl DataComponentImpl for SwingAnimationImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        compound.put_string("type", self.animation_type.to_name().to_string());
        compound.put_int("duration", self.duration);
        NbtTag::Compound(compound)
    }

    default_impl!(AttackAnimation);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct AdditionalTradeCostImpl;
impl AdditionalTradeCostImpl {
    pub const fn read_data(_data: &NbtTag) -> Option<Self> {
        Some(Self)
    }
}
impl DataComponentImpl for AdditionalTradeCostImpl {
    default_impl!(AdditionalTradeCost);
}

#[derive(Clone, Hash, PartialEq, Eq)]
pub struct StoredEnchantmentsImpl {
    pub enchantment: Cow<'static, [(&'static Enchantment, i32)]>,
}
impl StoredEnchantmentsImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let compound = data.extract_compound()?;
        let data = if let Some(NbtTag::Compound(levels)) = compound.child_tags.get("levels") {
            &levels.child_tags
        } else {
            &compound.child_tags
        };
        let mut enc = Vec::with_capacity(data.len());
        for (name, level) in data {
            let enchantment = Enchantment::from_name(name.as_ref())
                .or_else(|| Enchantment::from_name(&format!("minecraft:{name}")))?;
            enc.push((enchantment, level.extract_int()?));
        }
        Some(Self {
            enchantment: Cow::from(enc),
        })
    }
}
impl DataComponentImpl for StoredEnchantmentsImpl {
    fn write_data(&self) -> NbtTag {
        let mut data = NbtCompound::new();
        for (enc, level) in self.enchantment.iter() {
            data.put_int(enc.name, *level);
        }
        NbtTag::Compound(data)
    }
    fn get_hash(&self) -> i32 {
        let mut digest = Digest::new();
        digest.update(&[2u8]);
        for (enc, level) in self.enchantment.iter() {
            digest.update(&get_str_hash(enc.name).to_le_bytes());
            digest.update(&get_i32_hash(*level).to_le_bytes());
        }
        digest.update(&[3u8]);
        digest.finalize() as i32
    }
    default_impl!(StoredEnchantments);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct OminousBottleAmplifierImpl {
    pub amplifier: i32,
}
impl OminousBottleAmplifierImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        data.extract_int().map(|amplifier| Self { amplifier })
    }
}
impl DataComponentImpl for OminousBottleAmplifierImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::Int(self.amplifier)
    }
    fn get_hash(&self) -> i32 {
        get_i32_hash(self.amplifier) as i32
    }
    default_impl!(OminousBottleAmplifier);
}

/// An armor trim's material and pattern, kept as their raw NBT (each a registry
/// id or an inline definition) since Pumpkin does not yet model trim registries.
// TODO: replace `material`/`pattern` with typed trim material/pattern once those registries are modelled.
#[derive(Clone, Debug, PartialEq)]
pub struct TrimImpl {
    pub material: NbtTag,
    pub pattern: NbtTag,
}
impl TrimImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let compound = data.extract_compound()?;
        Some(Self {
            material: compound.get("material")?.clone(),
            pattern: compound.get("pattern")?.clone(),
        })
    }

    #[must_use]
    pub fn new(
        material: crate::trim_material::TrimMaterial,
        pattern: crate::trim_pattern::TrimPattern,
    ) -> Self {
        Self {
            material: NbtTag::String(material.asset_id().into()),
            pattern: NbtTag::String(pattern.asset_id().into()),
        }
    }

    #[must_use]
    pub fn material_enum(&self) -> Option<crate::trim_material::TrimMaterial> {
        self.material
            .extract_string()
            .and_then(crate::trim_material::TrimMaterial::from_name)
    }

    #[must_use]
    pub fn pattern_enum(&self) -> Option<crate::trim_pattern::TrimPattern> {
        self.pattern
            .extract_string()
            .and_then(crate::trim_pattern::TrimPattern::from_name)
    }
}
impl DataComponentImpl for TrimImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        compound.put("material", self.material.clone());
        compound.put("pattern", self.pattern.clone());
        NbtTag::Compound(compound)
    }
    default_impl!(Trim);
}

#[derive(Clone, Debug, PartialEq)]
pub struct MinimumAttackChargeImpl {
    pub charge: f32,
}
impl MinimumAttackChargeImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        data.extract_float().map(|charge| Self { charge })
    }
}
impl DataComponentImpl for MinimumAttackChargeImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::Float(self.charge)
    }
    fn get_hash(&self) -> i32 {
        get_f32_hash(self.charge) as i32
    }
    default_impl!(MinimumAttackCharge);
}
impl Hash for MinimumAttackChargeImpl {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.charge.to_bits().hash(state);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DamageTypeImpl {
    pub damage_type: DamageType,
}
impl DamageTypeImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let name = data.extract_string()?;
        let damage_type = DamageType::from_name(name.strip_prefix("minecraft:").unwrap_or(name))?;
        Some(Self { damage_type })
    }
}
impl DataComponentImpl for DamageTypeImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::String(format!("minecraft:{}", self.damage_type.registry_key()).into())
    }
    fn get_hash(&self) -> i32 {
        get_str_hash(self.damage_type.registry_key()) as i32
    }
    default_impl!(DamageType);
}
impl Hash for DamageTypeImpl {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.damage_type.id.hash(state);
    }
}

#[cfg(test)]
mod blocking_component_tests {
    use super::*;

    fn blocking_fixture() -> NbtTag {
        let mut component = NbtCompound::new();
        component.put_float("block_delay_seconds", 0.175);
        component.put_float("disable_cooldown_scale", 0.375);
        let mut projectile = NbtCompound::new();
        projectile.put_string("type", "#minecraft:is_projectile".to_string());
        projectile.put_float("horizontal_blocking_angle", 60.0);
        projectile.put_float("base", 1.0);
        projectile.put_float("factor", 0.25);
        let mut general = NbtCompound::new();
        general.put_float("horizontal_blocking_angle", 45.0);
        general.put_float("base", 0.5);
        general.put_float("factor", 0.5);
        component.put_list(
            "damage_reductions",
            vec![NbtTag::Compound(projectile), NbtTag::Compound(general)],
        );
        let mut wear = NbtCompound::new();
        wear.put_float("threshold", 3.0);
        wear.put_float("base", 1.0);
        wear.put_float("factor", 1.0);
        component.put_compound("item_damage", wear);
        component.put_list(
            "bypassed_by",
            vec![NbtTag::String("minecraft:mob_attack".into())],
        );
        component.put_string("block_sound", "minecraft:item.shield.block".to_string());
        let mut sound = NbtCompound::new();
        sound.put_string("sound_id", "example:disable".to_string());
        sound.put_float("range", 12.5);
        component.put_compound("disabled_sound", sound);
        NbtTag::Compound(component)
    }

    #[test]
    fn blocking_nbt_preserves_custom_fields_and_registry_names() {
        let input = blocking_fixture();
        let component = BlocksAttacksImpl::read_data(&input).unwrap();
        assert_eq!(component.write_data(), input);
        assert_eq!(component.block_delay_ticks(), 4);
        assert_eq!(component.disable_blocking_for_ticks(5.0), 38);
        assert_eq!(component.disable_blocking_for_ticks(0.0), 0);
    }

    #[test]
    fn reductions_resolve_partial_damage_by_type_and_horizontal_angle() {
        let component = BlocksAttacksImpl::read_data(&blocking_fixture()).unwrap();
        assert_eq!(
            component.resolve_blocked_damage(&DamageType::ARROW, 8.0, 0.0),
            7.5
        );
        assert_eq!(
            component.resolve_blocked_damage(&DamageType::PLAYER_ATTACK, 8.0, 0.0),
            4.5
        );
        assert_eq!(
            component.resolve_blocked_damage(&DamageType::ARROW, 8.0, 50.0f64.to_radians()),
            3.0
        );
        assert_eq!(
            component.resolve_blocked_damage(&DamageType::ARROW, 8.0, std::f64::consts::PI),
            0.0
        );
        // Independent sixty-degree probe, inside vanilla's float-rounded boundary.
        let boundary = std::f64::consts::FRAC_PI_3;
        assert_eq!(
            component.resolve_blocked_damage(&DamageType::ARROW, 8.0, boundary),
            3.0
        );
        assert_eq!(
            component.resolve_blocked_damage(&DamageType::ARROW, 8.0, boundary + 0.001),
            0.0
        );
        assert_eq!(
            component.resolve_blocked_damage(&DamageType::ARROW, 1.0, 0.0),
            1.0
        );
    }

    #[test]
    fn blocking_item_wear_uses_threshold_base_and_factor() {
        let component = BlocksAttacksImpl::read_data(&blocking_fixture()).unwrap();
        assert_eq!(component.item_damage.apply(2.99), 0);
        assert_eq!(component.item_damage.apply(3.0), 4);
        assert_eq!(component.item_damage.apply(6.75), 7);
    }

    #[test]
    fn death_protection_nbt_preserves_order_and_effect_defaults() {
        let mut clear = NbtCompound::new();
        clear.put_string("type", "minecraft:clear_all_effects".to_string());
        let mut regeneration = NbtCompound::new();
        regeneration.put_string("id", "minecraft:regeneration".to_string());
        regeneration.put_short("duration", 900);
        let mut apply = NbtCompound::new();
        apply.put_string("type", "minecraft:apply_effects".to_string());
        apply.put_list("effects", vec![NbtTag::Compound(regeneration)]);
        let mut protection = NbtCompound::new();
        protection.put_list(
            "death_effects",
            vec![NbtTag::Compound(clear), NbtTag::Compound(apply)],
        );
        let parsed = DeathProtectionImpl::read_data(&NbtTag::Compound(protection)).unwrap();
        assert!(matches!(
            parsed.death_effects[0],
            DeathEffect::ClearAllEffects
        ));
        let DeathEffect::ApplyEffects(effects, probability) = &parsed.death_effects[1] else {
            panic!("Missing apply_effects")
        };
        assert_eq!(*probability, 1.0);
        assert_eq!(effects[0].effect.amplifier, 0);
        assert_eq!(effects[0].effect.duration, 900);
        assert!(effects[0].effect.show_icon);
        assert_eq!(
            DeathProtectionImpl::read_data(&parsed.write_data()).unwrap(),
            parsed
        );
    }

    #[test]
    fn weapon_nbt_preserves_disable_duration() {
        let mut input = NbtCompound::new();
        input.put_int("item_damage_per_attack", 2);
        input.put_float("disable_blocking_for_seconds", 3.25);
        let input = NbtTag::Compound(input);
        assert_eq!(WeaponImpl::read_data(&input).unwrap().write_data(), input);
        let mut numeric = NbtCompound::new();
        numeric.put_byte("item_damage_per_attack", 2);
        numeric.put_double("disable_blocking_for_seconds", 3.25);
        let parsed = WeaponImpl::read_data(&NbtTag::Compound(numeric)).unwrap();
        assert_eq!(parsed.item_damage_per_attack, 2);
        assert_eq!(parsed.disable_blocking_for_seconds, 3.25);
    }

    #[test]
    fn blocking_nbt_accepts_numeric_snbt_and_rejects_invalid_fields() {
        let mut wear = NbtCompound::new();
        wear.put_int("threshold", 3);
        wear.put_int("base", 1);
        wear.put_int("factor", 1);
        let mut input = NbtCompound::new();
        input.put_compound("item_damage", wear);
        input.put_double("block_delay_seconds", 0.25);
        let parsed = BlocksAttacksImpl::read_data(&NbtTag::Compound(input.clone())).unwrap();
        assert_eq!(parsed.item_damage.apply(3.0), 4);
        assert_eq!(parsed.block_delay_ticks(), 5);
        input.put_float("block_delay_seconds", -1.0);
        assert!(BlocksAttacksImpl::read_data(&NbtTag::Compound(input.clone())).is_none());
        input.put_string("block_delay_seconds", "0.25".to_owned());
        assert!(BlocksAttacksImpl::read_data(&NbtTag::Compound(input.clone())).is_none());
        input.child_tags.remove("block_delay_seconds");
        input.put_string("damage_reductions", "bad".to_owned());
        assert!(BlocksAttacksImpl::read_data(&NbtTag::Compound(input)).is_none());
    }
    #[test]
    fn death_effect_amplifiers_use_unsigned_nbt_bytes_in_hidden_details() {
        let mut details = NbtCompound::new();
        details.put_byte("amplifier", -128);
        let mut visible = NbtCompound::new();
        visible.put_string("id", "minecraft:regeneration".into());
        visible.put_byte("amplifier", -1);
        visible.put_compound("hidden_effect", details);
        let effect = DeathStatusEffect::read_data(&NbtTag::Compound(visible), None, 0).unwrap();
        assert_eq!(effect.effect.amplifier, 255);
        assert_eq!(effect.hidden_effect.as_ref().unwrap().effect.amplifier, 128);
        let nbt = effect.as_nbt();
        let nbt = nbt.extract_compound().unwrap();
        assert_eq!(nbt.get("amplifier"), Some(&NbtTag::Byte(-1)));
        assert_eq!(
            nbt.get_compound("hidden_effect").unwrap().get("amplifier"),
            Some(&NbtTag::Byte(-128))
        );
    }

    #[test]
    fn combat_holder_sets_reject_any_invalid_list_entry() {
        for invalid in [NbtTag::Int(1), NbtTag::String("minecraft:missing".into())] {
            let list = NbtTag::List(vec![NbtTag::String("minecraft:fall".into()), invalid]);
            assert!(read_combat_holder_set::<DamageTypeImpl>(&list).is_none());
        }
        let effects = NbtTag::List(vec![
            NbtTag::String("minecraft:regeneration".into()),
            NbtTag::Byte(0),
        ]);
        assert!(read_combat_holder_set::<StatusEffect>(&effects).is_none());
    }
}
