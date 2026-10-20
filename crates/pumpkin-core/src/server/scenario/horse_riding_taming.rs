//! Ride attempts use the Java packet dispatcher, admitted entities and the normal tick graph.
use super::*;
use crate::entity::passive::horse::{FLAG_STANDING, HorseEntity};
use pumpkin_data::{advancement::Advancement, item::Item, item_stack::ItemStack};
use pumpkin_protocol::{VarInt, java::server::play::SInteract, ser::NetworkReadExt};
use std::sync::atomic::Ordering::Relaxed;

mod callback_lifecycle;
mod callbacks;
mod persistence;
mod saddle;

fn horse(entity: &Arc<dyn EntityBase>) -> &HorseEntity {
    entity.cast_any().downcast_ref().unwrap()
}

fn spawn(world: &Arc<World>) -> Arc<dyn EntityBase> {
    let entity = Scenario::mob(
        world,
        &EntityType::HORSE,
        SPAWN + Vector3::new(1.5, 0.0, 0.0),
    );
    script(&entity, 1, 0);
    entity
}

fn script(entity: &Arc<dyn EntityBase>, attempt: i32, temper: i32) {
    *horse(entity).taming_random.script.lock().unwrap() = Some((attempt, temper));
}

fn interact(scenario: &Scenario, session: &Session, entity: &Arc<dyn EntityBase>) {
    session.confirm_teleport(&scenario.server);
    session.send(
        &scenario.server,
        &SInteract {
            entity_id: VarInt(entity.get_entity().entity_id),
            r#type: VarInt(0),
            target_position: None,
            hand: Some(VarInt(0)),
            sneaking: false,
        },
    );
}

fn statuses(session: &Session, entity: &Arc<dyn EntityBase>) -> Vec<u8> {
    session
        .packets()
        .iter()
        .filter_map(|packet| {
            let mut data = packet.as_ref();
            if data.get_var_int().unwrap().0
                != pumpkin_data::packet::clientbound::play::ENTITY_EVENT.0
            {
                return None;
            }
            if data.len() != 5
                || i32::from_be_bytes(data[..4].try_into().unwrap())
                    != entity.get_entity().entity_id
            {
                return None;
            }
            Some(data[4])
        })
        .collect()
}

fn tamed_advancement(session: &Session) -> bool {
    let mut advancements = session.player.advancements.lock().unwrap();
    advancements
        .progress
        .get_mut_or_start_progress(Advancement::HUSBANDRY_TAME_AN_ANIMAL)
        .is_done()
}

fn assert_mounted(session: &Session, entity: &Arc<dyn EntityBase>) {
    assert!(
        session
            .player
            .get_entity()
            .get_vehicle()
            .is_some_and(|v| Arc::ptr_eq(&v, entity)),
        "Java empty-hand interaction did not mount"
    );
    assert!(
        entity
            .get_entity()
            .passengers
            .lock()
            .unwrap()
            .first()
            .is_some_and(|p| std::ptr::eq(p.get_entity(), session.player.get_entity()))
    );
}

fn assert_no_player_control(scenario: &Scenario, session: &Session, entity: &Arc<dyn EntityBase>) {
    use pumpkin_protocol::java::server::play::{
        Action, SMoveVehicle, SPlayerCommand, SPlayerInput,
    };
    let position = entity.get_entity().pos.load();
    let velocity = entity.get_entity().velocity.load();
    session.send(
        &scenario.server,
        &SPlayerInput {
            input: SPlayerInput::FORWARD | SPlayerInput::JUMP,
        },
    );
    session.send(
        &scenario.server,
        &SPlayerCommand {
            entity_id: VarInt(session.player.entity_id()),
            action: Action::StartHorseJump,
            jump_boost: VarInt(90),
        },
    );
    session.send(
        &scenario.server,
        &SMoveVehicle {
            x: position.x + 8.0,
            y: position.y + 1.0,
            z: position.z,
            yaw: 90.0,
            pitch: 0.0,
            on_ground: true,
        },
    );
    assert_eq!(
        entity.get_entity().pos.load(),
        position,
        "horse accepted player vehicle steering"
    );
    assert_eq!(
        entity.get_entity().velocity.load(),
        velocity,
        "horse accepted player jump"
    );
}

async fn opportunity(scenario: &Scenario, entity: &Arc<dyn EntityBase>) {
    // Literal RNG opportunities are delivered only by normal goal selector ticks.
    let before = horse(entity)
        .taming_random
        .rolls
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, temper)| *temper)
        .count();
    for _ in 0..40 {
        scenario.ticks(1).await;
        let after = horse(entity)
            .taming_random
            .rolls
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, temper)| *temper)
            .count();
        if after > before {
            return;
        }
    }
    panic!("normal riding goal produced no temper roll in 40 ticks");
}

