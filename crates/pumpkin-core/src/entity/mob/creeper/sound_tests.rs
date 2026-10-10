use super::CreeperEntity;
use crate::{
    entity::{Entity, EntityBase},
    net::java::{
        combat_test_support::TestPlayer,
        sound_test_support::{SoundPacket, decode_sounds},
    },
    server::{Server, combat_test_support},
    world::{World, spawn_test_support},
};
use bytes::Bytes;
use pumpkin_data::{
    Block,
    biome::Biome,
    entity::EntityType,
    item::Item,
    item_stack::ItemStack,
    sound::{Sound, SoundCategory},
};
use pumpkin_protocol::{VarInt, java::server::play::SInteract};
use pumpkin_util::{Hand, math::vector3::Vector3};
use std::sync::{Arc, atomic::Ordering::Relaxed};

const CREEPER_POSITION: Vector3<f64> = Vector3::new(9.5, 64.0, 8.5);
const SOUND_POSITION: Vector3<i32> = Vector3::new(76, 512, 68);

struct Fixture {
    server: Arc<Server>,
    world: Arc<World>,
    actor: TestPlayer,
    observer: TestPlayer,
    creeper: Arc<CreeperEntity>,
    _dir: tempfile::TempDir,
}

impl Fixture {
    fn new(hand: Hand) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(dir.path());
        let world = combat_test_support::world(&server, dir.path());
        spawn_test_support::publish(
            &world,
            spawn_test_support::proto(&Biome::PLAINS, &Block::STONE),
        );
        let actor = TestPlayer::new(&world);
        let observer = TestPlayer::new(&world);
        actor
            .player
            .get_entity()
            .set_pos(Vector3::new(8.5, 64.0, 8.5));
        observer
            .player
            .get_entity()
            .set_pos(Vector3::new(10.5, 64.0, 8.5));
        world.players.store(Arc::new(vec![
            actor.player.clone(),
            observer.player.clone(),
        ]));
        actor
            .player
            .inventory()
            .set_stack_in_hand(hand, ItemStack::new(1, &Item::FLINT_AND_STEEL));
        let creeper = CreeperEntity::new(Entity::new(
            world.clone(),
            CREEPER_POSITION,
            &EntityType::CREEPER,
        ));
        creeper.get_entity().has_no_gravity.store(true, Relaxed);
        creeper.mob_entity.set_no_ai(true);
        creeper.mob_entity.persistence_required.store(true, Relaxed);
        assert!(world.spawn_entity(creeper.clone()));
        let tracked = world
            .entity_tracker
            .get_tracked_entity(creeper.get_entity().entity_id)
            .unwrap();
        for player in [&actor.player, &observer.player] {
            tracked.seen_by.insert(player.gameprofile.id);
            tracked.add_pairing(player);
        }
        let mut fixture = Self {
            server,
            world,
            actor,
            observer,
            creeper,
            _dir: dir,
        };
        fixture.take_packets();
        fixture
    }

    fn ignite(&self, hand: Hand) {
        self.actor.client().handle_interact(
            &self.actor.player,
            &SInteract {
                entity_id: self.creeper.get_entity().entity_id.into(),
                r#type: VarInt(2),
                target_position: Some(Vector3::new(0.0, 0.5, 0.0)),
                hand: Some(VarInt(i32::from(hand == Hand::Left))),
                sneaking: false,
            },
            &self.server,
        );
        assert!(self.creeper.is_ignited());
        assert_eq!(
            self.actor
                .player
                .inventory()
                .get_stack_in_hand(hand)
                .get_damage(),
            1,
        );
    }

    fn take_packets(&mut self) -> [Vec<Bytes>; 2] {
        [self.actor.take_packets(), self.observer.take_packets()]
    }

    async fn finish(self) {
        self.world.level.shutdown().await.unwrap();
    }
}

fn assert_creeper_sound(actual: &SoundPacket, expected: Sound) {
    assert_eq!(actual.sound_id, expected as u16);
    assert_eq!(actual.category, SoundCategory::Hostile as i32);
    assert_eq!(actual.position, SOUND_POSITION);
    assert_eq!(actual.volume, 1.0);
}

async fn check_ignition(hand: Hand) {
    let mut fixture = Fixture::new(hand);
    fixture.ignite(hand);
    assert_eq!(fixture.creeper.current_fuse_time.load(Relaxed), 0);
    let [own_packets, observed_packets] = fixture.take_packets();
    fixture.finish().await;
    let observed = decode_sounds(&observed_packets);
    assert_eq!(observed.len(), 1);
    assert_creeper_sound(&observed[0], Sound::ItemFlintandsteelUse);
    assert!((0.8..=1.2).contains(&observed[0].pitch));
    // Creeper.mobInteract passes the initiating player as the sound exception.
    assert!(decode_sounds(&own_packets).is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue99_creeper_main_hand_ignition_excludes_actor() {
    check_ignition(Hand::Right).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue99_creeper_offhand_ignition_excludes_actor() {
    check_ignition(Hand::Left).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue99_creeper_primed_sound_still_reaches_both_players_once() {
    for hand in Hand::all() {
        let mut fixture = Fixture::new(hand);
        fixture.ignite(hand);
        let ignition = fixture.take_packets();
        fixture.world.tick(&fixture.server);
        assert_eq!(fixture.creeper.current_fuse_time.load(Relaxed), 1);
        let first_tick = fixture.take_packets();
        fixture.world.tick(&fixture.server);
        assert_eq!(fixture.creeper.current_fuse_time.load(Relaxed), 2);
        let second_tick = fixture.take_packets();
        fixture.finish().await;
        assert_eq!(decode_sounds(&ignition[1]).len(), 1);
        // Creeper.tick -> Entity.playSound has no initiating-player exception.
        let actor_sounds = decode_sounds(&first_tick[0]);
        let observer_sounds = decode_sounds(&first_tick[1]);
        assert_eq!(actor_sounds.len(), 1);
        assert_creeper_sound(&actor_sounds[0], Sound::EntityCreeperPrimed);
        assert_eq!(actor_sounds[0].pitch, 0.5);
        assert_eq!(actor_sounds, observer_sounds);
        for packets in second_tick {
            assert!(decode_sounds(&packets).is_empty());
        }
    }
}
