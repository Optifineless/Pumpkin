use super::{Entity, EntityBase};
use pumpkin_util::math::{boundingbox::BoundingBox, vector3::Vector3};

impl Entity {
    pub(crate) fn check_supporting_block(&self, caller: &dyn EntityBase, on_ground: bool) {
        let support = if on_ground {
            // Entity.checkSupportingBlock first probes the current collision footprint.
            let bounds = self.bounding_box.load();
            let footprint = BoundingBox::new(
                Vector3::new(bounds.min.x, bounds.min.y - 1.0e-6, bounds.min.z),
                Vector3::new(bounds.max.x, bounds.min.y, bounds.max.z),
            );
            let world = self.world.load_full();
            world.find_supporting_block(caller, footprint)
        } else {
            None
        };
        self.supporting_block_pos.store(support);
    }
}
