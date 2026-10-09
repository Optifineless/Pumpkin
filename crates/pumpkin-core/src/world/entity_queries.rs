use std::sync::Arc;

use pumpkin_util::math::boundingbox::BoundingBox;

use super::World;
use crate::entity::EntityBase;

impl World {
    /// Visits entities and players whose boxes intersect `aabb`.
    /// Entity handles are borrowed; intersecting player handles are upcast once.
    pub fn for_each_in_box(
        &self,
        aabb: &BoundingBox,
        mut visitor: impl FnMut(&Arc<dyn EntityBase>),
    ) {
        // Level.getEntities tests the box before handing an entity to its consumer.
        let entities = self.entities.load();
        let players = self.players.load();
        for entity in entities.iter() {
            if entity.get_entity().bounding_box.load().intersects(aabb) {
                visitor(entity);
            }
        }
        for player in players.iter() {
            if player.get_entity().bounding_box.load().intersects(aabb) {
                let entity: Arc<dyn EntityBase> = player.clone();
                visitor(&entity);
            }
        }
    }
}

#[cfg(test)]
mod tests;
