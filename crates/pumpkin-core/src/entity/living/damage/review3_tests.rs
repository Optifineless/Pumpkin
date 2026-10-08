#![expect(clippy::unwrap_used, reason = "Regression fixtures must be valid")]
use super::{
    tests::{hit, raise},
    *,
};
use crate::{
    entity::{Entity, player::Player},
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority, Payload,
        entity::{
            entity_damage::EntityDamageEvent, entity_death::PlayerDeathEvent,
            item_spawn::ItemSpawnEvent,
        },
        player::{player_bed::PlayerBedLeaveEvent, player_item_damage::PlayerItemDamageEvent},
    },
    server::{
        Server,
        combat_test_support::{server, world},
    },
};
use pumpkin_data::{Enchantment, entity::EntityType, item::Item, item_stack::ItemStack};
use pumpkin_util::Hand;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering::SeqCst},
};

struct Callback<F>(F);
impl<E: Payload, F: Fn(&mut E) + Send + Sync> EventHandler<E> for Callback<F> {
    fn handle_blocking<'a>(&'a self, _: &'a Arc<Server>, event: &'a mut E) -> BoxFuture<'a, ()> {
        (self.0)(event);
        Box::pin(async {})
    }
}
fn register<E: Payload + Send + Sync + 'static>(
    server: &Server,
    callback: impl Fn(&mut E) + Send + Sync + 'static,
) {
    server.plugin_manager.register::<E, _>(
        Arc::new(Callback(callback)),
        EventPriority::Normal,
        true,
    );
}
fn assert_new_life(player: &Player) {
    assert_eq!(player.living_entity.health.load(), 20.0);
    assert_eq!(player.living_entity.hurt_cooldown.load(Relaxed), 20);
    assert_eq!(player.get_entity().velocity.load(), Vector3::default());
    assert!(!player.get_entity().hurt_marked.load(Relaxed));
    assert!(
        !player.is_on_cooldown(crate::entity::item_use::cooldown_group(&ItemStack::new(
            1,
            &Item::SHIELD
        )))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verification3_shield_damage_callback_reset_stops_mob_response() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let victim = TestPlayer::new(&world).player;
    world.players.store(Arc::new(vec![victim.clone()]));
    let mob = LivingEntity::new(Entity::new(
        world,
        Vector3::new(0.0, 0.0, 1.0),
        &EntityType::ZOMBIE,
    ));
    mob.entity_equipment.lock().unwrap().put(
        &EquipmentSlot::MAIN_HAND,
        ItemStack::new(1, &Item::DIAMOND_AXE),
    );
    raise(&victim, 0.5);
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    register::<PlayerItemDamageEvent>(&server, move |event| {
        seen.fetch_add(1, SeqCst);
        event.player.living_entity.reset_state();
        // Keep the same stack UID, and raise it in the replacement life.
        let stack = event.player.inventory.held_item();
        event.player.living_entity.set_active_hand(
            Hand::Right,
            stack.clone(),
            stack.get_max_use_time() - 5,
        );
    });
    assert!(!hit(&victim, 8.0, DamageType::MOB_ATTACK, &mob));
    assert_eq!(calls.load(SeqCst), 1);
    assert_eq!(victim.inventory.held_item().get_damage(), 0);
    assert!(victim.living_entity.active_hand.lock().unwrap().is_some());
    assert_new_life(&victim);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verification3_first_armor_callback_reset_stops_remaining_slots() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let victim = TestPlayer::new(&world).player;
    world.players.store(Arc::new(vec![victim.clone()]));
    for (index, item) in [
        (36, &Item::IRON_BOOTS),
        (37, &Item::IRON_LEGGINGS),
        (38, &Item::IRON_CHESTPLATE),
        (39, &Item::IRON_HELMET),
    ] {
        victim.inventory.set_slot(index, ItemStack::new(1, item));
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    register::<PlayerItemDamageEvent>(&server, move |event| {
        if seen.fetch_add(1, SeqCst) == 0 {
            event.player.living_entity.reset_state();
        }
    });
    assert!(!victim.damage(victim.as_ref(), 8.0, DamageType::PLAYER_ATTACK));
    assert_eq!(calls.load(SeqCst), 1);
    for index in 36..40 {
        assert_eq!(victim.inventory.get_slot(index).get_damage(), 0);
    }
    assert_new_life(&victim);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verification3_helmet_callback_reset_stops_pre_cooldown_hit() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let victim = TestPlayer::new(&world).player;
    world.players.store(Arc::new(vec![victim.clone()]));
    victim
        .inventory
        .set_slot(39, ItemStack::new(1, &Item::IRON_HELMET));
    register::<PlayerItemDamageEvent>(&server, |event| event.player.living_entity.reset_state());
    assert!(!victim.damage(victim.as_ref(), 8.0, DamageType::FALLING_ANVIL));
    assert_eq!(victim.inventory.get_slot(39).get_damage(), 0);
    assert_new_life(&victim);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verification3_wake_callback_reset_stops_blocking_and_armor() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let victim = TestPlayer::new(&world).player;
    world.players.store(Arc::new(vec![victim.clone()]));
    victim.sleeping_since.store(Some(0));
    victim.sleeping_bed_pos.store(Some(BlockPos::new(0, 0, 0)));
    victim
        .inventory
        .set_slot(39, ItemStack::new(1, &Item::IRON_HELMET));
    register::<PlayerBedLeaveEvent>(&server, |event| {
        event.player.living_entity.reset_state();
        event
            .player
            .living_entity
            .no_action_time
            .store(777, Relaxed);
    });
    assert!(!victim.damage(victim.as_ref(), 8.0, DamageType::FALLING_ANVIL));
    assert_eq!(victim.inventory.get_slot(39).get_damage(), 0);
    assert_eq!(victim.living_entity.no_action_time.load(Relaxed), 777);
    assert_new_life(&victim);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verification3_sweep_callback_reset_aborts_primary_weapon_effects() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let attacker = TestPlayer::new(&world).player;
    let primary = TestPlayer::new(&world).player;
    let secondary = Arc::new(LivingEntity::new(Entity::new(
        world.clone(),
        Vector3::new(0.5, 100.0, 0.0),
        &EntityType::ZOMBIE,
    )));
    world
        .players
        .store(Arc::new(vec![attacker.clone(), primary.clone()]));
    world.entities.store(Arc::new(vec![secondary.clone()]));
    let mut sword = ItemStack::new(1, &Item::IRON_SWORD);
    sword.add_enchantment(&Enchantment::FIRE_ASPECT, 1);
    attacker.inventory.set_slot(0, sword);
    attacker.last_attacked_ticks.store(100, Relaxed);
    attacker.get_entity().on_ground.store(true, Relaxed);
    let target = primary.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    register::<EntityDamageEvent>(&server, move |event| {
        if event.entity_id == secondary.entity.entity_id {
            target.living_entity.reset_state();
            seen.fetch_add(1, SeqCst);
        }
    });
    attacker.attack(&(primary.clone() as Arc<dyn EntityBase>));
    assert_eq!(calls.load(SeqCst), 1);
    assert_eq!(primary.get_entity().fire_ticks.load(Relaxed), 0);
    assert_eq!(attacker.inventory.held_item().get_damage(), 0);
    assert_eq!(
        attacker.stats.lock().unwrap().get(
            crate::entity::player::statistics::StatisticCategory::Used,
            i32::from(Item::IRON_SWORD.id),
        ),
        0,
        "the stale primary attack still ran its item hook"
    );
    assert_new_life(&primary);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verification3_first_drop_callback_reset_preserves_second_stack() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let victim = TestPlayer::new(&world).player;
    world.players.store(Arc::new(vec![victim.clone()]));
    victim
        .inventory
        .set_slot(0, ItemStack::new(3, &Item::STONE));
    victim
        .inventory
        .set_slot(1, ItemStack::new(2, &Item::DIAMOND));
    let player = victim.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    register::<ItemSpawnEvent>(&server, move |_| {
        if seen.fetch_add(1, SeqCst) == 0 {
            player.living_entity.reset_state();
        }
    });
    victim.damage(victim.as_ref(), f32::MAX, DamageType::GENERIC_KILL);
    assert!(calls.load(SeqCst) >= 1);
    let world_count: u32 = world
        .entities
        .load()
        .iter()
        .filter_map(|e| e.get_item_entity())
        .map(|e| e.get_item_stack().lock().unwrap().clone())
        .filter(|stack| stack.item == &Item::DIAMOND)
        .map(|stack| u32::from(stack.item_count))
        .sum();
    let stack = victim.inventory.get_slot(1);
    let inventory_count = if stack.item == &Item::DIAMOND {
        u32::from(stack.item_count)
    } else {
        0
    };
    assert_eq!(
        world_count + inventory_count,
        2,
        "the second stack disappeared"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verification3_first_loot_callback_reset_preserves_generated_batch() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let tables = dir
        .path()
        .join("datapacks/review3/data/minecraft/loot_table/entities");
    std::fs::create_dir_all(&tables).unwrap();
    std::fs::write(
        dir.path().join("datapacks/review3/pack.mcmeta"),
        r#"{"pack":{"min_format":94,"max_format":94,"description":"test"}}"#,
    )
    .unwrap();
    std::fs::write(tables.join("zombie.json"),
        r#"{"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:stone"}]},{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:diamond"}]}]}"#).unwrap();
    server
        .datapack_manager
        .load_all(dir.path(), &["file/review3".into()], &server.recipe_manager);
    let world = world(&server, dir.path());
    let victim = Arc::new(LivingEntity::new(Entity::new(
        world.clone(),
        Vector3::default(),
        &EntityType::ZOMBIE,
    )));
    world.entities.store(Arc::new(vec![victim.clone()]));
    let mob = victim.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    register::<ItemSpawnEvent>(&server, move |_| {
        if seen.fetch_add(1, SeqCst) == 0 {
            mob.reset_state();
        }
    });
    victim.damage(victim.as_ref(), f32::MAX, DamageType::GENERIC_KILL);
    assert_eq!(
        calls.load(SeqCst),
        2,
        "generated loot disappeared after the first spawn"
    );
    assert_eq!(
        world
            .entities
            .load()
            .iter()
            .filter(|entity| entity.get_item_entity().is_some())
            .count(),
        2
    );
    assert_eq!(victim.health.load(), 20.0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verification3_healed_death_can_die_again_in_the_same_life() {
    for cancel in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let server = server(dir.path());
        let world = world(&server, dir.path());
        let victim = TestPlayer::new(&world).player;
        world.players.store(Arc::new(vec![victim.clone()]));
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        register::<PlayerDeathEvent>(&server, move |event| {
            if seen.fetch_add(1, SeqCst) == 0 {
                event.player.set_health(20.0);
                event.cancelled = cancel;
            }
        });
        victim.damage(victim.as_ref(), f32::MAX, DamageType::GENERIC_KILL);
        assert_eq!(victim.living_entity.health.load(), 20.0);
        assert!(!victim.living_entity.dead.load(Relaxed));
        victim.living_entity.hurt_cooldown.store(0, Relaxed);
        assert!(victim.damage(victim.as_ref(), f32::MAX, DamageType::GENERIC_KILL));
        assert_eq!(calls.load(SeqCst), 2);
        assert!(victim.living_entity.dead.load(Relaxed));
        assert_eq!(victim.living_entity.health.load(), 0.0);
    }
}
