use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering::SeqCst},
};

use pumpkin_data::{
    Enchantment, data_component_impl::ChargedProjectilesImpl, item::Item, item_stack::ItemStack,
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
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
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
            std::thread::scope(|scope| scope
                .spawn(move || owner.try_lock().is_some())
                .join()
                .unwrap()),
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
    let fixture = DeathTestWorld::new().await;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| action(&fixture)));
    fixture.server.shutdown().await;
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn projectile_item_events_crossbow_multishot_rejected_spawn_keeps_charge_and_wear() {
    for cancelled in [true, false] {
        run(|fixture| {
            let world = fixture.world();
            let player = TestPlayer::new(&world);
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
            let calls = cancelled.then(|| cancel(fixture, &player.player, false));
            CrossbowItem.normal_use(&Item::CROSSBOW, &player.player);
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
        })
        .await;
    }
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
