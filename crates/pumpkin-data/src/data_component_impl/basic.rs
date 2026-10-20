use crate::data_component_impl::{DataComponentImpl, get_i32_hash, get_str_hash};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_util::text::TextComponent;
use std::borrow::Cow;

#[derive(Clone, Debug, PartialEq)]
pub struct CustomDataImpl {
    pub data: NbtCompound,
}
impl CustomDataImpl {
    #[must_use]
    pub const fn new(data: NbtCompound) -> Self {
        Self { data }
    }
    #[must_use]
    pub fn read_data(tag: &NbtTag) -> Option<Self> {
        if let NbtTag::Compound(c) = tag {
            Some(Self { data: c.clone() })
        } else {
            None
        }
    }
}
impl DataComponentImpl for CustomDataImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::Compound(self.data.clone())
    }
    fn get_hash(&self) -> i32 {
        0
    }
    default_impl!(CustomData);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct MaxStackSizeImpl {
    pub size: u8,
}
impl MaxStackSizeImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        data.extract_int().map(|size| Self { size: size as u8 })
    }
}
impl DataComponentImpl for MaxStackSizeImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::Int(self.size as i32)
    }
    fn get_hash(&self) -> i32 {
        get_i32_hash(self.size as i32) as i32
    }
    default_impl!(MaxStackSize);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct MaxDamageImpl {
    pub max_damage: i32,
}
impl MaxDamageImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        data.extract_int().map(|max_damage| Self { max_damage })
    }
}
impl DataComponentImpl for MaxDamageImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::Int(self.max_damage)
    }
    default_impl!(MaxDamage);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct DamageImpl {
    pub damage: i32,
}
impl DamageImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        data.extract_int().map(|damage| Self { damage })
    }
}
impl DataComponentImpl for DamageImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::Int(self.damage)
    }
    fn get_hash(&self) -> i32 {
        get_i32_hash(self.damage) as i32
    }
    default_impl!(Damage);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct UnbreakableImpl;
impl UnbreakableImpl {
    pub const fn read_data(_data: &NbtTag) -> Option<Self> {
        Some(Self)
    }
}
impl DataComponentImpl for UnbreakableImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::Compound(NbtCompound::new())
    }
    fn get_hash(&self) -> i32 {
        0
    }
    default_impl!(Unbreakable);
}

#[derive(Clone, Debug)]
pub struct CustomNameImpl {
    pub name: TextComponent,
}
impl PartialEq for CustomNameImpl {
    fn eq(&self, other: &Self) -> bool {
        // DataComponentExactPredicate.test uses Component.equals, including contents and style.
        super::text_component_codec::canonical_component(&self.write_data())
            == super::text_component_codec::canonical_component(&other.write_data())
    }
}
impl Eq for CustomNameImpl {}
impl std::hash::Hash for CustomNameImpl {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        super::text_component_codec::canonical_component(&self.write_data()).hash(state);
    }
}
impl CustomNameImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        super::text_component_codec::read_component(data).map(|name| Self { name })
    }
}
impl DataComponentImpl for CustomNameImpl {
    fn write_data(&self) -> NbtTag {
        // DataComponents.CUSTOM_NAME uses ComponentSerialization.CODEC, without translation.
        super::text_component_codec::write_component(&self.name)
    }
    fn get_hash(&self) -> i32 {
        get_str_hash(self.name.clone().get_text().as_str()) as i32
    }
    default_impl!(CustomName);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct ItemNameImpl {
    pub name: Cow<'static, str>,
}
impl ItemNameImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let name = match data {
            NbtTag::String(name) => name.to_string(),
            NbtTag::Compound(component) => component
                .get_string("translate")
                .or_else(|| component.get_string("text"))?
                .to_owned(),
            _ => return None,
        };
        Some(Self {
            name: Cow::Owned(name),
        })
    }
}
impl DataComponentImpl for ItemNameImpl {
    fn write_data(&self) -> NbtTag {
        let mut component = NbtCompound::new();
        component.put_string("translate", self.name.to_string());
        NbtTag::Compound(component)
    }
    fn get_hash(&self) -> i32 {
        get_str_hash(&self.name) as i32
    }
    default_impl!(ItemName);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct ItemModelImpl {
    pub id: Cow<'static, str>,
}
impl ItemModelImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        data.extract_string().map(|id| Self {
            id: Cow::Owned(id.to_string()),
        })
    }
}
impl DataComponentImpl for ItemModelImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::String(self.id.clone().into_owned().into())
    }
    fn get_hash(&self) -> i32 {
        get_str_hash(self.id.as_ref()) as i32
    }
    default_impl!(ItemModel);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct LoreImpl {
    pub lines: Vec<TextComponent>,
}
impl LoreImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let NbtTag::List(lines) = data else {
            return None;
        };

        Some(Self {
            lines: lines
                .iter()
                .filter_map(NbtTag::extract_string)
                .map(|line| TextComponent::text(line.to_owned()))
                .collect(),
        })
    }
}
impl DataComponentImpl for LoreImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::List(
            self.lines
                .iter()
                .map(|line| NbtTag::String(line.clone().get_text().into_boxed_str()))
                .collect(),
        )
    }
    default_impl!(Lore);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Rarity {
    #[default]
    Common = 0,
    Uncommon = 1,
    Rare = 2,
    Epic = 3,
}

