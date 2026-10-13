use std::sync::Arc;

use rand::{RngExt, rng};

use super::World;
use crate::entity::{Entity, EntityBase, item::ItemEntity};
use pumpkin_data::{entity::EntityType, item_stack::ItemStack};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};

impl World {
    /// Runs drop callbacks before `commit`, then publishes the item only if payment succeeds.
    // ItemUtils.createFilledResult consumes before Player.drop; this keeps spawn callbacks before payment.
    pub(crate) fn drop_stack_with_commit(
        self: &Arc<Self>,
        pos: &BlockPos,
        stack: ItemStack,
        commit: impl FnOnce() -> bool,
    ) -> bool {
        if stack.is_empty() {
            return false;
        }

        let half_height = f64::from(EntityType::ITEM.dimension[1]) / 2.0;
        let spawn_pos = {
            let mut random = rng();
            Vector3::new(
                f64::from(pos.0.x) + 0.5 + random.random_range(-0.25..0.25),
                f64::from(pos.0.y) + 0.5 + random.random_range(-0.25..0.25) - half_height,
                f64::from(pos.0.z) + 0.5 + random.random_range(-0.25..0.25),
            )
        };

        let entity = Entity::new(self.clone(), spawn_pos, &EntityType::ITEM);
        let mut item_event = crate::plugin::api::events::entity::item_spawn::ItemSpawnEvent::new(
            entity.entity_id,
            spawn_pos,
            stack.item.registry_key.to_string(),
        );
        if let Some(server) = self.server.upgrade() {
            server
                .plugin_manager
                .fire_blocking(&server, &mut item_event);
        }

        if item_event.cancelled {
            commit();
            return false;
        }

        let item_entity: Arc<dyn EntityBase> = Arc::new(ItemEntity::new(entity, stack));
        let mut commit = Some(commit);
        let mut commit_result = None;
        let spawned = self.admit_spawn_tree(&item_entity, true, || {
            let result = commit.take().is_some_and(|commit| commit());
            commit_result = Some(result);
            result
        });
        if commit_result.is_none()
            && let Some(commit) = commit.take()
        {
            commit();
        }
        spawned
    }
}
