use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering::SeqCst},
};
use std::time::Duration;

use pumpkin_data::{
    Enchantment,
    data_component_impl::{ChargedProjectilesImpl, UnbreakableImpl},
    item::Item,
    item_stack::ItemStack,
    sound::Sound,
};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::{
    VarInt,
    java::server::play::{SPlayerAction, SUseItem, Status},
};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};

use crate::{
    entity::{EntityBase, death_test_world::DeathTestWorld, player::Player},
    item::{ItemBehaviour, items::crossbow::CrossbowItem},
    net::java::{combat_test_support::TestPlayer, sound_test_support::decode_sounds},
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::player::player_item_damage::PlayerItemDamageEvent,
        entity::{entity_spawn::EntitySpawnEvent, projectile_launch::ProjectileLaunchEvent},
    },
    server::Server,
};

struct Cancel {
    player: Arc<Player>,
    calls: Arc<AtomicUsize>,
}

impl Cancel {
    fn check_unlocked(&self) {
        let owner = self.player.inventory.slot_transaction(0);
        assert!(
            owner.try_lock().is_some(),
            "projectile callback held inventory ownership"
        );
        self.calls.fetch_add(1, SeqCst);
    }
}

impl EventHandler<ProjectileLaunchEvent> for Cancel {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut ProjectileLaunchEvent,
    ) -> BoxFuture<'a, ()> {
        if event.shooter_id == Some(self.player.get_entity().entity_id) {
            self.check_unlocked();
            event.cancelled = true;
        }
        Box::pin(async {})
    }
}

impl EventHandler<EntitySpawnEvent> for Cancel {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut EntitySpawnEvent,
    ) -> BoxFuture<'a, ()> {
        self.check_unlocked();
        event.cancelled = true;
        Box::pin(async {})
    }
}

pub fn cancel(fixture: &DeathTestWorld, player: &Arc<Player>, launch: bool) -> Arc<AtomicUsize> {
    let calls = Arc::new(AtomicUsize::new(0));
    let handler = Arc::new(Cancel {
        player: player.clone(),
        calls: calls.clone(),
    });
    if launch {
        fixture
            .server
            .plugin_manager
            .register::<ProjectileLaunchEvent, _>(handler, EventPriority::Normal, true);
    } else {
        fixture
            .server
            .plugin_manager
            .register::<EntitySpawnEvent, _>(handler, EventPriority::Normal, true);
    }
    calls
}

pub async fn run(action: impl FnOnce(&DeathTestWorld)) {
    let fixture = tokio::time::timeout(Duration::from_secs(60), DeathTestWorld::new())
        .await
        .expect("death test world startup timed out");
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| action(&fixture)));
    tokio::time::timeout(Duration::from_secs(60), fixture.server.shutdown())
        .await
        .expect("death test world shutdown timed out");
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

pub fn use_packet(player: &TestPlayer, fixture: &DeathTestWorld) {
    player.client().handle_use_item(
        &player.player,
        &SUseItem {
            hand: VarInt(0),
            sequence: VarInt(1),
            yaw: 0.0,
            pitch: 0.0,
        },
        &fixture.server,
    );
}

fn release_packet(player: &TestPlayer, fixture: &DeathTestWorld) {
    player
        .player
        .living_entity
        .item_use_time
        .store(72000 - 25, SeqCst);
    player.client().handle_player_action(
        &player.player,
        &SPlayerAction {
            status: VarInt(Status::ReleaseItemInUse as i32),
            position: BlockPos::new(0, 0, 0),
            face: 0,
            sequence: VarInt(2),
        },
        &fixture.server,
    );
}

