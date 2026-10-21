use super::support::*;
use crate::entity::{
    EntityBase,
    projectile::arrow::{ArrowEntity, ArrowPickup},
};
use pumpkin_data::{
    Enchantment, data_component_impl::UnbreakableImpl, item::Item, item_stack::ItemStack,
};
use pumpkin_inventory::Inventory;
use pumpkin_util::{GameMode, Hand};
use std::sync::Arc;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_pr204_1020_bow_both_vetoes_keep_payment_and_wear() {
    run(|f| {
        for hand in [Hand::Right, Hand::Left] {
            for veto in [Veto::Launch, Veto::Spawn, Veto::None] {
                for item in [&Item::ARROW, &Item::TIPPED_ARROW, &Item::SPECTRAL_ARROW] {
                    let mut p = player(f);
                    let hooks = Hooks::install(&f.server, &p.player);
                    hooks.configure(veto, false, None);
                    let mut bow = stack(1, &Item::BOW);
                    bow.set_data_component(
                        pumpkin_data::data_component_impl::UseCooldownImpl::new(
                            1.0,
                            Some("old_pr204_1020_bow".into()),
                        ),
                    );
                    let ammo = stack(1, item);
                    p.player.inventory().set_stack(slot(hand), bow.clone());
                    p.player.inventory().set_stack(9, ammo.clone());
                    p.take_packets();
                    let before = f.world().entities.load().len();
                    shoot(&p, f, hand);
                    let admitted = veto == Veto::None;
                    assert_eq!(
                        f.world().entities.load().len() - before,
                        usize::from(admitted)
                    );
                    assert_eq!(
                        hooks.counts(),
                        (1, usize::from(veto != Veto::Launch), usize::from(admitted))
                    );
                    let actual = p.player.inventory().get_stack(9);
                    if admitted {
                        assert!(actual.is_empty());
                    } else {
                        assert_eq!(actual.uid, ammo.uid);
                        assert!(actual.are_equal(&ammo));
                    }
                    assert_eq!(
                        p.player.inventory().get_stack(slot(hand)).get_damage(),
                        i32::from(admitted)
                    );
                    let packets = p.take_packets();
                    assert_slot(&packets, 9, &actual);
                    assert_slot(
                        &packets,
                        slot(hand),
                        &p.player.inventory().get_stack(slot(hand)),
                    );
                    assert_launch_sound(&packets, admitted);
                    if !admitted {
                        assert_quiet(&packets);
                    }
                    assert_eq!(used(&p.player, &Item::BOW), i32::from(admitted));
                    assert_eq!(p.player.is_on_cooldown("old_pr204_1020_bow"), admitted);
                    let order = &hooks.state.lock().unwrap().order;
                    assert_eq!(order[0].0, 'L');
                    if veto != Veto::Launch {
                        assert_eq!(order[1], ('S', order[0].1));
                    }
                }
            }
        }
        for creative in [false, true] {
            let p = player(f);
            let hooks = Hooks::install(&f.server, &p.player);
            hooks.configure(Veto::None, false, None);
            let mut bow = stack(1, &Item::BOW);
            if creative {
                p.player.gamemode.store(GameMode::Creative);
            } else {
                bow.add_enchantment(&Enchantment::INFINITY, 1);
            }
            p.player.inventory().set_stack(0, bow);
            p.player.inventory().set_stack(9, stack(2, &Item::ARROW));
            shoot(&p, f, Hand::Right);
            assert_eq!(loose(&p.player, &Item::ARROW), 2);
            let world = f.world();
            let entities = world.entities.load();
            let arrow = entities
                .last()
                .unwrap()
                .cast_any()
                .downcast_ref::<ArrowEntity>()
                .unwrap();
            assert_eq!(arrow.pickup.load(), ArrowPickup::CreativeOnly);
        }
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_pr204_1020_bow_later_multishot_admission_keeps_payment() {
    run(|f| {
        for veto in [Veto::Launch, Veto::Spawn] {
            let mut p = player(f);
            let hooks = Hooks::install(&f.server, &p.player);
            hooks.configure(veto, true, None);
            let mut bow = stack(1, &Item::BOW);
            bow.add_enchantment(&Enchantment::MULTISHOT, 1);
            p.player.inventory().set_stack(0, bow);
            p.player.inventory().set_stack(9, stack(2, &Item::ARROW));
            p.take_packets();
            let before = f.world().entities.load().len();
            shoot(&p, f, Hand::Right);
            assert_eq!(
                loose(&p.player, &Item::ARROW),
                1,
                "a later admitted Multishot round must spend the draw payment"
            );
            assert_eq!(f.world().entities.load().len() - before, 2);
            assert_eq!(p.player.inventory().get_stack(0).get_damage(), 2);
            let world = f.world();
            let entities = world.entities.load();
            for entity in &entities[before..] {
                let arrow = entity.cast_any().downcast_ref::<ArrowEntity>().unwrap();
                assert_eq!(arrow.pickup.load(), ArrowPickup::CreativeOnly);
                arrow.in_ground.store(true, SeqCst);
                arrow.shake_time.store(0, SeqCst);
                arrow.on_player_collision(&p.player);
            }
            assert_eq!(
                loose(&p.player, &Item::ARROW),
                1,
                "Survival recovered an intangible copy"
            );
            assert_slot(&p.take_packets(), 9, &p.player.inventory().get_stack(9));
        }
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_pr204_1020_crossbow_moved_uid_shared_payment() {
    run(|f| {
        for veto in [Veto::Launch, Veto::Spawn] {
            for partial in [false, true] {
                let mut p = player(f);
                let hooks = Hooks::install(&f.server, &p.player);
                let mut weapon = stack(1, &Item::CROSSBOW);
                weapon.add_enchantment(&Enchantment::MULTISHOT, 1);
                let uid = weapon.uid;
                p.player.inventory().set_stack(0, weapon);
                p.player.inventory().set_stack(9, stack(3, &Item::ARROW));
                shoot(&p, f, Hand::Right);
                assert_eq!(charge(&p.player.inventory().get_stack(0)), 3);
                assert_eq!(loose(&p.player, &Item::ARROW), 2);
                hooks.configure(
                    veto,
                    partial,
                    Some(Arc::new(|p| {
                        let firing = p.inventory().get_stack(0);
                        assert_eq!(charge(&firing), 3);
                        p.inventory().set_stack(4, firing);
                        p.inventory().set_stack(0, ItemStack::EMPTY.clone());
                        p.inventory().set_selected_slot(4);
                    })),
                );
                p.take_packets();
                let before = f.world().entities.load().len();
                use_item(&p, f, Hand::Right);
                let current = p.player.inventory().get_stack(4);
                assert_eq!(current.uid, uid);
                assert_eq!(
                    loose(&p.player, &Item::ARROW),
                    2,
                    "denial must retain the loaded payment without a loose refund"
                );
                assert_eq!(charge(&current), if partial { 0 } else { 3 });
                assert_eq!(current.get_damage(), if partial { 2 } else { 0 });
                let packets = p.take_packets();
                assert_slot(&packets, 0, &p.player.inventory().get_stack(0));
                assert_slot(&packets, 4, &current);
                if !partial {
                    assert_quiet(&packets);
                }
                // A denied charge stays loaded; successful volleys require a fresh reload.
                hooks.configure(veto, partial, None);
                let mut spent = usize::from(partial);
                for _ in 0..2 {
                    shoot(&p, f, Hand::Right);
                    spent += usize::from(partial);
                    hooks.configure(veto, partial, None);
                    use_item(&p, f, Hand::Right);
                    assert_eq!(
                        3,
                        loose(&p.player, &Item::ARROW)
                            + usize::from(charge(&p.player.inventory().get_stack(4)) > 0)
                            + spent,
                        "repeated attempts created a fresh charge entitlement"
                    );
                }
                assert_eq!(
                    f.world().entities.load().len() - before,
                    if partial { 6 } else { 0 }
                );
                verify_draw_pickup_outcomes(&p, before, spent);
            }
        }
        // Replacing the firing slot and changing selection cannot overwrite the callback's new charge.
        let p = player(f);
        let hooks = Hooks::install(&f.server, &p.player);
        p.player.inventory().set_stack(0, stack(1, &Item::CROSSBOW));
        p.player.inventory().set_stack(9, stack(2, &Item::ARROW));
        shoot(&p, f, Hand::Right);
        let replacement = p.player.inventory().get_stack(0).copy_with_count(1);
        let uid = replacement.uid;
        hooks.configure(
            Veto::Launch,
            false,
            Some(Arc::new(move |p| {
                p.inventory().set_stack(0, replacement.clone());
                p.inventory().set_selected_slot(4);
            })),
        );
        use_item(&p, f, Hand::Right);
        assert_eq!(p.player.inventory().get_stack(0).uid, uid);
        assert_eq!(charge(&p.player.inventory().get_stack(0)), 1);
        crossbow_arrow_variants(f);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_pr204_1020_crossbow_natural_break_never_refunds_spent_payment() {
    run(|f| {
        for item in [&Item::ARROW, &Item::FIREWORK_ROCKET] {
            for control in 0..6 {
                let mut p = player(f);
                let hooks = Hooks::install(&f.server, &p.player);
                let mut weapon = stack(1, &Item::CROSSBOW);
                weapon.add_enchantment(&Enchantment::MULTISHOT, 1);
                let wear = if item == &Item::FIREWORK_ROCKET { 3 } else { 1 };
                weapon.set_damage(weapon.get_max_damage().unwrap() - wear);
                if control == 2 {
                    weapon.set_data_component(UnbreakableImpl);
                }
                p.player.inventory().set_stack(0, weapon.clone());
                let ammo_slot = if item == &Item::FIREWORK_ROCKET {
                    40
                } else {
                    9
                };
                p.player.inventory().set_stack(ammo_slot, stack(2, item));
                shoot(&p, f, Hand::Right);
                assert_eq!(loose(&p.player, item), 1);
                hooks.configure(Veto::Launch, control != 3, None);
                if control == 1 {
                    hooks.state.lock().unwrap().cancel_damage = true;
                }
                if control == 4 {
                    hooks.state.lock().unwrap().damage_edit = Some(Arc::new(|p| {
                        let weapon = p.inventory().get_stack(0);
                        p.inventory().set_stack(4, weapon);
                        p.inventory().set_stack(0, ItemStack::EMPTY.clone());
                    }));
                }
                if control == 5 {
                    let restored = weapon.clone();
                    hooks.state.lock().unwrap().break_edit = Some(Arc::new(move |p| {
                        p.inventory().set_stack(0, restored.clone());
                    }));
                }
                p.take_packets();
                let before = f.world().entities.load().len();
                use_item(&p, f, Hand::Right);
                let fired = if control == 0 || control >= 4 {
                    1
                } else if control == 3 {
                    0
                } else {
                    2
                };
                assert_eq!(f.world().entities.load().len() - before, fired);
                let actual = p.player.inventory().get_stack(0);
                assert_damage_amounts(&hooks, wear);
                if control == 5 {
                    assert_eq!(actual.uid, weapon.uid);
                    assert_eq!(
                        hooks.counts(),
                        (3, 2, 1),
                        "break callback published a remaining prepared round"
                    );
                    assert_eq!(
                        p.player.stats.lock().unwrap().get(
                            pumpkin_data::statistic::StatisticCategory::Broken,
                            i32::from(Item::CROSSBOW.id)
                        ),
                        1
                    );
                } else if control == 0 || control == 4 {
                    assert!(
                        actual.is_empty(),
                        "admitted wear did not break the firing weapon"
                    );
                    assert_eq!(hooks.counts(), (3, 2, 1));
                } else if control == 3 {
                    assert_eq!(actual.uid, weapon.uid);
                    assert_eq!(actual.get_damage(), weapon.get_damage());
                    assert_eq!(charge(&actual), 3);
                    assert_eq!(hooks.counts(), (3, 0, 0));
                } else {
                    assert_eq!(actual.get_damage(), weapon.get_damage());
                    assert_eq!(charge(&actual), 0);
                }
                assert_eq!(
                    loose(&p.player, item),
                    1,
                    "natural break refunded a payment already spent by an intangible round"
                );
                let packets = p.take_packets();
                assert_slot(&packets, 0, &actual);
                if control == 3 {
                    assert_quiet(&packets);
                }
            }
        }
    })
    .await;
}

fn crossbow_arrow_variants(f: &crate::entity::death_test_world::DeathTestWorld) {
    for item in [&Item::TIPPED_ARROW, &Item::SPECTRAL_ARROW] {
        for veto in [Veto::Launch, Veto::Spawn, Veto::None] {
            let mut p = player(f);
            let hooks = Hooks::install(&f.server, &p.player);
            p.player.inventory().set_stack(0, stack(1, &Item::CROSSBOW));
            p.player.inventory().set_stack(9, stack(2, item));
            shoot(&p, f, Hand::Right);
            hooks.configure(veto, false, None);
            p.take_packets();
            let before = f.world().entities.load().len();
            use_item(&p, f, Hand::Right);
            let admitted = veto == Veto::None;
            let actual = p.player.inventory().get_stack(0);
            assert_eq!(
                hooks.counts(),
                (1, usize::from(veto != Veto::Launch), usize::from(admitted))
            );
            assert_eq!(
                f.world().entities.load().len() - before,
                usize::from(admitted)
            );
            assert_eq!(loose(&p.player, item), 1);
            assert_eq!(charge(&actual), usize::from(!admitted));
            if admitted {
                assert_eq!(actual.get_damage(), 1);
            } else {
                assert_eq!(actual.get_damage(), 0);
            }
            let packets = p.take_packets();
            assert_slot(&packets, 0, &actual);
            assert_launch_sound(&packets, admitted);
        }
    }
}

fn assert_damage_amounts(hooks: &Hooks, expected: i32) {
    assert!(
        hooks
            .state
            .lock()
            .unwrap()
            .damage_amounts
            .iter()
            .all(|amount| *amount == expected)
    );
}

fn verify_draw_pickup_outcomes(
    p: &crate::net::java::combat_test_support::TestPlayer,
    before: usize,
    spent: usize,
) {
    let world = p.player.world();
    let entities = world.entities.load()[before..].to_vec();
    for entity in entities {
        let arrow = entity.cast_any().downcast_ref::<ArrowEntity>().unwrap();
        assert_eq!(arrow.pickup.load(), ArrowPickup::CreativeOnly);
        arrow.in_ground.store(true, SeqCst);
        arrow.shake_time.store(0, SeqCst);
        arrow.on_player_collision(&p.player);
    }
    assert_eq!(super::items::ground(&p.player, &Item::ARROW), 0);
    assert_eq!(
        3,
        loose(&p.player, &Item::ARROW)
            + usize::from(charge(&p.player.inventory().get_stack(4)) > 0)
            + spent,
        "pickup/drop outcomes changed shared payment conservation"
    );
}
