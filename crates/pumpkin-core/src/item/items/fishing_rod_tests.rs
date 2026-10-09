use crate::entity::EntityBase;
use crate::entity::projectile::fishing_bobber::FishingBobberEntity;
use crate::net::java::combat_test_support::TestPlayer;
use crate::server::combat_test_support::{server, world};
use pumpkin_data::{item::Item, item_stack::ItemStack, sound::Sound};
use pumpkin_protocol::{VarInt, java::server::play::SUseItem, ser::NetworkReadExt};
use pumpkin_util::Hand;
use std::sync::atomic::Ordering::Relaxed;

use crate::plugin::{
    EventHandler, EventPriority,
    api::events::player::{
        fish::{PlayerFishEvent, PlayerFishState},
        player_interact_event::PlayerInteractEvent,
    },
};

#[derive(Default)]
struct FishHandler {
    cancel_interact: std::sync::atomic::AtomicBool,
    cancel_fish: crossbeam::atomic::AtomicCell<Option<PlayerFishState>>,
    states: std::sync::Mutex<Vec<(PlayerFishState, uuid::Uuid, Hand)>>,
}

impl EventHandler<PlayerInteractEvent> for FishHandler {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a std::sync::Arc<crate::server::Server>,
        event: &'a mut PlayerInteractEvent,
    ) -> futures::future::BoxFuture<'a, ()> {
        Box::pin(async move {
            event.cancelled = self.cancel_interact.load(Relaxed);
        })
    }
}