async fn remount(scenario: &Scenario, session: &Session, entity: &Arc<dyn EntityBase>) {
    script(entity, 1, 0);
    session.confirm_teleport(&scenario.server);
    scenario.ticks(61).await;
    // Use ordinary teleport to bring the owner back into entity-interaction range.
    let pos = entity.get_entity().pos.load();
    scenario.command(&format!(
        "tp {} {} {} {}",
        session.player.gameprofile.name,
        pos.x - 1.0,
        pos.y,
        pos.z
    ));
    session.loaded(&scenario.server);
    interact(scenario, session, entity);
    assert_mounted(session, entity);
}

async fn assert_selector_cadence(scenario: &Scenario, entity: &Arc<dyn EntityBase>) {
    let horse = horse(entity);
    let mut observed = 0;
    for _ in 0..8 {
        let before = horse.taming_random.rolls.lock().unwrap().len();
        scenario.ticks(1).await;
        let ticks = horse.mob_entity.ticks_lived.load(Relaxed);
        let rolls = horse.taming_random.rolls.lock().unwrap()[before..].to_vec();
        if (ticks + entity.get_entity().entity_id) % 2 != 0 {
            assert!(
                rolls.is_empty(),
                "riding attempt ran on the selector's intervening tick"
            );
        } else if !rolls.is_empty() {
            assert_eq!(
                rolls,
                [(25, false)],
                "selector must use one adjustedTickDelay(50) roll per admitted tick"
            );
            observed += 1;
        }
    }
    assert!(
        observed > 0,
        "normal riding selector never admitted the goal"
    );
}

