use super::{items::ground, support::*};
use crate::{entity::death_test_world::DeathTestWorld, net::java::combat_test_support::TestPlayer};
use pumpkin_data::{data_component_impl::CustomDataImpl, item::Item, item_stack::ItemStack};
use pumpkin_inventory::Inventory;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::Hand;
use std::sync::{Arc, atomic::AtomicUsize};

#[derive(Clone, Copy, Debug)]
enum Route {
    Bow,
    Trident,
    Snowball,
    Rocket,
}

impl Route {
    fn item(self) -> &'static Item {
        match self {
            Self::Bow => &Item::BOW,
            Self::Trident => &Item::TRIDENT,
            Self::Snowball => &Item::SNOWBALL,
            Self::Rocket => &Item::FIREWORK_ROCKET,
        }
    }

    fn payment(self) -> &'static Item {
        if matches!(self, Self::Bow) {
            &Item::ARROW
        } else {
            self.item()
        }
    }

    fn prepare(self, p: &TestPlayer, hand: Hand) -> (usize, ItemStack) {
        let count = if matches!(self, Self::Bow | Self::Trident) {
            1
        } else {
            2
        };
        p.player
            .inventory()
            .set_stack(slot(hand), stack(count, self.item()));
        let source = if matches!(self, Self::Bow) {
            9
        } else {
            slot(hand)
        };
        if matches!(self, Self::Bow) {
            p.player
                .inventory()
                .set_stack(source, stack(2, &Item::ARROW));
        }
        (source, p.player.inventory().get_stack(source))
    }

    fn dispatch(self, p: &TestPlayer, f: &DeathTestWorld, hand: Hand) {
        match self {
            Self::Bow | Self::Trident => shoot(p, f, hand),
            Self::Snowball => use_item(p, f, hand),
            Self::Rocket => block_rocket(p, f, hand),
        }
    }
}

macro_rules! restore_test {
    ($name:ident, $route:ident) => {
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn $name() {
            run(|f| restored_source_cases(f, Route::$route)).await;
        }
    };
}

restore_test!(
    old_pr204_1020_same_uid_bow_snapshot_never_mints_payment,
    Bow
);
restore_test!(
    old_pr204_1020_same_uid_trident_snapshot_never_mints_payment,
    Trident
);
restore_test!(
    old_pr204_1020_same_uid_snowball_snapshot_never_mints_payment,
    Snowball
);
restore_test!(
    old_pr204_1020_same_uid_rocket_snapshot_never_mints_payment,
    Rocket
);