async fn bow_case(launch: bool) {
    for cancelled in [false, true] {
        run(|fixture| {
            let world = fixture.world();
            let player = TestPlayer::new(&world);
            player
                .player
                .get_entity()
                .set_pos(Vector3::new(8.5, 64.0, 8.5));
            let bow = ItemStack::new(1, &Item::BOW);
            player.player.inventory.set_stack(0, bow);
            player
                .player
                .inventory
                .set_stack(1, ItemStack::new(3, &Item::ARROW));
            let calls = cancelled.then(|| cancel(fixture, &player.player, launch));
            use_packet(&player, fixture);
            release_packet(&player, fixture);
            assert_eq!(
                world.entities.load().len(),
                usize::from(!cancelled),
                "bow entity admission"
            );
            assert_eq!(
                player.player.inventory.count(&Item::ARROW),
                if cancelled { 3 } else { 2 },
                "bow ammunition"
            );
            assert_eq!(
                player.player.inventory.get_stack(0).get_damage(),
                i32::from(!cancelled),
                "bow wear"
            );
            if let Some(calls) = calls {
                assert_eq!(calls.load(SeqCst), 1);
            }
        })
        .await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn projectile_item_events_bow_cancelled_launch_keeps_ammo_and_wear() {
    bow_case(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn projectile_item_events_bow_rejected_spawn_keeps_ammo_and_wear() {
    bow_case(false).await;
}

struct CancelFirstLaunch {
    player: Arc<Player>,
    calls: Arc<AtomicUsize>,
}

impl EventHandler<ProjectileLaunchEvent> for CancelFirstLaunch {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut ProjectileLaunchEvent,
    ) -> BoxFuture<'a, ()> {
        if event.shooter_id == Some(self.player.get_entity().entity_id) {
            let owner = self.player.inventory.slot_transaction(0);
            assert!(
                owner.try_lock().is_some(),
                "projectile callback held inventory ownership"
            );
            event.cancelled = self.calls.fetch_add(1, SeqCst) == 0;
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn projectile_item_events_bow_first_multishot_veto_keeps_paid_arrow() {
    run(|fixture| {
        let world = fixture.world();
        let player = TestPlayer::new(&world);
        let mut bow = ItemStack::new(1, &Item::BOW);
        bow.add_enchantment(&Enchantment::MULTISHOT, 1);
        player.player.inventory.set_stack(0, bow);
        player
            .player
            .inventory
            .set_stack(1, ItemStack::new(3, &Item::ARROW));
        let calls = Arc::new(AtomicUsize::new(0));
        fixture
            .server
            .plugin_manager
            .register::<ProjectileLaunchEvent, _>(
                Arc::new(CancelFirstLaunch {
                    player: player.player.clone(),
                    calls: calls.clone(),
                }),
                EventPriority::Normal,
                true,
            );

        use_packet(&player, fixture);
        release_packet(&player, fixture);

        assert_eq!(calls.load(SeqCst), 3);
        assert_eq!(world.entities.load().len(), 2);
        assert_eq!(player.player.inventory.count(&Item::ARROW), 2);
        assert_eq!(player.player.inventory.get_stack(0).get_damage(), 2);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn projectile_item_events_crossbow_multishot_rejected_spawn_keeps_charge_and_wear() {
    for cancelled in [true, false] {
        run(|fixture| {
            let world = fixture.world();
            let mut player = TestPlayer::new(&world);
            let mut weapon = ItemStack::new(1, &Item::CROSSBOW);
            weapon.add_enchantment(&Enchantment::MULTISHOT, 1);
            player.player.inventory.set_stack(0, weapon.clone());
            player
                .player
                .inventory
                .set_stack(1, ItemStack::new(3, &Item::ARROW));
            CrossbowItem.normal_use(&Item::CROSSBOW, &player.player);
            player
                .player
                .living_entity
                .item_use_time
                .store(72000 - 25, SeqCst);
            CrossbowItem.on_stopped_using(&weapon, &player.player);
            let loaded = player.player.inventory.get_stack(0);
            assert_eq!(
                loaded
                    .get_data_component::<ChargedProjectilesImpl>()
                    .unwrap()
                    .projectiles
                    .len(),
                3
            );
            assert_eq!(
                player.player.inventory.count(&Item::ARROW),
                2,
                "loading paid one arrow"
            );
            player.take_packets();
            let calls = cancelled.then(|| cancel(fixture, &player.player, false));
            use_packet(&player, fixture);
            assert_eq!(world.entities.load().len(), if cancelled { 0 } else { 3 });
            let actual = player.player.inventory.get_stack(0);
            // Vanilla ProjectileWeaponItem.shoot charges wear for each admitted round.
            assert_eq!(actual.get_damage(), if cancelled { 0 } else { 3 });
            assert_eq!(player.player.inventory.count(&Item::ARROW), 2);
            if cancelled {
                assert!(
                    actual.are_equal(&loaded),
                    "rejected rounds must stay charged, including their paid/intangible markers"
                );
            } else {
                assert!(
                    actual
                        .get_data_component::<ChargedProjectilesImpl>()
                        .unwrap()
                        .projectiles
                        .is_empty()
                );
            }
            if let Some(calls) = calls {
                assert_eq!(calls.load(SeqCst), 3);
            }
            let shoot_sounds = decode_sounds(&player.take_packets())
                .iter()
                .filter(|sound| sound.sound_id == Sound::ItemCrossbowShoot as u16)
                .count();
            assert_eq!(shoot_sounds, usize::from(!cancelled));
        })
        .await;
    }
}

struct CountItemDamage(Arc<AtomicUsize>);

impl EventHandler<PlayerItemDamageEvent> for CountItemDamage {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        _: &'a mut PlayerItemDamageEvent,
    ) -> BoxFuture<'a, ()> {
        self.0.fetch_add(1, SeqCst);
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn projectile_item_events_unbreakable_trident_throw_skips_damage_event() {
    run(|fixture| {
        let world = fixture.world();
        let player = TestPlayer::new(&world);
        let mut trident = ItemStack::new(1, &Item::TRIDENT);
        trident.set_data_component(UnbreakableImpl);
        player.player.inventory.set_stack(0, trident);
        let calls = Arc::new(AtomicUsize::new(0));
        fixture
            .server
            .plugin_manager
            .register::<PlayerItemDamageEvent, _>(
                Arc::new(CountItemDamage(calls.clone())),
                EventPriority::Normal,
                true,
            );

        use_packet(&player, fixture);
        release_packet(&player, fixture);

        assert_eq!(world.entities.load().len(), 1);
        assert_eq!(calls.load(SeqCst), 0);
        let entities = world.entities.load();
        let thrown = entities[0]
            .cast_any()
            .downcast_ref::<crate::entity::projectile::trident::TridentEntity>()
            .expect("unbreakable trident projectile");
        let stack = thrown.item_stack.lock().unwrap();
        assert!(stack.is_unbreakable());
        assert_eq!(stack.get_damage(), 0);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn projectile_item_events_snowball_cancelled_launch_keeps_items() {
    for cancelled in [false, true] {
        run(|fixture| {
            let world = fixture.world();
            let player = TestPlayer::new(&world);
            player
                .player
                .inventory
                .set_stack(0, ItemStack::new(3, &Item::SNOWBALL));
            let calls = cancelled.then(|| cancel(fixture, &player.player, true));
            use_packet(&player, fixture);
            assert_eq!(world.entities.load().len(), usize::from(!cancelled));
            assert_eq!(
                player.player.inventory.count(&Item::SNOWBALL),
                if cancelled { 3 } else { 2 }
            );
            if let Some(calls) = calls {
                assert_eq!(calls.load(SeqCst), 1);
            }
        })
        .await;
    }
}
