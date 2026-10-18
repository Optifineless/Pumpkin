use super::World;
use crate::block::entities::{
    BlockEntity, jukebox::JukeboxBlockEntity, map::MapBlockEntity, sign::SignEntityRef,
};

impl World {
    pub(super) fn bind_block_entity_context(&self, entity: &dyn BlockEntity) {
        let jukebox = entity.as_any().downcast_ref::<JukeboxBlockEntity>();
        let sign = SignEntityRef::from_block_entity(entity);
        let map = entity.as_any().downcast_ref::<MapBlockEntity>();
        if jukebox.is_none() && sign.is_none() && map.is_none() {
            return;
        }
        let Some(server) = self.server.upgrade() else {
            return;
        };
        let worlds = server.worlds.load_full();
        let Some(world) = worlds
            .iter()
            .find(|world| std::ptr::eq(world.as_ref(), self))
        else {
            return;
        };
        if let Some(jukebox) = jukebox {
            // BlockEntity.setLevel supplies context before inventory automation can mutate it.
            jukebox.bind_world(world);
        }
        if let Some(sign) = sign {
            sign.front_text().bind_world(world, entity.get_position());
            sign.back_text().bind_world(world, entity.get_position());
        }
        if let Some(map) = map {
            map.bind_world(world);
        }
    }
}
