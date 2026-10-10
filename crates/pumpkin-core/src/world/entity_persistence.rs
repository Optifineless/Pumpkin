//! Entity.save/saveWithoutId and EntityType.loadPassengersRecursive.
use std::sync::{Arc, PoisonError};

use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_util::math::vector2::Vector2;

use super::World;
use crate::entity::{
    EntityBase, RemovalReason, player::Player, spawn_mount::UnpublishedRidingTree,
};

pub(super) fn should_save_root(entity: &dyn EntityBase) -> bool {
    // Entity.save excludes passengers; PersistentEntitySectionManager stores unloading trees.
    let base = entity.get_entity();
    (!base.is_removed() || base.removal_reason.load() == Some(RemovalReason::UnloadedToChunk))
        && base.get_vehicle().is_none()
}

pub(super) fn save_riding_tree(entity: &Arc<dyn EntityBase>) -> NbtCompound {
    let mut nbt = NbtCompound::new();
    entity.write_nbt(&mut nbt);
    // Entity.saveWithoutId stores a passenger's X/Z at its vehicle, retaining its own Y.
    if let Some(vehicle) = entity.get_entity().get_vehicle() {
        let pos = vehicle.get_entity().pos.load();
        nbt.put(
            "Pos",
            NbtTag::List(vec![
                NbtTag::Double(pos.x),
                NbtTag::Double(entity.get_entity().pos.load().y),
                NbtTag::Double(pos.z),
            ]),
        );
    }
    let passengers = entity
        .get_entity()
        .passengers
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let saved: Vec<_> = passengers
        .iter()
        .filter(|passenger| {
            let base = passenger.get_entity();
            (!base.is_removed()
                || base.removal_reason.load()
                    == Some(crate::entity::RemovalReason::UnloadedToChunk))
                && passenger.get_player().is_none()
        })
        .map(|passenger| NbtTag::Compound(save_riding_tree(passenger)))
        .collect();
    if !saved.is_empty() {
        nbt.put("Passengers", NbtTag::List(saved));
    }
    nbt
}

/// Unloads every member with its saved root, including passengers straddling a chunk boundary.
pub(super) fn root_chunk(entity: &Arc<dyn EntityBase>) -> Vector2<i32> {
    let mut root = entity.clone();
    while let Some(vehicle) = root.get_entity().get_vehicle() {
        root = vehicle;
    }
    root.get_entity().chunk_pos.load()
}

pub(super) fn detach_unloaded_trees(entities: &[Arc<dyn EntityBase>]) {
    for entity in entities {
        if entity.get_entity().get_vehicle().is_none() {
            drop(UnpublishedRidingTree::new(entity));
        }
    }
}

impl World {
    pub(super) fn restore_entity_tree(
        self: &Arc<Self>,
        nbt: &NbtCompound,
        player: Option<&Arc<Player>>,
    ) {
        let Some(position) = crate::entity::mob::spawn::configured_spawn_position(nbt) else {
            return;
        };
        let Some(entity) = crate::entity::mob::spawn::load_spawn_entity(self, nbt, position) else {
            return;
        };
        let mut tree = UnpublishedRidingTree::new(&entity);
        if self.insert_restored_riding_tree(&entity) {
            tree.keep_links();
            if let Some(player) = player {
                player.try_restore_vehicle(&entity);
            }
        }
    }
}
