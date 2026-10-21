use super::{crossbow_rollback::stock, support::*};
use crate::{entity::death_test_world::DeathTestWorld, net::java::combat_test_support::TestPlayer};
use pumpkin_data::{Enchantment, data_component_impl::ChargedProjectilesImpl, item::Item};
use pumpkin_inventory::Inventory;
use pumpkin_util::Hand;
use std::sync::Arc;

fn loaded(f: &DeathTestWorld, item: &'static Item) -> TestPlayer {
    let p = player(f);
    let mut weapon = stack(1, &Item::CROSSBOW);
    weapon.add_enchantment(&Enchantment::MULTISHOT, 1);
    p.player.inventory().set_stack(0, weapon);
    p.player.inventory().set_stack(
        if item == &Item::FIREWORK_ROCKET {
            40
        } else {
            9
        },
        stack(2, item),
    );
    shoot(&p, f, Hand::Right);
    assert_eq!(charge(&p.player.inventory().get_stack(0)), 3);
    assert_eq!(loose(&p.player, item), 1);
    p
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_pr204_1020_partial_multishot_restored_snapshot_spends_charge_once() {
    run(|f| {
        for item in [&Item::ARROW, &Item::FIREWORK_ROCKET] {
            for veto in [Veto::Launch, Veto::Spawn] {
                for denied in [0, 1] {
                    partial_restore_case(f, item, veto, denied);
                }
            }
        }
    })
    .await;
}

fn partial_restore_case(f: &DeathTestWorld, item: &'static Item, veto: Veto, denied: usize) {
    let p = loaded(f, item);
    let hooks = Hooks::install(&f.server, &p.player);
    let snapshot = p.player.inventory().get_stack(0);
    let before = f.world().entities.load().len();
    hooks.configure(veto, true, None);
    let mut state = hooks.state.lock().unwrap();
    state.deny_index = Some(denied);
    state.admission_observer = Some(Arc::new(move |p, stage, index| {
        assert_eq!(
            p.world().entities.load().len(),
            before,
            "preflight published an earlier round"
        );
        let current = p.inventory().get_stack(0);
        assert_eq!(current.uid, snapshot.uid);
        assert!(
            current.are_equal(&snapshot),
            "preflight changed the loaded weapon"
        );
        assert_eq!(current.get_damage(), 0);
        if stage == if veto == Veto::Launch { 'L' } else { 'S' } && index == denied {
            p.inventory().set_stack(0, snapshot.clone());
        }
    }));
    drop(state);
    use_item(&p, f, Hand::Right);
    assert_eq!(f.world().entities.load().len() - before, 2);
    for entity in f.world().entities.load().iter().skip(before) {
        if let Some(arrow) = entity
            .cast_any()
            .downcast_ref::<crate::entity::projectile::arrow::ArrowEntity>()
        {
            assert_eq!(
                charge(&arrow.get_weapon_item().unwrap()),
                0,
                "projectile retained its weapon's unretired payment snapshot"
            );
        }
    }
    assert_eq!(charge(&p.player.inventory().get_stack(0)), 0);
    assert_eq!(
        stock(&p.player, item),
        1,
        "partial success retained the spent paid round"
    );
    assert_eq!(
        p.player.inventory().get_stack(0).get_damage(),
        if item == &Item::ARROW { 2 } else { 6 }
    );
    assert_eq!(
        hooks.counts(),
        (3, if veto == Veto::Launch { 2 } else { 3 }, 2)
    );
    let calls = hooks.counts();
    use_item(&p, f, Hand::Right);
    assert_eq!(
        hooks.counts(),
        calls,
        "the cleared charge replayed without reloading"
    );
    assert_eq!(f.world().entities.load().len() - before, 2);
    assert_eq!(stock(&p.player, item), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_pr204_1020_partial_charge_callback_aborts_all_prepared_rounds() {
    run(|f| {
        for item in [&Item::ARROW, &Item::FIREWORK_ROCKET] {
            for veto in [Veto::Launch, Veto::Spawn, Veto::None] {
                for retained in 0..4 {
                    partial_charge_case(f, item, veto, retained);
                }
            }
        }
    })
    .await;
}

fn partial_charge_case(f: &DeathTestWorld, item: &'static Item, veto: Veto, retained: usize) {
    let p = loaded(f, item);
    let hooks = Hooks::install(&f.server, &p.player);
    let original = p.player.inventory().get_stack(0);
    let original_charge = original
        .get_data_component::<ChargedProjectilesImpl>()
        .unwrap();
    let mut replacement = original.clone();
    let indices: &[usize] = match retained {
        0 => &[0],
        1 => &[1],
        2 => &[0, 1],
        _ => &[],
    };
    replacement.set_data_component(ChargedProjectilesImpl {
        projectiles: indices
            .iter()
            .map(|index| original_charge.projectiles[*index].clone())
            .collect(),
    });
    let expected = replacement.clone();
    hooks.configure(veto, true, None);
    let mut state = hooks.state.lock().unwrap();
    state.deny_index = Some(1);
    state.admission_observer = Some(Arc::new(move |p, stage, index| {
        let selected = if veto == Veto::Spawn { 'S' } else { 'L' };
        if stage == selected && index == 1 {
            p.inventory().set_stack(0, replacement.clone());
        }
    }));
    drop(state);
    let before = f.world().entities.load().len();
    use_item(&p, f, Hand::Right);
    assert_eq!(
        f.world().entities.load().len(),
        before,
        "a changed loaded source paid a prepared round"
    );
    let current = p.player.inventory().get_stack(0);
    assert_eq!(current.uid, expected.uid);
    assert!(current.are_equal(&expected));
    assert_eq!(
        loose(&p.player, item),
        1,
        "callback charge edits received compensation"
    );
    assert_eq!(current.get_damage(), 0);
    assert_eq!(used(&p.player, &Item::CROSSBOW), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_pr204_1020_duplicate_retained_weapon_uid_is_not_retargeted() {
    run(|f| {
        let p = loaded(f, &Item::ARROW);
        let hooks = Hooks::install(&f.server, &p.player);
        hooks.configure(
            Veto::None,
            false,
            Some(Arc::new(|p| {
                let retained = p.inventory().get_stack(0);
                p.inventory().set_stack(4, retained);
                p.inventory().set_selected_slot(4);
            })),
        );
        let before = f.world().entities.load().len();
        use_item(&p, f, Hand::Right);
        assert_eq!(f.world().entities.load().len(), before);
        assert_eq!(charge(&p.player.inventory().get_stack(0)), 3);
        assert_eq!(charge(&p.player.inventory().get_stack(4)), 3);
        assert_eq!(loose(&p.player, &Item::ARROW), 1);
    })
    .await;
}
