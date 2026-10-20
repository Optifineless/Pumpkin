use super::VillagerEntity;
use pumpkin_nbt::compound::NbtCompound;
use std::sync::atomic::Ordering;

impl VillagerEntity {
    // Villager.readAdditionalSaveData reads FoodLevel as a byte. Older fork saves
    // used an int, whose complete value must survive the migration.
    pub(super) fn read_food_nbt(&self, nbt: &NbtCompound) {
        let food = nbt
            .get_byte("FoodLevel")
            .map(i32::from)
            .or(nbt.get_int("FoodLevel"))
            .unwrap_or_default();
        self.food_level.store(food, Ordering::Relaxed);
        self.warn_if_legacy_food_level(food);
    }

    pub(super) fn write_food_nbt(&self, nbt: &mut NbtCompound) {
        let food = self.food_level.load(Ordering::Relaxed);
        // Villager.addAdditionalSaveData writes a signed byte. Preserve a legacy
        // out-of-range int rather than silently wrapping or clamping its value.
        if let Ok(food) = i8::try_from(food) {
            nbt.put_byte("FoodLevel", food);
        } else {
            self.warn_if_legacy_food_level(food);
            nbt.put_int("FoodLevel", food);
        }
    }

    fn warn_if_legacy_food_level(&self, food: i32) {
        if i8::try_from(food).is_err()
            && !self.food_level_warning_logged.swap(true, Ordering::Relaxed)
        {
            tracing::warn!(
                villager = %self.mob_entity.living_entity.entity.entity_uuid,
                food_level = food,
                "Preserving villager FoodLevel outside the vanilla byte range as a legacy int"
            );
        }
    }
}
