use std::sync::atomic::Ordering;

use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use rustc_hash::FxHashMap;
use uuid::Uuid;

use super::{GossipType, VillagerEntity};
use crate::entity::EntityBase;

type Gossips = FxHashMap<Uuid, FxHashMap<GossipType, i32>>;

impl VillagerEntity {
    pub(super) fn snapshot_gossips(&self) -> Gossips {
        self.gossips
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub(super) fn write_gossips_nbt(nbt: &mut NbtCompound, gossips: &Gossips) {
        if gossips.is_empty() {
            return;
        }

        // GossipContainer.GossipEntry.CODEC stores Target, Type, and Value.
        let mut gossip_list = Vec::new();
        for (uuid, entries) in gossips {
            for (gossip_type, value) in entries {
                let mut gossip_nbt = NbtCompound::new();
                gossip_nbt.put_uuid("Target", *uuid);
                gossip_nbt.put_string("Type", gossip_type.name().to_owned());
                gossip_nbt.put_int("Value", *value);
                gossip_list.push(NbtTag::Compound(gossip_nbt));
            }
        }
        nbt.put_list("Gossips", gossip_list);
    }

    pub(super) fn read_gossips_nbt(&self, nbt: &NbtCompound) {
        // Preserve the fork's existing cooldown tag alongside vanilla Gossips.
        if let Some(last_share) = nbt.get_long("LastGossipShare") {
            self.last_gossip_share_time
                .store(last_share, Ordering::Relaxed);
        }

        let Some(gossip_list) = nbt.get_list("Gossips") else {
            return;
        };
        let mut gossips = Gossips::default();
        let mut invalid_targets = 0;
        for tag in gossip_list {
            let Some(gossip_nbt) = tag.extract_compound() else {
                continue;
            };
            // NbtCompound::get_uuid follows UUIDUtil.CODEC: four unsigned 32-bit limbs.
            let Some(uuid) = gossip_nbt.get_uuid("Target") else {
                invalid_targets += 1;
                continue;
            };
            let gossip_type = gossip_nbt
                .get_string("Type")
                .and_then(GossipType::from_name)
                .or_else(|| {
                    gossip_nbt
                        .get_int("Type")
                        .and_then(GossipType::from_legacy_id)
                });
            if let (Some(gossip_type), Some(value)) = (gossip_type, gossip_nbt.get_int("Value")) {
                gossips.entry(uuid).or_default().insert(gossip_type, value);
            }
        }
        if invalid_targets > 0 {
            tracing::warn!(
                villager = %self.get_entity().entity_uuid,
                invalid_targets,
                "Skipped villager gossip entries with invalid Target UUIDs"
            );
        }
        *self
            .gossips
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = gossips;
    }
}
