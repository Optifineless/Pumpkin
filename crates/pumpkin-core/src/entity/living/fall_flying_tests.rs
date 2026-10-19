use crate::{
    entity::{EntityBase, player::Player},
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::{
            entity::entity_toggle_glide::EntityToggleGlideEvent,
            player::player_item_damage::PlayerItemDamageEvent,
            world::generic_game::GenericGameEvent,
        },
    },
    server::{Server, combat_test_support},
};
use pumpkin_data::{
    data_component::DataComponent,
    data_component_impl::{EquipmentSlot, EquippableImpl},
    entity::EntityPose,
    item::Item,
    item_stack::ItemStack,
    packet::CURRENT_MC_VERSION,
};
use pumpkin_protocol::{
    VarInt,
    java::server::play::{SSetPlayerGround, SlotActionType},
};
use pumpkin_util::{
    GameMode,
    math::{vector2::Vector2, vector3::Vector3},
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering::Relaxed},
};

mod support;
use support::*;

mod callbacks;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g15_idle_and_creative_controls_do_not_wear_glider() {
    for creative in [false, true] {
        let f = Fixture::new();
        if creative {
            f.owner.player.set_gamemode(GameMode::Creative);
            f.start();
        } else {
            dispatch(
                &f.owner.player,
                &f.server,
                &SSetPlayerGround { on_ground: true },
            );
        }
        f.tick(40);
        assert_eq!(f.owner.player.inventory.get_slot(38).get_damage(), 0);
        assert_eq!(f.owner.player.inventory.get_slot(0).get_damage(), 0);
        f.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g15_real_ticks_wear_and_publish_equipped_glider_with_reset() {
    let mut f = Fixture::new();
    dispatch(
        &f.owner.player,
        &f.server,
        &SSetPlayerGround { on_ground: true },
    );
    f.tick(20);
    assert_eq!(
        f.owner.player.inventory.get_slot(38).get_damage(),
        0,
        "idle control"
    );
    let events = Arc::new(GlideEvents {
        player: f.owner.player.clone(),
        events: Mutex::default(),
    });
    f.server.plugin_manager.register::<GenericGameEvent, _>(
        events.clone(),
        EventPriority::Normal,
        true,
    );
    f.start();
    let start = f.owner.player.tick_counter.load(Relaxed);
    f.tick(19);
    assert_eq!(
        f.owner.player.inventory.get_slot(38).get_damage(),
        0,
        "wear before tick 20"
    );
    f.clear_packets();
    f.tick(1);
    f.assert_damage_published(38, &EquipmentSlot::CHEST, 1);
    f.tick(20);
    f.assert_damage_published(38, &EquipmentSlot::CHEST, 2);
    assert_eq!(
        *events.events.lock().unwrap(),
        [
            (start + 10, 0),
            (start + 20, 1),
            (start + 30, 1),
            (start + 40, 2)
        ],
        "10-tick glide event must follow boundary wear"
    );
    f.tick(9);
    dispatch(
        &f.owner.player,
        &f.server,
        &SSetPlayerGround { on_ground: true },
    );
    f.tick(1);
    f.assert_stopped();
    f.start();
    f.tick(19);
    assert_eq!(
        f.owner.player.inventory.get_slot(38).get_damage(),
        2,
        "restart inherited old glide ticks"
    );
    f.tick(1);
    f.assert_damage_published(38, &EquipmentSlot::CHEST, 3);
    assert_eq!(
        f.owner.player.inventory.get_slot(0).get_damage(),
        0,
        "held tool control"
    );
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g15_armor_pickup_and_shift_click_stop_and_sync_airborne_glide() {
    for mode in [SlotActionType::Pickup, SlotActionType::QuickMove] {
        let mut f = Fixture::new();
        f.start();
        f.tick(9);
        f.clear_packets();
        remove_chest(&f.owner.player, &f.server, mode);
        f.tick(1);
        f.assert_stopped();
        f.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g15_ground_packet_variants_and_empty_equipment_cleanup() {
    use pumpkin_protocol::java::server::play::{
        SPlayerPosition, SPlayerPositionRotation, SPlayerRotation,
    };
    for variant in 0..4 {
        for equipped in [false, true] {
            let mut f = Fixture::new();
            f.start();
            f.tick(1);
            f.clear_packets();
            if !equipped {
                remove_chest(&f.owner.player, &f.server, SlotActionType::QuickMove);
            }
            let player = &f.owner.player;
            match variant {
                0 => dispatch(player, &f.server, &SSetPlayerGround { on_ground: true }),
                1 => dispatch(
                    player,
                    &f.server,
                    &SPlayerPosition {
                        position: player.position(),
                        collision: 1,
                    },
                ),
                2 => dispatch(
                    player,
                    &f.server,
                    &SPlayerPositionRotation {
                        position: player.position(),
                        yaw: 0.0,
                        pitch: 0.0,
                        collision: 1,
                    },
                ),
                _ => dispatch(
                    player,
                    &f.server,
                    &SPlayerRotation {
                        yaw: 0.0,
                        pitch: 0.0,
                        ground: true,
                    },
                ),
            }
            f.tick(1);
            f.assert_stopped();
            f.close().await;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g15_creative_and_cancelled_damage_controls() {
    for creative in [false, true] {
        let f = Fixture::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let called = calls.clone();
        f.server
            .plugin_manager
            .register::<PlayerItemDamageEvent, _>(
                Arc::new(DamageCallback(move |event: &mut PlayerItemDamageEvent| {
                    called.fetch_add(1, Relaxed);
                    event.cancelled = true;
                })),
                EventPriority::Normal,
                true,
            );
        if creative {
            f.owner.player.set_gamemode(GameMode::Creative);
        }
        f.start();
        f.tick(40);
        assert_eq!(f.owner.player.inventory.get_slot(38).get_damage(), 0);
        assert_eq!(calls.load(Relaxed), if creative { 0 } else { 2 });
        assert!(f.owner.player.get_entity().is_fall_flying());
        f.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g15_ineligible_and_last_durability_gliders_stop() {
    for kind in 0..5 {
        let mut f = Fixture::new();
        let mut stack = ItemStack::new(1, &Item::ELYTRA);
        match kind {
            0 => stack.set_damage(stack.get_max_damage().unwrap() - 1),
            1 => stack.remove_data_component(DataComponent::Glider),
            2 => stack.remove_data_component(DataComponent::Equippable),
            3 => {
                let mut equippable = stack
                    .get_data_component::<EquippableImpl>()
                    .unwrap()
                    .clone();
                equippable.slot = &EquipmentSlot::HEAD;
                stack.set_data_component(equippable);
            }
            _ => stack = ItemStack::EMPTY.clone(),
        }
        f.start();
        f.tick(1);
        f.clear_packets();
        f.equip(38, stack.clone(), &EquipmentSlot::CHEST);
        f.tick(20);
        f.assert_stopped();
        assert_eq!(
            f.owner.player.inventory.get_slot(38).get_damage(),
            stack.get_damage()
        );
        f.close().await;
    }
    let mut f = Fixture::new();
    let mut stack = ItemStack::new(1, &Item::ELYTRA);
    let max = stack.get_max_damage().unwrap();
    stack.set_damage(max - 2);
    f.equip(38, stack, &EquipmentSlot::CHEST);
    f.start();
    f.tick(20);
    f.assert_damage_published(38, &EquipmentSlot::CHEST, max - 1);
    f.tick(1);
    assert!(!f.owner.player.get_entity().is_fall_flying());
    assert!(f.owner.player.get_entity().pose.load() == EntityPose::Standing);
    assert_eq!(
        f.owner.player.inventory.get_slot(38).item,
        &Item::ELYTRA,
        "last usable wear must not destroy glider"
    );
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g15_callback_removal_or_replacement_preserves_live_stack() {
    for remove in [false, true] {
        let mut f = Fixture::new();
        let replacement = ItemStack::new(1, &Item::ELYTRA);
        let incoming = replacement.clone();
        let server = f.server.clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let called = calls.clone();
        f.server
            .plugin_manager
            .register::<PlayerItemDamageEvent, _>(
                Arc::new(DamageCallback(move |event: &mut PlayerItemDamageEvent| {
                    if called.fetch_add(1, Relaxed) == 0 {
                        if remove {
                            remove_chest(&event.player, &server, SlotActionType::QuickMove);
                        } else {
                            event.player.inventory.set_slot(38, incoming.clone());
                        }
                    }
                })),
                EventPriority::Normal,
                true,
            );
        f.start();
        f.tick(20);
        assert_eq!(calls.load(Relaxed), 1, "wear callback not reached");
        assert_eq!(
            f.owner.player.inventory.get_slot(38).get_damage(),
            0,
            "stale sampled stack damaged replacement"
        );
        assert_eq!(f.owner.player.inventory.get_slot(0).get_damage(), 0);
        if remove {
            f.assert_stopped();
        } else {
            assert_eq!(f.owner.player.inventory.get_slot(38).uid, replacement.uid);
            f.tick(20);
            f.assert_damage_published(38, &EquipmentSlot::CHEST, 1);
        }
        f.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g15_eligible_non_chest_and_multiple_glider_slots() {
    let f = Fixture::new();
    let mut glider = ItemStack::new(1, &Item::ELYTRA);
    let mut equippable = glider
        .get_data_component::<EquippableImpl>()
        .unwrap()
        .clone();
    equippable.slot = &EquipmentSlot::HEAD;
    glider.set_data_component(equippable);
    f.equip(38, ItemStack::EMPTY.clone(), &EquipmentSlot::CHEST);
    f.equip(39, glider, &EquipmentSlot::HEAD);
    f.start();
    f.tick(20);
    assert_eq!(
        f.owner.player.inventory.get_slot(39).get_damage(),
        1,
        "eligible head glider was ignored"
    );
    f.equip(38, ItemStack::new(1, &Item::ELYTRA), &EquipmentSlot::CHEST);
    f.tick(80);
    assert_eq!(
        f.owner.player.inventory.get_slot(39).get_damage()
            + f.owner.player.inventory.get_slot(38).get_damage(),
        5,
        "one eligible equipped slot per damage boundary"
    );
    assert_eq!(f.owner.player.inventory.get_slot(0).get_damage(), 0);
    f.close().await;
}