#[tokio::test]
async fn feat_horses_s2_empty_hand_rides_buck_then_tame() {
    run(async {
        let scenario = Scenario::new().await;
        let owner = scenario.join(Uuid::new_v4(), "HorseRider").await;
        let entity = spawn(&owner.player.world());
        let horse = horse(&entity);
        let _watch = riding_lock_probe::watch(&entity);
        interact(&scenario, &owner, &entity);
        assert_mounted(&owner, &entity);
        assert!(!horse.is_tame(), "mount must not automatically tame");
        assert_no_player_control(&scenario, &owner, &entity);
        scenario.ticks(6).await;
        owner.packets();
        assert_selector_cadence(&scenario, &entity).await;
        script(&entity, 0, 0);
        opportunity(&scenario, &entity).await;
        assert_eq!(horse.get_temper(), 5, "failed first ride must add temper");
        assert!(
            !owner.player.get_entity().has_vehicle(),
            "failed ride must really dismount"
        );
        assert!(!entity.get_entity().has_passengers());
        assert!(horse.has_flag(FLAG_STANDING));
        assert_eq!(statuses(&owner, &entity), [6]);
        assert!(!tamed_advancement(&owner));
        owner.confirm_teleport(&scenario.server);
        scenario.ticks(20).await;
        assert!(
            !horse.has_flag(FLAG_STANDING),
            "bucking stand must expire within 20 ticks"
        );
        remount(&scenario, &owner, &entity).await;
        owner.packets();
        script(&entity, 0, 4);
        opportunity(&scenario, &entity).await;
        assert!(horse.is_tame());
        assert_eq!(horse.get_owner(), Some(owner.player.gameprofile.id));
        assert!(entity.get_mob().unwrap().is_tamed());
        assert_eq!(
            entity.get_mob().unwrap().get_owner_uuid(),
            horse.get_owner()
        );
        assert!(!horse.is_saddled());
        assert_no_player_control(&scenario, &owner, &entity);
        assert!(
            owner
                .player
                .inventory()
                .get_stack_in_hand(pumpkin_util::Hand::Right)
                .is_empty()
        );
        assert_eq!(statuses(&owner, &entity), [7]);
        assert!(tamed_advancement(&owner));
        let rolls = horse.taming_random.rolls.lock().unwrap().len();
        scenario.ticks(60).await;
        assert_eq!(horse.taming_random.rolls.lock().unwrap().len(), rolls);
        assert!(
            statuses(&owner, &entity).is_empty(),
            "tame horse must never emit another success"
        );
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn feat_horses_s2_roll_boundary_clamp_and_nonplayer() {
    run(async {
        let scenario = Scenario::new().await;
        let owner = scenario.join(Uuid::new_v4(), "HorseBoundary").await;
        let entity = spawn(&owner.player.world());
        horse(&entity).set_temper(5);
        interact(&scenario, &owner, &entity);
        script(&entity, 0, 5);
        opportunity(&scenario, &entity).await;
        assert!(!horse(&entity).is_tame(), "roll equal to temper must fail");
        assert_eq!(horse(&entity).get_temper(), 10);
        remount(&scenario, &owner, &entity).await;
        horse(&entity).set_temper(98);
        script(&entity, 0, 99);
        opportunity(&scenario, &entity).await;
        assert_eq!(
            horse(&entity).get_temper(),
            100,
            "gameplay increment must clamp"
        );
        owner.confirm_teleport(&scenario.server);
        let passenger = Scenario::mob(&owner.player.world(), &EntityType::CHICKEN, SPAWN);
        entity
            .get_entity()
            .add_passenger(entity.clone(), passenger.clone());
        script(&entity, 0, 0);
        for _ in 0..40 {
            scenario.ticks(1).await;
            if !passenger.get_entity().has_vehicle() {
                break;
            }
        }
        assert!(
            !passenger.get_entity().has_vehicle(),
            "non-player must be ejected"
        );
        assert_eq!(
            horse(&entity).get_temper(),
            100,
            "non-player cannot change temper"
        );
        let rolls = horse(&entity).taming_random.rolls.lock().unwrap().len();
        scenario.ticks(8).await;
        assert_eq!(
            horse(&entity).taming_random.rolls.lock().unwrap().len(),
            rolls,
            "empty horse must stop attempts"
        );
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn feat_horses_s2_disabled_and_unrelated_mobs_are_controls() {
    for (enabled, tamed) in [(false, false), (false, true), (true, true)] {
        run(async {
            let scenario = Scenario::new_with_horse_taming(false, enabled).await;
            let owner = scenario.join(Uuid::new_v4(), "HorseControl").await;
            let world = owner.player.world();
            let entity = spawn(&world);
            script(&entity, 0, 0);
            if tamed {
                horse(&entity).set_tame(true);
            }
            interact(&scenario, &owner, &entity);
            assert_mounted(&owner, &entity);
            owner.packets();
            scenario.ticks(30).await;
            assert_mounted(&owner, &entity);
            assert!(
                horse(&entity)
                    .taming_random
                    .rolls
                    .lock()
                    .unwrap()
                    .is_empty()
            );
            assert_eq!(horse(&entity).is_tame(), tamed);
            assert_eq!(horse(&entity).get_temper(), 0);
            assert!(statuses(&owner, &entity).is_empty());
            assert!(
                !owner.player.controls_vehicle(entity.as_ref()),
                "unsaddled horses have no player controller"
            );
            entity
                .get_entity()
                .remove_passenger(owner.player.entity_id());
            owner.confirm_teleport(&scenario.server);
            let baby = spawn(&world);
            baby.get_entity().age.store(-24000, Relaxed);
            script(&baby, 0, 0);
            interact(&scenario, &owner, &baby);
            assert!(
                !baby.get_entity().has_passengers(),
                "baby must not admit a mount"
            );
            let cow = Scenario::mob(&world, &EntityType::COW, SPAWN);
            interact(&scenario, &owner, &cow);
            assert!(!cow.get_entity().has_passengers());
            // Preserve the current generic food/breeding path; horse feeding is a later slice.
            owner
                .player
                .inventory()
                .set_stack_in_hand(pumpkin_util::Hand::Right, ItemStack::new(3, &Item::APPLE));
            let food = spawn(&world);
            interact(&scenario, &owner, &food);
            assert!(!food.get_entity().has_passengers());
            assert!(!horse(&food).is_tame());
            assert_eq!(
                owner
                    .player
                    .inventory()
                    .get_stack_in_hand(pumpkin_util::Hand::Right)
                    .item_count,
                2
            );
            scenario.ticks(8).await;
            assert!(horse(&baby).taming_random.rolls.lock().unwrap().is_empty());
            scenario.stop().await;
        })
        .await;
    }
}

#[tokio::test]
async fn feat_horses_s2_no_navigation_position_does_not_attempt() {
    run(async {
        let scenario = Scenario::new().await;
        let owner = scenario.join(Uuid::new_v4(), "HorseNoPath").await;
        let entity = spawn(&owner.player.world());
        // A zero-radius restriction has no legal destination (strict squared distance < 0).
        let mob = entity.get_mob().unwrap().get_mob_entity();
        mob.position_target
            .store(entity.get_entity().block_pos.load());
        mob.position_target_range.store(0, Relaxed);
        interact(&scenario, &owner, &entity);
        assert_mounted(&owner, &entity);
        script(&entity, 0, 0);
        owner.packets();
        scenario.ticks(12).await;
        assert_mounted(&owner, &entity);
        assert!(
            horse(&entity)
                .taming_random
                .rolls
                .lock()
                .unwrap()
                .is_empty()
        );
        assert_eq!(horse(&entity).get_temper(), 0);
        assert!(statuses(&owner, &entity).is_empty());
        scenario.stop().await;
    })
    .await;
}