impl Rarity {
    #[must_use]
    pub fn from_id(id: i32) -> Option<Self> {
        match id {
            0 => Some(Self::Common),
            1 => Some(Self::Uncommon),
            2 => Some(Self::Rare),
            3 => Some(Self::Epic),
            _ => None,
        }
    }

    #[must_use]
    pub fn to_id(self) -> i32 {
        self as i32
    }

    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "common" => Some(Self::Common),
            "uncommon" => Some(Self::Uncommon),
            "rare" => Some(Self::Rare),
            "epic" => Some(Self::Epic),
            _ => None,
        }
    }

    #[must_use]
    pub fn to_name(self) -> &'static str {
        match self {
            Self::Common => "common",
            Self::Uncommon => "uncommon",
            Self::Rare => "rare",
            Self::Epic => "epic",
        }
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub struct RarityImpl {
    pub rarity: Rarity,
}

impl RarityImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let name = data.extract_string()?;
        Some(Self {
            rarity: Rarity::from_name(name)?,
        })
    }
}

impl DataComponentImpl for RarityImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::String(self.rarity.to_name().into())
    }

    fn get_hash(&self) -> i32 {
        crate::data_component_impl::get_i32_hash(self.rarity.to_id()) as i32
    }

    default_impl!(Rarity);
}

#[derive(Clone, Debug, PartialEq)]
pub struct CustomModelDataImpl {
    pub floats: Vec<f32>,
    pub flags: Vec<bool>,
    pub strings: Vec<String>,
    pub colors: Vec<i32>,
}
impl CustomModelDataImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let compound = data.extract_compound()?;
        let floats = compound
            .get_list("floats")
            .map(|l| l.iter().filter_map(NbtTag::extract_float).collect())
            .unwrap_or_default();
        let flags = compound
            .get_list("flags")
            .map(|l| l.iter().filter_map(NbtTag::extract_bool).collect())
            .unwrap_or_default();
        let strings = compound
            .get_list("strings")
            .map(|l| {
                l.iter()
                    .filter_map(|t| t.extract_string().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        // Vanilla encodes the color list as ints, but tolerate a packed int array too.
        let colors = if let Some(arr) = compound.get_int_array("colors") {
            arr.to_vec()
        } else if let Some(l) = compound.get_list("colors") {
            l.iter().filter_map(NbtTag::extract_int).collect()
        } else {
            Vec::new()
        };
        Some(Self {
            floats,
            flags,
            strings,
            colors,
        })
    }
}
impl DataComponentImpl for CustomModelDataImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        compound.put_list(
            "floats",
            self.floats.iter().map(|f| NbtTag::Float(*f)).collect(),
        );
        compound.put_list(
            "flags",
            self.flags.iter().map(|b| NbtTag::Byte(*b as i8)).collect(),
        );
        compound.put_list(
            "strings",
            self.strings
                .iter()
                .map(|s| NbtTag::String(s.clone().into()))
                .collect(),
        );
        compound.put_list(
            "colors",
            self.colors.iter().map(|c| NbtTag::Int(*c)).collect(),
        );
        NbtTag::Compound(compound)
    }
    default_impl!(CustomModelData);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct TooltipDisplayImpl;
