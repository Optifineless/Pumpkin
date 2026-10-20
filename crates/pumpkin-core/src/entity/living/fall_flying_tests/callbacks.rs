use super::*;
use pumpkin_data::{effect::StatusEffect, potion::Effect};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g15_glide_event_callback_removal_is_revalidated() {
    let mut f = Fixture::new();
    let player = f.owner.player.clone();
    let server = Arc::downgrade(&f.server);
    f.server.plugin_manager.register::<GenericGameEvent, _>(
        Arc::new(GameCallback(move |event: &mut GenericGameEvent| {
            if event.event_key == "elytra_glide" {
                remove_chest(
                    &player,
                    &server.upgrade().unwrap(),
                    SlotActionType::QuickMove,
                );
            }
        })),
        EventPriority::Normal,
        true,
    );
    f.start();
    f.tick(9);
    f.clear_packets();
    f.tick(1);
    f.assert_stopped();
    assert_eq!(f.owner.player.inventory.get_slot(0).get_damage(), 0);
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g15_retired_session_during_wear_rejects_old_continuation() {
    for mark_departing in [false, true] {
        let f = Fixture::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let called = calls.clone();
        f.server
            .plugin_manager
            .register::<PlayerItemDamageEvent, _>(
                Arc::new(DamageCallback(move |event: &mut PlayerItemDamageEvent| {
                    called.fetch_add(1, Relaxed);
                    if mark_departing {
                        event.player.mark_departing();
                    } else {
                        let world = event.player.world();
                        world.players.store(Arc::new(Vec::new()));
                        assert!(
                            world
                                .get_player_by_uuid(event.player.gameprofile.id)
                                .is_some_and(|tracked| Arc::ptr_eq(&tracked, &event.player)),
                            "detached session must remain discoverable through tracking"
                        );
                    }
                })),
                EventPriority::Normal,
                true,
            );
        let glides = Arc::new(AtomicUsize::new(0));
        let emitted = glides.clone();
        f.server.plugin_manager.register::<GenericGameEvent, _>(
            Arc::new(GameCallback(move |event: &mut GenericGameEvent| {
                if event.event_key == "elytra_glide" {
                    emitted.fetch_add(1, Relaxed);
                }
            })),
            EventPriority::Normal,
            true,
        );
        f.start();
        f.tick(20);
        assert_eq!(calls.load(Relaxed), 1);
        assert_eq!(
            f.owner.player.inventory.get_slot(38).get_damage(),
            0,
            "retired session received old wear"
        );
        assert_eq!(glides.load(Relaxed), 1, "retired session emitted old glide");
        f.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g15_fall_distance_uses_accepted_glide_motion() {
    use pumpkin_protocol::java::server::play::SPlayerPositionRotation;
    for (descent, expected) in [(0.25, 1.0), (1.0, 8.0)] {
        let f = Fixture::new();
        f.start();
        dispatch(
            &f.owner.player,
            &f.server,
            &SPlayerPositionRotation {
                position: f.owner.player.position() + Vector3::new(0.0, -descent, 0.0),
                yaw: 0.0,
                pitch: 0.0,
                collision: 0,
            },
        );
        f.owner.player.living_entity.fall_distance.store(8.0);
        f.tick(1);
        assert_eq!(f.owner.player.living_entity.fall_distance.load(), expected);
        f.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g15_start_flight_requires_a_usable_glider_before_fall_distance_clamp() {
    use pumpkin_protocol::java::server::play::{
        Action, SPlayerCommand, SPlayerPosition, SPlayerRotation,
    };

    for has_glider in [false, true] {
        let f = Fixture::new();
        if !has_glider {
            remove_chest(&f.owner.player, &f.server, SlotActionType::QuickMove);
        }

        let player = &f.owner.player;
        let position = player.position();
        dispatch(
            player,
            &f.server,
            &SPlayerPosition {
                position: position + Vector3::new(0.0, -8.0, 0.0),
                collision: 0,
            },
        );
        assert_eq!(player.living_entity.fall_distance.load(), 8.0);
        dispatch(
            player,
            &f.server,
            &SPlayerRotation {
                yaw: 0.0,
                pitch: 0.0,
                ground: false,
            },
        );
        assert_eq!(player.get_known_movement(), Vector3::default());
        dispatch(
            player,
            &f.server,
            &SPlayerCommand {
                entity_id: player.entity_id().into(),
                action: Action::StartFlyingElytra,
                jump_boost: VarInt(0),
            },
        );
        let started_flying = player.get_entity().is_fall_flying();
        f.tick(1);
        let fall_distance_after_tick = player.living_entity.fall_distance.load();
        // #231 requires actual support for a zero-displacement landing.
        let support = pumpkin_util::math::position::BlockPos::floored_v(player.position()).down();
        f.world.set_block_state(
            &support,
            pumpkin_data::Block::STONE.default_state.id,
            pumpkin_world::world::BlockFlags::FORCE_STATE,
        );
        dispatch(
            player,
            &f.server,
            &SPlayerPosition {
                position: player.position(),
                collision: 1,
            },
        );

        assert_eq!(
            player.living_entity.health.load(),
            if has_glider { 20.0 } else { 15.0 },
            "a rejected glide start must preserve the full eight-block fall"
        );
        assert_eq!(started_flying, has_glider, "start packet admission");
        assert_eq!(
            fall_distance_after_tick,
            if has_glider { 1.0 } else { 8.0 },
            "only a valid glide start may clamp fall distance"
        );
        f.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g15_saved_fall_flying_retains_meaning_and_restarts_runtime_cadence() {
    let mut f = Fixture::new();
    f.start();
    f.tick(19);
    let mut old_save = pumpkin_nbt::NbtCompound::new();
    f.owner.player.living_entity.write_living_nbt(&mut old_save);
    assert_eq!(old_save.get_bool("FallFlying"), Some(true));
    assert!(old_save.get("fall_fly_ticks").is_none());
    f.owner
        .player
        .living_entity
        .read_living_nbt_non_mut(&old_save);
    assert!(f.owner.player.get_entity().is_fall_flying());
    f.tick(19);
    assert_eq!(
        f.owner.player.inventory.get_slot(38).get_damage(),
        0,
        "loaded flight inherited an unsaved partial cadence"
    );
    f.tick(1);
    f.assert_damage_published_next_tick(38, &EquipmentSlot::CHEST, 1);
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g15_custom_hand_glider_rechecks_selection_after_damage_callback() {
    use pumpkin_protocol::java::server::play::SSetHeldItem;
    let mut f = Fixture::new();
    let mut glider = ItemStack::new(1, &Item::ELYTRA);
    let mut equippable = glider
        .get_data_component::<EquippableImpl>()
        .unwrap()
        .clone();
    equippable.slot = &EquipmentSlot::MAIN_HAND;
    glider.set_data_component(equippable);
    f.equip(38, ItemStack::EMPTY.clone(), &EquipmentSlot::CHEST);
    f.equip(0, glider, &EquipmentSlot::MAIN_HAND);
    f.start();
    f.tick(20);
    f.assert_damage_published_next_tick(0, &EquipmentSlot::MAIN_HAND, 1);
    let server = Arc::downgrade(&f.server);
    let calls = Arc::new(AtomicUsize::new(0));
    let called = calls.clone();
    f.server
        .plugin_manager
        .register::<PlayerItemDamageEvent, _>(
            Arc::new(DamageCallback(move |event: &mut PlayerItemDamageEvent| {
                called.fetch_add(1, Relaxed);
                dispatch(
                    &event.player,
                    &server.upgrade().unwrap(),
                    &SSetHeldItem { slot: 1 },
                );
            })),
            EventPriority::Normal,
            true,
        );
    f.clear_packets();
    f.tick(19);
    assert_eq!(calls.load(Relaxed), 1);
    assert_eq!(f.owner.player.inventory.get_selected_slot(), 1);
    assert_eq!(
        f.owner.player.inventory.get_slot(0).get_damage(),
        1,
        "callback damaged a no-longer-equipped hand glider"
    );
    f.assert_stopped();
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g15_stop_toggle_honors_cancellation_and_callback_equipment_repair() {
    for repair in [false, true] {
        let mut f = Fixture::new();
        f.start();
        f.tick(5);
        let player = f.owner.player.clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let called = calls.clone();
        f.server
            .plugin_manager
            .register::<EntityToggleGlideEvent, _>(
                Arc::new(GlideCallback(move |event: &mut EntityToggleGlideEvent| {
                    assert!(!event.is_gliding);
                    if called.fetch_add(1, Relaxed) == 0 {
                        if repair {
                            player
                                .inventory
                                .set_slot(38, ItemStack::new(1, &Item::ELYTRA));
                        } else {
                            event.cancelled = true;
                        }
                    }
                })),
                EventPriority::Normal,
                true,
            );
        remove_chest(&f.owner.player, &f.server, SlotActionType::QuickMove);
        f.tick(1);
        assert_eq!(calls.load(Relaxed), 1, "missing stop-glide event");
        assert!(
            f.owner.player.get_entity().is_fall_flying(),
            "callback cancellation/repair was ignored"
        );
        f.clear_packets();
        if repair {
            f.tick(13);
            assert_eq!(f.owner.player.inventory.get_slot(38).get_damage(), 0);
            f.tick(1);
            f.assert_damage_published_next_tick(38, &EquipmentSlot::CHEST, 1);
            remove_chest(&f.owner.player, &f.server, SlotActionType::QuickMove);
        }
        f.tick(1);
        f.assert_stopped();
        assert_eq!(calls.load(Relaxed), 2);
        f.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g15_levitation_passenger_and_ability_flight_invalidate_glide() {
    for condition in 0..3 {
        let mut f = Fixture::new();
        f.start();
        f.tick(1);
        f.clear_packets();
        match condition {
            0 => f.owner.player.living_entity.add_effect(Effect {
                effect_type: &StatusEffect::LEVITATION,
                duration: 200,
                amplifier: 0,
                ambient: false,
                show_particles: false,
                show_icon: false,
                blend: false,
            }),
            1 => {
                f.observer
                    .player
                    .get_entity()
                    .add_passenger(f.observer.player.clone(), f.owner.player.clone());
            }
            _ => {
                f.owner.player.set_flying(true);
            }
        }
        f.tick(20);
        f.assert_stopped();
        assert_eq!(f.owner.player.inventory.get_slot(38).get_damage(), 0);
        f.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g15_wear_callback_life_reset_rejects_old_continuation() {
    let f = Fixture::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let called = calls.clone();
    f.server
        .plugin_manager
        .register::<PlayerItemDamageEvent, _>(
            Arc::new(DamageCallback(move |event: &mut PlayerItemDamageEvent| {
                called.fetch_add(1, Relaxed);
                event.player.living_entity.reset_state();
            })),
            EventPriority::Normal,
            true,
        );
    f.start();
    f.tick(20);
    assert_eq!(calls.load(Relaxed), 1);
    assert_eq!(f.owner.player.inventory.get_slot(38).get_damage(), 0);
    assert!(!f.owner.player.get_entity().is_fall_flying());
    f.close().await;
}
