use super::World;
use crate::block::entities::{BlockEntity, jukebox::JukeboxBlockEntity};

impl World {
    pub(super) fn bind_block_entity_context(&self, entity: &dyn BlockEntity) {
        if let Some(jukebox) = entity.as_any().downcast_ref::<JukeboxBlockEntity>()
            && let Some(server) = self.server.upgrade()
            && let Some(world) = server
                .worlds
                .load()
                .iter()
                .find(|world| std::ptr::eq(world.as_ref(), self))
        {
            // BlockEntity.setLevel supplies context before inventory automation can mutate it.
            jukebox.bind_world(world);
        }
    }
}