impl TooltipDisplayImpl {
    pub const fn read_data(_data: &NbtTag) -> Option<Self> {
        Some(Self)
    }
}
impl DataComponentImpl for TooltipDisplayImpl {
    default_impl!(TooltipDisplay);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct CreativeSlotLockImpl;
impl CreativeSlotLockImpl {
    pub const fn read_data(_data: &NbtTag) -> Option<Self> {
        Some(Self)
    }
}
impl DataComponentImpl for CreativeSlotLockImpl {
    default_impl!(CreativeSlotLock);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct EnchantmentGlintOverrideImpl;
impl EnchantmentGlintOverrideImpl {
    pub const fn read_data(_data: &NbtTag) -> Option<Self> {
        Some(Self)
    }
}
impl DataComponentImpl for EnchantmentGlintOverrideImpl {
    default_impl!(EnchantmentGlintOverride);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct TooltipStyleImpl {
    pub id: String,
}
impl TooltipStyleImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        data.extract_string().map(|id| Self { id: id.to_string() })
    }
}
impl DataComponentImpl for TooltipStyleImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::String(self.id.clone().into())
    }
    fn get_hash(&self) -> i32 {
        get_str_hash(&self.id) as i32
    }
    default_impl!(TooltipStyle);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct NoteBlockSoundImpl {
    pub sound: String,
}
impl NoteBlockSoundImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        data.extract_string().map(|sound| Self {
            sound: sound.to_string(),
        })
    }
}
impl DataComponentImpl for NoteBlockSoundImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::String(self.sound.clone().into())
    }
    fn get_hash(&self) -> i32 {
        get_str_hash(&self.sound) as i32
    }
    default_impl!(NoteBlockSound);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct BaseColorImpl {
    pub color: String,
}
impl BaseColorImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        data.extract_string().map(|color| Self {
            color: color.to_string(),
        })
    }
}
impl DataComponentImpl for BaseColorImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::String(self.color.clone().into())
    }
    fn get_hash(&self) -> i32 {
        get_str_hash(&self.color) as i32
    }
    default_impl!(BaseColor);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct ProvidesTrimMaterialImpl;
impl ProvidesTrimMaterialImpl {
    pub const fn read_data(_data: &NbtTag) -> Option<Self> {
        Some(Self)
    }
}
impl DataComponentImpl for ProvidesTrimMaterialImpl {
    default_impl!(ProvidesTrimMaterial);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct ProvidesBannerPatternsImpl;
impl ProvidesBannerPatternsImpl {
    pub const fn read_data(_data: &NbtTag) -> Option<Self> {
        Some(Self)
    }
}
impl DataComponentImpl for ProvidesBannerPatternsImpl {
    default_impl!(ProvidesBannerPatterns);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct BannerPatternLayer {
    pub pattern: String,
    pub color: crate::dye_color::DyeColor,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, Default)]
pub struct BannerPatternsImpl {
    pub layers: Vec<BannerPatternLayer>,
}

impl BannerPatternsImpl {
    pub const EMPTY: Self = Self { layers: Vec::new() };

    #[must_use]
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let mut layers = Vec::new();
        if let NbtTag::List(list) = data {
            for tag in list {
                if let Some(compound) = tag.extract_compound() {
                    let pattern = compound.get_string("pattern")?.to_string();
                    let color_str = compound.get_string("color")?;
                    let color = crate::dye_color::DyeColor::by_name(color_str).unwrap_or_default();
                    layers.push(BannerPatternLayer { pattern, color });
                }
            }
        }
        Some(Self { layers })
    }
}

impl DataComponentImpl for BannerPatternsImpl {
    fn write_data(&self) -> NbtTag {
        let mut list = Vec::new();
        for layer in &self.layers {
            let mut compound = NbtCompound::new();
            compound.put_string("pattern", layer.pattern.clone());
            compound.put_string("color", layer.color.name().to_string());
            list.push(NbtTag::Compound(compound));
        }
        NbtTag::List(list)
    }

    default_impl!(BannerPatterns);
}

/// A pot face's item template, including its count and component patch.
#[derive(Clone)]
pub struct PotDecoration {
    pub item: &'static crate::item::Item,
    pub count: i32,
    pub patch: Vec<(
        crate::data_component::DataComponent,
        Option<Box<dyn DataComponentImpl>>,
    )>,
    pub raw_components: Vec<(Box<str>, NbtTag)>,
}

