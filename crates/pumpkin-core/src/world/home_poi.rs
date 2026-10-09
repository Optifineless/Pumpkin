use std::sync::{Arc, Weak};

use pumpkin_data::{
    Block, BlockStateId,
    block_properties::{BedPart, WhiteBedLikeProperties},
    tag::{self, Taggable},
};
use pumpkin_util::math::{position::BlockPos, vector2::Vector2};

use super::{World, villager_poi::VillagerPoiStorage};
use crate::entity::EntityBase;

pub(super) struct HomeSite {
    occupied: bool,
    owner: Option<Weak<dyn EntityBase>>,
}

/// Identifies a vanilla HOME POI; occupied heads remain POIs, straw beds are excluded.
pub fn is_home(state: BlockStateId) -> bool {
    #[cfg(test)]
    HOME_STATE_CHECKS.with(|checks| checks.set(checks.get() + 1));
    // PoiTypes.BEDS includes every BedBlock head state, including occupied heads.
    let block = Block::from_state_id(state);
    block != &Block::STRAW_BED
        && block.has_tag(&tag::Block::MINECRAFT_BEDS)
        && WhiteBedLikeProperties::from_state_id(state).part == BedPart::Head
}

impl World {
    pub(super) fn update_home_poi(&self, pos: BlockPos, old: BlockStateId, new: BlockStateId) {
        if !is_home(old) && !is_home(new) {
            return;
        }
        let mut sites = self
            .villager_poi
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut storage = self
            .portal_poi
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if is_home(new) {
            let site = sites
                .homes
                .entry(pos.chunk_position())
                .or_default()
                .entry(pos)
                .or_insert(HomeSite {
                    occupied: false,
                    owner: None,
                });
            site.occupied = WhiteBedLikeProperties::from_state_id(new).occupied;
            // PoiSection.refresh preserves existing PoiRecord tickets, including unloaded owners.
            if storage.free_tickets(&pos, "minecraft:home").is_none() {
                storage.add_with_free_tickets(pos, "minecraft:home", 1);
            }
        } else {
            if let Some(homes) = sites.homes.get_mut(&pos.chunk_position()) {
                homes.remove(&pos);
            }
            storage.remove(&pos);
        }
    }