fn restored_source_cases(f: &DeathTestWorld, route: Route) {
    for hand in [Hand::Right, Hand::Left] {
        for veto in [Veto::Launch, Veto::Spawn] {
            let p = player(f);
            let hooks = Hooks::install(&f.server, &p.player);
            let (source, original) = route.prepare(&p, hand);
            let before = f.world().entities.load().len();
            for attempt in 0..3 {
                let snapshot = p.player.inventory().get_stack(source);
                hooks.configure(
                    veto,
                    false,
                    Some(Arc::new(move |p| {
                        p.inventory().set_stack(source, snapshot.clone());
                    })),
                );
                route.dispatch(&p, f, hand);
                assert_eq!(
                    loose(&p.player, route.payment()) + ground(&p.player, route.payment()),
                    usize::from(original.item_count),
                    "restored snapshot minted stock: {route:?}, {hand:?}, {veto:?}, {attempt}"
                );
                let actual = p.player.inventory().get_stack(source);
                assert_eq!(actual.uid, original.uid);
                assert!(actual.are_equal(&original));
                assert_eq!(f.world().entities.load().len(), before);
                assert_eq!(hooks.counts().2, 0);
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_pr204_1020_callback_stock_changes_abort_without_compensation() {
    run(|f| {
        for route in [Route::Bow, Route::Trident, Route::Snowball, Route::Rocket] {
            for veto in [Veto::Launch, Veto::Spawn, Veto::None] {
                for edit in 0..6 {
                    changed_source_case(f, route, veto, edit);
                }
            }
        }
    })
    .await;
}

fn changed_source_case(f: &DeathTestWorld, route: Route, veto: Veto, edit: usize) {
    let p = player(f);
    let hooks = Hooks::install(&f.server, &p.player);
    let (source, original) = route.prepare(&p, Hand::Right);
    let callback_net = Arc::new(AtomicUsize::new(usize::MAX));
    let observed = callback_net.clone();
    hooks.configure(
        veto,
        false,
        Some(Arc::new(move |p| {
            let live = p.inventory().get_stack(source);
            assert_eq!(live.uid, original.uid);
            assert!(
                live.are_equal(&original),
                "callback saw a server withdrawal"
            );
            let mut changed = live.clone();
            match edit {
                0 => changed.increment(1),
                1 => changed = live.copy_with_count(live.item_count),
                2 => changed = stack(1, &Item::DIAMOND),
                3 => {
                    p.inventory().set_stack(12, live);
                    changed = ItemStack::EMPTY.clone();
                }
                4 => {
                    let mut data = NbtCompound::new();
                    data.put_int("authoritative_callback", 394);
                    changed.set_data_component(CustomDataImpl::new(data));
                }
                _ => p.inventory().set_selected_slot(4),
            }
            p.inventory().set_stack(source, changed);
            observed.store(loose(p, route.payment()), SeqCst);
        })),
    );
    let before = f.world().entities.load().len();
    route.dispatch(&p, f, Hand::Right);
    let admitted = veto == Veto::None && edit == 5;
    assert_eq!(
        f.world().entities.load().len() - before,
        usize::from(admitted),
        "stale source published a projectile: {route:?}, {veto:?}, edit={edit}"
    );
    assert_eq!(
        loose(&p.player, route.payment()) + ground(&p.player, route.payment()),
        callback_net.load(SeqCst) - usize::from(admitted),
        "server changed callback stock: {route:?}, {veto:?}, edit={edit}"
    );
    assert_eq!(used(&p.player, route.item()), i32::from(admitted));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_pr204_1020_bow_retained_weapon_move_pays_pinned_ammunition() {
    run(|f| {
        let p = player(f);
        let hooks = Hooks::install(&f.server, &p.player);
        Route::Bow.prepare(&p, Hand::Right);
        let weapon = p.player.inventory().get_stack(0);
        hooks.configure(
            Veto::None,
            false,
            Some(Arc::new(|p| {
                p.inventory().set_stack(4, p.inventory().get_stack(0));
                p.inventory().set_stack(0, ItemStack::EMPTY.clone());
                p.inventory().set_selected_slot(4);
            })),
        );
        let before = f.world().entities.load().len();
        shoot(&p, f, Hand::Right);
        assert_eq!(f.world().entities.load().len() - before, 1);
        assert_eq!(loose(&p.player, &Item::ARROW), 1);
        let current = p.player.inventory().get_stack(4);
        assert_eq!(current.uid, weapon.uid);
        assert_eq!(current.get_damage(), 1);
        assert!(p.player.inventory().get_stack(0).is_empty());
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_pr204_1020_changed_payment_mode_aborts_unchanged_source() {
    run(|f| {
        for route in [Route::Bow, Route::Trident, Route::Snowball, Route::Rocket] {
            for mode in [
                pumpkin_util::GameMode::Creative,
                pumpkin_util::GameMode::Spectator,
            ] {
                let p = player(f);
                let (source, original) = route.prepare(&p, Hand::Right);
                let hooks = Hooks::install(&f.server, &p.player);
                hooks.configure(
                    Veto::None,
                    false,
                    Some(Arc::new(move |p| {
                        p.gamemode.store(mode);
                    })),
                );
                let before = f.world().entities.load().len();
                route.dispatch(&p, f, Hand::Right);
                assert_eq!(f.world().entities.load().len(), before);
                let actual = p.player.inventory().get_stack(source);
                assert_eq!(actual.uid, original.uid);
                assert!(actual.are_equal(&original));
                assert_eq!(used(&p.player, route.item()), 0);
            }
        }
    })
    .await;
}
