//! WorldClock.CODEC keys come from the registry, not dimension identifiers.

use super::data_files::WorldClocksData;

pub(super) fn migrate_dimension_aliases(clocks: &mut WorldClocksData) {
    remove_unregistered_aliases(clocks, |key| {
        pumpkin_data::registry::REGISTRY_V_26_3
            .iter()
            .find(|registry| registry.registry_id == "world_clock")
            .is_some_and(|registry| {
                registry.entries.iter().any(|entry| {
                    entry.name == key || key.strip_prefix("minecraft:") == Some(entry.name)
                })
            })
    });
}

fn remove_unregistered_aliases(clocks: &mut WorldClocksData, registered: impl Fn(&str) -> bool) {
    // Only undo keys emitted by the earlier fork; opaque/custom unloaded records are retained.
    for key in ["minecraft:the_nether", "minecraft:overworld_caves"] {
        if !registered(key) {
            clocks.clocks.remove(key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world_info::data_files::DimensionClock;

    #[test]
    fn clock_alias_migration_preserves_registered_collisions_and_unloaded_records() {
        let mut clocks = WorldClocksData::default();
        for key in [
            "minecraft:the_nether",
            "minecraft:overworld_caves",
            "custom:clock",
            "opaque:unloaded",
        ] {
            clocks.clocks.insert(key.into(), DimensionClock::default());
        }
        remove_unregistered_aliases(&mut clocks, |key| {
            matches!(key, "minecraft:the_nether" | "custom:clock")
        });
        assert!(clocks.clocks.contains_key("minecraft:the_nether"));
        assert!(!clocks.clocks.contains_key("minecraft:overworld_caves"));
        assert!(clocks.clocks.contains_key("custom:clock"));
        assert!(clocks.clocks.contains_key("opaque:unloaded"));
    }
}
