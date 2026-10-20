#![expect(clippy::unwrap_used, reason = "Valid bounded gameplay fixtures")]

use std::{sync::Arc, sync::atomic::Ordering::Relaxed};

use pumpkin_data::{entity::EntityType, item::Item, item_stack::ItemStack, sound::Sound};
use pumpkin_util::{Hand, math::vector3::Vector3};

use super::{
    b1019_g08_test_support::Fixture, cat::CatEntity, tamable::TamableAnimal, wolf::WolfEntity,
};
use crate::entity::{EntityBase, player::Player};

#[derive(Clone, Copy)]
enum Entry {
    Interact,
    InteractAt,
}

impl Entry {
    fn call(
        self,
        target: &Arc<dyn EntityBase>,
        player: &Arc<Player>,
        stack: &mut ItemStack,
    ) -> bool {
        match self {
            Self::Interact => target.interact(player, stack),
            Self::InteractAt => target.interact_at(player, stack, Vector3::new(0.0, 0.0, 0.0)),
        }
    }
}

async fn entity_handless_stack(kind: &'static EntityType, entry: Entry) {
    let fixture = Fixture::new();
    let target = fixture.mob(kind);
    let cat = target.cast_any().downcast_ref::<CatEntity>();
    let wolf = target.cast_any().downcast_ref::<WolfEntity>();
    if let Some(cat) = cat {
        cat.set_tame(true, Some(fixture.player.gameprofile.id));
        cat.set_collar_color(14);
    }
    if let Some(wolf) = wolf {
        wolf.tame(fixture.player.gameprofile.id);
        wolf.set_collar_color(14);
    }
    let sword = ItemStack::new(1, &Item::DIAMOND_SWORD);
    let offhand = ItemStack::new(1, &Item::STONE);
    fixture.set_hand(Hand::Right, sword.clone());
    fixture.set_hand(Hand::Left, offhand.clone());
    let milking = kind == &EntityType::COW;
    let item = if milking {
        &Item::BUCKET
    } else {
        &Item::BLUE_DYE
    };
    let mut stack = ItemStack::new(2, item);

    assert!(entry.call(&target, &fixture.player, &mut stack));

    assert_eq!(stack.get_item(), item);
    assert_eq!(stack.item_count, 1);
    if milking {
        assert_eq!(fixture.inventory_count(&Item::MILK_BUCKET), 1);
        assert_eq!(fixture.dropped_count(&Item::MILK_BUCKET), 0);
    }
    if let Some(cat) = cat {
        assert_eq!(cat.get_collar_color(), 11);
    }
    if let Some(wolf) = wolf {
        assert_eq!(wolf.get_collar_color(), 11);
    }
    assert!(fixture.hand(Hand::Right).are_equal(&sword));
    assert!(fixture.hand(Hand::Left).are_equal(&offhand));

    target.get_living_entity().unwrap().set_health(0.0);
    let unchanged = stack.clone();
    assert!(!entry.call(&target, &fixture.player, &mut stack));
    assert!(stack.are_equal(&unchanged), "dead mobs still reject use");
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g08_cow_entity_base_handless_interact_uses_detached_stack() {
    entity_handless_stack(&EntityType::COW, Entry::Interact).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g08_cow_entity_base_handless_interact_at_uses_detached_stack() {
    entity_handless_stack(&EntityType::COW, Entry::InteractAt).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g08_cat_entity_base_handless_interact_uses_detached_stack() {
    entity_handless_stack(&EntityType::CAT, Entry::Interact).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g08_cat_entity_base_handless_interact_at_uses_detached_stack() {
    entity_handless_stack(&EntityType::CAT, Entry::InteractAt).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g08_wolf_entity_base_handless_interact_uses_detached_stack() {
    entity_handless_stack(&EntityType::WOLF, Entry::Interact).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g08_wolf_entity_base_handless_interact_at_uses_detached_stack() {
    entity_handless_stack(&EntityType::WOLF, Entry::InteractAt).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g08_animal_handless_feeding_uses_detached_stacks() {
    for (kind, food) in [
        (&EntityType::COW, &Item::WHEAT),
        (&EntityType::CAT, &Item::COD),
        (&EntityType::WOLF, &Item::COOKED_BEEF),
    ] {
        let fixture = Fixture::new();
        let target = fixture.mob(kind);
        let mob = target.get_mob().unwrap();
        let animal = mob.as_animal().unwrap();
        let sword = ItemStack::new(1, &Item::DIAMOND_SWORD);
        let offhand = ItemStack::new(1, &Item::STONE);
        fixture.set_hand(Hand::Right, sword.clone());
        fixture.set_hand(Hand::Left, offhand.clone());
        let mut stack = ItemStack::new(2, food);

        assert!(animal.animal_interact(&fixture.player, &mut stack, Sound::EntityGenericEat));

        assert_eq!(stack.get_item(), food);
        assert_eq!(stack.item_count, 1);
        assert_eq!(mob.get_mob_entity().love_ticks.load(Relaxed), 600);
        assert!(fixture.hand(Hand::Right).are_equal(&sword));
        assert!(fixture.hand(Hand::Left).are_equal(&offhand));
        fixture.finish().await;
    }
}
