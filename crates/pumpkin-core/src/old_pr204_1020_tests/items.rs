use super::support::*;
use crate::entity::{
    EntityBase,
    item::ItemEntity,
    projectile::{arrow::ArrowPickup, trident::TridentEntity},
};
use pumpkin_data::{
    data_component_impl::{FireworksImpl, UnbreakableImpl},
    item::Item,
    item_stack::ItemStack,
};
use pumpkin_inventory::Inventory;
use pumpkin_util::{GameMode, Hand};
use std::sync::Arc;

pub(super) fn ground(p: &crate::entity::player::Player, item: &Item) -> usize {
    p.world()
        .entities
        .load()
        .iter()
        .filter_map(|e| e.cast_any().downcast_ref::<ItemEntity>())
        .map(|e| e.get_item_stack().lock().unwrap().clone())
        .filter(|s| s.item == item)
        .map(|s| usize::from(s.item_count))
        .sum()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_pr204_1020_throwables_restore_source_and_quiet_veto() {
    run(|f| {
        for item in [
            &Item::SNOWBALL,
            &Item::EGG,
            &Item::ENDER_PEARL,
            &Item::SPLASH_POTION,
            &Item::LINGERING_POTION,
            &Item::EXPERIENCE_BOTTLE,
            &Item::WIND_CHARGE,
        ] {
            for hand in [Hand::Right, Hand::Left] {
                for mode in [GameMode::Survival, GameMode::Creative] {
                    for count in [1, 3] {
                        for veto in [Veto::Launch, Veto::Spawn, Veto::None] {
                            let mut p = player(f);
                            p.player.gamemode.store(mode);
                            let hooks = Hooks::install(&f.server, &p.player);
                            hooks.configure(veto, false, None);
                            let original = stack(count, item);
                            p.player.inventory().set_stack(slot(hand), original.clone());
                            p.take_packets();
                            let before = f.world().entities.load().len();
                            use_item(&p, f, hand);
                            let admitted = veto == Veto::None;
                            assert_eq!(
                                f.world().entities.load().len() - before,
                                usize::from(admitted)
                            );
                            assert_eq!(hooks.counts(), (1, usize::from(veto != Veto::Launch), 0));
                            let actual = p.player.inventory().get_stack(slot(hand));
                            assert_eq!(
                                loose(&p.player, item),
                                usize::from(count)
                                    - usize::from(admitted && mode == GameMode::Survival)
                            );
                            if !admitted {
                                assert_eq!(
                                    actual.uid, original.uid,
                                    "last vetoed item changed source identity"
                                );
                                assert!(actual.are_equal(&original));
                            }
                            let packets = p.take_packets();
                            assert_launch_sound(&packets, admitted);
                            assert_slot(&packets, slot(hand), &actual);
                            if !admitted {
                                assert_quiet(&packets);
                                assert!(!p.player.is_on_cooldown(item.registry_key));
                            }
                            assert_eq!(used(&p.player, item), i32::from(admitted));
                        }
                    }
                }
            }
        }
        throwable_callback_controls(f);
    })
    .await;
}

fn admit_flight(
    p: &crate::net::java::combat_test_support::TestPlayer,
    f: &crate::entity::death_test_world::DeathTestWorld,
) {
    use pumpkin_protocol::{
        VarInt,
        java::server::play::{Action, SPlayerCommand},
    };
    p.player
        .inventory()
        .set_stack(38, ItemStack::new(1, &Item::ELYTRA));
    p.player.get_entity().on_ground.store(false, SeqCst);
    p.client().handle_player_command(
        &p.player,
        &SPlayerCommand {
            entity_id: VarInt(p.player.entity_id()),
            action: Action::StartFlyingElytra,
            jump_boost: VarInt(0),
        },
        &f.server,
    );
    assert!(p.player.get_entity().is_fall_flying());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_pr204_1020_firework_block_and_flying_claim_once() {
    run(|f| {
        for flying in [false, true] {
            for hand in [Hand::Right, Hand::Left] {
                for veto in [Veto::Launch, Veto::Spawn, Veto::None] {
                    for edit in 0..4 {
                        let mut p = player(f);
                        let hooks = Hooks::install(&f.server, &p.player);
                        let mut rocket = stack(2, &Item::FIREWORK_ROCKET);
                        rocket.set_data_component(FireworksImpl::new(2, Vec::new()));
                        p.player.inventory().set_stack(slot(hand), rocket.clone());
                        if flying {
                            admit_flight(&p, f);
                        }
                        let source = slot(hand);
                        hooks.configure(
                            veto,
                            false,
                            Some(Arc::new(move |p| {
                                assert_eq!(
                                    p.inventory().get_stack(source).item_count,
                                    2,
                                    "rocket callback did not observe the unconsumed source"
                                );
                                if edit == 1 {
                                    let mut grant = p.inventory().get_stack(source);
                                    grant.increment(1);
                                    p.inventory().set_stack(source, grant);
                                }
                                if edit == 2 {
                                    let moved = p.inventory().get_stack(source);
                                    p.inventory().set_stack(9, moved);
                                    p.inventory().set_stack(source, ItemStack::EMPTY.clone());
                                }
                                if edit == 3 {
                                    p.inventory().set_stack(source, stack(1, &Item::DIAMOND));
                                }
                            })),
                        );
                        p.take_packets();
                        let before = f.world().entities.load().len();
                        if flying {
                            use_item(&p, f, hand);
                        } else {
                            block_rocket(&p, f, hand);
                        }
                        let admitted = veto == Veto::None && edit == 0;
                        let callback_stock = match edit {
                            1 => 3,
                            3 => 0,
                            _ => 2,
                        };
                        let expected = callback_stock - usize::from(admitted);
                        assert_eq!(
                            loose(&p.player, &Item::FIREWORK_ROCKET),
                            expected,
                            "rocket authoritative claim/writeback lost stock or charged twice"
                        );
                        assert_eq!(
                            f.world().entities.load().len() - before,
                            usize::from(admitted)
                        );
                        if edit == 3 && !admitted {
                            assert_eq!(p.player.inventory().get_stack(source).item, &Item::DIAMOND);
                        }
                        let packets = p.take_packets();
                        assert_slot(&packets, source, &p.player.inventory().get_stack(source));
                        if !admitted {
                            assert_quiet(&packets);
                        }
                    }
                }
            }
        }
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_pr204_1020_trident_paid_receipt_and_eligible_wear() {
    run(|f| {
        for hand in [Hand::Right, Hand::Left] {
            for mode in [GameMode::Survival, GameMode::Creative] {
                for veto in [Veto::Launch, Veto::Spawn, Veto::None] {
                    for control in 0..5 {
                        let mut p = player(f);
                        p.player.gamemode.store(mode);
                        let hooks = Hooks::install(&f.server, &p.player);
                        hooks.configure(veto, false, None);
                        let mut trident = stack(1, &Item::TRIDENT);
                        trident.set_damage(7);
                        if control == 1 {
                            trident.set_data_component(UnbreakableImpl);
                        }
                        p.player.inventory().set_stack(slot(hand), trident.clone());
                        if control == 2 {
                            hooks.state.lock().unwrap().cancel_damage = true;
                        }
                        if control >= 3 {
                            hooks.state.lock().unwrap().damage_amount =
                                Some(if control == 3 { 4 } else { 300 });
                        }
                        p.take_packets();
                        let before = f.world().entities.load().len();
                        shoot(&p, f, hand);
                        let admitted = veto == Veto::None;
                        assert_eq!(
                            f.world().entities.load().len() - before,
                            usize::from(admitted)
                        );
                        let eligible = admitted && mode == GameMode::Survival && control != 1;
                        assert_eq!(
                            hooks.counts().2,
                            usize::from(eligible),
                            "ineligible or rejected trident emitted a damage event"
                        );
                        let actual = p.player.inventory().get_stack(slot(hand));
                        if admitted {
                            let world = f.world();
                            let entities = world.entities.load();
                            let thrown = entities
                                .last()
                                .unwrap()
                                .cast_any()
                                .downcast_ref::<TridentEntity>()
                                .unwrap();
                            let item = thrown.item_stack.lock().unwrap();
                            assert_eq!(
                                item.get_damage(),
                                (7 + if eligible && control != 2 {
                                    if control == 4 {
                                        300
                                    } else if control == 3 {
                                        4
                                    } else {
                                        1
                                    }
                                } else {
                                    0
                                })
                                .min(trident.get_max_damage().unwrap() - 1)
                            );
                            let mut expected = trident.clone();
                            expected.set_damage(item.get_damage());
                            assert!(item.are_items_and_components_equal(&expected));
                            assert_eq!(
                                thrown.pickup.load(),
                                if mode == GameMode::Creative {
                                    ArrowPickup::CreativeOnly
                                } else {
                                    ArrowPickup::Allowed
                                }
                            );
                            if mode == GameMode::Survival {
                                assert_eq!(item.uid, trident.uid);
                                assert!(actual.is_empty());
                            } else {
                                assert!(actual.are_equal(&trident));
                            }
                        } else {
                            assert_eq!(actual.uid, trident.uid);
                            assert!(actual.are_equal(&trident));
                        }
                        let packets = p.take_packets();
                        assert_launch_sound(&packets, admitted);
                        assert_slot(&packets, slot(hand), &actual);
                        if !admitted {
                            assert_quiet(&packets);
                        }
                    }
                }
            }
        }
        trident_callback_controls(f);
    })
    .await;
}

fn throwable_callback_controls(f: &crate::entity::death_test_world::DeathTestWorld) {
    for edit in 0..4 {
        for veto in [Veto::Launch, Veto::Spawn] {
            let mut p = player(f);
            let hooks = Hooks::install(&f.server, &p.player);
            let original = stack(1, &Item::EGG);
            let uid = original.uid;
            p.player.inventory().set_stack(0, original);
            hooks.configure(
                veto,
                false,
                Some(Arc::new(move |p| {
                    if edit == 0 {
                        p.inventory().set_stack(0, stack(1, &Item::DIAMOND));
                    }
                    if edit == 1 {
                        p.inventory().set_stack(0, stack(2, &Item::EGG));
                    }
                    if edit == 2 {
                        p.inventory().set_selected_slot(4);
                        p.inventory().set_stack(4, stack(1, &Item::DIAMOND));
                    }
                    if edit == 3 {
                        for i in 0..p.inventory().size() {
                            p.inventory()
                                .set_stack(i, ItemStack::new(64, &Item::DIAMOND));
                        }
                    }
                })),
            );
            let before = ground(&p.player, &Item::EGG);
            p.take_packets();
            use_item(&p, f, Hand::Right);
            assert_eq!(
                loose(&p.player, &Item::EGG) + ground(&p.player, &Item::EGG) - before,
                match edit {
                    1 => 2,
                    2 => 1,
                    _ => 0,
                },
                "denial changed the callback's authoritative stock"
            );
            if edit == 0 || edit == 3 {
                assert_eq!(p.player.inventory().get_stack(0).item, &Item::DIAMOND);
            }
            if edit == 2 {
                assert_eq!(p.player.inventory().get_stack(0).uid, uid);
                assert_eq!(p.player.inventory().get_stack(4).item, &Item::DIAMOND);
            }
            assert_slot(&p.take_packets(), 0, &p.player.inventory().get_stack(0));
        }
    }
}

fn trident_callback_controls(f: &crate::entity::death_test_world::DeathTestWorld) {
    for veto in [Veto::Launch, Veto::Spawn] {
        let p = player(f);
        let hooks = Hooks::install(&f.server, &p.player);
        p.player.inventory().set_stack(0, stack(1, &Item::TRIDENT));
        hooks.configure(
            veto,
            false,
            Some(Arc::new(|p| {
                p.inventory().set_stack(0, stack(1, &Item::DIAMOND));
            })),
        );
        shoot(&p, f, Hand::Right);
        assert_eq!(p.player.inventory().get_stack(0).item, &Item::DIAMOND);
        assert_eq!(loose(&p.player, &Item::TRIDENT), 0);
    }
    let p = player(f);
    let hooks = Hooks::install(&f.server, &p.player);
    let mut trident = stack(1, &Item::TRIDENT);
    trident.set_damage(trident.get_max_damage().unwrap() - 1);
    p.player.inventory().set_stack(0, trident.clone());
    shoot(&p, f, Hand::Right);
    assert_eq!(hooks.counts(), (0, 0, 0));
    assert!(p.player.inventory().get_stack(0).are_equal(&trident));
}
