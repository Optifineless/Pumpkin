use super::{crossbow_rollback::stock, support::*};
use crate::{entity::death_test_world::DeathTestWorld, net::java::combat_test_support::TestPlayer};
use pumpkin_data::{
    Enchantment,
    data_component_impl::{ChargedProjectilesImpl, CustomDataImpl, IntangibleProjectileImpl},
    item::Item,
    item_stack::ItemStack,
};
use pumpkin_inventory::Inventory;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::Hand;
use std::sync::Arc;

#[derive(Clone, Copy, Debug)]
enum Retained {
    Full,
    Paid,
    PaidAndCopy,
    Copy,
    Copies,
    Empty,
    FreshUid,
    OtherItem,
    Repeated,
    MultiplePaid,
    SplitCount,
    MergedCount,
}

macro_rules! retention_test {
    ($name:ident, $retained:ident, $veto:ident) => {
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn $name() {
            run(|f| {
                for item in [&Item::ARROW, &Item::FIREWORK_ROCKET] {
                    retention_case(f, Veto::$veto, Retained::$retained, item);
                    admitted_control(f, item);
                }
            })
            .await;
        }
    };
}

retention_test!(old_pr204_1020_paid_full_launch, Full, Launch);
retention_test!(old_pr204_1020_paid_full_spawn, Full, Spawn);
retention_test!(old_pr204_1020_paid_only_launch, Paid, Launch);
retention_test!(old_pr204_1020_paid_only_spawn, Paid, Spawn);
retention_test!(old_pr204_1020_paid_plus_copy_launch, PaidAndCopy, Launch);
retention_test!(old_pr204_1020_paid_plus_copy_spawn, PaidAndCopy, Spawn);
retention_test!(old_pr204_1020_paid_only_copy_launch, Copy, Launch);
retention_test!(old_pr204_1020_paid_only_copy_spawn, Copy, Spawn);
retention_test!(old_pr204_1020_paid_only_copies_launch, Copies, Launch);
retention_test!(old_pr204_1020_paid_only_copies_spawn, Copies, Spawn);
retention_test!(old_pr204_1020_paid_empty_launch, Empty, Launch);
retention_test!(old_pr204_1020_paid_empty_spawn, Empty, Spawn);
retention_test!(old_pr204_1020_paid_fresh_uid_launch, FreshUid, Launch);
retention_test!(old_pr204_1020_paid_fresh_uid_spawn, FreshUid, Spawn);
retention_test!(old_pr204_1020_paid_other_item_launch, OtherItem, Launch);
retention_test!(old_pr204_1020_paid_other_item_spawn, OtherItem, Spawn);
retention_test!(old_pr204_1020_paid_repeated_launch, Repeated, Launch);
retention_test!(old_pr204_1020_paid_repeated_spawn, Repeated, Spawn);
retention_test!(old_pr204_1020_paid_multiple_launch, MultiplePaid, Launch);
retention_test!(old_pr204_1020_paid_multiple_spawn, MultiplePaid, Spawn);
retention_test!(old_pr204_1020_paid_split_count_launch, SplitCount, Launch);
retention_test!(old_pr204_1020_paid_split_count_spawn, SplitCount, Spawn);
retention_test!(old_pr204_1020_paid_merged_count_launch, MergedCount, Launch);
retention_test!(old_pr204_1020_paid_merged_count_spawn, MergedCount, Spawn);

fn loaded_player(f: &DeathTestWorld, item: &'static Item) -> TestPlayer {
    let p = player(f);
    let mut weapon = stack(1, &Item::CROSSBOW);
    weapon.add_enchantment(&Enchantment::MULTISHOT, 1);
    p.player.inventory().set_stack(0, weapon);
    // Rockets must be held opposite the crossbow to be selected as ammunition.
    let ammo_slot = if item == &Item::FIREWORK_ROCKET {
        40
    } else {
        9
    };
    p.player.inventory().set_stack(ammo_slot, stack(2, item));
    shoot(&p, f, Hand::Right);
    assert_eq!(loose(&p.player, item), 1);
    let loaded = p.player.inventory().get_stack(0);
    let charge = loaded
        .get_data_component::<ChargedProjectilesImpl>()
        .unwrap();
    assert_eq!(charge.projectiles.len(), 3);
    for (index, nbt) in charge.projectiles.iter().enumerate() {
        let round = ItemStack::read_item_stack(nbt).unwrap();
        assert_eq!(round.item, item);
        assert_eq!(
            round
                .get_data_component::<IntangibleProjectileImpl>()
                .is_some(),
            index != 0,
            "real loading must distinguish its paid round from Multishot copies"
        );
    }
    p
}

