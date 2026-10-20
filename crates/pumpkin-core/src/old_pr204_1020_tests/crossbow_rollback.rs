use super::{items::ground, support::*};
use crate::{
    entity::{death_test_world::DeathTestWorld, player::Player},
    net::java::combat_test_support::TestPlayer,
};
use pumpkin_data::{
    Enchantment,
    data_component_impl::{
        ChargedProjectilesImpl, CustomDataImpl, IntangibleProjectileImpl, UseCooldownImpl,
    },
    item::Item,
    item_stack::ItemStack,
};
use pumpkin_inventory::Inventory;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::Hand;
use std::sync::Arc;

const COOLDOWN: &str = "old_pr204_1020_crossbow_rollback";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_pr204_1020_crossbow_restored_charge_launch_conserves_stock() {
    run(|f| restored_charge_cases(f, Veto::Launch)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_pr204_1020_crossbow_restored_charge_spawn_conserves_stock() {
    run(|f| restored_charge_cases(f, Veto::Spawn)).await;
}

fn restored_charge_cases(f: &DeathTestWorld, veto: Veto) {
    for hand in [Hand::Right, Hand::Left] {
        for multishot in [false, true] {
            for item in [&Item::ARROW, &Item::FIREWORK_ROCKET] {
                restored_charge_case(f, veto, hand, multishot, item);
            }
        }
    }
}

fn restored_charge_case(
    f: &DeathTestWorld,
    veto: Veto,
    hand: Hand,
    multishot: bool,
    item: &'static Item,
) {
    let mut p = player(f);
    let hooks = Hooks::install(&f.server, &p.player);
    let mut weapon = stack(1, &Item::CROSSBOW);
    weapon.set_data_component(UseCooldownImpl::new(1.0, Some(COOLDOWN.into())));
    if multishot {
        weapon.add_enchantment(&Enchantment::MULTISHOT, 1);
    }
    p.player.inventory().set_stack(slot(hand), weapon);
    // Crossbows prefer hand-held rockets; keep the ammunition opposite the weapon.
    let ammo_slot = if item == &Item::FIREWORK_ROCKET {
        slot(if hand == Hand::Right {
            Hand::Left
        } else {
            Hand::Right
        })
    } else {
        9
    };
    p.player.inventory().set_stack(ammo_slot, stack(2, item));
    shoot(&p, f, hand);
    let loaded = p.player.inventory().get_stack(slot(hand));
    let rounds = if multishot { 3 } else { 1 };
    assert_eq!(charge(&loaded), rounds);
    assert_eq!(loose(&p.player, item), 1);
    assert_eq!(stock(&p.player, item), 2);
    // A rollback may also edit unrelated components. Only the charge owns this payment.
    let mut restored = loaded.clone();
    let mut data = NbtCompound::new();
    data.put_int("rollback_edit", 394);
    restored.set_data_component(CustomDataImpl::new(data));
    let before = f.world().entities.load().len();
    p.take_packets();
    for attempt in 0..3 {
        let snapshot = restored.clone();
        hooks.configure(
            veto,
            false,
            Some(Arc::new(move |p| {
                assert_eq!(charge(&p.inventory().get_stack(slot(hand))), 0);
                p.inventory().set_stack(slot(hand), snapshot.clone());
            })),
        );
        use_item(&p, f, hand);
        assert_eq!(
            stock(&p.player, item),
            2,
            "{veto:?}, {hand:?}, multishot={multishot}, ammo={}, attempt={attempt}",
            item.registry_key
        );
        let current = p.player.inventory().get_stack(slot(hand));
        assert_eq!(current.uid, loaded.uid);
        assert_eq!(
            current.get_data_component::<ChargedProjectilesImpl>(),
            loaded.get_data_component::<ChargedProjectilesImpl>()
        );
        assert!(current.are_equal(&restored));
        assert_eq!(current.get_damage(), 0);
        assert_eq!(f.world().entities.load().len(), before);
        assert_eq!(
            hooks.counts(),
            (rounds, if veto == Veto::Spawn { rounds } else { 0 }, 0)
        );
        assert_eq!(used(&p.player, &Item::CROSSBOW), 0);
        assert!(!p.player.is_on_cooldown(COOLDOWN));
        let packets = p.take_packets();
        assert_quiet(&packets);
        assert_slot(&packets, slot(hand), &current);
    }
    hooks.configure(Veto::None, false, None);
    assert_successful_draw(&mut p, f, hand, item, rounds, before, &hooks);
}

fn assert_successful_draw(
    p: &mut TestPlayer,
    f: &DeathTestWorld,
    hand: Hand,
    item: &Item,
    rounds: usize,
    before: usize,
    hooks: &Hooks,
) {
    let uid = p.player.inventory().get_stack(slot(hand)).uid;
    use_item(p, f, hand);
    let current = p.player.inventory().get_stack(slot(hand));
    assert_eq!(current.uid, uid);
    assert_eq!(charge(&current), 0);
    assert_eq!(
        stock(&p.player, item),
        1,
        "successful draw refunded its payment"
    );
    assert_eq!(f.world().entities.load().len() - before, rounds);
    assert_eq!(hooks.counts(), (rounds, rounds, rounds));
    assert_eq!(
        current.get_damage(),
        rounds as i32 * if item == &Item::FIREWORK_ROCKET { 3 } else { 1 }
    );
    assert_eq!(used(&p.player, &Item::CROSSBOW), 1);
    assert!(p.player.is_on_cooldown(COOLDOWN));
    assert_launch_sound(&p.take_packets(), true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_pr204_1020_crossbow_other_charge_still_refunds_original_payment() {
    run(|f| {
        for veto in [Veto::Launch, Veto::Spawn] {
            for fresh_uid in [false, true] {
                let p = player(f);
                let hooks = Hooks::install(&f.server, &p.player);
                p.player.inventory().set_stack(0, stack(1, &Item::CROSSBOW));
                p.player.inventory().set_stack(9, stack(2, &Item::ARROW));
                shoot(&p, f, Hand::Right);
                let loaded = p.player.inventory().get_stack(0);
                let mut replacement = if fresh_uid {
                    loaded.copy_with_count(1)
                } else {
                    loaded
                };
                if !fresh_uid {
                    let mut rocket = NbtCompound::new();
                    stack(1, &Item::FIREWORK_ROCKET).write_item_stack(&mut rocket);
                    replacement.set_data_component(ChargedProjectilesImpl {
                        projectiles: vec![rocket],
                    });
                }
                let expected = replacement.clone();
                hooks.configure(
                    veto,
                    false,
                    Some(Arc::new(move |p| {
                        p.inventory().set_stack(0, replacement.clone());
                    })),
                );
                let before = f.world().entities.load().len();
                use_item(&p, f, Hand::Right);
                let current = p.player.inventory().get_stack(0);
                assert_eq!(current.uid, expected.uid);
                assert!(current.are_equal(&expected));
                assert_eq!(loose(&p.player, &Item::ARROW), 2);
                assert_eq!(ground(&p.player, &Item::ARROW), 0);
                assert_eq!(f.world().entities.load().len(), before);
                assert_eq!(current.get_damage(), 0);
            }
        }
    })
    .await;
}

pub(super) fn stock(player: &Player, item: &Item) -> usize {
    let loaded: usize = (0..player.inventory().size())
        .map(|slot| player.inventory().get_stack(slot))
        .filter_map(|stack| {
            stack
                .get_data_component::<ChargedProjectilesImpl>()
                .cloned()
        })
        .flat_map(|charge| charge.projectiles)
        .filter_map(|nbt| ItemStack::read_item_stack(&nbt))
        .filter(|stack| {
            stack.item == item
                && stack
                    .get_data_component::<IntangibleProjectileImpl>()
                    .is_none()
        })
        .map(|stack| usize::from(stack.item_count))
        .sum();
    loose(player, item) + loaded + ground(player, item)
}
