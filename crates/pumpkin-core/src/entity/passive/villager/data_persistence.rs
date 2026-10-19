use std::sync::atomic::Ordering;

use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_util::sync::LockClass;

use super::{VillagerData, VillagerEntity, VillagerProfession, VillagerType};

pub(super) enum VillagerPersistenceLock {}

impl LockClass for VillagerPersistenceLock {
    const NAME: &'static str = "villager persistence";
    const LEVEL: u8 = 30;
}

/// Registry names that this build cannot represent in the protocol data.
#[derive(Clone, Default)]
pub(super) struct VillagerDataPersistence {
    unknown_type: Option<String>,
    unknown_profession: Option<String>,
    warned_unknown_registry: bool,
}

struct LoadedVillagerData {
    data: VillagerData,
    unknown_type: Option<String>,
    unknown_profession: Option<String>,
}

impl LoadedVillagerData {
    const fn default_data() -> Self {
        Self {
            data: VillagerData::new(VillagerType::Plains, VillagerProfession::None, 1),
            unknown_type: None,
            unknown_profession: None,
        }
    }

    fn decode(nbt: &NbtCompound, previous: VillagerData) -> Self {
        // VillagerData.CODEC defaults missing/unknown registries and level to plains/none/1.
        if nbt.get("type").is_some()
            || nbt.get("profession").is_some()
            || nbt.get("level").is_some()
        {
            let type_name = nbt.get_string("type");
            let profession_name = nbt.get_string("profession");
            let r#type = type_name.and_then(VillagerType::from_name);
            let profession = profession_name.and_then(VillagerProfession::from_name);
            return Self {
                data: VillagerData::new(
                    r#type.unwrap_or(VillagerType::Plains),
                    profession.unwrap_or(VillagerProfession::None),
                    nbt.get_int("level").unwrap_or(1).max(1),
                ),
                unknown_type: type_name.filter(|_| r#type.is_none()).map(str::to_owned),
                unknown_profession: profession_name
                    .filter(|_| profession.is_none())
                    .map(str::to_owned),
            };
        }

        // Saves from earlier fork builds used uppercase numeric registry ids.
        let type_id = nbt.get_int("Type");
        let profession_id = nbt.get_int("Profession");
        let level = nbt.get_int("Level");
        if type_id.is_some() || profession_id.is_some() || level.is_some() {
            Self {
                data: VillagerData {
                    r#type: type_id.map_or(previous.r#type, VarInt),
                    profession: profession_id.map_or(previous.profession, VarInt),
                    level: level.map_or(previous.level, |value| VarInt(value.max(1))),
                },
                unknown_type: None,
                unknown_profession: None,
            }
        } else {
            Self::default_data()
        }
    }
}

impl VillagerEntity {
    pub(super) fn write_villager_data_nbt(&self, nbt: &mut NbtCompound) {
        let data = *self
            .villager_data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let retained = self
            .villager_data_persistence
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let mut encoded = NbtCompound::new();
        encoded.put_string(
            "type",
            retained
                .unknown_type
                .unwrap_or_else(|| format!("minecraft:{}", data.type_enum().to_name())),
        );
        encoded.put_string(
            "profession",
            retained
                .unknown_profession
                .unwrap_or_else(|| format!("minecraft:{}", data.profession_enum().to_name())),
        );
        encoded.put_int("level", data.level.0);
        nbt.put_compound("VillagerData", encoded);
        nbt.put_bool(
            "VillagerDataFinalized",
            self.villager_data_finalized.load(Ordering::Relaxed),
        );
    }

    pub(super) fn read_villager_data_nbt(&self, nbt: &NbtCompound) {
        let previous = *self
            .villager_data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let loaded = nbt
            .get_compound("VillagerData")
            .map(|data| LoadedVillagerData::decode(data, previous));
        // Villager.readAdditionalSaveData: a decoded VillagerData also finalizes it.
        if !nbt.get_bool("VillagerDataFinalized").unwrap_or(false) && loaded.is_none() {
            return;
        }
        self.villager_data_finalized.store(true, Ordering::Relaxed);
        let loaded = loaded.unwrap_or_else(LoadedVillagerData::default_data);
        *self
            .villager_data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = loaded.data;

        let has_unknown = loaded.unknown_type.is_some() || loaded.unknown_profession.is_some();
        let should_warn = {
            let mut retained = self
                .villager_data_persistence
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            retained.unknown_type.clone_from(&loaded.unknown_type);
            retained
                .unknown_profession
                .clone_from(&loaded.unknown_profession);
            let should_warn = has_unknown && !retained.warned_unknown_registry;
            retained.warned_unknown_registry |= has_unknown;
            should_warn
        };
        if should_warn {
            tracing::warn!(
                villager = %self.mob_entity.living_entity.entity.entity_uuid,
                villager_type = ?loaded.unknown_type,
                profession = ?loaded.unknown_profession,
                "Preserving unknown villager registry names in saved data"
            );
        }
    }
}
