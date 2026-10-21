use super::{EmittedGameEvent, World};
use crate::{
    entity::{EntityBase, player::Player},
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority, api::events::world::generic_game::GenericGameEvent,
    },
    server::{Server, combat_test_support},
};
use pumpkin_data::{Block, BlockStateId, game_event::GameEvent, item::Item, item_stack::ItemStack};
use pumpkin_protocol::{VarInt, java::server::play::SUseItem};
use pumpkin_util::{
    Hand,
    math::{vector2::Vector2, vector3::Vector3},
};
use std::sync::{Arc, Mutex, Weak};
use uuid::Uuid;

const LIMIT: usize = 64;
static OBSERVERS: Mutex<Vec<(usize, Weak<Mutex<Observations>>)>> = Mutex::new(Vec::new());
pub const DEPARTURE: Vector3<f64> = Vector3::new(8.5, 65.2, 8.5);

#[derive(Debug, PartialEq)]
pub struct Source {
    pub id: i32,
    pub uuid: Uuid,
    pub instance: usize,
    pub world: usize,
    pub position: Vector3<f64>,
    pub life: Option<u64>,
}

#[derive(Debug, PartialEq)]
pub struct Forwarded {
    pub event: GameEvent,
    pub position: Vector3<f64>,
    pub source: Option<Source>,
    pub affected_state: Option<BlockStateId>,
}

#[derive(Debug, PartialEq)]
pub struct Plugin {
    pub key: String,
    pub position: Vector3<f64>,
    pub damage: i32,
    pub count: u8,
    pub health: f32,
}

#[derive(Default)]
pub struct Observations {
    pub plugins: Vec<Plugin>,
    pub forwarded: Vec<Forwarded>,
    pub order: Vec<&'static str>,
}

pub fn observe(world: &World, event: &EmittedGameEvent<'_>) {
    let sink = OBSERVERS
        .lock()
        .unwrap()
        .iter()
        .find(|(id, _)| *id == std::ptr::from_ref(world) as usize)
        .and_then(|(_, sink)| sink.upgrade());
    if let Some(sink) = sink {
        let record = Forwarded {
            event: event.event,
            position: event.position,
            source: event.source.map(|source| Source {
                id: source.entity_id,
                uuid: source.uuid,
                instance: std::ptr::from_ref(source.entity) as usize,
                world: std::ptr::from_ref(source.world) as usize,
                position: source.position,
                life: source.life,
            }),
            affected_state: event.affected_state,
        };
        let mut sink = sink.lock().unwrap();
        assert!(
            sink.order.len() < LIMIT,
            "bounded game-event observation sink overflow"
        );
        sink.forwarded.push(record);
        sink.order.push("dispatch");
    }
}

type Edit = dyn Fn(&Arc<Server>, &mut GenericGameEvent) + Send + Sync;

pub struct Capture {
    pub player: Arc<Player>,
    pub sink: Arc<Mutex<Observations>>,
    pub edit: Option<Box<Edit>>,
}

impl EventHandler<GenericGameEvent> for Capture {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        event: &'a mut GenericGameEvent,
    ) -> BoxFuture<'a, ()> {
        let stack = self.player.inventory().held_item();
        let record = Plugin {
            key: event.event_key.clone(),
            position: event.position,
            damage: stack.get_damage(),
            count: stack.item_count,
            health: self.player.living_entity.health.load(),
        };
        let mut sink = self.sink.lock().unwrap();
        assert!(
            sink.order.len() < LIMIT,
            "bounded plugin observation sink overflow"
        );
        sink.plugins.push(record);
        sink.order.push("plugin");
        drop(sink);
        if let Some(edit) = &self.edit {
            edit(server, event);
        }
        Box::pin(async {})
    }
}

pub struct Scenario {
    pub server: Arc<Server>,
    pub world: Arc<World>,
    pub owner: TestPlayer,
    pub sink: Arc<Mutex<Observations>>,
    pub _dir: tempfile::TempDir,
}

