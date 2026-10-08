use crate::data_component_impl::DataComponentImpl;
use pumpkin_nbt::tag::NbtTag;
use std::borrow::Cow;

macro_rules! string_variant {
    ($name:ident, $struct_name:ident) => {
        #[derive(Clone, Debug, Hash, PartialEq, Eq)]
        pub struct $struct_name {
            pub value: Cow<'static, str>,
        }
        impl $struct_name {
            pub fn read_data(data: &NbtTag) -> Option<Self> {
                data.extract_string().map(|v| Self {
                    value: Cow::Owned(v.to_string()),
                })
            }
        }
        impl DataComponentImpl for $struct_name {
            fn write_data(&self) -> NbtTag {
                NbtTag::String(self.value.clone().into_owned().into())
            }
            fn get_hash(&self) -> i32 {
                crate::data_component_impl::get_str_hash(self.value.as_ref()) as i32
            }
            default_impl!($name);
        }
    };
}

string_variant!(VillagerVariant, VillagerVariantImpl);
string_variant!(WolfVariant, WolfVariantImpl);
string_variant!(WolfSoundVariant, WolfSoundVariantImpl);
string_variant!(WolfCollar, WolfCollarImpl);
string_variant!(FoxVariant, FoxVariantImpl);
string_variant!(SalmonSize, SalmonSizeImpl);
string_variant!(ParrotVariant, ParrotVariantImpl);
string_variant!(TropicalFishPattern, TropicalFishPatternImpl);
string_variant!(TropicalFishBaseColor, TropicalFishBaseColorImpl);
string_variant!(TropicalFishPatternColor, TropicalFishPatternColorImpl);
string_variant!(MooshroomVariant, MooshroomVariantImpl);
string_variant!(RabbitVariant, RabbitVariantImpl);
string_variant!(PigVariant, PigVariantImpl);
string_variant!(PigSoundVariant, PigSoundVariantImpl);
string_variant!(CowVariant, CowVariantImpl);
string_variant!(CowSoundVariant, CowSoundVariantImpl);
string_variant!(ChickenVariant, ChickenVariantImpl);
string_variant!(ChickenSoundVariant, ChickenSoundVariantImpl);
string_variant!(ZombieNautilusVariant, ZombieNautilusVariantImpl);
string_variant!(FrogVariant, FrogVariantImpl);
string_variant!(HorseVariant, HorseVariantImpl);
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct PaintingVariantImpl {
    pub value: Cow<'static, str>,
}
impl PaintingVariantImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        // PaintingVariant.CODEC resolves the registry holder before accepting the item.
        let name = data.extract_string()?;
        let variant = crate::painting_variant::PaintingVariant::from_name(name)?;
        Some(Self {
            value: Cow::Borrowed(variant.asset_id()),
        })
    }
}
impl DataComponentImpl for PaintingVariantImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::String(self.value.clone().into_owned().into())
    }
    fn get_hash(&self) -> i32 {
        crate::data_component_impl::get_str_hash(self.value.as_ref()) as i32
    }
    default_impl!(PaintingVariant);
}
string_variant!(LlamaVariant, LlamaVariantImpl);
string_variant!(AxolotlVariant, AxolotlVariantImpl);
string_variant!(CatVariant, CatVariantImpl);
string_variant!(CatSoundVariant, CatSoundVariantImpl);
string_variant!(CatCollar, CatCollarImpl);
string_variant!(SheepColor, SheepColorImpl);
string_variant!(ShulkerColor, ShulkerColorImpl);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn painting_nbt_rejects_unknown_registry_names() {
        assert!(
            PaintingVariantImpl::read_data(&NbtTag::String("minecraft:missing".into())).is_none()
        );
        assert_eq!(
            PaintingVariantImpl::read_data(&NbtTag::String("minecraft:alban".into()))
                .unwrap()
                .value,
            "minecraft:alban"
        );
    }
}