    pub(super) fn register_chunk_home_pois(&self, pos: Vector2<i32>) {
        // PoiManager.checkConsistencyWithBlocks: rebuild HOME records from loaded block states.
        let Some(chunk) = self
            .level
            .loaded_chunks
            .get(&pos)
            .map(|chunk| chunk.clone())
        else {
            return;
        };
        {
            let sites = self
                .villager_poi
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if sites
                .indexed_home_chunks
                .get(&pos)
                .and_then(Weak::upgrade)
                .is_some_and(|indexed| Arc::ptr_eq(&indexed, &chunk))
            {
                return;
            }
        }
        let min_y = chunk.section.min_y;
        let heads = {
            let sections = chunk
                .section
                .block_sections
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            sections
                .iter()
                .enumerate()
                .filter(|(_, palette)| palette.maybe_has(is_home))
                .flat_map(|(section, palette)| {
                    palette
                        .iter()
                        .enumerate()
                        .filter(|(_, state)| is_home(*state))
                        .map(move |(index, state)| {
                            (
                                BlockPos::new(
                                    pos.x * 16 + (index & 15) as i32,
                                    min_y + (section * 16 + (index >> 8)) as i32,
                                    pos.y * 16 + ((index >> 4) & 15) as i32,
                                ),
                                state,
                            )
                        })
                })
                .collect::<Vec<_>>()
        };
        // PoiSection.refresh also discards saved records whose blocks disappeared while unloaded.
        let stale = self
            .portal_poi
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_in_square(
                BlockPos::new(pos.x * 16 + 8, 0, pos.y * 16 + 8),
                8,
                Some("minecraft:home"),
            )
            .into_iter()
            .filter(|head| {
                head.chunk_position() == pos && !heads.iter().any(|(present, _)| present == head)
            })
            .collect::<Vec<_>>();
        for head in stale {
            self.villager_poi
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .homes
                .get_mut(&pos)
                .map(|homes| homes.remove(&head));
            self.portal_poi
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&head);
        }
        for (head, state) in heads {
            self.update_home_poi(head, Block::AIR.default_state.id, state);
        }
        self.villager_poi
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .indexed_home_chunks
            .insert(pos, Arc::downgrade(&chunk));
    }

    pub(super) fn unregister_chunk_home_pois(&self, pos: Vector2<i32>) {
        // Keep PoiSection.pack's saved tickets; discard this fork's transient block/owner cache.
        let mut sites = self
            .villager_poi
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        sites.homes.remove(&pos);
        sites.indexed_home_chunks.remove(&pos);
    }

    /// Returns unoccupied HOME heads with a free claim ticket in `AcquirePoi`'s 48-block range.
    pub(crate) fn available_homes(&self, origin: BlockPos) -> Vec<BlockPos> {
        // AcquirePoi.SCAN_RANGE; the chunk-load hook also indexes generated and disk-loaded beds.
        const SCAN_RANGE: i32 = 48;
        let chunks = ((origin.0.x - SCAN_RANGE) >> 4..=(origin.0.x + SCAN_RANGE) >> 4)
            .flat_map(|x| {
                ((origin.0.z - SCAN_RANGE) >> 4..=(origin.0.z + SCAN_RANGE) >> 4)
                    .map(move |z| Vector2::new(x, z))
            })
            .collect::<Vec<_>>();
        for &chunk in &chunks {
            self.register_chunk_home_pois(chunk);
        }
        let sites = self
            .villager_poi
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut storage = self
            .portal_poi
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut candidates = chunks
            .iter()
            .filter_map(|chunk| sites.homes.get(chunk))
            .flat_map(|homes| homes.iter())
            .filter_map(|(pos, site)| {
                let distance = pos.to_f64().squared_distance_to_vec(&origin.to_f64());
                (distance <= f64::from(SCAN_RANGE).powi(2)).then_some((distance, pos, site))
            })
            .filter(|(_, pos, site)| {
                storage.free_tickets(pos, "minecraft:home") == Some(1)
                    && !site.occupied
                    && site
                        .owner
                        .as_ref()
                        .and_then(VillagerPoiStorage::live_owner)
                        .is_none()
            })
            .map(|(distance, pos, _)| (distance, *pos))
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            left.0
                .total_cmp(&right.0)
                .then_with(|| left.1.0.x.cmp(&right.1.0.x))
        });
        candidates.into_iter().map(|(_, pos)| pos).collect()
    }

    /// Takes the single HOME ticket atomically; the current owner may renew it after loading.
    pub(crate) fn claim_home(&self, pos: BlockPos, owner: Weak<dyn EntityBase>) -> bool {
        let Some(state) = self.get_block_state_id_if_loaded(&pos) else {
            // ValidateNearbyPoi cannot invalidate a persisted reservation before block publication.
            return owner
                .upgrade()
                .is_some_and(|owner| owner.get_home_pos() == Some(pos));
        };
        if !is_home(state) {
            return false;
        }
        self.update_home_poi(pos, Block::AIR.default_state.id, state);
        let Some(claimant) = owner.upgrade() else {
            return false;
        };
        let mut sites = self
            .villager_poi
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(site) = sites
            .homes
            .get_mut(&pos.chunk_position())
            .and_then(|homes| homes.get_mut(&pos))
        else {
            return false;
        };
        let current = site.owner.as_ref().and_then(VillagerPoiStorage::live_owner);
        // Saved HOME memory renews an existing ticket; a new claimant must have a free ticket.
        let mut storage = self
            .portal_poi
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let restoring = claimant.get_home_pos() == Some(pos);
        if current.as_ref().is_some_and(|current| {
            current.get_entity().entity_uuid != claimant.get_entity().entity_uuid
        }) || !restoring
            && (site.occupied || storage.free_tickets(&pos, "minecraft:home") != Some(1))
        {
            return false;
        }
        site.owner = Some(owner);
        if storage.free_tickets(&pos, "minecraft:home") != Some(0) {
            storage.add_with_free_tickets(pos, "minecraft:home", 0);
        }
        true
    }

    /// Releases a HOME ticket only when it belongs to this villager.
    pub(crate) fn release_home(&self, pos: BlockPos, owner: uuid::Uuid) {
        let mut sites = self
            .villager_poi
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let site = sites
            .homes
            .get_mut(&pos.chunk_position())
            .and_then(|homes| homes.get_mut(&pos));
        if site
            .as_ref()
            .and_then(|site| site.owner.as_ref())
            .and_then(Weak::upgrade)
            .is_none_or(|current| current.get_entity().entity_uuid == owner)
        {
            if let Some(site) = site {
                site.owner = None;
            }
            let mut storage = self
                .portal_poi
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // PoiManager.release does not recreate a record when its bed has disappeared.
            if storage.free_tickets(&pos, "minecraft:home").is_some() {
                storage.add_with_free_tickets(pos, "minecraft:home", 1);
            }
        }
    }
}

#[cfg(test)]
thread_local! { static HOME_STATE_CHECKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
#[cfg(test)]
mod tests;
