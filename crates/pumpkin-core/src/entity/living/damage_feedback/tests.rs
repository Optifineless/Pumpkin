use crate::{
    entity::{EntityBase, r#type::from_type},
    net::java::{
        combat_test_support::TestPlayer,
        sound_test_support::{SoundPacket, decode_sounds},
    },
    server::{Server, combat_test_support},
    world::World,
};
use bytes::Bytes;
use pumpkin_data::{
    damage::DamageType,
    entity::{EntityStatus, EntityType},
    packet::clientbound::play::{
        DAMAGE_EVENT, ENTITY_EVENT, HURT_ANIMATION, PLAYER_COMBAT_KILL, SET_HEALTH,
    },
    sound::{Sound, SoundCategory},
};
use pumpkin_protocol::ser::NetworkReadExt;
use pumpkin_util::math::{vector2::Vector2, vector3::Vector3};
use std::sync::{Arc, atomic::Ordering::Relaxed};

const PLAYER_POSITION: Vector3<f64> = Vector3::new(8.5, 64.0, 8.5);
const PLAYER_SOUND_POSITION: Vector3<i32> = Vector3::new(68, 512, 68);
const COW_SOUND_POSITION: Vector3<i32> = Vector3::new(76, 512, 68);

struct Fixture {
    world: Arc<World>,
    victim: TestPlayer,
    observer: TestPlayer,
    _server: Arc<Server>,
    _dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(dir.path());
        let world = combat_test_support::world(&server, dir.path());
        combat_test_support::publish_empty_chunk(&world, Vector2::new(0, 0));
        let victim = TestPlayer::new(&world);
        let observer = TestPlayer::new(&world);
        victim.player.get_entity().set_pos(PLAYER_POSITION);
        observer
            .player
            .get_entity()
            .set_pos(Vector3::new(10.5, 64.0, 8.5));
        world.players.store(Arc::new(vec![
            victim.player.clone(),
            observer.player.clone(),
        ]));
        let tracked = world
            .entity_tracker
            .get_tracked_entity(victim.player.entity_id())
            .unwrap();
        tracked.seen_by.insert(observer.player.gameprofile.id);
        tracked.add_pairing(&observer.player);
        let mut fixture = Self {
            world,
            victim,
            observer,
            _server: server,
            _dir: dir,
        };
        fixture.take_packets();
        fixture
    }

    fn spawn_cow(&mut self) -> Arc<dyn EntityBase> {
        let cow = from_type(
            &EntityType::COW,
            Vector3::new(9.5, 64.0, 8.5),
            &self.world,
            uuid::Uuid::new_v4(),
        );
        assert!(self.world.spawn_entity(cow.clone()));
        let tracked = self
            .world
            .entity_tracker
            .get_tracked_entity(cow.get_entity().entity_id)
            .unwrap();
        for player in [&self.victim.player, &self.observer.player] {
            tracked.seen_by.insert(player.gameprofile.id);
            tracked.add_pairing(player);
        }
        self.take_packets();
        cow
    }

    fn take_packets(&mut self) -> [Vec<Bytes>; 2] {
        [self.victim.take_packets(), self.observer.take_packets()]
    }

    async fn finish(self) {
        self.world.level.shutdown().await.unwrap();
    }
}

fn packet_bodies(packets: &[Bytes], packet_id: i32) -> impl Iterator<Item = &[u8]> {
    packets.iter().filter_map(move |packet| {
        let mut data = packet.as_ref();
        (data.get_var_int().unwrap().0 == packet_id).then_some(data)
    })
}

fn assert_damage_event(packets: &[Bytes], entity_id: i32, damage_type: DamageType) {
    let mut events = packet_bodies(packets, DAMAGE_EVENT.0);
    let mut data = events.next().unwrap();
    assert_eq!(data.get_var_int().unwrap().0, entity_id);
    assert_eq!(data.get_var_int().unwrap().0, i32::from(damage_type.id));
    assert_eq!(data.get_var_int().unwrap().0, 0);
    assert_eq!(data.get_var_int().unwrap().0, 0);
    assert!(!data.get_bool().unwrap());
    assert!(data.is_empty());
    assert!(events.next().is_none());
}

fn assert_death_event(packets: &[Bytes], entity_id: i32) {
    let mut events = packet_bodies(packets, ENTITY_EVENT.0);
    let mut data = events.next().unwrap();
    assert_eq!(data.get_i32_be().unwrap(), entity_id);
    assert_eq!(data.get_i8().unwrap(), EntityStatus::Death as i8);
    assert!(data.is_empty());
    assert!(events.next().is_none());
}

