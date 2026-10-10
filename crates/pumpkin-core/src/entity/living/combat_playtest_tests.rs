#![expect(
    clippy::unwrap_used,
    reason = "Combat regression fixtures must be valid"
)]

use super::*;
use crate::{
    entity::ai::{
        control::{MoveControlTrait, move_control::MoveControl},
        goal::{Goal, melee_attack::MeleeAttackGoal, ocelot_attack::OcelotAttackGoal},
    },
    net::java::combat_test_support::TestPlayer,
    server::combat_test_support::{server, world},
};
use pumpkin_data::data_component_impl::{BlocksAttacksImpl, WeaponImpl};
use pumpkin_util::math::{vector2::Vector2, vector3::Vector3};
use pumpkin_world::chunk::ChunkData;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn playtest_summoned_axe_zombie_and_attack_goals_disable_shields() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let fixture = TestPlayer::new(&world);
    let player = fixture.player;
    player.get_entity().set_pos(Vector3::new(4.5, 64.0, 4.5));
    let chunk = ChunkData::empty_sync(0, 0);
    world.level.loaded_chunks.insert(Vector2::new(0, 0), chunk);
    for kind in [&EntityType::ZOMBIE, &EntityType::OCELOT] {
        let attacker = crate::entity::r#type::from_type(
            kind,
            Vector3::new(4.5, 64.0, 5.0),
            &world,
            uuid::Uuid::new_v4(),
        );
        let mut weapon = NbtCompound::new();
        weapon.put_string("id", "minecraft:iron_axe".into());
        weapon.put_int("count", 1);
        let mut equipment = NbtCompound::new();
        equipment.put("mainhand", NbtTag::Compound(weapon));
        let mut nbt = NbtCompound::new();
        nbt.put("equipment", NbtTag::Compound(equipment));
        nbt.put_bool("PersistenceRequired", true);
        // SummonCommand.createEntity loads equipment before insertion and skips finalization with NBT.
        attacker.read_nbt_non_mut(&nbt);
        let living = attacker.get_living_entity().unwrap();
        let axe = living.held_item(attacker.as_ref());
        assert_eq!(axe.item, &Item::IRON_AXE);
        let seconds = axe
            .get_data_component::<WeaponImpl>()
            .unwrap()
            .disable_blocking_for_seconds;
        assert_eq!(attacker.get_seconds_to_disable_blocking(), seconds);
        let mob = attacker.get_mob().unwrap();
        mob.get_mob_entity().set_target(Some(player.clone()));
        let shield = ItemStack::new(1, &Item::SHIELD);
        let blocking = shield.get_data_component::<BlocksAttacksImpl>().unwrap();
        let ticks = blocking.disable_blocking_for_ticks(seconds);
        player.item_cooldowns.lock().unwrap().clear();
        player.inventory.set_slot(0, shield.clone());
        player.living_entity.set_active_hand(
            Hand::Right,
            shield.clone(),
            shield.get_max_use_time() - blocking.block_delay_ticks(),
        );
        let mut goal: Box<dyn Goal> = if kind == &EntityType::ZOMBIE {
            Box::new(MeleeAttackGoal::new(1.0, true))
        } else {
            Box::new(OcelotAttackGoal::new())
        };
        if kind == &EntityType::OCELOT {
            assert!(goal.can_start(mob));
        }
        goal.tick(mob);
        assert_eq!(
            player
                .item_cooldowns
                .lock()
                .unwrap()
                .get(crate::entity::item_use::cooldown_group(&shield))
                .map(|cooldown| cooldown.duration),
            Some(ticks),
            "{} did not disable the shield",
            kind.resource_name
        );
        assert_eq!(player.living_entity.health.load(), 20.0);
        assert!(player.living_entity.get_item_blocking_with().is_none());
    }
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn playtest_skeleton_ground_displacement_matches_twenty_vanilla_ticks() {
    let dir = tempfile::tempdir().unwrap();
    let world = test_support::armor_test_world(dir.path());
    let chunk = ChunkData::empty_sync(0, 0);
    for x in 0..16 {
        for z in 0..16 {
            chunk.set_block_absolute_y(x, 63, z, pumpkin_data::Block::STONE.default_state.id);
        }
    }
    world.level.loaded_chunks.insert(Vector2::new(0, 0), chunk);
    let skeleton = crate::entity::r#type::from_type(
        &EntityType::SKELETON,
        Vector3::new(4.5, 64.0, 2.5),
        &world,
        uuid::Uuid::new_v4(),
    );
    let mob = skeleton.get_mob().unwrap();
    let living = skeleton.get_living_entity().unwrap();
    // MoveControl.tick -> Mob.setSpeed -> LivingEntity.travelInAir/getFrictionInfluencedSpeed.
    // Expected values independently evaluated with Java float rounding, friction 0.6F * 0.91F.
    for (modifier, distance, last_step) in [
        (1.0, 2.587_743_091_167_33, 0.137_664_454_198_548_8),
        (1.2, 3.726_350_347_424_655, 0.198_236_829_800_357_1),
    ] {
        let entity = skeleton.get_entity();
        entity.set_pos(Vector3::new(4.5, 64.0, 2.5));
        // Start settled on the floor, with travelInAir's residual downward gravity.
        entity.velocity.store(Vector3::new(0.0, -0.0784, 0.0));
        entity.on_ground.store(true, Relaxed);
        let mut control = MoveControl::default();
        let mut step = 0.0;
        for _ in 0..20 {
            control.set_wanted_position(4.5, 64.0, 14.5, modifier);
            control.tick(mob);
            let before = entity.pos.load().z;
            living.travel_in_air(skeleton.as_ref());
            step = entity.pos.load().z - before;
        }
        assert!(
            (entity.pos.load().z - 2.5 - distance).abs() < 1e-6,
            "modifier {modifier}: distance {}, last step {step}",
            entity.pos.load().z - 2.5
        );
        assert!((step - last_step).abs() < 1e-7);
    }
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn playtest_skeleton_bow_strafe_uses_quarter_control_speed() {
    let dir = tempfile::tempdir().unwrap();
    let world = test_support::armor_test_world(dir.path());
    let chunk = ChunkData::empty_sync(0, 0);
    for x in 0..16 {
        for z in 0..16 {
            chunk.set_block_absolute_y(x, 63, z, pumpkin_data::Block::STONE.default_state.id);
        }
    }
    world.level.loaded_chunks.insert(Vector2::new(0, 0), chunk);
    let skeleton = crate::entity::r#type::from_type(
        &EntityType::SKELETON,
        Vector3::new(4.5, 64.0, 4.5),
        &world,
        uuid::Uuid::new_v4(),
    );
    let mob = skeleton.get_mob().unwrap();
    let living = skeleton.get_living_entity().unwrap();
    let entity = skeleton.get_entity();
    entity.velocity.store(Vector3::new(0.0, -0.0784, 0.0));
    entity.on_ground.store(true, Relaxed);
    let mut control = MoveControl::default();
    // RangedBowAttackGoal.tick strafes with +/-0.5; MoveControl.strafe sets modifier 0.25.
    // Independent Java float evaluation: both axes travel 1.2938715456 blocks in 20 ticks.
    for _ in 0..20 {
        control.strafe(0.5, 0.5);
        control.tick(mob);
        living.travel_in_air(skeleton.as_ref());
    }
    let displacement = entity.pos.load() - Vector3::new(4.5, 64.0, 4.5);
    assert!((displacement.x - 1.293_871_545_583_665).abs() < 1e-6);
    assert!((displacement.z - 1.293_871_545_583_665).abs() < 1e-6);
    crate::server::fixture_lifecycle::finish().await;
}
