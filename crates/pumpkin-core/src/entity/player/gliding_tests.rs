use crate::{
    entity::{EntityBase, player::Player},
    net::{
        ClientPlatform, bedrock::combat_test_support::TestBedrockPlayer,
        java::combat_test_support::TestPlayer,
    },
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::entity::entity_toggle_glide::EntityToggleGlideEvent,
    },
    server::{Server, combat_test_support},
    world::World,
};
use pumpkin_data::{
    effect::StatusEffect, item::Item, item_stack::ItemStack, packet::CURRENT_MC_VERSION,
    potion::Effect,
};
use pumpkin_protocol::{
    VarInt,
    bedrock::server::{
        player_action::{PlayerActionType, SPlayerAction},
        player_auth_input::{InputData, SPlayerAuthInput},
    },
    java::server::play::{Action, SPlayerCommand},
};
use pumpkin_util::{
    GameMode,
    math::{position::BlockPos, vector2::Vector2, vector3::Vector3},
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering::Relaxed},
};

mod support;
use support::*;
mod callbacks;

async fn all_real_inputs(route: Route) {
    Fixture::check(async |f| {
        f.request(route, true);
        assert!(
            f.player(route).get_entity().is_fall_flying(),
            "{route:?} did not admit real glide request"
        );
        assert_eq!(f.toggles.load(Relaxed), 1, "one successful toggle");
        f.tick(19);
        assert_eq!(
            f.player(route).inventory.get_slot(38).get_damage(),
            0,
            "wear before tick 20"
        );
        f.assert_metadata(route, true);
        f.request(route, true);
        assert_eq!(f.toggles.load(Relaxed), 1, "repeated start fired toggle");
        f.tick(1);
        assert_eq!(
            f.player(route).inventory.get_slot(38).get_damage(),
            1,
            "tick 20 wear, repeated start reset cadence"
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g08_bedrock_glide_all_real_inputs_auth() {
    all_real_inputs(Route::Auth).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g08_bedrock_glide_all_real_inputs_legacy() {
    all_real_inputs(Route::Legacy).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g08_bedrock_glide_all_real_inputs_embedded() {
    all_real_inputs(Route::Embedded).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g08_bedrock_glide_all_real_inputs_duplicate() {
    all_real_inputs(Route::Both).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g08_bedrock_glide_all_real_inputs_java_control() {
    all_real_inputs(Route::Java).await;
}

#[derive(Clone, Copy, Debug)]
enum Denial {
    Missing,
    Broken,
    WrongSlot,
    Ground,
    Passenger,
    Levitation,
    Water,
    Lava,
    Ability,
    Dead,
    Unloaded,
    Detached,
}

fn deny(f: &Fixture, denial: Denial) {
    let player = &f.owner.player;
    let entity = player.get_entity();
    match denial {
        Denial::Missing => player.inventory.set_slot(38, ItemStack::EMPTY.clone()),
        Denial::Broken => {
            let mut stack = ItemStack::new(1, &Item::ELYTRA);
            stack.set_damage(stack.get_max_damage().unwrap() - 1);
            player.inventory.set_slot(38, stack);
        }
        Denial::WrongSlot => {
            player.inventory.set_slot(38, ItemStack::EMPTY.clone());
            player
                .inventory
                .set_slot(0, ItemStack::new(1, &Item::ELYTRA));
        }
        Denial::Ground => entity.on_ground.store(true, Relaxed),
        Denial::Passenger => f
            .java
            .player
            .get_entity()
            .add_passenger(f.java.player.clone(), player.clone()),
        Denial::Levitation => player.living_entity.add_effect(Effect {
            effect_type: &StatusEffect::LEVITATION,
            duration: 200,
            amplifier: 0,
            ambient: false,
            show_particles: false,
            show_icon: false,
            blend: false,
        }),
        Denial::Water => entity.touching_water.store(true, Relaxed),
        Denial::Lava => entity.touching_lava.store(true, Relaxed),
        Denial::Ability => {
            player.set_gamemode(GameMode::Creative);
            player.set_flying(true);
        }
        Denial::Dead => {
            player.living_entity.dead.store(true, Relaxed);
            player.living_entity.set_health(0.0);
        }
        Denial::Unloaded => player.set_client_loaded(false),
        Denial::Detached => f.world.players.store(Arc::new(vec![
            f.java.player.clone(),
            f.observer.player.clone(),
        ])),
    }
}

async fn eligibility_and_stop(route: Route) {
    for denial in [
        Denial::Missing,
        Denial::Broken,
        Denial::WrongSlot,
        Denial::Ground,
        Denial::Passenger,
        Denial::Levitation,
        Denial::Water,
        Denial::Lava,
        Denial::Ability,
        Denial::Dead,
        Denial::Unloaded,
        Denial::Detached,
    ] {
        Fixture::check(async |f| {
            deny(f, denial);
            let before = f.owner.player.inventory.get_slot(38).get_damage();
            f.request(route, true);
            assert!(
                !f.owner.player.get_entity().is_fall_flying(),
                "admitted {denial:?} through {route:?}"
            );
            assert_eq!(f.toggles.load(Relaxed), 0, "ineligible toggle {denial:?}");
            f.tick(20);
            assert_eq!(
                f.owner.player.inventory.get_slot(38).get_damage(),
                before,
                "invalid wear {denial:?}"
            );
            if matches!(denial, Denial::Ability) {
                assert!(f.owner.player.is_flying());
            }
        })
        .await;
    }
    Fixture::check(async |f| {
        f.request(route, true);
        assert!(
            f.owner.player.get_entity().is_fall_flying(),
            "stop test did not start through {route:?}"
        );
        f.tick(9);
        f.clear_packets();
        f.request(route, false);
        assert!(
            !f.owner.player.get_entity().is_fall_flying(),
            "stop ignored {route:?}"
        );
        assert_eq!(f.toggles.load(Relaxed), 2);
        f.tick(20);
        f.assert_metadata(route, false);
        assert_eq!(
            f.owner.player.inventory.get_slot(38).get_damage(),
            0,
            "stopped glide kept wearing"
        );
        // A stop request must never end Creative ability flight.
        f.owner.player.set_gamemode(GameMode::Creative);
        f.owner.player.set_flying(true);
        f.request(route, false);
        assert!(f.owner.player.is_flying());
        assert_eq!(f.toggles.load(Relaxed), 2);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g08_bedrock_glide_eligibility_and_stop_auth() {
    eligibility_and_stop(Route::Auth).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g08_bedrock_glide_eligibility_and_stop_legacy() {
    eligibility_and_stop(Route::Legacy).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g08_bedrock_glide_eligibility_and_stop_embedded() {
    eligibility_and_stop(Route::Embedded).await;
}
