#![expect(
    clippy::unwrap_used,
    reason = "Registered entities and decoded packets use bounded gameplay fixtures"
)]

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering::Relaxed},
};

use pumpkin_data::{entity::EntityType, item::Item, item_stack::ItemStack, sound::Sound};
use pumpkin_util::{GameMode, Hand};

use super::{
    b1020d_g04_test_support::{Fixture, decode_sounds},
    tamable::TamableAnimal,
    wolf::WolfEntity,
};
use crate::{
    entity::{ageable::AgeableMob, mob::Mob},
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::player::player_interact_entity_event::PlayerInteractEntityEvent,
    },
    server::Server,
};

#[derive(Clone, Copy, Debug)]
enum FoodEffect {
    Heal,
    Baby,
    Love,
}

fn prepare(wolf: &WolfEntity, effect: FoodEffect) {
    let living = &wolf.mob_entity.living_entity;
    wolf.set_age(if matches!(effect, FoodEffect::Baby) {
        -24_000
    } else {
        0
    });
    wolf.set_age_locked(false);
    wolf.ageable_data.forced_age.store(0, Relaxed);
    wolf.ageable_data.forced_age_timer.store(0, Relaxed);
    wolf.mob_entity.set_love_ticks(0, None);
    wolf.mob_entity.breeding_cooldown.store(0, Relaxed);
    living.set_max_health(40.0);
    living.set_health(if matches!(effect, FoodEffect::Heal) {
        10.0
    } else {
        40.0
    });
}

fn assert_effect(wolf: &WolfEntity, effect: FoodEffect) {
    assert_eq!(
        wolf.mob_entity.living_entity.health.load(),
        if matches!(effect, FoodEffect::Heal) {
            26.0
        } else {
            40.0
        },
        "feeding healing remains intact: {effect:?}"
    );
    assert_eq!(
        wolf.get_age(),
        if matches!(effect, FoodEffect::Baby) {
            -21_600
        } else {
            0
        },
        "feeding growth remains intact: {effect:?}"
    );
    assert_eq!(
        wolf.ageable_data.forced_age.load(Relaxed),
        if matches!(effect, FoodEffect::Baby) {
            2_400
        } else {
            0
        }
    );
    assert_eq!(
        wolf.mob_entity.love_ticks.load(Relaxed),
        if matches!(effect, FoodEffect::Love) {
            600
        } else {
            0
        },
        "feeding love remains intact: {effect:?}"
    );
}

