use super::support::*;
use crate::entity::projectile::arrow::{ArrowEntity, ArrowPickup};
use pumpkin_data::{data_component_impl::UnbreakableImpl, item::Item};
use pumpkin_inventory::Inventory;
use pumpkin_util::{GameMode, Hand};

// Separate tests keep the success and already-working spawn-refund controls executable when
// a deliberately reverted regression panics at the first denied case of its matrix.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_pr204_1020_normal_success_controls() {
    run(|f| {
        for item in [
            &Item::SNOWBALL,
            &Item::EGG,
            &Item::ENDER_PEARL,
            &Item::SPLASH_POTION,
            &Item::LINGERING_POTION,
            &Item::EXPERIENCE_BOTTLE,
            &Item::WIND_CHARGE,
            &Item::BOW,
            &Item::CROSSBOW,
            &Item::TRIDENT,
            &Item::FIREWORK_ROCKET,
        ] {
            let mut p = player(f);
            let hooks = Hooks::install(&f.server, &p.player);
            hooks.configure(Veto::None, false, None);
            p.player.inventory().set_stack(0, stack(1, item));
            if item == &Item::BOW || item == &Item::CROSSBOW {
                p.player.inventory().set_stack(9, stack(2, &Item::ARROW));
            }
            let before = f.world().entities.load().len();
            p.take_packets();
            if item == &Item::FIREWORK_ROCKET {
                block_rocket(&p, f, Hand::Right);
            } else if item == &Item::BOW || item == &Item::CROSSBOW || item == &Item::TRIDENT {
                shoot(&p, f, Hand::Right);
                if item == &Item::CROSSBOW {
                    use_item(&p, f, Hand::Right);
                }
            } else {
                use_item(&p, f, Hand::Right);
            }
            assert_eq!(hooks.counts().0, 1);
            assert_eq!(hooks.counts().1, 1);
            assert_eq!(f.world().entities.load().len() - before, 1);
            if item == &Item::BOW || item == &Item::CROSSBOW {
                assert_eq!(loose(&p.player, &Item::ARROW), 1);
                assert_eq!(p.player.inventory().get_stack(0).get_damage(), 1);
                let world = f.world();
                let entities = world.entities.load();
                let arrow = entities
                    .last()
                    .unwrap()
                    .cast_any()
                    .downcast_ref::<ArrowEntity>()
                    .unwrap();
                assert_eq!(arrow.pickup.load(), ArrowPickup::Allowed);
            } else {
                assert!(p.player.inventory().get_stack(0).is_empty());
            }
            assert_eq!(
                used(&p.player, item),
                1,
                "successful use statistic changed for {}",
                item.registry_key
            );
        }
        let p = player(f);
        let hooks = Hooks::install(&f.server, &p.player);
        hooks.configure(Veto::None, false, None);
        p.player
            .inventory()
            .set_stack(0, stack(1, &Item::FISHING_ROD));
        use_item(&p, f, Hand::Right);
        assert_eq!(hooks.counts().0, 1);
        assert_eq!(used(&p.player, &Item::FISHING_ROD), 1);
        let hook = p
            .player
            .world()
            .get_entity_by_id(p.player.fishing_bobber.load(SeqCst))
            .unwrap();
        hook.get_entity().on_ground.store(true, SeqCst);
        use_item(&p, f, Hand::Right);
        assert_eq!(p.player.inventory().get_stack(0).get_damage(), 2);
        let mut p = player(f);
        let hooks = Hooks::install(&f.server, &p.player);
        super::dispenser::case(f, &mut p, &hooks, &Item::ARROW, Veto::None, 0);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_pr204_1020_existing_spawn_refund_and_creative_controls() {
    run(|f| {
        for item in [
            &Item::EGG,
            &Item::SNOWBALL,
            &Item::ENDER_PEARL,
            &Item::EXPERIENCE_BOTTLE,
            &Item::WIND_CHARGE,
        ] {
            let p = player(f);
            let hooks = Hooks::install(&f.server, &p.player);
            hooks.configure(Veto::Spawn, false, None);
            p.player.inventory().set_stack(0, stack(3, item));
            use_item(&p, f, Hand::Right);
            assert_eq!(
                loose(&p.player, item),
                3,
                "existing spawn refund stopped conserving stock"
            );
        }
        for veto in [Veto::Launch, Veto::Spawn] {
            let p = player(f);
            p.player.gamemode.store(GameMode::Creative);
            let hooks = Hooks::install(&f.server, &p.player);
            hooks.configure(veto, false, None);
            p.player.inventory().set_stack(0, stack(1, &Item::TRIDENT));
            shoot(&p, f, Hand::Right);
            assert_eq!(loose(&p.player, &Item::TRIDENT), 1);
            assert_eq!(hooks.counts().2, 0);
        }
        let p = player(f);
        let hooks = Hooks::install(&f.server, &p.player);
        hooks.configure(Veto::None, false, None);
        let mut trident = stack(1, &Item::TRIDENT);
        trident.set_data_component(UnbreakableImpl);
        p.player.inventory().set_stack(0, trident);
        shoot(&p, f, Hand::Right);
        assert_eq!(hooks.counts(), (1, 1, 0));
    })
    .await;
}
