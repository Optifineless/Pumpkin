use super::*;
use crate::{
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority, api::events::world::generic_game::GenericGameEvent,
    },
    server::{
        Server,
        combat_test_support::{server, world},
    },
};
use pumpkin_data::{data_component::DataComponent, item_stack::ItemStack};
use pumpkin_protocol::{VarInt, java::server::play::SUseItem};
use std::sync::Mutex;

struct InteractionVibrations {
    player: Arc<Player>,
    events: Mutex<Vec<(String, i32)>>,
}

impl EventHandler<GenericGameEvent> for InteractionVibrations {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut GenericGameEvent,
    ) -> BoxFuture<'a, ()> {
        self.events.lock().unwrap().push((
            event.event_key.clone(),
            self.player.inventory().held_item().get_damage(),
        ));
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fishing_use_vibrations_require_enabled_component_and_follow_durability() {
    // FishingRodItem.use -> ItemStack.causeUseVibration: absent and false both suppress it.
    for interact_vibrations in [Some(true), Some(false), None] {
        let dir = tempfile::tempdir().unwrap();
        let server = server(dir.path());
        let world = world(&server, dir.path());
        let owner = TestPlayer::new(&world);
        let mut rod = ItemStack::new(1, &Item::FISHING_ROD);
        if let Some(enabled) = interact_vibrations {
            rod.set_data_component(UseEffectsImpl {
                interact_vibrations: enabled,
                ..UseEffectsImpl::DEFAULT
            });
        } else {
            rod.remove_data_component(DataComponent::UseEffects);
        }
        owner.player.inventory().set_stack_in_hand(Hand::Right, rod);
        let handler = Arc::new(InteractionVibrations {
            player: owner.player.clone(),
            events: Mutex::new(Vec::new()),
        });
        server.plugin_manager.register::<GenericGameEvent, _>(
            handler.clone(),
            EventPriority::Normal,
            true,
        );
        let packet = SUseItem {
            hand: VarInt(0),
            sequence: VarInt(1),
            yaw: 0.0,
            pitch: 0.0,
        };
        owner
            .client()
            .handle_use_item(&owner.player, &packet, &server);
        let hook = world
            .get_entity_by_id(owner.player.fishing_bobber.load(Relaxed))
            .unwrap();
        hook.get_entity().on_ground.store(true, Relaxed);
        owner
            .client()
            .handle_use_item(&owner.player, &packet, &server);
        assert_eq!(owner.player.inventory().held_item().get_damage(), 2);
        let events = handler.events.lock().unwrap();
        if interact_vibrations == Some(true) {
            assert_eq!(
                *events,
                vec![
                    (GameEvent::ItemInteractStart.name().to_owned(), 0),
                    (GameEvent::ItemInteractFinish.name().to_owned(), 2),
                ]
            );
        } else {
            assert!(events.is_empty());
        }
    }
    crate::server::fixture_lifecycle::finish().await;
}