fn assert_health(packets: &[Bytes], health: f32) {
    let mut updates = packet_bodies(packets, SET_HEALTH.0);
    let mut data = updates.next().unwrap();
    assert_eq!(data.get_f32_be().unwrap(), health);
    assert_eq!(data.get_var_int().unwrap().0, 20);
    assert_eq!(data.get_f32_be().unwrap(), 5.0);
    assert!(data.is_empty());
    assert!(updates.next().is_none());
}

fn assert_combat_death(packets: &[Bytes], entity_id: i32) {
    let mut events = packet_bodies(packets, PLAYER_COMBAT_KILL.0);
    let mut data = events.next().unwrap();
    assert_eq!(data.get_var_int().unwrap().0, entity_id);
    assert!(!data.is_empty(), "combat death includes a death message");
    assert!(events.next().is_none());
}

fn assert_sound(
    actual: &SoundPacket,
    expected: Sound,
    category: SoundCategory,
    position: Vector3<i32>,
) {
    assert_eq!(actual.sound_id, expected as u16);
    assert_eq!(actual.category, category as i32);
    assert_eq!(actual.position, position);
    assert_eq!(actual.volume, 1.0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue99_player_hurt_excludes_victim_and_preserves_damage_effects() {
    let mut fixture = Fixture::new();
    let victim = fixture.victim.player.clone();
    let mut captures = Vec::new();
    for (kind, sound, health) in [
        (DamageType::GENERIC, Sound::EntityPlayerHurt, 18.0),
        (DamageType::DROWN, Sound::EntityPlayerHurtDrown, 16.0),
        (DamageType::ON_FIRE, Sound::EntityPlayerHurtOnFire, 14.0),
        (
            DamageType::SWEET_BERRY_BUSH,
            Sound::EntityPlayerHurtSweetBerryBush,
            12.0,
        ),
        (DamageType::FREEZE, Sound::EntityPlayerHurtFreeze, 10.0),
    ] {
        victim.living_entity.hurt_cooldown.store(0, Relaxed);
        fixture.take_packets();
        assert!(victim.damage(victim.as_ref(), 2.0, kind));
        assert_eq!(victim.living_entity.health.load(), health);
        captures.push((kind, sound, health, fixture.take_packets()));
    }
    fixture.finish().await;
    // ClientPacketListener.handleDamageEvent supplies the victim's local sound.
    for (kind, sound, health, [own_packets, observed_packets]) in captures {
        assert_damage_event(&own_packets, victim.entity_id(), kind);
        assert_damage_event(&observed_packets, victim.entity_id(), kind);
        assert_health(&own_packets, health);
        let observed = decode_sounds(&observed_packets);
        assert_eq!(observed.len(), 1, "{kind:?}");
        assert_sound(
            &observed[0],
            sound,
            SoundCategory::Players,
            PLAYER_SOUND_POSITION,
        );
        assert_eq!(observed[0].pitch, 1.0);
        assert!(decode_sounds(&own_packets).is_empty(), "{kind:?}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue99_player_death_excludes_victim_and_keeps_death_event() {
    let mut fixture = Fixture::new();
    let victim = fixture.victim.player.clone();
    assert!(victim.damage(victim.as_ref(), 100.0, DamageType::GENERIC));
    assert_eq!(victim.living_entity.health.load(), 0.0);
    assert!(victim.living_entity.dead.load(Relaxed));
    let [own_packets, observed_packets] = fixture.take_packets();
    fixture.finish().await;
    for packets in [&own_packets, &observed_packets] {
        assert_damage_event(packets, victim.entity_id(), DamageType::GENERIC);
        // LivingEntity.handleEntityEvent(3) plays the victim's death sound locally.
        assert_death_event(packets, victim.entity_id());
    }
    assert_combat_death(&own_packets, victim.entity_id());
    let observed = decode_sounds(&observed_packets);
    assert_eq!(observed.len(), 1);
    assert_sound(
        &observed[0],
        Sound::EntityPlayerDeath,
        SoundCategory::Players,
        PLAYER_SOUND_POSITION,
    );
    assert_eq!(observed[0].pitch, 1.0);
    assert!(decode_sounds(&own_packets).is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue99_thorns_excludes_only_the_primary_player_sound() {
    let mut fixture = Fixture::new();
    let victim = fixture.victim.player.clone();
    assert!(victim.damage(victim.as_ref(), 2.0, DamageType::THORNS));
    assert_eq!(victim.living_entity.health.load(), 18.0);
    let [own_packets, observed_packets] = fixture.take_packets();
    fixture.finish().await;
    for packets in [&own_packets, &observed_packets] {
        assert_damage_event(packets, victim.entity_id(), DamageType::THORNS);
    }
    assert_health(&own_packets, 18.0);
    let observed = decode_sounds(&observed_packets);
    assert_eq!(observed.len(), 2);
    for (actual, expected) in observed
        .iter()
        .zip([Sound::EntityPlayerHurt, Sound::EnchantThornsHit])
    {
        assert_sound(
            actual,
            expected,
            SoundCategory::Players,
            PLAYER_SOUND_POSITION,
        );
        assert_eq!(actual.pitch, 1.0);
    }
    // LivingEntity.playSecondaryHurtSound passes a null exception for THORNS.
    assert_eq!(decode_sounds(&own_packets), observed[1..]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue99_nonplayer_hurt_remains_a_broadcast() {
    let mut fixture = Fixture::new();
    let cow = fixture.spawn_cow();
    let living = cow.get_living_entity().unwrap();
    let health = living.health.load();
    assert!(cow.damage(cow.as_ref(), 2.0, DamageType::GENERIC));
    assert_eq!(living.health.load(), health - 2.0);
    let packets = fixture.take_packets();
    fixture.finish().await;
    for capture in &packets {
        assert_damage_event(capture, cow.get_entity().entity_id, DamageType::GENERIC);
    }
    let first = decode_sounds(&packets[0]);
    let second = decode_sounds(&packets[1]);
    assert_eq!(first.len(), 1);
    assert_eq!(second.len(), 1);
    assert_sound(
        &first[0],
        Sound::EntityCowHurt,
        SoundCategory::Neutral,
        COW_SOUND_POSITION,
    );
    assert!((0.8..=1.2).contains(&first[0].pitch));
    assert_eq!(first[0].seed, second[0].seed);
    assert_eq!(first, second);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue99_cooldown_rejection_and_excess_do_not_replay_feedback() {
    let mut fixture = Fixture::new();
    let victim = fixture.victim.player.clone();
    assert!(victim.damage(victim.as_ref(), 6.0, DamageType::GENERIC));
    assert_eq!(victim.living_entity.health.load(), 14.0);
    let initial = fixture.take_packets();
    assert!(!victim.damage(victim.as_ref(), 6.0, DamageType::GENERIC));
    assert_eq!(victim.living_entity.health.load(), 14.0);
    let rejected = fixture.take_packets();
    assert!(victim.damage(victim.as_ref(), 10.0, DamageType::GENERIC));
    assert_eq!(victim.living_entity.health.load(), 10.0);
    let excess = fixture.take_packets();
    fixture.finish().await;
    for packets in &initial {
        assert_damage_event(packets, victim.entity_id(), DamageType::GENERIC);
    }
    assert_health(&initial[0], 14.0);
    assert_health(&excess[0], 10.0);
    assert_eq!(decode_sounds(&initial[1]).len(), 1);
    for packets in rejected.iter().chain(&excess) {
        assert!(decode_sounds(packets).is_empty());
        assert!(packet_bodies(packets, DAMAGE_EVENT.0).next().is_none());
        assert!(packet_bodies(packets, HURT_ANIMATION.0).next().is_none());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue99_silent_player_hurt_still_reaches_the_observer() {
    let mut fixture = Fixture::new();
    let victim = fixture.victim.player.clone();
    victim.get_entity().set_silent(true);
    fixture.take_packets();
    assert!(victim.damage(victim.as_ref(), 2.0, DamageType::GENERIC));
    assert_eq!(victim.living_entity.health.load(), 18.0);
    let [own_packets, observed_packets] = fixture.take_packets();
    fixture.finish().await;
    for packets in [&own_packets, &observed_packets] {
        assert_damage_event(packets, victim.entity_id(), DamageType::GENERIC);
    }
    assert_health(&own_packets, 18.0);
    // Player.playSound overrides Entity.playSound without its Silent check.
    let observed = decode_sounds(&observed_packets);
    assert_eq!(observed.len(), 1);
    assert_sound(
        &observed[0],
        Sound::EntityPlayerHurt,
        SoundCategory::Players,
        PLAYER_SOUND_POSITION,
    );
    assert_eq!(observed[0].pitch, 1.0);
    assert!(decode_sounds(&own_packets).is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue99_silent_mob_hurt_suppresses_only_its_sound() {
    let mut fixture = Fixture::new();
    let cow = fixture.spawn_cow();
    cow.get_entity().set_silent(true);
    fixture.take_packets();
    let living = cow.get_living_entity().unwrap();
    let health = living.health.load();
    assert!(cow.damage(cow.as_ref(), 2.0, DamageType::GENERIC));
    assert_eq!(living.health.load(), health - 2.0);
    let packets = fixture.take_packets();
    fixture.finish().await;
    for capture in &packets {
        assert_damage_event(capture, cow.get_entity().entity_id, DamageType::GENERIC);
        assert!(decode_sounds(capture).is_empty());
    }
}
