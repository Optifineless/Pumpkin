use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering::SeqCst},
};

use pumpkin_data::{
    Enchantment,
    data_component_impl::{ChargedProjectilesImpl, CustomDataImpl, IntangibleProjectileImpl},
    item::Item,
    item_stack::ItemStack,
};
use pumpkin_inventory::Inventory;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::{GameMode, Hand};

use crate::{
    block::entities::chest::ChestBlockEntity,
    entity::{EntityBase, death_test_world::DeathTestWorld, player::Player},
    inventory_transaction_tests::{click, open_chest},
    item::{ItemBehaviour, items::crossbow::CrossbowItem},
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority, entity::projectile_launch::ProjectileLaunchEvent,
    },
    server::Server,
};

async fn run(action: impl FnOnce(&DeathTestWorld)) {
    let fixture = DeathTestWorld::new().await;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| action(&fixture)));
    fixture.server.shutdown().await;
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

fn charged(stack: &ItemStack) -> Vec<ItemStack> {
    stack
        .get_data_component::<ChargedProjectilesImpl>()
        .unwrap()
        .projectiles
        .iter()
        .map(|nbt| ItemStack::read_item_stack(nbt).unwrap())
        .collect()
}

fn release(player: &Player, observed: &ItemStack) {
    player.living_entity.item_use_time.store(72000 - 25, SeqCst);
    CrossbowItem.on_stopped_using(observed, player);
}

