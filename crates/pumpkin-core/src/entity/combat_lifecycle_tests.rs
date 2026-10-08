use super::*;
use crate::entity::player::Player;
use crate::net::java::combat_test_support::TestPlayer;
use crate::plugin::entity::entity_resurrect::EntityResurrectEvent;
use crate::plugin::player::{
    player_item_break::PlayerItemBreakEvent, player_item_consume::PlayerItemConsumeEvent,
    player_item_damage::PlayerItemDamageEvent,
};
use crate::plugin::{BoxFuture, EventHandler, EventPriority, Payload};
use crate::server::combat_test_support::{server, world};
use pumpkin_protocol::java::server::play::{SSetHeldItem, SUseItem};
use pumpkin_protocol::ser::NetworkReadExt;
use std::sync::atomic::AtomicUsize;

struct Handler<F>(F);
impl<E: Payload, F: Fn(&mut E) + Send + Sync> EventHandler<E> for Handler<F> {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut E,
    ) -> BoxFuture<'a, ()> {
        (self.0)(event);
        Box::pin(async {})
    }
}

fn register<E: Payload + Send + Sync + 'static>(
    server: &Server,
    action: impl Fn(&mut E) + Send + Sync + 'static,
) {
    server
        .plugin_manager
        .register::<E, _>(Arc::new(Handler(action)), EventPriority::Normal, true);
}

fn effect(kind: &'static StatusEffect, duration: i32, amplifier: u8) -> Effect {
    Effect {
        effect_type: kind,
        duration,
        amplifier,
        ambient: false,
        show_particles: true,
        show_icon: true,
        blend: false,
    }
}

