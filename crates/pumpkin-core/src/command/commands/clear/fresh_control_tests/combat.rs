use super::*;
use pumpkin_data::{
    damage::DamageType,
    packet::{
        CURRENT_MC_VERSION,
        serverbound::play::{ATTACK, SET_CARRIED_ITEM},
    },
};
use pumpkin_protocol::{
    ClientPacket, RawPacket,
    java::server::play::{SAttack, SSetHeldItem},
};
use pumpkin_util::math::vector3::Vector3;

fn enqueue(player: &Player, id: i32, packet: &impl ClientPacket) {
    let mut payload = Vec::new();
    packet
        .write_packet_data(&mut payload, &CURRENT_MC_VERSION)
        .unwrap();
    player.inbound_packets.push(RawPacket {
        id,
        payload: payload.into(),
    });
}

async fn queued_switch_damage(ticks: u32) {
    let mut fixture = ClearFixture::new();
    assert!(fixture.server.advanced_config.pvp.enabled);
    let attacker = fixture.target.player.clone();
    let victim = fixture.observer.player.clone();
    attacker.get_entity().set_pos(Vector3::new(0.0, 64.0, 0.0));
    victim.get_entity().set_pos(Vector3::new(1.0, 64.0, 0.0));
    attacker
        .get_entity()
        .on_ground
        .store(true, Ordering::Relaxed);
    victim.get_entity().on_ground.store(true, Ordering::Relaxed);
    assert!(!attacker.get_entity().is_sprinting());
    assert_eq!(attacker.living_entity.fall_distance.load(), 0.0);
    assert_eq!(attacker.gamemode.load(), pumpkin_util::GameMode::Survival);
    assert_eq!(victim.gamemode.load(), pumpkin_util::GameMode::Survival);
    assert_eq!(victim.living_entity.health.load(), 20.0);
    assert_eq!(victim.living_entity.get_absorption(), 0.0);
    assert!(!victim.abilities.lock().unwrap().invulnerable);
    assert_eq!(
        victim.living_entity.get_attribute_value(&Attributes::ARMOR),
        0.0
    );
    assert_eq!(
        victim
            .living_entity
            .get_attribute_value(&Attributes::ARMOR_TOUGHNESS),
        0.0
    );
    assert_eq!(attacker.inventory.get_selected_slot(), 0);
    attacker.inventory.set_slot(1, ItemStack::EMPTY.clone());
    let base_damage = attacker
        .living_entity
        .get_attribute_base(&Attributes::ATTACK_DAMAGE) as f32;
    let base_speed = attacker
        .living_entity
        .get_attribute_base(&Attributes::ATTACK_SPEED);
    prime_sword(&mut fixture);
    assert_eq!(attacker.inventory.held_item().item, &Item::DIAMOND_SWORD);
    assert!(
        attacker
            .world()
            .get_player_by_uuid(attacker.gameprofile.id)
            .is_some()
    );
    assert!(
        attacker
            .world()
            .get_entity_or_part(victim.entity_id())
            .is_some()
    );
    fixture.observer.take_packets();
    attacker.last_attacked_ticks.store(ticks, Ordering::Relaxed);
    // Vanilla Player attack arithmetic, using independent unarmed bases rather than stale getters.
    let charge = ((ticks as f32 + 0.5) / (20.0 / base_speed) as f32).clamp(0.0, 1.0);
    let expected = base_damage * (0.2 + charge * charge * 0.8);
    enqueue(&attacker, SET_CARRIED_ITEM.0, &SSetHeldItem { slot: 1 });
    enqueue(
        &attacker,
        ATTACK.0,
        &SAttack {
            entity_id: victim.entity_id().into(),
        },
    );
    assert_eq!(attacker.inbound_packets.len(), 2);
    let before = victim.living_entity.health.load();
    attacker.tick(&fixture.server);
    assert!(
        attacker.inbound_packets.is_empty(),
        "real inbound batch was not consumed"
    );
    assert!(!fixture.target.client().is_closed());
    assert_eq!(attacker.inventory.get_selected_slot(), 1);
    assert!(attacker.inventory.held_item().is_empty());
    assert!(
        victim.living_entity.hurt_cooldown.load(Ordering::Relaxed) > 0,
        "real queued attack never damaged the victim"
    );
    let damage = before - victim.living_entity.health.load();
    assert!(damage > 0.0);
    close_fixture(&fixture).await;
    assert!(
        (damage - expected).abs() < 0.00001,
        "same inbound batch used stale sword combat modifiers: ticks={ticks}, damage={damage}, expected={expected}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_slot_switch_uses_empty_hand_damage_in_the_same_tick() {
    queued_switch_damage(100).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_slot_switch_uses_empty_hand_speed_for_partial_charge() {
    queued_switch_damage(2).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_clear_removes_armor_before_damage_without_a_target_tick() {
    let fixture = ClearFixture::new();
    let target = &fixture.target.player;
    let naked = &fixture.observer.player;
    fixture.equip_diamond_armor();
    assert!(
        target.living_entity.get_attribute_value(&Attributes::ARMOR)
            > naked.living_entity.get_attribute_value(&Attributes::ARMOR)
    );
    assert_eq!(dispatch(&fixture, "clear @s"), 4);
    for slot in [
        EquipmentSlot::HEAD,
        EquipmentSlot::CHEST,
        EquipmentSlot::LEGS,
        EquipmentSlot::FEET,
    ] {
        assert!(
            target
                .inventory
                .entity_equipment
                .lock()
                .unwrap()
                .get(&slot)
                .is_empty()
        );
    }
    let before = target.living_entity.health.load();
    let naked_before = naked.living_entity.health.load();
    assert!(
        target
            .living_entity
            .damage(target.as_ref(), 8.0, DamageType::MOB_ATTACK)
    );
    assert!(
        naked
            .living_entity
            .damage(naked.as_ref(), 8.0, DamageType::MOB_ATTACK)
    );
    let damage = before - target.living_entity.health.load();
    let naked_damage = naked_before - naked.living_entity.health.load();
    assert_eq!(
        naked_damage, 8.0,
        "fresh naked control did not admit ordinary damage"
    );
    assert!(damage > 0.0);
    close_fixture(&fixture).await;
    assert_eq!(
        damage, naked_damage,
        "incoming damage after real clear retained stale armor"
    );
}
