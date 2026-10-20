#![expect(clippy::unwrap_used, reason = "Bounded real Java packet race fixture")]
use super::rejected_prediction_test_support::{
    Gate, GatedSlot, assert_same, helmet, replace_helmet,
};
use crate::{entity::death_test_world::DeathTestWorld, net::java::combat_test_support::TestPlayer};
use pumpkin_data::packet::CURRENT_MC_VERSION;
use pumpkin_inventory::{Inventory, screen_handler::ScreenHandler};
use pumpkin_protocol::{
    ClientPacket, RawPacket, VarInt,
    codec::item_stack_seralizer::OptionalItemStackHash,
    java::server::play::{SClickSlot, SlotActionType},
    packet::MultiVersionJavaPacket,
};
use pumpkin_util::GameMode;
use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

async fn race(mode: GameMode, cursed: bool) {
    let world = DeathTestWorld::new().await;
    let fixture = Arc::new(TestPlayer::new(&world.world()));
    world
        .world()
        .players
        .store(Arc::new(vec![fixture.player.clone()]));
    fixture.player.gamemode.store(mode);
    fixture.player.inventory().set_stack(39, helmet(false));
    let (gate, entered, resume) = Gate::new();
    let mut handler = fixture.player.player_screen_handler.lock().unwrap();
    let inner = handler.get_behaviour().slots[5].clone();
    handler.get_behaviour_mut().slots[5] = Arc::new(GatedSlot {
        inner,
        gate,
        after_preview: false,
        armed: AtomicBool::new(false),
        committed_destination: None,
    });
    drop(handler);
    let click_fixture = fixture.clone();
    let server = world.server.clone();
    let click = tokio::task::spawn_blocking(move || {
        let mut payload = Vec::new();
        SClickSlot {
            sync_id: VarInt(0),
            revision: VarInt(0),
            slot: 5,
            button: SClickSlot::BUTTON_LEFT,
            mode: SlotActionType::QuickMove,
            length_of_array: VarInt(0),
            array_of_changed_slots: Vec::new(),
            carried_item: OptionalItemStackHash(None),
        }
        .write_packet_data(&mut payload, &CURRENT_MC_VERSION)
        .unwrap();
        click_fixture
            .client()
            .handle_play_packet(
                &click_fixture.player,
                &server,
                &RawPacket {
                    id: SClickSlot::to_id(CURRENT_MC_VERSION),
                    payload: payload.into(),
                },
            )
            .unwrap();
    });
    tokio::task::spawn_blocking(move || entered.recv_timeout(Duration::from_secs(10)).unwrap())
        .await
        .unwrap();
    let replacement = replace_helmet(&world.server, &fixture.player, cursed);
    resume.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(15), click)
        .await
        .unwrap()
        .unwrap();
    let denied = mode == GameMode::Survival && cursed;
    let inventory = fixture.player.inventory();
    assert_eq!(
        (0..inventory.size())
            .map(|slot| u32::from(inventory.get_stack(slot).item_count))
            .sum::<u32>(),
        1
    );
    assert!(world.world().entities.load().is_empty());
    assert_eq!(
        inventory.get_stack(39).is_empty(),
        !denied,
        "lock-pinned cursed armor must stay equipped"
    );
    if denied {
        assert_same(&inventory.get_stack(39), &replacement);
    }
    let targets: Vec<_> = (0..36)
        .map(|slot| inventory.get_stack(slot))
        .filter(|stack| !stack.is_empty())
        .collect();
    assert_eq!(
        targets.len(),
        usize::from(!denied),
        "QuickMove destinations"
    );
    if !denied {
        assert!(targets[0].are_equal(&replacement));
    }
    world.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g02_quickmove_rechecks_locked_armor_take_rule() {
    super::rejected_prediction_test_support::bounded(async {
        race(GameMode::Survival, true).await;
    })
    .await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g02_quickmove_uncursed_replacement_control() {
    super::rejected_prediction_test_support::bounded(async {
        race(GameMode::Survival, false).await;
    })
    .await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g02_quickmove_creative_cursed_control() {
    super::rejected_prediction_test_support::bounded(async {
        race(GameMode::Creative, true).await;
    })
    .await;
}
