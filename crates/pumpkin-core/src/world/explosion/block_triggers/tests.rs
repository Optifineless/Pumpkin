use super::*;
use crate::{
    plugin::{
        BoxFuture, EventHandler, EventPriority, api::events::world::generic_game::GenericGameEvent,
    },
    server::combat_test_support,
    world::{
        explosion::BlockInteraction,
        spawn_test_support::{proto, publish},
    },
};
use pumpkin_data::biome::Biome;

struct Events(std::sync::Mutex<Vec<String>>);
impl EventHandler<GenericGameEvent> for Events {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<crate::server::Server>,
        event: &'a mut GenericGameEvent,
    ) -> BoxFuture<'a, ()> {
        self.0.lock().unwrap().push(event.event_key.clone());
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn followup2_wind_charge_door_toggle_emits_one_game_event() {
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let events = Arc::new(Events(std::sync::Mutex::new(Vec::new())));
    server.plugin_manager.register::<GenericGameEvent, _>(
        events.clone(),
        EventPriority::Normal,
        true,
    );
    let pos = BlockPos::new(5, 64, 5);
    world.set_block_state(
        &pos,
        Block::OAK_DOOR.default_state.id,
        BlockFlags::NOTIFY_ALL,
    );
    let mut explosion = Explosion::new(1.0, pos.to_centered_f64(), BlockInteraction::TriggerBlock);
    explosion.source = Some(crate::entity::r#type::from_type(
        &EntityType::WIND_CHARGE,
        pos.to_centered_f64(),
        &world,
        uuid::Uuid::new_v4(),
    ));
    let state = world.get_block_state(&pos);
    explosion.trigger_block(&world, &pos, &Block::OAK_DOOR, state);
    assert_eq!(*events.0.lock().unwrap(), vec![GameEvent::BlockOpen.name()]);
    world.level.shutdown().await.unwrap();
}