impl PotDecoration {
    /// Reads a registry name or a complete item template from pot-decoration NBT.
    fn read_data(data: &NbtTag) -> Option<Self> {
        if let NbtTag::String(name) = data {
            let item = crate::item::Item::from_registry_key(name)?;
            if item.id == crate::item::Item::AIR.id {
                return None;
            }
            return Some(Self {
                item,
                count: 1,
                patch: Vec::new(),
                raw_components: Vec::new(),
            });
        }

        let compound = data.extract_compound()?;
        let item = crate::item::Item::from_registry_key(compound.get_string("id")?)?;
        if item.id == crate::item::Item::AIR.id {
            return None;
        }
        let count = match compound.get("count") {
            Some(count) => count.extract_int()?,
            None => 1,
        };
        if !(1..=i32::from(crate::item::Item::ABSOLUTE_MAX_STACK_SIZE)).contains(&count) {
            return None;
        }

        let mut patch = Vec::new();
        let mut raw_components = Vec::new();
        if let Some(components) = compound.get("components") {
            let components = components.extract_compound()?;
            let mut seen = [false; 256];
            for (name, value) in &components.child_tags {
                let original_name = name;
                let (name, removed) = name
                    .strip_prefix('!')
                    .map_or((name.as_ref(), false), |name| (name, true));
                let Some(id) = crate::data_component::DataComponent::try_from_name(name) else {
                    crate::item_stack::raw_components::warn_preserved_component(original_name);
                    raw_components.push((original_name.clone(), value.clone()));
                    continue;
                };
                if !Self::is_persistent_component(id)
                    || std::mem::replace(&mut seen[usize::from(id.to_id())], true)
                {
                    return None;
                }
                let value = if removed {
                    if !value.extract_compound()?.child_tags.is_empty() {
                        return None;
                    }
                    None
                } else {
                    let Some(decoded) = crate::data_component_impl::read_data(id, value) else {
                        crate::item_stack::raw_components::warn_preserved_component(original_name);
                        raw_components.push((original_name.clone(), value.clone()));
                        continue;
                    };
                    Some(decoded)
                };
                patch.push((id, value));
            }
        }
        Some(Self {
            item,
            count,
            patch,
            raw_components,
        })
    }

    /// Persists the template while omitting default count and empty component fields.
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        compound.put_string("id", format!("minecraft:{}", self.item.registry_key));
        if self.count != 1 {
            compound.put_int("count", self.count);
        }
        let mut components = NbtCompound::new();
        for (id, value) in &self.patch {
            if !Self::is_persistent_component(*id) {
                continue;
            }
            match value {
                Some(value) => components.put(id.to_name(), value.write_data()),
                None => components
                    .put_compound(format!("!{}", id.to_name()).as_str(), NbtCompound::new()),
            }
        }
        crate::item_stack::raw_components::write_raw_components(
            &self.patch,
            &self.raw_components,
            &mut components,
        );
        if !components.child_tags.is_empty() {
            compound.put_compound("components", components);
        }
        NbtTag::Compound(compound)
    }

    /// Excludes network-only components from persistent item templates.
    const fn is_persistent_component(id: crate::data_component::DataComponent) -> bool {
        !matches!(
            id,
            crate::data_component::DataComponent::CreativeSlotLock
                | crate::data_component::DataComponent::AdditionalTradeCost
                | crate::data_component::DataComponent::MapPostProcessing
        )
    }

    /// Builds the item stack emitted by a cracked decorated pot.
    #[must_use]
    pub fn to_item_stack(&self) -> crate::item_stack::ItemStack {
        let mut stack = crate::item_stack::ItemStack::new(self.count as u8, self.item);
        stack.patch.clone_from(&self.patch);
        stack.raw_components.clone_from(&self.raw_components);
        stack
    }
}

impl std::fmt::Debug for PotDecoration {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.write_data(), formatter)
    }
}

impl PartialEq for PotDecoration {
    fn eq(&self, other: &Self) -> bool {
        self.item.id == other.item.id
            && self.count == other.count
            && crate::item_stack::raw_components::raw_components_equal(
                &self.raw_components,
                &other.raw_components,
            )
            && self.patch.len() == other.patch.len()
            && self.patch.iter().all(|(id, value)| {
                other.patch.iter().any(|(other_id, other_value)| {
                    id == other_id
                        && match (value, other_value) {
                            (Some(value), Some(other_value)) => value.equal(other_value.as_ref())
                                && crate::item_stack::raw_components::embedded_raw_components_equal(
                                    *id,
                                    value.as_ref(),
                                    other_value.as_ref(),
                                ),
                            (None, None) => true,
                            _ => false,
                        }
                })
            })
    }
}