impl EventHandler<PlayerFishEvent> for FishHandler {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a std::sync::Arc<crate::server::Server>,
        event: &'a mut PlayerFishEvent,
    ) -> futures::future::BoxFuture<'a, ()> {
        Box::pin(async move {
            self.states
                .lock()
                .unwrap()
                .push((event.state, event.hook_uuid, event.hand));
            event.cancelled = self.cancel_fish.load() == Some(event.state);
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fishing_packet_events_preserve_cancellation_real_hook_identity_and_retrieval_state() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let fixture = TestPlayer::new(&world);
    fixture
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::new(1, &Item::FISHING_ROD));
    let handler = std::sync::Arc::new(FishHandler::default());
    server.plugin_manager.register::<PlayerInteractEvent, _>(
        handler.clone(),
        EventPriority::Normal,
        true,
    );
    server.plugin_manager.register::<PlayerFishEvent, _>(
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
    handler.cancel_interact.store(true, Relaxed);
    fixture
        .client()
        .handle_use_item(&fixture.player, &packet, &server);
    assert_eq!(fixture.player.fishing_bobber.load(Relaxed), -1);
    assert!(handler.states.lock().unwrap().is_empty());
    handler.cancel_interact.store(false, Relaxed);
    handler.cancel_fish.store(Some(PlayerFishState::Fishing));
    fixture
        .client()
        .handle_use_item(&fixture.player, &packet, &server);
    assert!(world.entities.load().is_empty());
    handler.cancel_fish.store(None);
    fixture
        .client()
        .handle_use_item(&fixture.player, &packet, &server);
    let entity = world
        .get_entity_by_id(fixture.player.fishing_bobber.load(Relaxed))
        .unwrap();
    let bobber = entity
        .cast_any()
        .downcast_ref::<FishingBobberEntity>()
        .unwrap();
    {
        let states = handler.states.lock().unwrap();
        assert!(states.last().unwrap().0 == PlayerFishState::Fishing);
        assert_eq!(states.last().unwrap().1, bobber.entity.entity_uuid);
        drop(states);
        bobber.bite_countdown.store(20, Relaxed);
        handler.cancel_fish.store(Some(PlayerFishState::CaughtFish));
        fixture
            .client()
            .handle_use_item(&fixture.player, &packet, &server);
        assert_eq!(
            fixture.player.fishing_bobber.load(Relaxed),
            bobber.entity.entity_id
        );
        assert_eq!(fixture.player.inventory().held_item().get_damage(), 0);
        assert_eq!(world.entities.load().len(), 1);
        assert!(handler.states.lock().unwrap().last().unwrap().0 == PlayerFishState::CaughtFish);
        bobber.bite_countdown.store(0, Relaxed);
        handler.cancel_fish.store(None);
        fixture
            .client()
            .handle_use_item(&fixture.player, &packet, &server);
        assert_eq!(fixture.player.fishing_bobber.load(Relaxed), -1);
        assert!(handler.states.lock().unwrap().last().unwrap().0 == PlayerFishState::ReelIn);
    };
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn fishing_use_packet_spawns_a_client_visible_owned_bobber_and_throw_sound() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let mut fixture = TestPlayer::new(&world);
    fixture
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::new(1, &Item::FISHING_ROD));
    fixture.take_packets();
    fixture.client().handle_use_item(
        &fixture.player,
        &SUseItem {
            hand: VarInt(0),
            sequence: VarInt(1),
            yaw: 90.0,
            pitch: -30.0,
        },
        &server,
    );
    let id = fixture.player.fishing_bobber.load(Relaxed);
    assert_ne!(id, -1);
    let entity = world.get_entity_by_id(id).unwrap();
    assert!(entity.cast_any().is::<FishingBobberEntity>());
    assert_eq!(
        entity.get_owner_id(),
        Some(fixture.player.get_entity().entity_id)
    );
    let packets = fixture.take_packets();
    assert!(packets.iter().any(|bytes| {
        let mut packet = bytes.as_ref();
        packet.get_var_int().unwrap().0 == pumpkin_data::packet::clientbound::play::SOUND.0
            && packet.get_var_int().unwrap().0 == Sound::EntityFishingBobberThrow as i32 + 1
    }));
    // FishingHook.getAddEntityPacket/recreateFromPacket: zero makes the client discard the hook.
    assert_eq!(
        entity.get_entity().create_spawn_packet().data.0,
        fixture.player.get_entity().entity_id
    );
    let velocity = entity.get_entity().velocity.load();
    assert!(velocity.x < -0.9 && velocity.y > 0.5 && velocity.z.abs() < 0.001);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn fishing_retrieve_packet_uses_offhand_rod_rolls_loot_launches_catches_and_awards_xp() {
    use pumpkin_data::statistic::{CustomStatistic, StatisticCategory};
    use pumpkin_util::math::vector3::Vector3;
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let fixture = TestPlayer::new(&world);
    fixture
        .player
        .advancements
        .lock()
        .unwrap()
        .set_player(&fixture.player);
    fixture
        .player
        .inventory()
        .set_stack_in_hand(Hand::Left, ItemStack::new(1, &Item::FISHING_ROD));
    let use_item = SUseItem {
        hand: VarInt(1),
        sequence: VarInt(1),
        yaw: 0.0,
        pitch: 0.0,
    };
    fixture
        .client()
        .handle_use_item(&fixture.player, &use_item, &server);
    let hook = world
        .get_entity_by_id(fixture.player.fishing_bobber.load(Relaxed))
        .unwrap();
    let bobber = hook
        .cast_any()
        .downcast_ref::<FishingBobberEntity>()
        .unwrap();
    bobber.bite_countdown.store(20, Relaxed);
    bobber
        .entity
        .set_pos(fixture.player.position().add_raw(0.0, 0.0, 16.0));
    let json = serde_json::json!({"pools":[
        {"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:salmon","condition":{"type":"minecraft:match_tool","predicate":{"items":"minecraft:fishing_rod"}}}]},
        {"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:stick"}]}
    ]});
    let mut unsupported = std::collections::BTreeSet::new();
    let table =
        pumpkin_util::loot_table::parse_loot_table(&json.to_string(), &mut unsupported).unwrap();
    assert!(unsupported.is_empty());
    server.datapack_manager.insert_loot_table(
        "minecraft:gameplay/fishing".to_owned(),
        std::sync::Arc::new(table),
    );
    fixture
        .client()
        .handle_use_item(&fixture.player, &use_item, &server);
    assert_eq!(fixture.player.fishing_bobber.load(Relaxed), -1);
    assert_eq!(fixture.player.inventory().off_hand_item().get_damage(), 1);
    assert!(fixture.player.inventory().held_item().is_empty());
    let entities = world.entities.load_full();
    let catches: Vec<_> = entities
        .iter()
        .filter_map(|entity| entity.get_item_entity())
        .collect();
    assert_eq!(catches.len(), 2);
    let items: Vec<_> = catches
        .iter()
        .map(|item| item.get_item_stack().lock().unwrap().item)
        .collect();
    assert!(items.contains(&&Item::SALMON) && items.contains(&&Item::STICK));
    for catch in catches {
        assert_eq!(
            catch.get_entity().velocity.load(),
            Vector3::new(0.0, 0.32, -1.6)
        );
    }
    {
        let stats = fixture.player.stats.lock().unwrap();
        assert_eq!(
            stats.get(
                StatisticCategory::Custom,
                CustomStatistic::FishCaught as i32
            ),
            1
        );
        assert_eq!(
            stats.get(StatisticCategory::Used, i32::from(Item::FISHING_ROD.id)),
            1
        );
        drop(stats);
        assert!(
            fixture
                .player
                .has_advancement(pumpkin_data::advancement::Advancement::HUSBANDRY_FISHY_BUSINESS)
        );
        assert_catch_experience(&entities, &fixture.player);
    };
    crate::server::fixture_lifecycle::finish().await;
}

fn assert_catch_experience(
    entities: &[std::sync::Arc<dyn EntityBase>],
    player: &std::sync::Arc<crate::entity::player::Player>,
) {
    use crate::entity::experience_orb::ExperienceOrbEntity;
    use pumpkin_data::entity::EntityType;
    let orbs: Vec<_> = entities
        .iter()
        .filter(|entity| entity.get_entity().entity_type == &EntityType::EXPERIENCE_ORB)
        .collect();
    assert_eq!(orbs.len(), 2);
    for orb in orbs {
        assert!(orb.cast_any().is::<ExperienceOrbEntity>());
        assert_eq!(
            orb.get_entity().pos.load(),
            player.position().add_raw(0.0, 0.5, 0.5)
        );
        player
            .experience_pick_up_delay
            .store(0, std::sync::atomic::Ordering::Relaxed);
        let total = || {
            pumpkin_util::math::experience::points_to_level(player.get_experience_level())
                + player.get_total_experience()
        };
        let before = total();
        orb.on_player_collision(player);
        assert!((1..=6).contains(&(total() - before)));
    }
}
