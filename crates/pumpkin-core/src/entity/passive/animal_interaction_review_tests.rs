//! Regressions use the real Java entity interaction and synchronous plugin callbacks.
use super::*;
use pumpkin_data::{
    data_component_impl::{CustomNameImpl, UseCooldownImpl, UseRemainderImpl},
    sound::{Sound, SoundCategory},
    statistic::StatisticCategory,
};
use pumpkin_util::{Hand, text::TextComponent};

#[path = "animal_interaction_review_support.rs"]
mod support;
use support::{Effects, SoundPacket, Stage, take_sounds, wolf};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review116_wolf_stew_returns_a_bowl_in_the_used_hand() {
    for offhand in [false, true] {
        let fixture = Fixture::new();
        let wolf = wolf(&fixture);
        let inventory = fixture.player.player.inventory();
        let source = if offhand { 40 } else { 0 };
        let other = if offhand { 0 } else { 40 };
        let untouched = ItemStack::new(1, &Item::DIAMOND);
        inventory.set_stack(other, untouched.clone());
        inventory.set_stack(source, ItemStack::new(1, &Item::RABBIT_STEW));
        fixture.interact(wolf.as_ref(), offhand);
        assert_eq!(inventory.get_stack(source).get_item(), &Item::BOWL);
        assert_eq!(inventory.get_stack(source).item_count, 1);
        assert!(inventory.get_stack(other).are_equal(&untouched));
        assert_eq!(fixture.inventory_count(&Item::BOWL), 1);
        assert_eq!(fixture.dropped_count(&Item::BOWL), 0);
        assert_eq!(wolf.mob_entity.living_entity.health.load(), 30.0);

        fixture.player.player.gamemode.store(GameMode::Creative);
        wolf.mob_entity.living_entity.set_health(10.0);
        inventory.set_stack(source, ItemStack::new(1, &Item::RABBIT_STEW));
        fixture.interact(wolf.as_ref(), offhand);
        assert_eq!(inventory.get_stack(source).get_item(), &Item::RABBIT_STEW);
        assert_eq!(inventory.get_stack(source).item_count, 1);
        assert_eq!(fixture.inventory_count(&Item::BOWL), 0);
        assert_eq!(wolf.mob_entity.living_entity.health.load(), 30.0);
        fixture.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review116_wolf_stacked_food_preserves_custom_remainders_and_overflow() {
    for full in [false, true] {
        let fixture = Fixture::new();
        let wolf = wolf(&fixture);
        let inventory = fixture.player.player.inventory();
        if full {
            for slot in 0..36 {
                inventory.set_stack(slot, ItemStack::new(64, &Item::STONE));
            }
        }
        let mut remainder = ItemStack::new(2, &Item::BOWL);
        remainder.set_data_component(CustomNameImpl {
            name: TextComponent::text("Wolf feeding remainder"),
        });
        let mut food = ItemStack::new(2, &Item::COOKED_BEEF);
        food.set_data_component(UseRemainderImpl {
            remainder: None,
            template: Some(Box::new(remainder.clone())),
        });
        food.set_data_component(UseCooldownImpl::new(1.0, Some("test:wolf_food".into())));
        inventory.set_stack(40, food);
        fixture.interact(wolf.as_ref(), true);
        assert!(!fixture.player.player.is_on_cooldown("test:wolf_food"));
        assert_eq!(inventory.off_hand_item().item_count, 1);
        assert_eq!(inventory.off_hand_item().get_item(), &Item::COOKED_BEEF);
        assert_eq!(
            fixture.inventory_count(&Item::BOWL),
            if full { 0 } else { 2 }
        );
        assert_eq!(fixture.dropped_count(&Item::BOWL), if full { 2 } else { 0 });
        let actual = if full {
            fixture
                .world
                .entities
                .load()
                .iter()
                .find_map(|entity| {
                    let item = entity.cast_any().downcast_ref::<ItemEntity>()?;
                    let stack = item.get_item_stack().lock().unwrap().clone();
                    (stack.get_item() == &Item::BOWL).then_some(stack)
                })
                .unwrap()
        } else {
            (0..inventory.size())
                .map(|slot| inventory.get_stack(slot))
                .find(|stack| stack.get_item() == &Item::BOWL)
                .unwrap()
        };
        assert!(actual.are_equal(&remainder));
        fixture.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review116_wolf_remainder_precedes_heal_and_entity_interact_callbacks() {
    for hand in [Hand::Right, Hand::Left] {
        let mut fixture = Fixture::new();
        let wolf = wolf(&fixture);
        let effects = Effects::watch(&mut fixture, wolf.clone(), hand);
        fixture
            .player
            .player
            .inventory()
            .set_stack_in_hand(hand, ItemStack::new(1, &Item::RABBIT_STEW));
        fixture.interact(wolf.as_ref(), hand == Hand::Left);
        {
            let observations = effects.observations.lock().unwrap();
            assert_eq!(observations.len(), 2);
            assert_eq!(observations[0].stage, Stage::Heal);
            assert_eq!(observations[0].held.get_item(), &Item::BOWL);
            assert_eq!(observations[0].health, 10.0);
            assert_eq!(observations[1].stage, Stage::Interact);
            assert_eq!(observations[1].held.get_item(), &Item::BOWL);
            assert_eq!(observations[1].health, 30.0);
            assert_eq!(observations[1].position, Some(wolf.get_entity().pos.load()));
        };
        fixture.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review116_mob_callbacks_keep_direct_hand_replacements() {
    for stage in [Stage::Heal, Stage::Interact] {
        for replacement in [ItemStack::new(1, &Item::DIAMOND), ItemStack::EMPTY.clone()] {
            let mut fixture = Fixture::new();
            let wolf = wolf(&fixture);
            let effects = Effects::watch(&mut fixture, wolf.clone(), Hand::Left);
            *effects.replacement.lock().unwrap() = Some((stage, replacement.clone()));
            fixture
                .player
                .player
                .inventory()
                .set_stack(40, ItemStack::new(1, &Item::RABBIT_STEW));
            fixture.interact(wolf.as_ref(), true);
            assert!(
                effects
                    .observations
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|record| record.stage == stage)
            );
            assert!(
                fixture
                    .player
                    .player
                    .inventory()
                    .off_hand_item()
                    .are_equal(&replacement)
            );
            assert_eq!(wolf.mob_entity.living_entity.health.load(), 30.0);
            fixture.finish().await;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review116_tamed_wolf_does_not_eat_bones_but_wild_wolf_still_uses_them() {
    let fixture = Fixture::new();
    let wolf = wolf(&fixture);
    let inventory = fixture.player.player.inventory();
    inventory.set_stack(40, ItemStack::new(1, &Item::BONE));
    fixture.interact(wolf.as_ref(), true);
    assert_eq!(wolf.mob_entity.living_entity.health.load(), 10.0);
    assert_eq!(inventory.off_hand_item().get_item(), &Item::BONE);
    assert_eq!(inventory.off_hand_item().item_count, 1);
    assert!(wolf.is_ordered_to_sit());
    assert!(!wolf.mob_entity.is_in_love());

    wolf.set_tame(false);
    wolf.set_ordered_to_sit(false);
    fixture.interact(wolf.as_ref(), true);
    assert!(inventory.off_hand_item().is_empty());
    assert_eq!(wolf.mob_entity.living_entity.health.load(), 10.0);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review116_milking_sound_and_event_use_vanilla_recipients_and_order() {
    for hand in [Hand::Right, Hand::Left] {
        let mut fixture = Fixture::new();
        let cow = CowEntity::new(fixture.entity(&EntityType::COW));
        assert!(fixture.world.spawn_entity(cow.clone()));
        let effects = Effects::watch(&mut fixture, cow.clone(), hand);
        fixture
            .player
            .player
            .inventory()
            .set_stack_in_hand(hand, ItemStack::new(1, &Item::BUCKET));
        fixture.interact(cow.as_ref(), hand == Hand::Left);
        let expected = SoundPacket {
            id: Sound::EntityCowMilk as i32,
            category: SoundCategory::Players as i32,
            position: Vector3::new(68, 512, 68),
            volume: 1.0,
            pitch: 1.0,
        };
        assert!(
            take_sounds(&mut fixture.player).is_empty(),
            "the initiating client predicts the milk sound"
        );
        assert_eq!(effects.sounds(), vec![expected.clone()]);
        {
            let observations = effects.observations.lock().unwrap();
            assert_eq!(observations.len(), 1);
            assert_eq!(observations[0].stage, Stage::Interact);
            assert_eq!(observations[0].held.get_item(), &Item::MILK_BUCKET);
            assert_eq!(observations[0].position, Some(cow.get_entity().pos.load()));
            assert_eq!(observations[0].sounds, vec![expected]);
        };
        fixture.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review116_wolf_feeding_is_silent_for_actor_and_observer() {
    for (age, health) in [(0, 10.0), (0, 40.0), (-24000, 40.0)] {
        let mut fixture = Fixture::new();
        let wolf = wolf(&fixture);
        wolf.set_age(age);
        wolf.mob_entity.living_entity.set_health(health);
        let effects = Effects::watch(&mut fixture, wolf.clone(), Hand::Right);
        fixture
            .player
            .player
            .inventory()
            .set_stack(0, ItemStack::new(1, &Item::COOKED_BEEF));
        fixture.interact(wolf.as_ref(), false);
        assert!(fixture.player.player.inventory().held_item().is_empty());
        assert!(
            take_sounds(&mut fixture.player).is_empty(),
            "no wolf eating sound in vanilla"
        );
        assert!(effects.sounds().is_empty());
        assert_eq!(
            effects
                .observations
                .lock()
                .unwrap()
                .iter()
                .filter(|record| record.stage == Stage::Interact)
                .count(),
            1
        );
        fixture.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review116_mob_feeding_and_milking_do_not_increment_used_statistics() {
    let fixture = Fixture::new();
    let cow = CowEntity::new(fixture.entity(&EntityType::COW));
    assert!(fixture.world.spawn_entity(cow.clone()));
    let wolf = wolf(&fixture);
    for (target, item) in [
        (cow.as_ref() as &dyn EntityBase, &Item::BUCKET),
        (cow.as_ref() as &dyn EntityBase, &Item::WHEAT),
        (wolf.as_ref() as &dyn EntityBase, &Item::COOKED_BEEF),
    ] {
        fixture
            .player
            .player
            .inventory()
            .set_stack(40, ItemStack::new(1, item));
        fixture.interact(target, true);
        assert_eq!(
            fixture
                .player
                .player
                .get_stat(StatisticCategory::Used, i32::from(item.id)),
            0,
            "generic interaction must not award Used for {}",
            item.registry_key
        );
    }
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review116_refused_baby_milking_emits_no_entity_interaction_or_sound() {
    let mut fixture = Fixture::new();
    let cow = CowEntity::new(fixture.entity(&EntityType::COW));
    cow.set_baby(true);
    assert!(fixture.world.spawn_entity(cow.clone()));
    let effects = Effects::watch(&mut fixture, cow.clone(), Hand::Right);
    fixture
        .player
        .player
        .inventory()
        .set_stack(0, ItemStack::new(1, &Item::BUCKET));
    fixture.interact(cow.as_ref(), false);
    assert_eq!(
        fixture.player.player.inventory().held_item().get_item(),
        &Item::BUCKET
    );
    assert!(effects.observations.lock().unwrap().is_empty());
    assert!(take_sounds(&mut fixture.player).is_empty());
    assert!(effects.sounds().is_empty());
    fixture.finish().await;
}
