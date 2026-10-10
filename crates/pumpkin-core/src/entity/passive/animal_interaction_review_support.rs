use super::super::*;
use crate::{
    entity::player::Player,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::{
            entity::entity_regain_health::EntityRegainHealthEvent,
            world::generic_game::GenericGameEvent,
        },
    },
};
use pumpkin_data::game_event::GameEvent;
use pumpkin_protocol::ser::NetworkReadExt;
use pumpkin_util::Hand;
use std::sync::Mutex;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct SoundPacket {
    pub id: i32,
    pub category: i32,
    pub position: Vector3<i32>,
    pub volume: f32,
    pub pitch: f32,
}

pub(super) fn take_sounds(player: &mut TestPlayer) -> Vec<SoundPacket> {
    player
        .take_packets()
        .into_iter()
        .filter_map(|bytes| {
            let mut data = bytes.as_ref();
            if data.get_var_int().unwrap().0 != pumpkin_data::packet::clientbound::play::SOUND.0 {
                return None;
            }
            Some(SoundPacket {
                id: data.get_var_int().unwrap().0 - 1,
                category: data.get_var_int().unwrap().0,
                position: Vector3::new(
                    data.get_i32().unwrap(),
                    data.get_i32().unwrap(),
                    data.get_i32().unwrap(),
                ),
                volume: data.get_f32().unwrap(),
                pitch: data.get_f32().unwrap(),
            })
        })
        .collect()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Stage {
    Heal,
    Interact,
}

pub(super) struct Observation {
    pub stage: Stage,
    pub held: ItemStack,
    pub health: f32,
    pub position: Option<Vector3<f64>>,
    pub sounds: Vec<SoundPacket>,
}

pub(super) struct Effects {
    player: Arc<Player>,
    target: Arc<dyn EntityBase>,
    hand: Hand,
    observer: Mutex<TestPlayer>,
    pub observations: Mutex<Vec<Observation>>,
    pub replacement: Mutex<Option<(Stage, ItemStack)>>,
}

impl Effects {
    pub fn watch(fixture: &mut Fixture, target: Arc<dyn EntityBase>, hand: Hand) -> Arc<Self> {
        let mut observer = TestPlayer::new(&fixture.world);
        observer
            .player
            .get_entity()
            .set_pos(Vector3::new(10.5, 64.0, 8.5));
        fixture.world.players.store(Arc::new(vec![
            fixture.player.player.clone(),
            observer.player.clone(),
        ]));
        fixture.player.take_packets();
        observer.take_packets();
        let effects = Arc::new(Self {
            player: fixture.player.player.clone(),
            target,
            hand,
            observer: Mutex::new(observer),
            observations: Mutex::new(Vec::new()),
            replacement: Mutex::new(None),
        });
        fixture
            .server
            .plugin_manager
            .register::<GenericGameEvent, _>(effects.clone(), EventPriority::Normal, true);
        fixture
            .server
            .plugin_manager
            .register::<EntityRegainHealthEvent, _>(effects.clone(), EventPriority::Normal, true);
        effects
    }

    fn record(&self, stage: Stage, position: Option<Vector3<f64>>) {
        let held = self.player.inventory().get_stack_in_hand(self.hand);
        let health = self.target.get_living_entity().unwrap().health.load();
        let sounds = take_sounds(&mut self.observer.lock().unwrap());
        self.observations.lock().unwrap().push(Observation {
            stage,
            held,
            health,
            position,
            sounds,
        });
        let replacement = self
            .replacement
            .lock()
            .unwrap()
            .as_ref()
            .filter(|(at, _)| *at == stage)
            .map(|(_, stack)| stack.clone());
        if let Some(replacement) = replacement {
            self.player
                .inventory()
                .set_stack_in_hand(self.hand, replacement);
        }
    }

    pub fn sounds(&self) -> Vec<SoundPacket> {
        let mut sounds: Vec<_> = self
            .observations
            .lock()
            .unwrap()
            .iter()
            .flat_map(|event| event.sounds.clone())
            .collect();
        sounds.extend(take_sounds(&mut self.observer.lock().unwrap()));
        sounds
    }
}

impl EventHandler<GenericGameEvent> for Effects {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut GenericGameEvent,
    ) -> BoxFuture<'a, ()> {
        if event.event_key == GameEvent::EntityInteract.name() {
            self.record(Stage::Interact, Some(event.position));
        }
        Box::pin(async {})
    }
}

impl EventHandler<EntityRegainHealthEvent> for Effects {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut EntityRegainHealthEvent,
    ) -> BoxFuture<'a, ()> {
        if event.entity_id == self.target.get_entity().entity_id {
            self.record(Stage::Heal, None);
        }
        Box::pin(async {})
    }
}

pub(super) fn wolf(fixture: &Fixture) -> Arc<WolfEntity> {
    let wolf = WolfEntity::new(fixture.entity(&EntityType::WOLF));
    wolf.set_tame(true);
    wolf.set_owner(Some(fixture.player.player.gameprofile.id));
    wolf.mob_entity.living_entity.set_max_health(40.0);
    wolf.mob_entity.living_entity.set_health(10.0);
    assert!(fixture.world.spawn_entity(wolf.clone()));
    wolf
}
