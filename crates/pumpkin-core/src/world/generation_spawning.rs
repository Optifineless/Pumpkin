//! `WorldGenRegion.addFreshEntity` and publication of generation-stage births.

use std::sync::Arc;

use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::vector2::Vector2;
use pumpkin_world::generation::proto_chunk::GenerationCache;

use super::World;
use crate::entity::EntityBase;

/// `NaturalSpawner.spawnMobsForChunkGeneration`'s placement-to-retention path.
pub(super) fn spawn_mob(
    world: &Arc<World>,
    cache: &mut dyn GenerationCache,
    ty: &'static pumpkin_data::entity::EntityType,
    position: pumpkin_util::math::vector3::Vector3<f64>,
    group: &mut Option<crate::entity::mob::spawn::SpawnGroupData>,
) -> bool {
    use crate::entity::mob::spawn::{SpawnReason, finalize_spawn_in_view};
    let view = super::spawn_view::SpawnView::generation(world, cache);
    let pos = pumpkin_util::math::position::BlockPos::floored_v(position);
    if !view.is_space_empty(ty.get_spawn_bounding_box(position.x, position.y, position.z))
        || !crate::entity::r#type::check_spawn_rules_in_view(
            ty,
            &view,
            &pos,
            false,
            SpawnReason::ChunkGeneration,
        )
    {
        return false;
    }
    // Constructors only establish defaults. SpawnContext consumes the accessor during finalization.
    let entity = crate::entity::r#type::from_type(ty, position, world, uuid::Uuid::new_v4());
    entity
        .get_entity()
        .set_rotation(rand::random::<f32>() * 360.0, 0.0);
    if entity.get_mob().is_some_and(|mob| {
        !crate::entity::mob::walk_target::check(mob, &view)
            || !crate::entity::mob::spawn::check_spawn_obstruction_in_view(mob, &view)
    }) {
        return false;
    }
    *group = finalize_spawn_in_view(
        &entity,
        world,
        &view,
        SpawnReason::ChunkGeneration,
        group.take(),
    );
    retain_entity(cache, &entity);
    true
}

/// Saves the finalized riding tree in its owning generation chunk, without live-world insertion.
pub fn retain_entity(cache: &mut dyn GenerationCache, entity: &Arc<dyn EntityBase>) {
    let root = crate::entity::spawn_mount::spawn_root(entity);
    let entity = &root;
    let _riding_tree = crate::entity::spawn_mount::UnpublishedRidingTree::new(entity);
    materialize_generation_riders(entity);
    let nbt = super::entity_persistence::save_riding_tree(entity);
    // WorldGenRegion.addFreshEntity saves the mob in the chunk containing its position.
    let pos = entity.get_entity().chunk_pos.load();
    if let Some(chunk) = cache.get_chunk_mut(pos.x, pos.y) {
        chunk.pending_entities.push(nbt);
    }
}

fn materialize_generation_riders(entity: &Arc<dyn EntityBase>) {
    crate::entity::spawn_mount::materialize_pending_riders(entity);
    let passengers = entity
        .get_entity()
        .passengers
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    for passenger in passengers {
        materialize_generation_riders(&passenger);
    }
}

impl World {
    pub(super) fn retain_structure_entities(
        self: &Arc<Self>,
        cache: &mut dyn GenerationCache,
        entities: Vec<NbtCompound>,
    ) {
        for nbt in entities {
            let Some(position) = crate::entity::mob::spawn::configured_spawn_position(&nbt) else {
                continue;
            };
            let Some(entity) = crate::entity::mob::spawn::load_spawn_entity(self, &nbt, position)
            else {
                continue;
            };
            // StructureTemplate.placeEntities finalizes fresh mobs after loading template data.
            crate::entity::mob::spawn::finalize_spawn_in_view(
                &entity,
                self,
                &super::spawn_view::SpawnView::generation(self, cache),
                crate::entity::mob::spawn::SpawnReason::Structure,
                None,
            );
            let root = crate::entity::spawn_mount::spawn_root(&entity);
            let chunk_pos = root.get_entity().chunk_pos.load();
            retain_entity(cache, &root);
            if let Some(chunk) = cache.get_chunk_mut(chunk_pos.x, chunk_pos.y)
                && let Some(nbt) = chunk.pending_entities.last_mut()
            {
                nbt.put_bool("MurgicraftFreshStructure", true);
            }
        }
    }

    pub(super) fn register_loaded_chunk_ticks(&self, pos: Vector2<i32>) {
        // ServerChunkCache publication makes the chunk's saved ticks available for ticking.
        if self
            .level
            .read_chunk_sync(&pos, |chunk| {
                chunk.block_ticks.has_ticks() || chunk.fluid_ticks.has_ticks()
            })
            .unwrap_or(false)
        {
            self.level.chunks_with_scheduled_ticks.insert(pos);
        }
    }

    pub(super) fn publish_generated_entities(self: &Arc<Self>, pos: Vector2<i32>) {
        let Some(chunk) = self.level.read_chunk_sync(&pos, Clone::clone) else {
            return;
        };
        let data = chunk.get_custom_data("murgicraft", "generated_entities");
        if self
            .level
            .remove_retained_chunk_custom_data(&chunk, "murgicraft", "generated_entities")
            .is_err()
        {
            return;
        }
        if let Some(list) = data
            .as_ref()
            .and_then(pumpkin_nbt::tag::NbtTag::extract_list)
        {
            for nbt in list
                .iter()
                .filter_map(pumpkin_nbt::tag::NbtTag::extract_compound)
            {
                let Some(position) = crate::entity::mob::spawn::configured_spawn_position(nbt)
                else {
                    continue;
                };
                // StructureTemplate.placeEntities creates fresh births; disk restoration is silent.
                // Finalization already ran against the generation view in either case.
                if let Some(entity) =
                    crate::entity::mob::spawn::load_spawn_entity(self, nbt, position)
                {
                    let mut riding_tree =
                        crate::entity::spawn_mount::UnpublishedRidingTree::new(&entity);
                    let accepted = if nbt.get_bool("MurgicraftFreshStructure").unwrap_or(false) {
                        self.spawn_entity_with_passengers(&entity)
                    } else {
                        self.insert_restored_riding_tree(&entity)
                    };
                    if accepted {
                        riding_tree.keep_links();
                    }
                }
            }
        }
    }
}
