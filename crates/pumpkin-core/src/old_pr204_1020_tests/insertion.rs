use super::support::*;
use crate::{entity::r#type::from_type, world::projectile_test_support};
use pumpkin_data::{Enchantment, entity::EntityType, item::Item};
use pumpkin_inventory::Inventory;
use pumpkin_util::{Hand, math::vector3::Vector3};

fn enable_packet_probes(p: &crate::net::java::combat_test_support::TestPlayer, hooks: &Hooks) {
    p.client().set_player(p.player.clone());
    p.client()
        .version
        .store(pumpkin_util::version::JavaMinecraftVersion::V_26_2);
    hooks.state.lock().unwrap().check_reservations = true;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_pr204_1020_late_insertion_rejection_preserves_payment() {
    run(|f| {
        for item in [
            &Item::SNOWBALL,
            &Item::TRIDENT,
            &Item::BOW,
            &Item::CROSSBOW,
            &Item::FIREWORK_ROCKET,
        ] {
            let p = player(f);
            let hooks = Hooks::install(&f.server, &p.player);
            p.player.inventory().set_stack(0, stack(1, item));
            if item == &Item::BOW || item == &Item::CROSSBOW {
                p.player.inventory().set_stack(9, stack(2, &Item::ARROW));
            }
            if item == &Item::CROSSBOW {
                shoot(&p, f, Hand::Right);
            }
            let source = p.player.inventory().get_stack(0);
            let ammo = p.player.inventory().get_stack(9);
            hooks.configure(Veto::None, false, None);
            enable_packet_probes(&p, &hooks);
            let (_registration, _) = projectile_test_support::install(&f.world(), usize::MAX, None);
            let before = f.world().entities.load().len();
            if item == &Item::TRIDENT || item == &Item::BOW {
                shoot(&p, f, Hand::Right);
            } else if item == &Item::FIREWORK_ROCKET {
                block_rocket(&p, f, Hand::Right);
            } else {
                use_item(&p, f, Hand::Right);
            }
            let actual = p.player.inventory().get_stack(0);
            assert_eq!(actual.uid, source.uid);
            assert!(
                actual.are_equal(&source),
                "late append failure changed the source"
            );
            assert!(p.player.inventory().get_stack(9).are_equal(&ammo));
            assert_eq!(f.world().entities.load().len(), before);
            assert_eq!(hooks.counts().2, 0);
            assert_eq!(used(&p.player, item), 0);
            assert_reservations_released(f.world());
            assert_owner_released(p.player.inventory().slot_transaction(0));
        }
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_pr204_1020_multishot_first_append_failure_uses_next_prepared_round() {
    run(|f| {
        let p = player(f);
        let hooks = Hooks::install(&f.server, &p.player);
        let mut bow = stack(1, &Item::BOW);
        bow.add_enchantment(&Enchantment::MULTISHOT, 1);
        p.player.inventory().set_stack(0, bow);
        p.player.inventory().set_stack(9, stack(2, &Item::ARROW));
        hooks.configure(Veto::None, false, None);
        enable_packet_probes(&p, &hooks);
        let (_registration, probe) = projectile_test_support::install(&f.world(), 1, None);
        let before = f.world().entities.load().len();
        shoot(&p, f, Hand::Right);
        assert_eq!(hooks.counts(), (3, 3, 2));
        assert_eq!(f.world().entities.load().len() - before, 2);
        assert_eq!(loose(&p.player, &Item::ARROW), 1);
        assert_eq!(p.player.inventory().get_stack(0).get_damage(), 2);
        assert_eq!(probe.attempts.load(SeqCst), 2);
        assert!(hooks.state.lock().unwrap().packet_probes > 0);
        assert_reservations_released(f.world());
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_pr204_1020_append_retry_debits_only_once() {
    run(|f| {
        let p = player(f);
        let hooks = Hooks::install(&f.server, &p.player);
        p.player.inventory().set_stack(0, stack(2, &Item::SNOWBALL));
        hooks.configure(Veto::None, false, None);
        enable_packet_probes(&p, &hooks);
        let unrelated = from_type(
            &EntityType::COW,
            Vector3::new(12.0, 64.0, 12.0),
            &f.world(),
            uuid::Uuid::new_v4(),
        );
        let (_registration, probe) =
            projectile_test_support::install(&f.world(), 0, Some(unrelated));
        let before = f.world().entities.load().len();
        use_item(&p, f, Hand::Right);
        assert!(
            probe.attempts.load(SeqCst) >= 2,
            "test did not force a list append retry"
        );
        assert_eq!(loose(&p.player, &Item::SNOWBALL), 1);
        assert_eq!(f.world().entities.load().len() - before, 2);
        assert_eq!(used(&p.player, &Item::SNOWBALL), 1);
        assert_reservations_released(f.world());
        assert_owner_released(p.player.inventory().slot_transaction(0));
    })
    .await;
}
