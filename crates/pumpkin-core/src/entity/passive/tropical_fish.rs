use std::sync::Arc;

use pumpkin_data::sound::Sound;

use crate::entity::{
    Entity, EntityBase,
    mob::{Mob, MobEntity},
    passive::fish,
};

// TropicalFish.DEFAULT_VARIANT is kob/white/white.
const DEFAULT_VARIANT: i32 = 0;

/// Represents a Tropical Fish, a passive fish of warm oceans.
///
/// Wiki: <https://minecraft.wiki/w/Tropical_Fish>
pub struct TropicalFishEntity {
    pub mob_entity: MobEntity,
    // TropicalFish.DATA_ID_TYPE_VARIANT.
    variant: std::sync::atomic::AtomicI32,
}

impl TropicalFishEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        fish::init(&mob_entity);
        // TropicalFish.DEFAULT_VARIANT is kob/white/white (packed zero).
        Arc::new(Self {
            mob_entity,
            variant: std::sync::atomic::AtomicI32::new(DEFAULT_VARIANT),
        })
    }

    // TropicalFish.applyImplicitComponents / packVariant.
    pub(crate) fn apply_bucket_components(&self, stack: &pumpkin_data::item_stack::ItemStack) {
        use pumpkin_data::data_component_impl::{
            TropicalFishBaseColorImpl, TropicalFishPatternColorImpl, TropicalFishPatternImpl,
        };
        use pumpkin_data::dye_color::DyeColor;
        let pattern = stack
            .get_data_component::<TropicalFishPatternImpl>()
            .and_then(TropicalFishPatternImpl::variant_id)
            .unwrap_or(DEFAULT_VARIANT);
        let base = stack
            .get_data_component::<TropicalFishBaseColorImpl>()
            .and_then(|value| DyeColor::by_name(&value.value))
            .unwrap_or(DyeColor::White);
        let color = stack
            .get_data_component::<TropicalFishPatternColorImpl>()
            .and_then(|value| DyeColor::by_name(&value.value))
            .unwrap_or(DyeColor::White);
        self.set_packed_variant(pattern | i32::from(base.id()) << 16 | i32::from(color.id()) << 24);
    }

    fn set_packed_variant(&self, value: i32) {
        self.variant
            .store(value, std::sync::atomic::Ordering::Relaxed);
        self.get_entity().set_synced_data(
            pumpkin_data::tracked_data::tropical_fish::DATA_ID_TYPE_VARIANT,
            pumpkin_protocol::VarInt(value),
        );
    }
}

impl Mob for TropicalFishEntity {
    // TropicalFish.addAdditionalSaveData / readAdditionalSaveData.
    fn mob_write_nbt(&self, nbt: &mut pumpkin_nbt::compound::NbtCompound) {
        nbt.put_int(
            "Variant",
            self.variant.load(std::sync::atomic::Ordering::Relaxed),
        );
    }

    fn mob_read_nbt(&self, nbt: &pumpkin_nbt::compound::NbtCompound) {
        self.set_packed_variant(nbt.get_int("Variant").unwrap_or(DEFAULT_VARIANT));
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn mob_tick(&self, _caller: &dyn EntityBase) {
        fish::flop(self, Sound::EntityTropicalFishFlop);
    }

    fn custom_travel(&self, caller: &dyn EntityBase) -> bool {
        fish::travel(self, caller)
    }

    fn mob_is_pushed_by_fluids(&self) -> bool {
        false
    }
}
