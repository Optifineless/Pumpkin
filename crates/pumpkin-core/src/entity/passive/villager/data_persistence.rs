use std::sync::atomic::Ordering;

use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_util::sync::LockClass;

use super::{VillagerData, VillagerEntity, VillagerProfession, VillagerType};

pub enum VillagerPersistenceLock {}

impl LockClass for VillagerPersistenceLock {
    const NAME: &'static str = "villager persistence";
    const LEVEL: u8 = 30;
}

/// Registry names that this build cannot represent in the protocol data.
#[derive(Clone, Default)]
pub struct VillagerDataPersistence {
    unknown_type: Option<RetainedRegistryName>,
    unknown_profession: Option<RetainedRegistryName>,
    warned_unknown_registry: bool,
}

#[derive(Clone)]
struct RetainedRegistryName {
    name: String,
    fallback: VarInt,
}

pub struct LoadedVillagerData {
    pub(crate) data: VillagerData,
    unknown_type: Option<String>,
    unknown_profession: Option<String>,
}

impl LoadedVillagerData {
    pub(crate) const fn default_data() -> Self {
        Self {
            data: VillagerData::new(VillagerType::Plains, VillagerProfession::None, 1),
            unknown_type: None,
            unknown_profession: None,
        }
    }

    pub(crate) fn decode(nbt: &NbtCompound, previous: VillagerData) -> Self {
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

    pub(crate) fn unknown_names(&self) -> (Option<&str>, Option<&str>) {
        (
            self.unknown_type.as_deref(),
            self.unknown_profession.as_deref(),
        )
    }
}

impl VillagerDataPersistence {
    pub(crate) const fn has_unknown_names(&self) -> bool {
        self.unknown_type.is_some() || self.unknown_profession.is_some()
    }

    pub(crate) fn replace(&mut self, loaded: &LoadedVillagerData) -> bool {
        self.unknown_type = loaded
            .unknown_type
            .as_ref()
            .map(|name| RetainedRegistryName {
                name: name.clone(),
                fallback: loaded.data.r#type,
            });
        self.unknown_profession =
            loaded
                .unknown_profession
                .as_ref()
                .map(|name| RetainedRegistryName {
                    name: name.clone(),
                    fallback: loaded.data.profession,
                });
        let has_unknown = self.has_unknown_names();
        let should_warn = has_unknown && !self.warned_unknown_registry;
        self.warned_unknown_registry |= has_unknown;
        should_warn
    }

    pub(crate) fn snapshot_for_save(&mut self, data: VillagerData) -> Self {
        // Public native/Wasm writers can bypass the setter, so validate at save time.
        if self
            .unknown_type
            .as_ref()
            .is_some_and(|retained| retained.fallback != data.r#type)
        {
            self.unknown_type = None;
        }
        if self
            .unknown_profession
            .as_ref()
            .is_some_and(|retained| retained.fallback != data.profession)
        {
            self.unknown_profession = None;
        }
        self.clone()
    }

    pub(crate) fn encode(&self, data: VillagerData) -> NbtCompound {
        let mut encoded = NbtCompound::new();
        encoded.put_string(
            "type",
            self.unknown_type.as_ref().map_or_else(
                || format!("minecraft:{}", data.type_enum().to_name()),
                |retained| retained.name.clone(),
            ),
        );
        encoded.put_string(
            "profession",
            self.unknown_profession.as_ref().map_or_else(
                || format!("minecraft:{}", data.profession_enum().to_name()),
                |retained| retained.name.clone(),
            ),
        );
        encoded.put_int("level", data.level.0);
        encoded
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
            .snapshot_for_save(data);
        nbt.put_compound("VillagerData", retained.encode(data));
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

        let should_warn = self
            .villager_data_persistence
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .replace(&loaded);
        if should_warn {
            let (villager_type, profession) = loaded.unknown_names();
            tracing::warn!(
                villager = %self.mob_entity.living_entity.entity.entity_uuid,
                villager_type = ?villager_type,
                profession = ?profession,
                "Preserving unknown villager registry names in saved data"
            );
        }
    }
}
