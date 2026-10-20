use super::{Callback, Fixture, Item, Ordering, Route, register_callback, weapon};

#[derive(Clone, Copy)]
enum Outcome {
    Accepted,
    Empty,
    Cancelled,
}

async fn check_recharge(outcome: Outcome) {
    for route in [Route::JavaSingle, Route::JavaStack] {
        let mut fixture = Fixture::new(route).await;
        let player = fixture.player().clone();
        let stack = weapon(3, &Item::DIAMOND_SPEAR);
        if !matches!(outcome, Outcome::Empty) {
            fixture.equip(&stack);
        }
        let withdrawn = if matches!(route, Route::JavaStack) {
            3
        } else {
            1
        };
        let calls = if matches!(outcome, Outcome::Accepted) {
            None
        } else {
            Some(register_callback(&fixture, Callback::Cancel, withdrawn))
        };
        player.last_attacked_ticks.store(23, Ordering::Relaxed);
        fixture.drop_packet();
        let held = player.inventory.held_item();
        let ground = fixture.ground();
        match outcome {
            Outcome::Accepted => {
                assert_eq!(held.item_count, 3 - withdrawn);
                if withdrawn != 3 {
                    assert_eq!(held.uid, stack.uid);
                    assert!(held.are_items_and_components_equal(&stack));
                }
                assert_eq!(ground.len(), 1);
                assert_eq!(ground[0].item_count, withdrawn);
                assert!(ground[0].are_items_and_components_equal(&stack));
            }
            Outcome::Empty => {
                assert!(held.is_empty());
                assert!(ground.is_empty());
            }
            Outcome::Cancelled => {
                assert_eq!(held.uid, stack.uid);
                assert!(held.are_equal(&stack));
                assert!(ground.is_empty());
            }
        }
        if let Some(calls) = calls {
            assert_eq!(
                calls.load(Ordering::Relaxed),
                usize::from(matches!(outcome, Outcome::Cancelled))
            );
        }
        assert_eq!(
            player.last_attacked_ticks.load(Ordering::Relaxed),
            0,
            "Java drop retained attack recharge after stock checks passed"
        );
        fixture.close().await;
    }
}

#[tokio::test]
async fn b1020b_g07_q_drop_resets_attack_recharge() {
    check_recharge(Outcome::Accepted).await;
}

#[tokio::test]
async fn b1020b_g07_empty_q_drop_resets_attack_recharge() {
    check_recharge(Outcome::Empty).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g07_cancelled_q_drop_resets_attack_recharge() {
    check_recharge(Outcome::Cancelled).await;
}
