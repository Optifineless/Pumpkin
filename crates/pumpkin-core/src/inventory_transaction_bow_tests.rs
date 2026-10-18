use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering::SeqCst},
};

use pumpkin_data::{Block, Enchantment, biome::Biome, item::Item, item_stack::ItemStack};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::{
    VarInt,
    java::server::play::{SPlayerAction, SUseItem, Status},
};
use pumpkin_util::{
    GameMode, Hand,
    math::{position::BlockPos, vector3::Vector3},
};

use crate::{
    entity::{EntityBase, death_test_world::DeathTestWorld, player::Player},
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority, entity::projectile_launch::ProjectileLaunchEvent,
    },
    server::Server,
    world::World,
};

async fn run(action: impl FnOnce(&DeathTestWorld)) {
    let fixture = DeathTestWorld::new().await;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| action(&fixture)));
    fixture.server.shutdown().await;
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

fn ready_player(world: &Arc<World>) -> TestPlayer {
    crate::world::spawn_test_support::publish(
        world,
        crate::world::spawn_test_support::proto(&Biome::PLAINS, &Block::STONE),
    );
    let player = TestPlayer::new(world);
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(8.5, 64.0, 8.5));
    player
}

fn release_bow_through_use_packet(player: &TestPlayer, fixture: &DeathTestWorld) {
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
    assert_eq!(
        *player
            .player
            .living_entity
            .active_hand
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        Some(Hand::Right),
        "the item-use packet did not start bow use"
    );
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

struct LaunchLockCheck {
    player: Arc<Player>,
    calls: Arc<AtomicUsize>,
}

impl EventHandler<ProjectileLaunchEvent> for LaunchLockCheck {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut ProjectileLaunchEvent,
    ) -> BoxFuture<'a, ()> {
        if event.shooter_id == Some(self.player.get_entity().entity_id) {
            let owner = self.player.inventory.slot_transaction(0);
            let acquired = std::thread::scope(|scope| {
                scope
                    .spawn(move || owner.try_lock().is_some())
                    .join()
                    .unwrap()
            });
            assert!(
                acquired,
                "bow launch callback ran under inventory ownership"
            );
            self.calls.fetch_add(1, SeqCst);
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inventory_transactions_bow_infinity_arrow_is_not_recoverable_in_survival() {
    run(|fixture| {
        use crate::entity::projectile::arrow::{ArrowEntity, ArrowPickup};

        let world = fixture.world();
        let player = ready_player(&world);
        let mut bow = ItemStack::new(1, &Item::BOW);
        bow.add_enchantment(&Enchantment::INFINITY, 1);
        player.player.inventory.set_stack(0, bow);
        player
            .player
            .inventory
            .set_stack(1, ItemStack::new(1, &Item::ARROW));

        let calls = Arc::new(AtomicUsize::new(0));
        fixture
            .server
            .plugin_manager
            .register::<ProjectileLaunchEvent, _>(
                Arc::new(LaunchLockCheck {
                    player: player.player.clone(),
                    calls: calls.clone(),
                }),
                EventPriority::Normal,
                true,
            );
        release_bow_through_use_packet(&player, fixture);

        assert_eq!(calls.load(SeqCst), 1);
        assert_eq!(player.player.inventory.get_stack(1).item_count, 1);
        let entities = world.entities.load();
        assert_eq!(entities.len(), 1);
        let arrow = entities[0]
            .cast_any()
            .downcast_ref::<ArrowEntity>()
            .unwrap();
        assert!(matches!(arrow.pickup.load(), ArrowPickup::CreativeOnly));
        arrow.in_ground.store(true, SeqCst);
        arrow.shake_time.store(0, SeqCst);
        arrow.on_player_collision(&player.player);
        assert_eq!(
            player.player.inventory.get_stack(1).item_count,
            1,
            "survival recovered an Infinity arrow that cost no ammunition"
        );
        assert!(!arrow.get_entity().is_removed());
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inventory_transactions_bow_infinity_tipped_and_spectral_arrows_are_paid_and_recoverable() {
    run(|fixture| {
        use crate::entity::projectile::arrow::{ArrowEntity, ArrowPickup};

        let world = fixture.world();
        let player = ready_player(&world);
        for item in [&Item::TIPPED_ARROW, &Item::SPECTRAL_ARROW] {
            let mut bow = ItemStack::new(1, &Item::BOW);
            bow.add_enchantment(&Enchantment::INFINITY, 1);
            player.player.inventory.set_stack(0, bow);
            player
                .player
                .inventory
                .set_stack(1, ItemStack::new(1, item));
            let existing_entities = world.entities.load().len();

            release_bow_through_use_packet(&player, fixture);

            assert!(
                player.player.inventory.get_stack(1).is_empty(),
                "Infinity incorrectly made {} free",
                item.registry_key
            );
            let entities = world.entities.load();
            let arrow = entities[existing_entities]
                .cast_any()
                .downcast_ref::<ArrowEntity>()
                .unwrap();
            assert!(matches!(arrow.pickup.load(), ArrowPickup::Allowed));
            assert_eq!(arrow.item_stack.read().unwrap().item, item);
            arrow.in_ground.store(true, SeqCst);
            arrow.shake_time.store(0, SeqCst);
            arrow.on_player_collision(&player.player);
            assert_eq!(player.player.inventory.count(item), 1);
            assert!(arrow.get_entity().is_removed());
        }
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inventory_transactions_bow_creative_arrow_is_creative_only() {
    run(|fixture| {
        use crate::entity::projectile::arrow::{ArrowEntity, ArrowPickup};

        let world = fixture.world();
        let player = ready_player(&world);
        player.player.gamemode.store(GameMode::Creative);
        player
            .player
            .inventory
            .set_stack(0, ItemStack::new(1, &Item::BOW));
        player
            .player
            .inventory
            .set_stack(1, ItemStack::new(1, &Item::ARROW));

        release_bow_through_use_packet(&player, fixture);

        assert_eq!(player.player.inventory.get_stack(1).item_count, 1);
        let entities = world.entities.load();
        let arrow = entities[0]
            .cast_any()
            .downcast_ref::<ArrowEntity>()
            .unwrap();
        assert!(matches!(arrow.pickup.load(), ArrowPickup::CreativeOnly));
    })
    .await;
}