fn retention_case(f: &DeathTestWorld, veto: Veto, retained: Retained, item: &'static Item) {
    let mut p = loaded_player(f, item);
    let hooks = Hooks::install(&f.server, &p.player);
    let original = original_charge(&p, retained);
    let stock_before = stock(&p.player, item);
    assert_eq!(
        stock_before,
        if matches!(
            retained,
            Retained::MultiplePaid | Retained::SplitCount | Retained::MergedCount
        ) {
            3
        } else {
            2
        }
    );
    let attempts = if matches!(retained, Retained::Repeated) {
        3
    } else {
        1
    };
    for attempt in 0..attempts {
        let firing = p.player.inventory().get_stack(0);
        let rounds = charge(&firing);
        let replacement = replacement(&original, retained, item, attempt);
        let expected = replacement.clone();
        hooks.configure(
            veto,
            false,
            Some(Arc::new(move |p| {
                assert_eq!(charge(&p.inventory().get_stack(0)), 0);
                p.inventory().set_stack(0, replacement.clone());
            })),
        );
        let before = f.world().entities.load().len();
        p.take_packets();
        use_item(&p, f, Hand::Right);
        let current = p.player.inventory().get_stack(0);
        assert_eq!(current.uid, expected.uid, "{veto:?}, {retained:?}");
        assert!(
            current.are_equal(&expected),
            "callback charge/component edits lost: {veto:?}, {retained:?}"
        );
        let callback_created = usize::from(matches!(retained, Retained::FreshUid));
        assert_eq!(
            stock(&p.player, item),
            stock_before + callback_created,
            "physical stock changed: {veto:?}, {retained:?}, ammo={}, attempt={attempt}",
            item.registry_key
        );
        if matches!(retained, Retained::OtherItem) {
            assert_eq!(
                stock(&p.player, other(item)),
                1,
                "callback's new ammunition disappeared"
            );
        }
        assert_eq!(f.world().entities.load().len(), before);
        assert_eq!(
            hooks.counts(),
            (rounds, if veto == Veto::Spawn { rounds } else { 0 }, 0)
        );
        assert_eq!(current.get_damage(), 0);
        assert_eq!(used(&p.player, &Item::CROSSBOW), 0);
        let packets = p.take_packets();
        assert_quiet(&packets);
        assert_slot(&packets, 0, &current);
    }
}

fn original_charge(p: &TestPlayer, retained: Retained) -> ItemStack {
    let mut original = p.player.inventory().get_stack(0);
    let mut charge = original
        .get_data_component::<ChargedProjectilesImpl>()
        .unwrap()
        .clone();
    // Supported saved charges may hold several paid entries or stacked paid counts.
    match retained {
        Retained::MultiplePaid | Retained::MergedCount => {
            charge.projectiles.push(charge.projectiles[0].clone());
        }
        Retained::SplitCount => set_count(&mut charge.projectiles[0], 2),
        _ => {}
    }
    original.set_data_component(charge);
    p.player.inventory().set_stack(0, original.clone());
    original
}

fn set_count(nbt: &mut NbtCompound, count: u8) {
    let mut round = ItemStack::read_item_stack(nbt).unwrap();
    round.item_count = count;
    *nbt = NbtCompound::new();
    round.write_item_stack(nbt);
}

fn replacement(
    original: &ItemStack,
    retained: Retained,
    item: &'static Item,
    attempt: usize,
) -> ItemStack {
    let mut replacement = if matches!(retained, Retained::FreshUid) {
        original.copy_with_count(1)
    } else {
        original.clone()
    };
    let original_charge = original
        .get_data_component::<ChargedProjectilesImpl>()
        .unwrap();
    let indices: &[usize] = match retained {
        Retained::Full | Retained::FreshUid => &[0, 1, 2],
        Retained::Paid | Retained::MultiplePaid | Retained::SplitCount | Retained::MergedCount => {
            &[0]
        }
        Retained::PaidAndCopy => &[0, 1],
        Retained::Copy => &[1],
        Retained::Copies => &[1, 2],
        Retained::Empty | Retained::OtherItem => &[],
        Retained::Repeated => {
            if attempt == 0 {
                &[0, 1]
            } else {
                &[0]
            }
        }
    };
    let mut projectiles: Vec<_> = indices
        .iter()
        .map(|i| original_charge.projectiles[*i].clone())
        .collect();
    match retained {
        Retained::SplitCount => set_count(&mut projectiles[0], 1),
        Retained::MergedCount => set_count(&mut projectiles[0], 2),
        _ => {}
    }
    if matches!(retained, Retained::Paid) {
        // Payment matching depends on item and tangibility, even if callback NBT differs.
        let mut paid = ItemStack::read_item_stack(&projectiles[0]).unwrap();
        let mut data = NbtCompound::new();
        data.put_int("edited_paid_round", 394);
        paid.set_data_component(CustomDataImpl::new(data));
        projectiles[0] = NbtCompound::new();
        paid.write_item_stack(&mut projectiles[0]);
    }
    if matches!(retained, Retained::OtherItem) {
        let mut nbt = NbtCompound::new();
        stack(1, other(item)).write_item_stack(&mut nbt);
        projectiles.push(nbt);
    }
    replacement.set_data_component(ChargedProjectilesImpl { projectiles });
    let mut data = NbtCompound::new();
    data.put_int("callback_edit", 394);
    replacement.set_data_component(CustomDataImpl::new(data));
    replacement
}

fn other(item: &'static Item) -> &'static Item {
    if item == &Item::ARROW {
        &Item::FIREWORK_ROCKET
    } else {
        &Item::ARROW
    }
}

fn admitted_control(f: &DeathTestWorld, item: &'static Item) {
    let p = loaded_player(f, item);
    let hooks = Hooks::install(&f.server, &p.player);
    hooks.configure(Veto::None, false, None);
    let before = f.world().entities.load().len();
    use_item(&p, f, Hand::Right);
    assert_eq!(charge(&p.player.inventory().get_stack(0)), 0);
    assert_eq!(
        stock(&p.player, item),
        1,
        "admitted control refunded a spent paid round"
    );
    assert_eq!(f.world().entities.load().len() - before, 3);
    assert_eq!(hooks.counts(), (3, 3, 3));
}