struct LaunchEdit {
    player: Arc<Player>,
    calls: Arc<AtomicUsize>,
}
impl EventHandler<ProjectileLaunchEvent> for LaunchEdit {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut ProjectileLaunchEvent,
    ) -> BoxFuture<'a, ()> {
        if event.shooter_id == Some(self.player.get_entity().entity_id) {
            // A foreign thread detects an outer owner; same-thread try_lock is reentrant.
            let owner = self.player.inventory.slot_transaction(0);
            std::thread::scope(|scope| {
                scope
                    .spawn(move || {
                        assert!(
                            owner.try_lock().is_some(),
                            "launch callback retained inventory ownership"
                        );
                    })
                    .join()
                    .unwrap();
            });
            self.calls.fetch_add(1, SeqCst);
            let mut stack = self.player.inventory.get_stack(0);
            let mut data = NbtCompound::new();
            data.put_int("launch_edit", 7);
            stack.set_data_component(CustomDataImpl::new(data));
            self.player.inventory.set_stack(0, stack);
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inventory_transactions_crossbow_launch_callback_cannot_keep_ammunition() {
    run(|fixture| {
        let world = fixture.world();
        let player = TestPlayer::new(&world);
        let mut arrow = NbtCompound::new();
        ItemStack::new(1, &Item::ARROW).write_item_stack(&mut arrow);
        let mut weapon = ItemStack::new(1, &Item::CROSSBOW);
        weapon.set_data_component(ChargedProjectilesImpl {
            projectiles: vec![arrow],
        });
        let uid = weapon.uid;
        player.player.inventory.set_stack(0, weapon);
        let calls = Arc::new(AtomicUsize::new(0));
        fixture
            .server
            .plugin_manager
            .register::<ProjectileLaunchEvent, _>(
                Arc::new(LaunchEdit {
                    player: player.player.clone(),
                    calls: calls.clone(),
                }),
                EventPriority::Normal,
                true,
            );
        CrossbowItem.normal_use(&Item::CROSSBOW, &player.player);
        assert_eq!(calls.load(SeqCst), 1);
        assert_eq!(world.entities.load().len(), 1);
        let actual = player.player.inventory.get_stack(0);
        assert_eq!((actual.uid, actual.item_count), (uid, 1));
        assert_eq!(
            actual
                .get_data_component::<CustomDataImpl>()
                .unwrap()
                .data
                .get_int("launch_edit"),
            Some(7)
        );
        assert!(
            charged(&actual).is_empty(),
            "launch left charged ammunition on the source weapon"
        );
        CrossbowItem.normal_use(&Item::CROSSBOW, &player.player);
        assert_eq!(
            calls.load(SeqCst),
            1,
            "a second use fired the same charge again"
        );
        assert_eq!(world.entities.load().len(), 1);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inventory_transactions_crossbow_loading_rejects_weapon_moved_by_real_click() {
    run(|fixture| {
        let world = fixture.world();
        let player = TestPlayer::new(&world);
        player
            .player
            .inventory
            .set_stack(0, ItemStack::new(1, &Item::CROSSBOW));
        player
            .player
            .inventory
            .set_stack(1, ItemStack::new(4, &Item::ARROW));
        let observed = player.player.inventory.get_stack(0);
        CrossbowItem.normal_use(&Item::CROSSBOW, &player.player);
        let chest = Arc::new(ChestBlockEntity::new(
            pumpkin_util::math::position::BlockPos::new(8, 64, 8),
        ));
        open_chest(&player, chest.clone());
        let mut packet = click();
        packet.slot = 54;
        player.player.on_slot_click(packet, &fixture.server);
        // An admitted quick-move uses a new receipt UID; verify the real move and retain that identity.
        let moved = chest.get_stack(0);
        assert!(moved.are_equal(&observed));
        assert!(player.player.inventory.get_stack(0).is_empty());
        release(&player.player, &observed);
        assert_eq!(
            player.player.inventory.get_stack(1).item_count,
            4,
            "moved active weapon still paid for a rejected load"
        );
        let remaining = chest.get_stack(0);
        assert_eq!(remaining.uid, moved.uid);
        assert!(remaining.are_equal(&moved));
        assert!(charged(&remaining).is_empty());
        assert!(player.player.inventory.get_stack(0).is_empty());
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inventory_transactions_crossbow_loading_rejects_replaced_active_identity() {
    run(|fixture| {
        let player = TestPlayer::new(&fixture.world());
        player
            .player
            .inventory
            .set_stack(0, ItemStack::new(1, &Item::CROSSBOW));
        player
            .player
            .inventory
            .set_stack(1, ItemStack::new(4, &Item::ARROW));
        let observed = player.player.inventory.get_stack(0);
        CrossbowItem.normal_use(&Item::CROSSBOW, &player.player);
        let replacement = ItemStack::new(1, &Item::CROSSBOW);
        let uid = replacement.uid;
        assert_ne!(uid, observed.uid);
        assert!(replacement.are_equal(&observed));
        player.player.inventory.set_stack(0, replacement);
        release(&player.player, &observed);
        assert_eq!(
            player.player.inventory.get_stack(1).item_count,
            4,
            "replacement active weapon still paid for a rejected load"
        );
        let actual = player.player.inventory.get_stack(0);
        assert_eq!(actual.uid, uid);
        assert!(charged(&actual).is_empty());
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inventory_transactions_crossbow_infinite_ammunition_is_not_paid_twice() {
    run(|fixture| {
        let player = TestPlayer::new(&fixture.world());
        let mut weapon = ItemStack::new(1, &Item::CROSSBOW);
        weapon.add_enchantment(&Enchantment::INFINITY, 1);
        player.player.inventory.set_stack(0, weapon);
        player
            .player
            .inventory
            .set_stack(1, ItemStack::new(4, &Item::ARROW));
        let observed = player.player.inventory.get_stack(0);
        CrossbowItem.normal_use(&Item::CROSSBOW, &player.player);
        release(&player.player, &observed);
        assert_eq!(
            player.player.inventory.get_stack(1).item_count,
            4,
            "infinite crossbow ammunition was paid again"
        );
        let projectiles = charged(&player.player.inventory.get_stack(0));
        assert_eq!(projectiles.len(), 1);
        assert!(
            projectiles[0]
                .get_data_component::<IntangibleProjectileImpl>()
                .is_some()
        );
        CrossbowItem.normal_use(&Item::CROSSBOW, &player.player);
        let world = player.player.world();
        let entities = world.entities.load();
        assert_eq!(entities.len(), 1);
        let arrow = entities[0]
            .cast_any()
            .downcast_ref::<crate::entity::projectile::arrow::ArrowEntity>()
            .unwrap();
        assert!(
            matches!(
                arrow.pickup.load(),
                crate::entity::projectile::arrow::ArrowPickup::CreativeOnly
            ),
            "Infinity receipt fired a recoverable arrow"
        );
        assert_eq!(player.player.inventory.get_stack(1).item_count, 4);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inventory_transactions_crossbow_paid_and_creative_load_preserve_stack_components() {
    run(|fixture| {
        let player = TestPlayer::new(&fixture.world());
        for mode in [GameMode::Survival, GameMode::Creative] {
            player.player.gamemode.store(mode);
            player
                .player
                .inventory
                .set_stack(0, ItemStack::new(1, &Item::CROSSBOW));
            let mut ammo = ItemStack::new(4, &Item::SPECTRAL_ARROW);
            let mut data = NbtCompound::new();
            data.put_int("ammo_edit", 9);
            ammo.set_data_component(CustomDataImpl::new(data));
            let ammo_uid = ammo.uid;
            player.player.inventory.set_stack(1, ammo);
            let observed = player.player.inventory.get_stack(0);
            CrossbowItem.normal_use(&Item::CROSSBOW, &player.player);
            release(&player.player, &observed);
            let left = player.player.inventory.get_stack(1);
            assert_eq!(
                (left.uid, left.item_count),
                (ammo_uid, if mode == GameMode::Survival { 3 } else { 4 })
            );
            let loaded = player.player.inventory.get_stack(0);
            assert_eq!(loaded.uid, observed.uid);
            let projectiles = charged(&loaded);
            assert_eq!(projectiles.len(), 1);
            assert_eq!(projectiles[0].item, &Item::SPECTRAL_ARROW);
            assert_eq!(
                projectiles[0]
                    .get_data_component::<CustomDataImpl>()
                    .unwrap()
                    .data
                    .get_int("ammo_edit"),
                Some(9)
            );
            assert_eq!(
                projectiles[0]
                    .get_data_component::<IntangibleProjectileImpl>()
                    .is_some(),
                mode == GameMode::Creative
            );
        }
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inventory_transactions_crossbow_multishot_paid_center_only_is_survival_recoverable() {
    run(|fixture| {
        use crate::entity::projectile::arrow::{ArrowEntity, ArrowPickup};

        let world = fixture.world();
        let player = TestPlayer::new(&world);
        let mut weapon = ItemStack::new(1, &Item::CROSSBOW);
        weapon.add_enchantment(&Enchantment::MULTISHOT, 1);
        player.player.inventory.set_stack(0, weapon);
        player
            .player
            .inventory
            .set_stack(1, ItemStack::new(3, &Item::ARROW));
        let observed = player.player.inventory.get_stack(0);

        CrossbowItem.normal_use(&Item::CROSSBOW, &player.player);
        release(&player.player, &observed);
        assert_eq!(player.player.inventory.get_stack(1).item_count, 2);
        let projectiles = charged(&player.player.inventory.get_stack(0));
        assert_eq!(projectiles.len(), 3);
        assert_eq!(
            projectiles
                .iter()
                .map(|projectile| {
                    projectile
                        .get_data_component::<IntangibleProjectileImpl>()
                        .is_some()
                })
                .collect::<Vec<_>>(),
            [false, true, true]
        );

        CrossbowItem.normal_use(&Item::CROSSBOW, &player.player);
        let entities = world.entities.load();
        assert_eq!(entities.len(), 3);
        for (entity, expected) in entities.iter().zip([
            ArrowPickup::Allowed,
            ArrowPickup::CreativeOnly,
            ArrowPickup::CreativeOnly,
        ]) {
            let arrow = entity.cast_any().downcast_ref::<ArrowEntity>().unwrap();
            assert_eq!(arrow.pickup.load(), expected);
        }
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inventory_transactions_crossbow_intangible_fired_arrow_cannot_be_recovered() {
    run(|fixture| {
        use crate::entity::projectile::arrow::{ArrowEntity, ArrowPickup};
        let world = fixture.world();
        let player = TestPlayer::new(&world);
        player.player.gamemode.store(GameMode::Survival);
        let mut projectile = ItemStack::new(1, &Item::ARROW);
        projectile.set_data_component(IntangibleProjectileImpl);
        let mut nbt = NbtCompound::new();
        projectile.write_item_stack(&mut nbt);
        let mut weapon = ItemStack::new(1, &Item::CROSSBOW);
        weapon.set_data_component(ChargedProjectilesImpl {
            projectiles: vec![nbt],
        });
        player.player.inventory.set_stack(0, weapon);
        assert!(player.player.find_arrow().is_none());
        CrossbowItem.normal_use(&Item::CROSSBOW, &player.player);
        let entities = world.entities.load();
        assert_eq!(entities.len(), 1);
        let arrow = entities[0]
            .cast_any()
            .downcast_ref::<ArrowEntity>()
            .unwrap();
        assert!(
            matches!(arrow.pickup.load(), ArrowPickup::CreativeOnly),
            "Intangible fired arrow allowed survival pickup"
        );
        arrow.in_ground.store(true, SeqCst);
        arrow.shake_time.store(0, SeqCst);
        arrow.on_player_collision(&player.player);
        assert!(
            player.player.find_arrow().is_none(),
            "survival recovered free ammunition"
        );
        assert!(!arrow.get_entity().is_removed());
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inventory_transactions_crossbow_offhand_load_and_fire_keep_main_hand() {
    run(|fixture| {
        let world = fixture.world();
        let player = TestPlayer::new(&world);
        player.player.gamemode.store(GameMode::Survival);
        let main = ItemStack::new(1, &Item::IRON_SWORD);
        player.player.inventory.set_stack(0, main.clone());
        let offhand = pumpkin_inventory::player::player_inventory::PlayerInventory::OFF_HAND_SLOT;
        let weapon = ItemStack::new(1, &Item::CROSSBOW);
        let uid = weapon.uid;
        player.player.inventory.set_stack(offhand, weapon.clone());
        player
            .player
            .inventory
            .set_stack(1, ItemStack::new(3, &Item::ARROW));
        CrossbowItem.normal_use_with_hand(&Item::CROSSBOW, &player.player, 0.0, 0.0, Hand::Left);
        release(&player.player, &weapon);
        assert_eq!(
            charged(&player.player.inventory.get_stack(offhand)).len(),
            1,
            "offhand source was not charged"
        );
        assert_eq!(player.player.inventory.get_stack(1).item_count, 2);
        CrossbowItem.normal_use_with_hand(&Item::CROSSBOW, &player.player, 0.0, 0.0, Hand::Left);
        assert_eq!(world.entities.load().len(), 1);
        let actual = player.player.inventory.get_stack(offhand);
        assert_eq!(actual.uid, uid);
        assert!(charged(&actual).is_empty());
        let actual_main = player.player.inventory.get_stack(0);
        assert_eq!(actual_main.uid, main.uid);
        assert!(
            actual_main.are_equal(&main),
            "offhand loading or wear changed main hand"
        );
    })
    .await;
}
