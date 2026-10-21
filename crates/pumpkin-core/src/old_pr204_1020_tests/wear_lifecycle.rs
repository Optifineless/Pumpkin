use super::support::*;
use crate::entity::projectile::trident::TridentEntity;
use pumpkin_data::{Enchantment, item::Item};
use pumpkin_inventory::Inventory;
use pumpkin_util::Hand;
use std::sync::Arc;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_pr204_1020_admitted_wear_life_replacement_stops_remaining_rounds() {
    run(|f| {
        for item in [&Item::BOW, &Item::CROSSBOW] {
            let p = player(f);
            let hooks = Hooks::install(&f.server, &p.player);
            let mut weapon = stack(1, item);
            weapon.add_enchantment(&Enchantment::MULTISHOT, 1);
            let uid = weapon.uid;
            p.player.inventory().set_stack(0, weapon);
            p.player.inventory().set_stack(9, stack(2, &Item::ARROW));
            if item == &Item::CROSSBOW {
                shoot(&p, f, Hand::Right);
            }
            hooks.configure(Veto::None, false, None);
            hooks.state.lock().unwrap().damage_edit = Some(Arc::new(|p| {
                p.living_entity.reset_state();
            }));
            let life = p.player.living_entity.damage_lifecycle();
            let before = f.world().entities.load().len();
            if item == &Item::BOW {
                shoot(&p, f, Hand::Right);
            } else {
                use_item(&p, f, Hand::Right);
            }
            assert_ne!(p.player.living_entity.damage_lifecycle(), life);
            assert_eq!(
                f.world().entities.load().len() - before,
                1,
                "replacement life published an old action's remaining rounds"
            );
            assert_eq!(
                loose(&p.player, &Item::ARROW),
                1,
                "partial success returned or repeated the shared payment"
            );
            let current = p.player.inventory().get_stack(0);
            assert_eq!(current.uid, uid);
            assert_eq!(
                current.get_damage(),
                0,
                "old-shot wear reached a weapon in the replacement life"
            );
            assert_eq!(charge(&current), 0);
            assert_eq!(hooks.counts(), (3, 3, 1));
        }
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_pr204_1020_trident_damage_callback_replaced_life_preserves_transfer() {
    run(|f| {
        let p = player(f);
        let hooks = Hooks::install(&f.server, &p.player);
        let mut weapon = stack(1, &Item::TRIDENT);
        weapon.set_damage(7);
        p.player.inventory().set_stack(0, weapon.clone());
        hooks.configure(Veto::None, false, None);
        hooks.state.lock().unwrap().damage_edit = Some(Arc::new(|p| {
            p.living_entity.reset_state();
        }));
        let life = p.player.living_entity.damage_lifecycle();
        let before = f.world().entities.load().len();
        shoot(&p, f, Hand::Right);
        assert_ne!(p.player.living_entity.damage_lifecycle(), life);
        assert_eq!(f.world().entities.load().len() - before, 1);
        let entities = f.world().entities.load_full();
        let projectile = entities
            .last()
            .unwrap()
            .cast_any()
            .downcast_ref::<TridentEntity>()
            .unwrap();
        let transferred = projectile.item_stack.lock().unwrap();
        assert_eq!(transferred.uid, weapon.uid);
        assert!(
            transferred.are_equal(&weapon),
            "old life changed the admitted trident's wear"
        );
        assert!(p.player.inventory().get_stack(0).is_empty());
        assert_eq!(hooks.counts(), (1, 1, 1));
    })
    .await;
}