fn raise(player: &Player, stack: ItemStack) {
    let duration = stack.get_max_use_time() - 5;
    player.inventory.set_slot(0, stack.clone());
    player
        .living_entity
        .set_active_hand(Hand::Right, stack, duration);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cooldown_and_hand_changes_use_the_real_packet_and_swap_handlers() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let fixture = TestPlayer::new(&world);
    let player = &fixture.player;
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    register::<PlayerItemConsumeEvent>(&server, move |_| {
        seen.fetch_add(1, Relaxed);
    });
    player
        .inventory
        .set_slot(0, ItemStack::new(1, &Item::SHIELD));
    player.start_cooldown("shield".into(), 100);
    fixture.client().handle_use_item(
        player,
        &SUseItem {
            hand: VarInt(0),
            sequence: VarInt(1),
            yaw: 0.0,
            pitch: 0.0,
        },
        &server,
    );
    assert_eq!(calls.load(Relaxed), 0);
    assert!(player.living_entity.active_hand.lock().unwrap().is_none());
    assert_eq!(
        player.living_entity.livings_flags.load(Relaxed) & LivingEntity::USING_ITEM_FLAG,
        0
    );

    raise(player, ItemStack::new(1, &Item::SHIELD));
    fixture
        .client()
        .handle_set_held_item(&server, player, &SSetHeldItem { slot: 1 });
    assert!(player.living_entity.active_hand.lock().unwrap().is_none());
    player.inventory.set_selected_slot(0);
    raise(player, ItemStack::new(1, &Item::SHIELD));
    player.swap_item();
    assert!(player.living_entity.active_hand.lock().unwrap().is_none());
    assert_eq!(player.inventory.off_hand_item().item, &Item::SHIELD);
    raise(player, ItemStack::new(1, &Item::SHIELD));
    let mut worn = player.inventory.held_item();
    worn.set_damage(17);
    player.inventory.set_slot(0, worn);
    player.living_entity.tick(player.as_ref(), &server);
    assert_eq!(
        player
            .living_entity
            .item_in_use
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .get_damage(),
        17
    );
    player
        .inventory
        .set_slot(0, ItemStack::new(1, &Item::DIAMOND));
    player.living_entity.tick(player.as_ref(), &server);
    assert!(player.living_entity.active_hand.lock().unwrap().is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shield_callbacks_preserve_identical_replacements_and_publish_the_live_broken_slot() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let mut fixture = TestPlayer::new(&world);
    let player = fixture.player.clone();
    let mode = Arc::new(AtomicUsize::new(0));
    let callback_mode = mode.clone();
    register::<PlayerItemDamageEvent>(&server, move |event| {
        if callback_mode.load(Relaxed) == 0 {
            event
                .player
                .inventory
                .set_slot(0, ItemStack::new(1, &Item::SHIELD));
        }
    });
    let breaks = Arc::new(AtomicUsize::new(0));
    let callback_breaks = breaks.clone();
    register::<PlayerItemBreakEvent>(&server, move |event| {
        callback_breaks.fetch_add(1, Relaxed);
        event
            .player
            .inventory
            .set_slot(0, ItemStack::new(1, &Item::DIAMOND));
    });
    let mut shield = ItemStack::new(1, &Item::SHIELD);
    shield.set_damage(shield.get_max_damage().unwrap() - 1);
    let attacker = LivingEntity::new(Entity::new(
        world,
        Vector3::new(0.0, 100.0, 1.0),
        &EntityType::ZOMBIE,
    ));
    raise(&player, shield.clone());
    let block = || {
        player.living_entity.apply_item_blocking(
            player.as_ref(),
            &DamageType::MOB_ATTACK,
            4.0,
            None,
            Some(&attacker),
        )
    };
    assert_eq!(block(), 4.0);
    assert_eq!(player.inventory.held_item().get_damage(), 0);
    assert_eq!(breaks.load(Relaxed), 0);
    player.living_entity.clear_active_hand();
    mode.store(1, Relaxed);
    raise(&player, shield);
    fixture.take_packets();
    assert_eq!(block(), 4.0);
    assert_eq!(breaks.load(Relaxed), 1);
    assert_eq!(player.inventory.held_item().item, &Item::DIAMOND);
    assert!(player.living_entity.active_hand.lock().unwrap().is_none());
    let mut updates = Vec::new();
    for bytes in fixture.take_packets() {
        let mut data = bytes.as_ref();
        if data.get_var_int().unwrap().0
            == pumpkin_data::packet::clientbound::play::SET_PLAYER_INVENTORY.0
        {
            let slot = data.get_var_int().unwrap().0;
            let count = data.get_var_int().unwrap().0;
            let item = if count > 0 {
                data.get_var_int().unwrap().0
            } else {
                -1
            };
            updates.push((slot, count, item));
        }
    }
    assert!(updates.contains(&(0, 1, i32::from(Item::DIAMOND.id))));
    assert!(
        !updates
            .iter()
            .any(|(slot, count, _)| *slot == 0 && *count == 0)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resurrection_revalidates_the_original_slot_and_stack_after_plugins() {
    for replace in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let server = server(dir.path());
        let world = world(&server, dir.path());
        let fixture = TestPlayer::new(&world);
        let player = fixture.player.clone();
        player
            .inventory
            .set_slot(0, ItemStack::new(2, &Item::TOTEM_OF_UNDYING));
        player
            .inventory
            .set_slot(1, ItemStack::new(1, &Item::DIAMOND));
        let weak = Arc::downgrade(&player);
        register::<EntityResurrectEvent>(&server, move |_| {
            let player = weak.upgrade().unwrap();
            if replace {
                player
                    .inventory
                    .set_slot(0, ItemStack::new(2, &Item::TOTEM_OF_UNDYING));
            }
            player.inventory.set_selected_slot(1);
        });
        assert_eq!(
            player
                .living_entity
                .try_use_death_protector(player.as_ref(), &DamageType::FALL),
            !replace
        );
        assert_eq!(
            player.inventory.get_slot(0).item_count,
            if replace { 2 } else { 1 }
        );
        assert_eq!(player.inventory.get_slot(1).item, &Item::DIAMOND);
    }
}

#[tokio::test]
async fn boss_effect_rejection_and_absorption_removal_run_the_shared_lifecycle() {
    let dir = tempfile::tempdir().unwrap();
    let world = test_support::armor_test_world(dir.path());
    for kind in [&EntityType::WITHER, &EntityType::ENDER_DRAGON] {
        let living = LivingEntity::new(Entity::new(world.clone(), Vector3::default(), kind));
        living.add_effect(effect(&StatusEffect::GLOWING, 100, 0));
        assert!(!living.has_effect(&StatusEffect::GLOWING));
    }
    let living = LivingEntity::new(Entity::new(world, Vector3::default(), &EntityType::COW));
    living.update_attribute(&Attributes::MAX_ABSORPTION, |attribute| {
        attribute.base_value = 8.0;
    });
    living.set_absorption(6.0);
    assert!(!living.remove_effect(&StatusEffect::ABSORPTION));
    assert_eq!(living.absorption.load(), 6.0);
    living.add_effect(effect(&StatusEffect::ABSORPTION, 100, 0));
    living.set_absorption(6.0);
    assert!(living.remove_effect(&StatusEffect::ABSORPTION));
    assert_eq!(living.absorption.load(), 6.0);
    living.add_effect(effect(&StatusEffect::ABSORPTION, 100, 0));
    living.set_absorption(0.0);
    living.tick_effects();
    assert!(!living.has_effect(&StatusEffect::ABSORPTION));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn periodic_damage_resurrection_cannot_publish_a_removed_hidden_effect() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let mut fixture = TestPlayer::new(&world);
    let player = fixture.player.clone();
    player
        .inventory
        .set_slot(0, ItemStack::new(1, &Item::TOTEM_OF_UNDYING));
    player.living_entity.set_health(1.0);
    player
        .living_entity
        .add_effect(effect(&StatusEffect::WITHER, 100, 0));
    player
        .living_entity
        .add_effect(effect(&StatusEffect::WITHER, 1, 6));
    fixture.take_packets();
    player.living_entity.tick_effects();
    assert_eq!(player.living_entity.health.load(), 1.0);
    assert!(player.inventory.held_item().is_empty());
    assert!(!player.living_entity.has_effect(&StatusEffect::WITHER));
    assert!(player.living_entity.has_effect(&StatusEffect::ABSORPTION));
    let mut absorption_sent = false;
    for bytes in fixture.take_packets() {
        let mut data = bytes.as_ref();
        if data.get_var_int().unwrap().0
            == pumpkin_data::packet::clientbound::play::UPDATE_MOB_EFFECT.0
        {
            data.get_var_int().unwrap();
            let kind = data.get_var_int().unwrap().0;
            assert_ne!(kind, i32::from(StatusEffect::WITHER.id));
            absorption_sent |= kind == i32::from(StatusEffect::ABSORPTION.id);
        }
    }
    assert!(absorption_sent);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn periodic_damage_callback_refreshes_the_live_effect_before_decrement() {
    use crate::plugin::entity::entity_damage::EntityDamageEvent;
    for replace in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let server = server(dir.path());
        let world = world(&server, dir.path());
        let fixture = TestPlayer::new(&world);
        let player = &fixture.player;
        player
            .living_entity
            .add_effect(effect(&StatusEffect::WITHER, 2, 6));
        let weak = Arc::downgrade(player);
        register::<EntityDamageEvent>(&server, move |_| {
            let player = weak.upgrade().unwrap();
            if replace {
                player.living_entity.remove_effect(&StatusEffect::WITHER);
            }
            player
                .living_entity
                .add_effect(effect(&StatusEffect::WITHER, 100, 6));
        });
        player.living_entity.tick_effects();
        assert_eq!(
            player
                .living_entity
                .get_effect(&StatusEffect::WITHER)
                .unwrap()
                .duration,
            if replace { 100 } else { 99 }
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepted_stat_events_update_statistics_and_all_matching_objectives() {
    use crate::plugin::player::player_statistic_increment::PlayerStatisticIncrementEvent;
    use crate::world::scoreboard::ScoreboardObjective;
    use pumpkin_protocol::java::client::play::RenderType;
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let fixture = TestPlayer::new(&world);
    let player = &fixture.player;
    for name in ["first", "second"] {
        world.scoreboard.lock().unwrap().add_objective(
            world.as_ref(),
            ScoreboardObjective::new(
                name,
                TextComponent::text(name),
                RenderType::Integer,
                None,
                "minecraft.used:minecraft.shield",
            ),
        );
    }
    let cancel = Arc::new(AtomicBool::new(true));
    let flag = cancel.clone();
    register::<PlayerStatisticIncrementEvent>(&server, move |event| {
        event.cancelled = flag.load(Relaxed);
        event.amount = 3;
    });
    player.increment_stat(StatisticCategory::Used, i32::from(Item::SHIELD.id), 1);
    assert_eq!(
        player
            .stats
            .lock()
            .unwrap()
            .get(StatisticCategory::Used, i32::from(Item::SHIELD.id)),
        0
    );
    assert_eq!(
        world
            .scoreboard
            .lock()
            .unwrap()
            .get_score_value(&player.gameprofile.name, "first"),
        None
    );
    cancel.store(false, Relaxed);
    player.increment_stat(StatisticCategory::Used, i32::from(Item::SHIELD.id), 1);
    assert_eq!(
        player
            .stats
            .lock()
            .unwrap()
            .get(StatisticCategory::Used, i32::from(Item::SHIELD.id)),
        3
    );
    for name in ["first", "second"] {
        assert_eq!(
            world
                .scoreboard
                .lock()
                .unwrap()
                .get_score_value(&player.gameprofile.name, name),
            Some(3)
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resurrection_keeps_combat_credit_and_skips_death_events_and_drops() {
    use crate::entity::death_test_world::DeathTestWorld;
    use crate::plugin::entity::entity_death::{EntityDeathEvent, PlayerDeathEvent};
    let fixture = DeathTestWorld::new().await;
    let victim = fixture.player("Protected");
    let attacker = fixture.player("Attacker");
    let living = &victim.living_entity;
    living.entity_equipment.lock().unwrap().put(
        &EquipmentSlot::OFF_HAND,
        ItemStack::new(1, &Item::TOTEM_OF_UNDYING),
    );
    victim
        .inventory
        .set_slot(1, ItemStack::new(3, &Item::DIAMOND));
    let deaths = Arc::new(AtomicUsize::new(0));
    let seen = deaths.clone();
    register::<EntityDeathEvent>(&fixture.server, move |_| {
        seen.fetch_add(1, Relaxed);
    });
    let seen = deaths.clone();
    register::<PlayerDeathEvent>(&fixture.server, move |_| {
        seen.fetch_add(1, Relaxed);
    });
    let resurrections = Arc::new(AtomicUsize::new(0));
    let seen = resurrections.clone();
    let weak = Arc::downgrade(&victim);
    let attacker_id = attacker.gameprofile.id;
    register::<EntityResurrectEvent>(&fixture.server, move |_| {
        let victim = weak.upgrade().unwrap();
        let living = &victim.living_entity;
        assert_eq!(living.health.load(), 0.0);
        assert!(!living.dead.load(Relaxed));
        assert_eq!(
            living.get_kill_credit().unwrap().get_entity().entity_uuid,
            attacker_id
        );
        assert_eq!(
            living
                .combat_tracker
                .lock()
                .unwrap()
                .get_killer_entry()
                .unwrap()
                .damage,
            100.0
        );
        seen.fetch_add(1, Relaxed);
    });
    assert!(victim.damage_with_context(
        &*victim,
        100.0,
        DamageType::PLAYER_ATTACK,
        None,
        Some(&*attacker),
        Some(&*attacker),
    ));
    assert_eq!(resurrections.load(Relaxed), 1);
    assert_eq!(deaths.load(Relaxed), 0);
    assert_eq!(living.health.load(), 1.0);
    assert!(!living.dead.load(Relaxed));
    assert!(!living.entity.is_removed());
    assert!(victim.inventory.off_hand_item().is_empty());
    assert_eq!(victim.inventory.get_slot(1).item_count, 3);
    assert!(fixture.world().entities.load().is_empty());
    assert!(living.has_effect(&StatusEffect::ABSORPTION));
    assert!(victim.damage(&*victim, f32::MAX, DamageType::GENERIC_KILL));
    assert_eq!(deaths.load(Relaxed), 2);
    assert!(living.dead.load(Relaxed));
    assert!(victim.inventory.get_slot(1).is_empty());
    assert!(
        fixture
            .world()
            .entities
            .load()
            .iter()
            .any(|entity| entity.get_item_entity().is_some())
    );
}
