//! Java interaction regressions for the fluids/animals issue lane.
use crate::{
    entity::{
        Entity, EntityBase,
        ageable::AgeableMob,
        item::ItemEntity,
        mob::Mob,
        passive::{cat::CatEntity, cow::CowEntity, tamable::TamableAnimal, wolf::WolfEntity},
    },
    net::java::combat_test_support::TestPlayer,
    server::{Server, combat_test_support},
    world::World,
};
use pumpkin_data::{
    data_component::DataComponent, data_component_impl::FoodImpl, entity::EntityType, item::Item,
    item_stack::ItemStack,
};
use pumpkin_inventory::Inventory;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::{codec::var_int::VarInt, java::server::play::SInteract};
use pumpkin_util::{
    GameMode,
    math::{vector2::Vector2, vector3::Vector3},
    version::JavaMinecraftVersion,
};
use std::sync::{Arc, atomic::Ordering::Relaxed};

#[path = "animal_interaction_review_tests.rs"]
mod review;

struct Fixture {
    server: Arc<Server>,
    world: Arc<World>,
    player: TestPlayer,
    _directory: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        let world = combat_test_support::world(&server, directory.path());
        combat_test_support::publish_empty_chunk(&world, Vector2::new(0, 0));
        let player = TestPlayer::new(&world);
        player
            .player
            .get_entity()
            .set_pos(Vector3::new(8.5, 64.0, 8.5));
        Self {
            server,
            world,
            player,
            _directory: directory,
        }
    }

    fn entity(&self, kind: &'static EntityType) -> Entity {
        Entity::new(self.world.clone(), Vector3::new(9.5, 64.0, 8.5), kind)
    }

    fn kitten(&self, age: i32) -> Arc<CatEntity> {
        let entity = self.entity(&EntityType::CAT);
        entity.set_age(age);
        let cat = CatEntity::new(entity);
        cat.set_tame(true, Some(self.player.player.gameprofile.id));
        cat.set_sitting(false);
        let living = &cat.mob_entity.living_entity;
        living.set_health(living.get_max_health());
        assert!(self.world.spawn_entity(cat.clone()));
        cat
    }

    fn interact(&self, target: &dyn EntityBase, offhand: bool) {
        self.player.client().handle_interact(
            &self.player.player,
            &SInteract {
                entity_id: VarInt(target.get_entity().entity_id),
                r#type: VarInt(2),
                target_position: Some(Vector3::new(0.0, 0.5, 0.0)),
                hand: Some(VarInt(i32::from(offhand))),
                sneaking: false,
            },
            &self.server,
        );
    }

    fn inventory_count(&self, item: &Item) -> usize {
        let inventory = self.player.player.inventory();
        (0..inventory.size())
            .map(|slot| inventory.get_stack(slot))
            .filter(|stack| stack.get_item() == item)
            .map(|stack| usize::from(stack.item_count))
            .sum()
    }

    fn dropped_count(&self, item: &Item) -> usize {
        self.world
            .entities
            .load()
            .iter()
            .filter_map(|entity| {
                let dropped = entity.cast_any().downcast_ref::<ItemEntity>()?;
                let stack = dropped.get_item_stack().lock().unwrap();
                (stack.get_item() == item).then_some(usize::from(stack.item_count))
            })
            .sum()
    }

    async fn finish(self) {
        self.world.level.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn animal_interaction_milking_replaces_only_the_used_hand_and_refuses_babies() {
    let fixture = Fixture::new();
    let cow = CowEntity::new(fixture.entity(&EntityType::COW));
    cow.set_baby(true);
    assert!(fixture.world.spawn_entity(cow.clone()));
    let inventory = fixture.player.player.inventory();
    inventory.set_stack(0, ItemStack::new(1, &Item::BUCKET));
    fixture.interact(cow.as_ref(), false);
    assert_eq!(inventory.held_item().get_item(), &Item::BUCKET);
    assert_eq!(inventory.held_item().item_count, 1);
    assert_eq!(fixture.inventory_count(&Item::MILK_BUCKET), 0);

    cow.set_baby(false);
    for offhand in [false, true] {
        let source = if offhand { 40 } else { 0 };
        let other = if offhand { 0 } else { 40 };
        let sword = ItemStack::new(1, &Item::WOODEN_SWORD);
        inventory.set_stack(source, ItemStack::new(1, &Item::BUCKET));
        inventory.set_stack(other, sword.clone());
        fixture.interact(cow.as_ref(), offhand);
        let result = inventory.get_stack(source);
        assert_eq!(
            (result.get_item(), result.item_count),
            (&Item::MILK_BUCKET, 1)
        );
        assert!(inventory.get_stack(other).are_equal(&sword));
        assert_eq!(fixture.inventory_count(&Item::MILK_BUCKET), 1);
        assert_eq!(fixture.dropped_count(&Item::MILK_BUCKET), 0);
    }
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn animal_interaction_milking_handles_stacks_full_inventory_and_creative_duplicates() {
    let fixture = Fixture::new();
    let cow = CowEntity::new(fixture.entity(&EntityType::COW));
    assert!(fixture.world.spawn_entity(cow.clone()));
    let inventory = fixture.player.player.inventory();
    inventory.set_stack(40, ItemStack::new(2, &Item::BUCKET));
    fixture.interact(cow.as_ref(), true);
    assert_eq!(inventory.off_hand_item().get_item(), &Item::BUCKET);
    assert_eq!(inventory.off_hand_item().item_count, 1);
    assert_eq!(fixture.inventory_count(&Item::MILK_BUCKET), 1);
    assert_eq!(fixture.dropped_count(&Item::MILK_BUCKET), 0);

    for slot in 0..36 {
        inventory.set_stack(slot, ItemStack::new(64, &Item::STONE));
    }
    inventory.set_stack(40, ItemStack::new(2, &Item::BUCKET));
    fixture.interact(cow.as_ref(), true);
    assert_eq!(inventory.off_hand_item().item_count, 1);
    assert_eq!(fixture.inventory_count(&Item::MILK_BUCKET), 0);
    assert_eq!(fixture.dropped_count(&Item::MILK_BUCKET), 1);

    fixture.player.player.gamemode.store(GameMode::Creative);
    inventory.set_stack(0, ItemStack::EMPTY.clone());
    for _ in 0..2 {
        fixture.interact(cow.as_ref(), true);
        assert_eq!(inventory.off_hand_item().get_item(), &Item::BUCKET);
        assert_eq!(inventory.off_hand_item().item_count, 1);
        assert_eq!(fixture.inventory_count(&Item::MILK_BUCKET), 1);
        assert_eq!(fixture.dropped_count(&Item::MILK_BUCKET), 1);
    }
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn animal_interaction_cat_parent_feeding_precedes_sitting() {
    let fixture = Fixture::new();
    let cat = CatEntity::new(fixture.entity(&EntityType::CAT));
    cat.set_tame(true, Some(fixture.player.player.gameprofile.id));
    cat.set_sitting(false);
    assert!(fixture.world.spawn_entity(cat.clone()));
    let living = &cat.mob_entity.living_entity;
    let max = living.get_max_health();
    let inventory = fixture.player.player.inventory();
    living.set_health(max - 2.0);
    inventory.set_stack(0, ItemStack::new(2, &Item::COD));
    fixture.interact(cat.as_ref(), false);
    assert_eq!(living.health.load(), max);
    assert_eq!(inventory.held_item().item_count, 1);
    assert!(!cat.mob_entity.is_in_love());

    for offhand in [false, true] {
        cat.mob_entity.reset_love_ticks();
        cat.set_sitting(false);
        let slot = if offhand { 40 } else { 0 };
        inventory.set_stack(slot, ItemStack::new(2, &Item::SALMON));
        fixture.interact(cat.as_ref(), offhand);
        assert_eq!(cat.mob_entity.love_ticks.load(Relaxed), 600);
        assert_eq!(inventory.get_stack(slot).item_count, 1);
        assert!(!cat.is_sitting());
    }

    cat.mob_entity.reset_love_ticks();
    cat.get_entity().set_age(-24000);
    inventory.set_stack(0, ItemStack::new(1, &Item::COD));
    fixture.interact(cat.as_ref(), false);
    assert_eq!(cat.get_entity().age.load(Relaxed), -21600);
    assert!(inventory.held_item().is_empty());
    assert!(!cat.mob_entity.is_in_love());
    assert!(!cat.is_sitting());

    inventory.set_stack(0, ItemStack::new(1, &Item::STICK));
    fixture.interact(cat.as_ref(), false);
    assert!(
        cat.is_sitting(),
        "unhandled owner interaction still toggles sitting"
    );
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn animal_interaction_cat_healing_components_and_nonowner_parent_admission() {
    let fixture = Fixture::new();
    let cat = CatEntity::new(fixture.entity(&EntityType::CAT));
    cat.set_tame(true, Some(fixture.player.player.gameprofile.id));
    cat.set_sitting(true);
    assert!(fixture.world.spawn_entity(cat.clone()));
    let living = &cat.mob_entity.living_entity;
    let inventory = fixture.player.player.inventory();
    let mut custom = ItemStack::new(1, &Item::COD);
    custom.set_data_component(FoodImpl {
        nutrition: 3,
        saturation: 0.0,
        can_always_eat: false,
    });
    let mut absent = ItemStack::new(1, &Item::COD);
    absent.remove_data_component(DataComponent::Food);
    for (food, expected) in [(custom, 4.0), (absent, 2.0)] {
        living.set_health(1.0);
        inventory.set_stack(40, food);
        fixture.interact(cat.as_ref(), true);
        assert_eq!(living.health.load(), expected);
        assert!(inventory.off_hand_item().is_empty());
        assert!(!cat.mob_entity.is_in_love());
        assert!(cat.is_sitting());
    }

    cat.set_tame(true, Some(uuid::Uuid::new_v4()));
    cat.set_sitting(false);
    living.set_health(living.get_max_health());
    inventory.set_stack(40, ItemStack::new(2, &Item::COD));
    fixture.interact(cat.as_ref(), true);
    assert_eq!(cat.mob_entity.love_ticks.load(Relaxed), 600);
    assert_eq!(inventory.off_hand_item().item_count, 1);
    assert!(
        !cat.is_sitting(),
        "parent food consumption is not owner-only"
    );
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn animal_interaction_wolf_healing_uses_effective_food_before_consumption() {
    let fixture = Fixture::new();
    let wolf = WolfEntity::new(fixture.entity(&EntityType::WOLF));
    TamableAnimal::tame(wolf.as_ref(), fixture.player.player.gameprofile.id);
    assert!(fixture.world.spawn_entity(wolf.clone()));
    let living = &wolf.mob_entity.living_entity;
    living.set_max_health(40.0);
    let inventory = fixture.player.player.inventory();
    let steak = ItemStack::new(1, &Item::COOKED_BEEF);
    assert_eq!(steak.get_data_component::<FoodImpl>().unwrap().nutrition, 8);
    let mut custom = steak.clone();
    custom.set_data_component(FoodImpl {
        nutrition: 3,
        saturation: 0.0,
        can_always_eat: false,
    });
    let mut absent = steak.clone();
    absent.remove_data_component(DataComponent::Food);
    for (food, before, expected) in [
        (steak.clone(), 10.0, 26.0),
        (steak.clone(), 39.0, 40.0),
        (custom, 10.0, 16.0),
        (absent, 10.0, 12.0),
    ] {
        living.set_health(before);
        inventory.set_stack(40, food);
        fixture.interact(wolf.as_ref(), true);
        assert_eq!(living.health.load(), expected);
        assert!(inventory.off_hand_item().is_empty());
    }
    fixture.player.player.gamemode.store(GameMode::Creative);
    living.set_health(10.0);
    inventory.set_stack(40, steak);
    fixture.interact(wolf.as_ref(), true);
    assert_eq!(living.health.load(), 26.0);
    assert_eq!(inventory.off_hand_item().item_count, 1);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn animal_interaction_baby_growth_records_forced_age_and_whole_seconds() {
    let fixture = Fixture::new();
    let cow = CowEntity::new(fixture.entity(&EntityType::COW));
    cow.set_baby(true);
    assert!(fixture.world.spawn_entity(cow.clone()));
    let inventory = fixture.player.player.inventory();
    inventory.set_stack(0, ItemStack::new(2, &Item::WHEAT));
    fixture.interact(cow.as_ref(), false);
    assert_eq!(cow.get_age(), -21600);
    assert_eq!(inventory.held_item().item_count, 1);
    assert_eq!(cow.ageable_data.forced_age.load(Relaxed), 2400);
    assert_eq!(cow.ageable_data.forced_age_timer.load(Relaxed), 40);
    let mut saved = NbtCompound::new();
    cow.write_custom_nbt(&mut saved);
    assert_eq!(saved.get_int("Age"), Some(-21600));
    assert_eq!(saved.get_int("ForcedAge"), Some(2400));
    let restored = CowEntity::new(fixture.entity(&EntityType::COW));
    restored.read_custom_nbt(&saved);
    assert_eq!(restored.get_age(), -21600);
    assert_eq!(restored.ageable_data.forced_age.load(Relaxed), 2400);

    for (age, expected_age, expected_forced) in [
        (-399, -379, 20),
        (-200, -180, 20),
        (-199, -199, 0),
        (-19, -19, 0),
        (-1, -1, 0),
    ] {
        cow.set_age(age);
        cow.ageable_data.forced_age.store(0, Relaxed);
        inventory.set_stack(0, ItemStack::new(1, &Item::WHEAT));
        fixture.interact(cow.as_ref(), false);
        assert_eq!(cow.get_age(), expected_age, "whole-second feeding at {age}");
        assert_eq!(cow.ageable_data.forced_age.load(Relaxed), expected_forced);
        assert!(inventory.held_item().is_empty());
        assert!(cow.is_baby());
    }
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn animal_interaction_repeated_feeding_keeps_baby_metadata_and_respects_admission() {
    let fixture = Fixture::new();
    let cow = CowEntity::new(fixture.entity(&EntityType::COW));
    cow.set_baby(true);
    assert!(fixture.world.spawn_entity(cow.clone()));
    let metadata = || {
        cow.get_entity()
            .synched_data
            .get_non_default_values_for_version(&JavaMinecraftVersion::V_26_3)
            .unwrap()
    };
    let baby_metadata = metadata();
    let inventory = fixture.player.player.inventory();
    fixture.player.player.gamemode.store(GameMode::Creative);
    inventory.set_stack(0, ItemStack::new(1, &Item::WHEAT));
    for _ in 0..128 {
        fixture.interact(cow.as_ref(), false);
    }
    assert!(
        cow.is_baby(),
        "without ticks, vanilla rounding does not feed a calf to adulthood"
    );
    assert_eq!(metadata(), baby_metadata);
    assert_eq!(inventory.held_item().item_count, 1);
    assert!(cow.ageable_data.forced_age.load(Relaxed) > 0);

    fixture.player.player.gamemode.store(GameMode::Survival);
    cow.set_baby(true);
    cow.set_age_locked(true);
    let forced = cow.ageable_data.forced_age.load(Relaxed);
    fixture.interact(cow.as_ref(), false);
    assert_eq!(cow.get_age(), -24000);
    assert_eq!(cow.ageable_data.forced_age.load(Relaxed), forced);
    assert_eq!(inventory.held_item().item_count, 1);
    assert!(!cow.mob_entity.is_in_love());

    // The existing AgeableMob setter owns the metadata boundary; global ticking
    // is intentionally not simulated by this interaction regression.
    cow.set_age_locked(false);
    cow.set_age(-1);
    cow.ageable_ai_step();
    assert_eq!(cow.get_age(), 0);
    assert!(!cow.is_baby());
    assert_ne!(metadata(), baby_metadata);
    cow.mob_entity.breeding_cooldown.store(10, Relaxed);
    fixture.interact(cow.as_ref(), false);
    assert!(!cow.mob_entity.is_in_love());
    assert_eq!(inventory.held_item().item_count, 1);
    cow.mob_entity.breeding_cooldown.store(0, Relaxed);
    fixture.interact(cow.as_ref(), false);
    assert_eq!(cow.mob_entity.love_ticks.load(Relaxed), 600);
    assert!(inventory.held_item().is_empty());
    fixture.finish().await;
}

fn kitten_metadata(cat: &CatEntity) -> Box<[u8]> {
    cat.get_entity()
        .synched_data
        .get_non_default_values_for_version(&JavaMinecraftVersion::V_26_3)
        .unwrap()
}

fn assert_kitten_feeding_boundary(fixture: &Fixture, age: i32, offhand: bool) {
    // Set the age before spawning so Cat.mob_init_data_tracker publishes a baby.
    let cat = fixture.kitten(age);
    let baby_metadata = kitten_metadata(&cat);
    let inventory = fixture.player.player.inventory();
    let (slot, other_slot) = if offhand { (40, 0) } else { (0, 40) };
    let other_hand = ItemStack::new(1, &Item::STICK);
    inventory.set_stack(slot, ItemStack::new(2, &Item::COD));
    inventory.set_stack(other_slot, other_hand.clone());

    fixture.interact(cat.as_ref(), offhand);

    // Vanilla consumes the fish even when whole-second rounding gives zero growth.
    assert_eq!(cat.get_entity().age.load(Relaxed), age);
    assert!(
        inventory
            .get_stack(slot)
            .are_equal(&ItemStack::new(1, &Item::COD))
    );
    assert!(inventory.get_stack(other_slot).are_equal(&other_hand));
    assert!(!cat.mob_entity.is_in_love());
    assert!(!cat.is_sitting());
    assert_eq!(kitten_metadata(&cat), baby_metadata);
    let ageable = cat
        .as_ageable()
        .expect("kitten feeding exposes ageable data");
    assert!(ageable.is_baby());
    assert_eq!(ageable.get_ageable_data().forced_age.load(Relaxed), 0);
    assert_eq!(
        ageable.get_ageable_data().forced_age_timer.load(Relaxed),
        40
    );
    let mut saved = NbtCompound::new();
    cat.write_custom_nbt(&mut saved);
    assert_eq!(saved.get_int("Age"), Some(age));
    assert_eq!(saved.get_int("ForcedAge"), Some(0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn animal_interaction_kitten_feeding_rounds_down_at_199_ticks() {
    let fixture = Fixture::new();
    for offhand in [false, true] {
        assert_kitten_feeding_boundary(&fixture, -199, offhand);
    }
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn animal_interaction_kitten_feeding_keeps_last_tick_baby_state() {
    let fixture = Fixture::new();
    for offhand in [false, true] {
        assert_kitten_feeding_boundary(&fixture, -1, offhand);
    }
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn animal_interaction_kitten_growth_persists_forced_age_and_respects_lock() {
    let fixture = Fixture::new();
    let cat = fixture.kitten(-200);
    let baby_metadata = kitten_metadata(&cat);
    let inventory = fixture.player.player.inventory();
    inventory.set_stack(40, ItemStack::new(2, &Item::SALMON));
    fixture.interact(cat.as_ref(), true);
    assert_eq!(cat.get_entity().age.load(Relaxed), -180);
    assert_eq!(inventory.off_hand_item().item_count, 1);
    assert_eq!(kitten_metadata(&cat), baby_metadata);
    assert!(!cat.is_sitting());
    assert!(!cat.mob_entity.is_in_love());

    let mut saved = NbtCompound::new();
    cat.write_custom_nbt(&mut saved);
    assert_eq!(saved.get_int("Age"), Some(-180));
    assert_eq!(saved.get_int("ForcedAge"), Some(20));
    let ageable = cat
        .as_ageable()
        .expect("kitten feeding exposes ageable data");
    assert_eq!(
        ageable.get_ageable_data().forced_age_timer.load(Relaxed),
        40
    );
    ageable.set_age_locked(true);
    fixture.interact(cat.as_ref(), true);
    assert_eq!(ageable.get_age(), -180);
    assert_eq!(ageable.get_ageable_data().forced_age.load(Relaxed), 20);
    assert_eq!(inventory.off_hand_item().item_count, 1);
    assert!(!cat.mob_entity.is_in_love());
    assert!(cat.is_sitting(), "unhandled owner feeding reaches sitting");

    cat.write_custom_nbt(&mut saved);
    assert_eq!(saved.get_bool("AgeLocked"), Some(true));
    let restored = CatEntity::new(fixture.entity(&EntityType::CAT));
    restored.read_custom_nbt(&saved);
    assert!(fixture.world.spawn_entity(restored.clone()));
    let restored_ageable = restored.as_ageable().expect("restored kitten is ageable");
    assert_eq!(restored_ageable.get_age(), -180);
    assert_eq!(
        restored_ageable.get_ageable_data().forced_age.load(Relaxed),
        20
    );
    assert!(restored_ageable.is_age_locked());
    assert!(restored_ageable.is_baby());

    // Exercise the shared setter locally; this does not simulate world ticking.
    restored_ageable.set_age_locked(false);
    restored_ageable.set_age(-1);
    let restored_baby_metadata = kitten_metadata(&restored);
    restored_ageable.ageable_ai_step();
    assert_eq!(restored_ageable.get_age(), 0);
    assert!(!restored_ageable.is_baby());
    assert_ne!(kitten_metadata(&restored), restored_baby_metadata);
    fixture.finish().await;
}