fn assert_no_eating_sound(fixture: &mut Fixture) {
    let actor_sounds = decode_sounds(&fixture.actor.take_packets());
    assert!(
        actor_sounds.is_empty(),
        "wolf food must not publish an ambient eating sound to the actor: {actor_sounds:?}"
    );
    for observer in &mut fixture.observers {
        let sounds = decode_sounds(&observer.take_packets());
        assert!(
            sounds.is_empty(),
            "wolf food must not publish an ambient eating sound to observers: {sounds:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g04_wolf_food_keeps_effects_without_ambient_eating() {
    let mut fixture = Fixture::new();
    let target = fixture.mob(&EntityType::WOLF);
    let wolf = target.cast_any().downcast_ref::<WolfEntity>().unwrap();
    TamableAnimal::tame(wolf, fixture.player.gameprofile.id);
    for hand in [Hand::Right, Hand::Left] {
        for at in [false, true] {
            for mode in [GameMode::Survival, GameMode::Creative] {
                fixture.player.gamemode.store(mode);
                for effect in [FoodEffect::Heal, FoodEffect::Baby, FoodEffect::Love] {
                    prepare(wolf, effect);
                    let food = ItemStack::new(2, &Item::COOKED_BEEF);
                    fixture.set_hand(hand, food.clone());
                    fixture.drain_packets();
                    fixture.interact(target.as_ref(), hand, at);
                    assert_effect(wolf, effect);
                    let remaining = fixture.hand(hand);
                    assert_eq!(remaining.get_item(), &Item::COOKED_BEEF);
                    assert_eq!(
                        remaining.item_count,
                        if mode == GameMode::Creative { 2 } else { 1 },
                        "feeding payment remains intact: {hand:?}, {at}, {mode:?}, {effect:?}"
                    );
                    assert_eq!(remaining.uid, food.uid);
                    assert_no_eating_sound(&mut fixture);
                }
            }
        }
    }
    fixture.finish().await;
}

struct CancelFeeding(AtomicBool);

impl EventHandler<PlayerInteractEntityEvent> for CancelFeeding {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerInteractEntityEvent,
    ) -> BoxFuture<'a, ()> {
        event.cancelled = self.0.load(Relaxed);
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g04_wolf_cancelled_and_refused_food_keeps_payment_and_effects() {
    let mut fixture = Fixture::new();
    let target = fixture.mob(&EntityType::WOLF);
    let wolf = target.cast_any().downcast_ref::<WolfEntity>().unwrap();
    TamableAnimal::tame(wolf, fixture.player.gameprofile.id);
    let cancellation = Arc::new(CancelFeeding(AtomicBool::new(true)));
    fixture
        .server
        .plugin_manager
        .register::<PlayerInteractEntityEvent, _>(
            cancellation.clone(),
            EventPriority::Normal,
            true,
        );
    for hand in [Hand::Right, Hand::Left] {
        for at in [false, true] {
            for mode in [GameMode::Survival, GameMode::Creative] {
                fixture.player.gamemode.store(mode);
                for effect in [FoodEffect::Heal, FoodEffect::Baby, FoodEffect::Love] {
                    prepare(wolf, effect);
                    let health = wolf.mob_entity.living_entity.health.load();
                    let age = wolf.get_age();
                    let food = ItemStack::new(2, &Item::COOKED_BEEF);
                    fixture.set_hand(hand, food.clone());
                    fixture.drain_packets();
                    fixture.interact(target.as_ref(), hand, at);
                    assert!(fixture.hand(hand).are_equal(&food));
                    assert_eq!(fixture.hand(hand).uid, food.uid);
                    assert_eq!(wolf.mob_entity.living_entity.health.load(), health);
                    assert_eq!(wolf.get_age(), age);
                    assert_eq!(wolf.ageable_data.forced_age.load(Relaxed), 0);
                    assert_eq!(wolf.mob_entity.love_ticks.load(Relaxed), 0);
                    assert_no_eating_sound(&mut fixture);
                }
            }
        }
    }
    cancellation.0.store(false, Relaxed);
    for hand in [Hand::Right, Hand::Left] {
        for at in [false, true] {
            for mode in [GameMode::Survival, GameMode::Creative] {
                fixture.player.gamemode.store(mode);
                for refusal in 0..3 {
                    prepare(wolf, FoodEffect::Love);
                    let item = match refusal {
                        0 => &Item::STICK,
                        1 => {
                            wolf.set_age(-24_000);
                            wolf.set_age_locked(true);
                            &Item::COOKED_BEEF
                        }
                        _ => {
                            wolf.mob_entity.set_love_ticks(600, None);
                            &Item::COOKED_BEEF
                        }
                    };
                    let age = wolf.get_age();
                    let love = wolf.mob_entity.love_ticks.load(Relaxed);
                    let input = ItemStack::new(2, item);
                    fixture.set_hand(hand, input.clone());
                    fixture.drain_packets();
                    fixture.interact(target.as_ref(), hand, at);
                    assert!(fixture.hand(hand).are_equal(&input));
                    assert_eq!(wolf.mob_entity.living_entity.health.load(), 40.0);
                    assert_eq!(wolf.get_age(), age);
                    assert_eq!(wolf.ageable_data.forced_age.load(Relaxed), 0);
                    assert_eq!(wolf.mob_entity.love_ticks.load(Relaxed), love);
                    assert_no_eating_sound(&mut fixture);
                }
            }
        }
    }
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g04_autonomous_ambient_sound_stays_audible() {
    let mut fixture = Fixture::new();
    let target = fixture.mob(&EntityType::BAT);
    let bat = target
        .cast_any()
        .downcast_ref::<crate::entity::mob::bat::BatEntity>()
        .unwrap();
    // Wolf has no autonomous ambient tick in this base. Exercise the existing
    // Bat mob tick rather than synthesizing an ambient sound publication.
    bat.set_roosting(false);
    fixture.drain_packets();
    for _ in 0..81 {
        bat.mob_tick(target.as_ref());
    }
    for observer in &mut fixture.observers {
        let sounds = decode_sounds(&observer.take_packets());
        assert_eq!(
            sounds.len(),
            1,
            "a real autonomous ambient path stays audible"
        );
        assert_eq!(sounds[0].sound_id, Sound::EntityBatAmbient as u16);
        assert_eq!(
            sounds[0].category,
            pumpkin_data::sound::SoundCategory::Ambient as i32
        );
    }
    fixture.finish().await;
}
