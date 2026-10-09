use crate::entity::EntityBase;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::text::color::NamedColor;
use std::sync::{Mutex, MutexGuard, PoisonError};

// Waypoint.Icon.cloneAndAssignStyle substitutes this RGB value for a black team.
const BLACK_TEAM_COLOR: i32 = -13_619_152;
/// The default style from `WaypointStyleAssets.DEFAULT`.
pub const DEFAULT: &str = "minecraft:default";

/// The saved style and color overrides for a locator-bar icon.
#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct LocatorBarIcon {
    pub(crate) style: Option<String>,
    pub(crate) color: Option<i32>,
}

impl LocatorBarIcon {
    /// Returns the client icon without changing the saved overrides.
    pub(crate) fn clone_and_assign_style(&self, entity: &dyn EntityBase) -> Self {
        // Waypoint.Icon.cloneAndAssignStyle keeps explicit color ahead of the team override.
        let mut icon = self.clone();
        if icon.color.is_none() {
            icon.color = entity.get_team().map(|team| {
                if team.color == NamedColor::Black {
                    BLACK_TEAM_COLOR
                } else {
                    let rgb = team.color.to_rgb();
                    i32::from_be_bytes([0, rgb.red, rgb.green, rgb.blue])
                }
            });
        }
        icon
    }
}

// LivingEntity.locatorBarIcon owns Waypoint.Icon independently of client connections.
/// Owns an entity's saved locator-bar icon.
#[derive(Default)]
pub struct WaypointIcon {
    saved: Mutex<LocatorBarIcon>,
}

impl WaypointIcon {
    fn lock(&self) -> MutexGuard<'_, LocatorBarIcon> {
        self.saved.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Changes the saved icon and returns a snapshot after releasing the storage lock.
    pub(crate) fn mutate(&self, change: impl FnOnce(&mut LocatorBarIcon)) -> LocatorBarIcon {
        let mut icon = self.lock();
        change(&mut icon);
        icon.clone()
    }

    #[cfg(test)]
    pub(crate) fn snapshot(&self) -> LocatorBarIcon {
        self.lock().clone()
    }

    pub(crate) fn write_nbt(&self, nbt: &mut NbtCompound) {
        // LivingEntity.addAdditionalSaveData writes Waypoint.Icon.CODEC only when it has data.
        let icon = self.lock();
        if icon.style.is_none() && icon.color.is_none() {
            return;
        }
        let mut compound = NbtCompound::new();
        compound.put_string("style", icon.style.as_deref().unwrap_or(DEFAULT).to_owned());
        if let Some(color) = icon.color {
            compound.put_int("color", color);
        }
        nbt.put_compound("locator_bar_icon", compound);
    }

    pub(crate) fn read_nbt(&self, nbt: &NbtCompound) {
        // LivingEntity.readAdditionalSaveData replaces missing data with a default Waypoint.Icon.
        let icon =
            nbt.get_compound("locator_bar_icon")
                .map_or_else(LocatorBarIcon::default, |compound| LocatorBarIcon {
                    style: compound
                        .get_string("style")
                        .filter(|style| *style != DEFAULT)
                        .map(str::to_owned),
                    color: compound.get_int("color"),
                });
        *self.lock() = icon;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review3_waypoint_icon_nbt_preserves_overrides_and_replaces_missing_data() {
        let icon = WaypointIcon::default();
        let mut expected_icon = NbtCompound::new();
        expected_icon.put_string("style", "minecraft:bowtie".into());
        expected_icon.put_int("color", 0x123456);
        let mut expected = NbtCompound::new();
        expected.put_compound("locator_bar_icon", expected_icon);
        icon.read_nbt(&expected);
        assert_eq!(icon.snapshot().style.as_deref(), Some("minecraft:bowtie"));
        assert_eq!(icon.snapshot().color, Some(0x123456));
        let mut saved = NbtCompound::new();
        icon.write_nbt(&mut saved);
        assert_eq!(saved, expected);

        // The color-only CODEC still saves the default style.
        let mut color_only = NbtCompound::new();
        color_only.put_string("style", "minecraft:default".into());
        color_only.put_int("color", 0);
        expected.put_compound("locator_bar_icon", color_only);
        icon.read_nbt(&expected);
        assert!(icon.snapshot().style.is_none());
        assert_eq!(icon.snapshot().color, Some(0));
        saved = NbtCompound::new();
        icon.write_nbt(&mut saved);
        assert_eq!(saved, expected);

        let mut style_only = NbtCompound::new();
        style_only.put_string("style", "minecraft:bowtie".into());
        expected.put_compound("locator_bar_icon", style_only);
        icon.read_nbt(&expected);
        assert_eq!(icon.snapshot().style.as_deref(), Some("minecraft:bowtie"));
        assert!(icon.snapshot().color.is_none());

        icon.read_nbt(&NbtCompound::new());
        saved = NbtCompound::new();
        icon.write_nbt(&mut saved);
        assert!(saved.get_compound("locator_bar_icon").is_none());
        assert_eq!(icon.snapshot(), LocatorBarIcon::default());
    }
}