/// The four optional decorated-pot faces, ordered back, left, right, then front.
#[derive(Clone, Debug, PartialEq)]
pub struct PotDecorationsImpl {
    pub decorations: [Option<PotDecoration>; 4],
}
impl PotDecorationsImpl {
    pub const EMPTY: Self = Self {
        decorations: [const { None }; 4],
    };
    const FACES: [&'static str; 4] = ["back", "left", "right", "front"];

    /// Reads named optional item templates; legacy sherd names remain supported separately.
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let compound = data.extract_compound()?;
        let mut decorations = Self::EMPTY.decorations;
        for (face, name) in decorations.iter_mut().zip(Self::FACES) {
            if let Some(data) = compound.get(name) {
                *face = Some(PotDecoration::read_data(data)?);
            }
        }
        Some(Self { decorations })
    }

    /// Reads the fork's existing four-name `sherds` list without changing its saved format.
    pub fn from_legacy_sherds(sherds: &[NbtTag]) -> Option<Self> {
        if sherds.len() != Self::FACES.len() {
            return None;
        }
        let mut decorations = Self::EMPTY.decorations;
        for (decoration, sherd) in decorations.iter_mut().zip(sherds) {
            *decoration = Some(PotDecoration::read_data(sherd)?);
        }
        Some(Self { decorations })
    }
}
impl DataComponentImpl for PotDecorationsImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        for (face, name) in self.decorations.iter().zip(Self::FACES) {
            if let Some(face) = face {
                compound.put(name, face.write_data());
            }
        }
        NbtTag::Compound(compound)
    }
    default_impl!(PotDecorations);
}

/// The lock's item predicate, kept as its raw NBT compound since Pumpkin does
/// not yet model item predicates.
// TODO: replace `predicate` with a typed item predicate once item predicates are modelled.
#[derive(Clone, Debug, PartialEq)]
pub struct LockImpl {
    pub predicate: NbtCompound,
}
impl LockImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        data.extract_compound().map(|predicate| Self {
            predicate: predicate.clone(),
        })
    }
}
impl DataComponentImpl for LockImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::Compound(self.predicate.clone())
    }
    default_impl!(Lock);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct BreakSoundImpl;
impl BreakSoundImpl {
    pub const fn read_data(_data: &NbtTag) -> Option<Self> {
        Some(Self)
    }
}
impl DataComponentImpl for BreakSoundImpl {
    default_impl!(BreakSound);
}

#[derive(Clone, Debug, PartialEq)]
pub struct SoundEvent {
    pub sound_name: Cow<'static, str>,
    pub range: Option<f32>,
}
impl std::hash::Hash for SoundEvent {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.sound_name.hash(state);
        if let Some(val) = self.range {
            true.hash(state);
            unsafe { (*(&raw const val).cast::<u32>()).hash(state) };
        } else {
            false.hash(state);
        }
    }
}
impl SoundEvent {
    pub const fn new(sound_name: Cow<'static, str>, range: Option<f32>) -> Self {
        Self { sound_name, range }
    }
}

#[cfg(test)]
mod decorated_pot_tests {
    use super::*;
    use crate::item::Item;

    #[test]
    fn b1019_g03_pot_decorations_round_trip_and_read_legacy_sherds() {
        let decoration = |item, count| {
            Some(PotDecoration {
                item,
                count,
                patch: Vec::new(),
                raw_components: Vec::new(),
            })
        };
        let original = PotDecorationsImpl {
            decorations: [
                decoration(&Item::BRICK, 2),
                decoration(&Item::ANGLER_POTTERY_SHERD, 1),
                None,
                decoration(&Item::ARCHER_POTTERY_SHERD, 3),
            ],
        };
        let encoded = original.write_data();
        assert_eq!(PotDecorationsImpl::read_data(&encoded), Some(original));

        let legacy = [
            NbtTag::String("minecraft:brick".into()),
            NbtTag::String("minecraft:angler_pottery_sherd".into()),
            NbtTag::String("minecraft:brick".into()),
            NbtTag::String("minecraft:archer_pottery_sherd".into()),
        ];
        let migrated = PotDecorationsImpl::from_legacy_sherds(&legacy).unwrap();
        assert_eq!(migrated.decorations[0].as_ref().unwrap().item, &Item::BRICK);
        assert_eq!(
            migrated.decorations[1].as_ref().unwrap().item,
            &Item::ANGLER_POTTERY_SHERD
        );
        assert_eq!(migrated.decorations[2].as_ref().unwrap().item, &Item::BRICK);
        assert_eq!(
            migrated.decorations[3].as_ref().unwrap().item,
            &Item::ARCHER_POTTERY_SHERD
        );
    }
}