pub fn attach(world: &World) -> Arc<Mutex<Observations>> {
    let sink = Arc::new(Mutex::new(Observations::default()));
    let mut observers = OBSERVERS.lock().unwrap();
    observers
        .retain(|(id, sink)| *id != std::ptr::from_ref(world) as usize && sink.strong_count() > 0);
    assert!(
        observers.len() < LIMIT,
        "bounded game-event observer registry overflow"
    );
    observers.push((std::ptr::from_ref(world) as usize, Arc::downgrade(&sink)));
    sink
}

impl Scenario {
    pub fn new(enabled: bool) -> Self {
        Self::with_movement(enabled, true)
    }

    pub fn with_movement(enabled: bool, movement: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut server = combat_test_support::server(dir.path());
        Arc::get_mut(&mut server)
            .unwrap()
            .advanced_config
            .sculk_game_events
            .enabled = enabled;
        Arc::get_mut(&mut server)
            .unwrap()
            .advanced_config
            .movement_game_events
            .enabled = movement;
        let world = combat_test_support::world(&server, dir.path());
        server.worlds.store(Arc::new(vec![world.clone()]));
        let chunk = pumpkin_world::chunk::ChunkData::empty_sync(0, 0);
        chunk.set_block_absolute_y(8, 64, 8, Block::STONE.default_state.id);
        world.level.loaded_chunks.insert(Vector2::new(0, 0), chunk);
        let owner = TestPlayer::new(&world);
        owner.player.get_entity().set_pos(DEPARTURE);
        let sink = attach(&world);
        Self {
            server,
            world,
            owner,
            sink,
            _dir: dir,
        }
    }

    pub fn capture(&self, edit: Option<Box<Edit>>) {
        self.server.plugin_manager.register::<GenericGameEvent, _>(
            Arc::new(Capture {
                player: self.owner.player.clone(),
                sink: self.sink.clone(),
                edit,
            }),
            EventPriority::Normal,
            true,
        );
    }

    pub fn rod(&self, component: Option<bool>) {
        let mut rod = ItemStack::new(1, &Item::FISHING_ROD);
        if let Some(enabled) = component {
            rod.set_data_component(pumpkin_data::data_component_impl::UseEffectsImpl {
                interact_vibrations: enabled,
                ..pumpkin_data::data_component_impl::UseEffectsImpl::DEFAULT
            });
        } else {
            rod.remove_data_component(pumpkin_data::data_component::DataComponent::UseEffects);
        }
        self.owner
            .player
            .inventory()
            .set_stack_in_hand(Hand::Right, rod);
    }

    pub fn use_item(&self) {
        use_item(&self.owner.player, &self.server);
    }

    pub fn assert_source(&self, source: &Source, position: Vector3<f64>) {
        assert_eq!(source.id, self.owner.player.get_entity().entity_id);
        assert_eq!(source.uuid, self.owner.player.get_entity().entity_uuid);
        assert_eq!(
            source.instance,
            std::ptr::from_ref(self.owner.player.get_entity()) as usize
        );
        assert_eq!(source.world, Arc::as_ptr(&self.world) as usize);
        assert_eq!(source.position, position);
        assert_eq!(
            source.life,
            Some(self.owner.player.living_entity.damage_lifecycle())
        );
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        OBSERVERS
            .lock()
            .unwrap()
            .retain(|(id, _)| *id != Arc::as_ptr(&self.world) as usize);
    }
}

pub fn use_item(player: &Arc<Player>, server: &Arc<Server>) {
    let crate::net::ClientPlatform::Java(client) = player.client.as_ref() else {
        panic!("Java fixture")
    };
    client.handle_use_item(
        player,
        &SUseItem {
            hand: VarInt(0),
            sequence: VarInt(1),
            yaw: 0.0,
            pitch: 0.0,
        },
        server,
    );
}
