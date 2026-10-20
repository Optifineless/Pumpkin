#![expect(
    clippy::unwrap_used,
    reason = "Zombie door regressions use real server and world fixtures"
)]

use super::{ZombieEntityBase, drowned::DrownedEntity, zombie::ZombieEntity};
use crate::{
    entity::{
        Entity, EntityBase,
        ai::{
            control::move_control::MoveControl, goal::break_door::BreakDoorGoal,
            pathfinder::Navigator,
        },
        death_test_world::DeathTestWorld,
        mob::{
            Mob, MobEntity,
            equipment::RegionalDifficulty,
            spawn::{self, SpawnGroupData, SpawnReason},
        },
        player::Player,
    },
    net::java::combat_test_support::TestPlayer,
    server::Server,
    world::{
        World,
        spawn_test_support::{proto, publish},
    },
};
use pumpkin_data::{
    Block,
    biome::Biome,
    block_properties::{DoubleBlockHalf, OakDoorLikeProperties},
    entity::EntityType,
};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::{
    Difficulty,
    math::{position::BlockPos, vector3::Vector3},
};
use pumpkin_world::world::BlockFlags;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

const DOOR: BlockPos = BlockPos::new(6, 64, 8);
const ZOMBIE_START: Vector3<f64> = Vector3::new(5.5, 64.0, 8.5);
const PLAYER_TARGET: Vector3<f64> = Vector3::new(12.5, 64.0, 8.5);
const MAX_BREAK_TICKS: usize = 260;

struct DoorFixture {
    host: DeathTestWorld,
    world: Arc<World>,
    player: TestPlayer,
}

struct SelectorStopProbe {
    zombie: Arc<ZombieEntityBase>,
    selector_unlocked_during_stop: AtomicBool,
}

impl Mob for SelectorStopProbe {
    fn get_mob_entity(&self) -> &MobEntity {
        let mob_entity = self.zombie.get_mob_entity();
        self.selector_unlocked_during_stop.store(
            mob_entity.goals_selector.try_lock().is_ok(),
            Ordering::Relaxed,
        );
        mob_entity
    }
}

impl DoorFixture {
    async fn new() -> Self {
        let host = DeathTestWorld::new().await;
        let world = host.world();
        publish(&world, proto(&Biome::PLAINS, &Block::STONE));
        let player = TestPlayer::new(&world);
        player.player.get_entity().set_pos(PLAYER_TARGET);
        set_world_rules(&host.server, &world, Difficulty::Hard, true);
        Self {
            host,
            world,
            player,
        }
    }

    fn build_corridor(&self) {
        for x in 2..=14 {
            for z in 7..=9 {
                set_block(&self.world, BlockPos::new(x, 63, z), &Block::STONE);
            }
            for y in 64..=65 {
                set_block(&self.world, BlockPos::new(x, y, 7), &Block::STONE);
                set_block(&self.world, BlockPos::new(x, y, 9), &Block::STONE);
            }
            set_block(&self.world, BlockPos::new(x, 66, 8), &Block::STONE);
        }
        for x in [2, 14] {
            for z in 7..=9 {
                for y in 64..=65 {
                    set_block(&self.world, BlockPos::new(x, y, z), &Block::STONE);
                }
            }
        }
    }

    fn zombie(&self, enabled: bool) -> Arc<ZombieEntity> {
        let entity = Entity::new(self.world.clone(), ZOMBIE_START, &EntityType::ZOMBIE);
        entity.on_ground.store(true, Ordering::Relaxed);
        ZombieEntity::with_can_break_doors(entity, enabled)
    }

    fn closed_wooden_door(&self) {
        set_door(&self.world, DOOR, &Block::OAK_DOOR);
    }

    async fn finish(self) {
        self.host.server.shutdown().await;
    }
}

fn set_world_rules(
    server: &Arc<Server>,
    world: &Arc<World>,
    difficulty: Difficulty,
    mob_griefing: bool,
) {
    server.level_info.rcu(|info| {
        let mut info = (**info).clone();
        info.difficulty = difficulty;
        info.game_rules.mob_griefing = mob_griefing;
        info
    });
    world.level_info.rcu(|info| {
        let mut info = (**info).clone();
        info.difficulty = difficulty;
        info.game_rules.mob_griefing = mob_griefing;
        info
    });
}

fn set_block(world: &Arc<World>, pos: BlockPos, block: &'static Block) {
    world.set_block_state(
        &pos,
        block.default_state.id,
        BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
    );
}

fn set_door(world: &Arc<World>, lower: BlockPos, block: &'static Block) {
    set_block(world, lower, block);
    let upper_state = if block == &Block::OAK_DOOR {
        let mut properties = OakDoorLikeProperties::from_state_id(block.default_state.id);
        properties.half = DoubleBlockHalf::Upper;
        properties.to_state_id(block)
    } else {
        block.default_state.id
    };
    world.set_block_state(
        &lower.up(),
        upper_state,
        BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
    );
}

fn request_target_route(mob: &dyn Mob, target: &Arc<Player>) -> bool {
    let target_entity: Arc<dyn EntityBase> = target.clone();
    mob.get_mob_entity().set_target(Some(target_entity));
    let target_living = target.get_living_entity().unwrap();
    let mut navigator = mob
        .get_mob_entity()
        .navigator
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    navigator.move_to_entity(target_living, 1.0, mob.get_mob_entity());
    navigator.tick(mob.get_mob_entity(), mob);
    navigator
        .get_path()
        .is_some_and(|path| !path.is_done() && path.get_node_count() > 1)
}

fn has_target_route(mob: &dyn Mob, fixture: &DoorFixture) -> bool {
    request_target_route(mob, &fixture.player.player)
}

fn goal_count<G: crate::entity::ai::goal::Goal + 'static>(mob: &dyn Mob) -> usize {
    mob.get_mob_entity()
        .goals_selector
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .goal_count::<G>()
}

fn goal_running<G: crate::entity::ai::goal::Goal + 'static>(mob: &dyn Mob) -> bool {
    mob.get_mob_entity()
        .goals_selector
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .is_goal_running::<G>()
}

fn start_goals(mob: &dyn Mob) {
    mob.get_mob_entity()
        .goals_selector
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .tick(mob);
}

fn tick_mob(mob: &dyn Mob, world: &World, server: &Server, ticks: usize) {
    for _ in 0..ticks {
        world
            .level_time
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .world_age += 1;
        mob.get_entity().age.fetch_add(1, Ordering::Relaxed);
        EntityBase::tick(mob, mob, server);
    }
}

fn tick_goals(mob: &dyn Mob, world: &World, ticks: usize) {
    for _ in 0..ticks {
        world
            .level_time
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .world_age += 1;
        mob.get_entity().age.fetch_add(1, Ordering::Relaxed);
        mob.get_mob_entity()
            .goals_selector
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .tick(mob);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g02_zombie_door_entrypoints_keep_navigation_and_goal() {
    let mut fixture = DoorFixture::new().await;
    fixture.build_corridor();
    fixture.closed_wooden_door();
    b1020d_g02_check_public_entrypoints(&mut fixture);
    b1020d_g02_check_nbt_spawn_and_exclusions(&fixture);
    fixture.finish().await;
}

fn b1020d_g02_check_public_entrypoints(fixture: &mut DoorFixture) {
    let constructed = fixture.zombie(true);
    assert!(constructed.can_break_doors(), "constructor flag");
    assert_eq!(goal_count::<BreakDoorGoal>(constructed.as_ref()), 1);
    assert!(
        has_target_route(constructed.as_ref(), fixture),
        "with_can_break_doors must enable the actual navigator through a closed wooden door"
    );

    let zombie = fixture.zombie(false);
    assert_eq!(goal_count::<BreakDoorGoal>(zombie.as_ref()), 0);
    assert!(!has_target_route(zombie.as_ref(), fixture));
    zombie.set_can_break_doors(false);
    zombie.set_can_break_doors(true);
    assert!(
        has_target_route(zombie.as_ref(), fixture),
        "the concrete public setter must forward to navigation and goal registration"
    );
    zombie.set_can_break_doors(true);
    assert_eq!(
        goal_count::<BreakDoorGoal>(zombie.as_ref()),
        1,
        "repeated enable must keep exactly one registered BreakDoorGoal"
    );

    zombie
        .get_entity()
        .horizontal_collision
        .store(true, Ordering::Relaxed);
    start_goals(zombie.as_ref());
    assert!(goal_running::<BreakDoorGoal>(zombie.as_ref()));
    fixture.player.take_packets();
    zombie.set_can_break_doors(false);
    assert!(!goal_running::<BreakDoorGoal>(zombie.as_ref()));
    assert_eq!(goal_count::<BreakDoorGoal>(zombie.as_ref()), 0);
    let stopped_packets = fixture.player.take_packets();
    let cleared_stage = [DOOR, DOOR.up()].into_iter().any(|pos| {
        fixture
            .player
            .client()
            .serialize_packet(
                &pumpkin_protocol::java::client::play::CSetBlockDestroyStage::new(
                    zombie.get_entity().entity_id.into(),
                    pos,
                    -1,
                ),
            )
            .is_ok_and(|expected| stopped_packets.iter().any(|packet| packet == &expected))
    });
    assert!(
        cleared_stage,
        "stopping the active door goal clears its break stage"
    );

    let probe_entity = Entity::new(fixture.world.clone(), ZOMBIE_START, &EntityType::ZOMBIE);
    probe_entity.on_ground.store(true, Ordering::Relaxed);
    let probe_zombie = ZombieEntityBase::with_can_break_doors(probe_entity, true);
    assert!(has_target_route(probe_zombie.as_ref(), fixture));
    probe_zombie
        .get_entity()
        .horizontal_collision
        .store(true, Ordering::Relaxed);
    start_goals(probe_zombie.as_ref());
    assert!(goal_running::<BreakDoorGoal>(probe_zombie.as_ref()));
    let stop_probe = SelectorStopProbe {
        zombie: probe_zombie.clone(),
        selector_unlocked_during_stop: AtomicBool::new(false),
    };
    probe_zombie.set_can_break_doors(false, &stop_probe);
    assert!(
        stop_probe
            .selector_unlocked_during_stop
            .load(Ordering::Relaxed),
        "the removed running door goal must stop after selector ownership is released"
    );

    zombie.set_can_break_doors(true);
    zombie.set_can_break_doors(true);
    assert_eq!(goal_count::<BreakDoorGoal>(zombie.as_ref()), 1);
    assert!(has_target_route(zombie.as_ref(), fixture));
    zombie.set_can_break_doors(false);
    assert_eq!(goal_count::<BreakDoorGoal>(zombie.as_ref()), 0);
    zombie.set_can_break_doors(true);
    assert_eq!(goal_count::<BreakDoorGoal>(zombie.as_ref()), 1);
    assert!(has_target_route(zombie.as_ref(), fixture));
}

fn b1020d_g02_check_nbt_spawn_and_exclusions(fixture: &DoorFixture) {
    let restored = fixture.zombie(false);
    let mut nbt = NbtCompound::new();
    nbt.put_bool("CanBreakDoors", true);
    EntityBase::read_nbt_non_mut(restored.as_ref(), &nbt);
    assert!(
        restored.can_break_doors(),
        "literal CanBreakDoors NBT restore"
    );
    assert_eq!(goal_count::<BreakDoorGoal>(restored.as_ref()), 1);
    assert!(has_target_route(restored.as_ref(), fixture));

    let defaulted = fixture.zombie(true);
    EntityBase::read_nbt_non_mut(defaulted.as_ref(), &NbtCompound::new());
    assert!(
        !defaulted.can_break_doors(),
        "missing NBT tag defaults to false"
    );
    assert_eq!(goal_count::<BreakDoorGoal>(defaulted.as_ref()), 0);

    let finalized = fixture.zombie(false);
    let finalized_entity: Arc<dyn EntityBase> = finalized.clone();
    finalized.finalize_spawn_with_context(
        &finalized_entity,
        &crate::world::spawn_view::SpawnView::live(&fixture.world),
        &RegionalDifficulty {
            base_difficulty: Difficulty::Hard,
            effective_difficulty: 4.0,
            special_multiplier: 10.0,
        },
        SpawnReason::Natural,
        Some(SpawnGroupData::Zombie {
            is_baby: false,
            can_spawn_jockey: false,
        }),
    );
    assert!(
        finalized.can_break_doors(),
        "fresh-spawn finalization positive control"
    );
    assert_eq!(goal_count::<BreakDoorGoal>(finalized.as_ref()), 1);
    assert!(has_target_route(finalized.as_ref(), fixture));

    let drowned_entity = Entity::new(fixture.world.clone(), ZOMBIE_START, &EntityType::DROWNED);
    let drowned = DrownedEntity::with_can_break_doors(drowned_entity, true);
    let mut drowned_nbt = NbtCompound::new();
    drowned_nbt.put_bool("CanBreakDoors", true);
    EntityBase::read_nbt_non_mut(drowned.as_ref(), &drowned_nbt);
    assert_eq!(goal_count::<BreakDoorGoal>(drowned.as_ref()), 0);
    assert!(
        !has_target_route(drowned.as_ref(), fixture),
        "non-ground drowned navigation must remain unable to break doors"
    );

    let non_ground = fixture.zombie(true);
    non_ground
        .get_mob_entity()
        .configure_movement(Navigator::water_bound(false), MoveControl::default());
    non_ground.set_can_break_doors(true);
    assert!(!non_ground.can_break_doors());
    assert_eq!(goal_count::<BreakDoorGoal>(non_ground.as_ref()), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g02_enabled_zombie_plans_and_breaks_wooden_door() {
    let fixture = DoorFixture::new().await;
    fixture.build_corridor();
    fixture.closed_wooden_door();
    fixture.world.level_time.lock().unwrap().world_age = 20;
    b1020d_g02_check_enabled_disabled_and_re_disabled(&fixture);
    b1020d_g02_check_iron_and_drowned_controls(&fixture);
    b1020d_g02_check_difficulty_and_griefing_controls(&fixture);
    b1020d_g02_check_reload_admission(&fixture);
    fixture.finish().await;
}

fn b1020d_g02_check_enabled_disabled_and_re_disabled(fixture: &DoorFixture) {
    let enabled = fixture.zombie(true);
    assert!(has_target_route(enabled.as_ref(), fixture));
    enabled
        .get_entity()
        .horizontal_collision
        .store(true, Ordering::Relaxed);
    start_goals(enabled.as_ref());
    assert!(goal_running::<BreakDoorGoal>(enabled.as_ref()));
    tick_goals(enabled.as_ref(), &fixture.world, MAX_BREAK_TICKS);
    assert_eq!(
        fixture.world.get_block(&DOOR).id,
        Block::AIR.id,
        "wooden door lower half"
    );
    assert_eq!(
        fixture.world.get_block(&DOOR.up()).id,
        Block::AIR.id,
        "wooden door upper half"
    );
    assert!(
        has_target_route(enabled.as_ref(), fixture),
        "the removed-door clear corridor remains pathable"
    );
    tick_mob(enabled.as_ref(), &fixture.world, &fixture.host.server, 100);
    assert!(
        enabled.get_entity().pos.load().x > f64::from(DOOR.0.x + 1),
        "the zombie traverses the corridor after breaking the door"
    );

    set_world_rules(&fixture.host.server, &fixture.world, Difficulty::Hard, true);
    fixture.closed_wooden_door();
    let disabled = fixture.zombie(false);
    assert!(!has_target_route(disabled.as_ref(), fixture));
    disabled
        .get_entity()
        .horizontal_collision
        .store(true, Ordering::Relaxed);
    start_goals(disabled.as_ref());
    tick_goals(disabled.as_ref(), &fixture.world, MAX_BREAK_TICKS);
    assert_eq!(
        fixture.world.get_block(&DOOR).id,
        Block::OAK_DOOR.id,
        "disabled control"
    );
    assert!(!goal_running::<BreakDoorGoal>(disabled.as_ref()));

    let re_disabled = fixture.zombie(true);
    assert!(has_target_route(re_disabled.as_ref(), fixture));
    re_disabled
        .get_entity()
        .horizontal_collision
        .store(true, Ordering::Relaxed);
    start_goals(re_disabled.as_ref());
    assert!(goal_running::<BreakDoorGoal>(re_disabled.as_ref()));
    re_disabled.set_can_break_doors(false);
    tick_goals(re_disabled.as_ref(), &fixture.world, MAX_BREAK_TICKS);
    assert_eq!(
        fixture.world.get_block(&DOOR).id,
        Block::OAK_DOOR.id,
        "re-disabled control"
    );
    assert!(!goal_running::<BreakDoorGoal>(re_disabled.as_ref()));
}

fn b1020d_g02_check_iron_and_drowned_controls(fixture: &DoorFixture) {
    set_door(&fixture.world, DOOR, &Block::IRON_DOOR);
    let iron_control = fixture.zombie(true);
    assert!(!has_target_route(iron_control.as_ref(), fixture));
    iron_control
        .get_entity()
        .horizontal_collision
        .store(true, Ordering::Relaxed);
    start_goals(iron_control.as_ref());
    tick_goals(iron_control.as_ref(), &fixture.world, MAX_BREAK_TICKS);
    assert_eq!(
        fixture.world.get_block(&DOOR).id,
        Block::IRON_DOOR.id,
        "iron-door control"
    );
    assert!(!goal_running::<BreakDoorGoal>(iron_control.as_ref()));

    fixture.closed_wooden_door();
    let drowned = DrownedEntity::with_can_break_doors(
        Entity::new(fixture.world.clone(), ZOMBIE_START, &EntityType::DROWNED),
        true,
    );
    let mut drowned_nbt = NbtCompound::new();
    drowned_nbt.put_bool("CanBreakDoors", true);
    EntityBase::read_nbt_non_mut(drowned.as_ref(), &drowned_nbt);
    has_target_route(drowned.as_ref(), fixture);
    drowned
        .get_entity()
        .horizontal_collision
        .store(true, Ordering::Relaxed);
    start_goals(drowned.as_ref());
    tick_goals(drowned.as_ref(), &fixture.world, MAX_BREAK_TICKS);
    assert_eq!(
        fixture.world.get_block(&DOOR).id,
        Block::OAK_DOOR.id,
        "non-ground control"
    );
    assert!(!goal_running::<BreakDoorGoal>(drowned.as_ref()));
}

fn b1020d_g02_check_difficulty_and_griefing_controls(fixture: &DoorFixture) {
    set_world_rules(
        &fixture.host.server,
        &fixture.world,
        Difficulty::Normal,
        true,
    );
    let wrong_difficulty = fixture.zombie(true);
    assert!(has_target_route(wrong_difficulty.as_ref(), fixture));
    wrong_difficulty
        .get_entity()
        .horizontal_collision
        .store(true, Ordering::Relaxed);
    start_goals(wrong_difficulty.as_ref());
    tick_goals(wrong_difficulty.as_ref(), &fixture.world, MAX_BREAK_TICKS);
    assert_eq!(
        fixture.world.get_block(&DOOR).id,
        Block::OAK_DOOR.id,
        "difficulty control"
    );
    assert!(!goal_running::<BreakDoorGoal>(wrong_difficulty.as_ref()));

    set_world_rules(
        &fixture.host.server,
        &fixture.world,
        Difficulty::Hard,
        false,
    );
    let griefing_disabled = fixture.zombie(true);
    assert!(has_target_route(griefing_disabled.as_ref(), fixture));
    griefing_disabled
        .get_entity()
        .horizontal_collision
        .store(true, Ordering::Relaxed);
    start_goals(griefing_disabled.as_ref());
    tick_goals(griefing_disabled.as_ref(), &fixture.world, MAX_BREAK_TICKS);
    assert_eq!(
        fixture.world.get_block(&DOOR).id,
        Block::OAK_DOOR.id,
        "mobGriefing control"
    );
    assert!(!goal_running::<BreakDoorGoal>(griefing_disabled.as_ref()));
}

fn b1020d_g02_check_reload_admission(fixture: &DoorFixture) {
    set_world_rules(&fixture.host.server, &fixture.world, Difficulty::Hard, true);
    let saved_enabled = fixture.zombie(true);
    let saved_disabled = fixture.zombie(false);
    let mut enabled_nbt = NbtCompound::new();
    let mut disabled_nbt = NbtCompound::new();
    EntityBase::write_nbt(saved_enabled.as_ref(), &mut enabled_nbt);
    EntityBase::write_nbt(saved_disabled.as_ref(), &mut disabled_nbt);
    let loaded_enabled =
        spawn::load_spawn_entity(&fixture.world, &enabled_nbt, ZOMBIE_START).unwrap();
    let loaded_disabled =
        spawn::load_spawn_entity(&fixture.world, &disabled_nbt, ZOMBIE_START).unwrap();
    let loaded_enabled_mob = loaded_enabled.get_mob().unwrap();
    let loaded_disabled_mob = loaded_disabled.get_mob().unwrap();
    assert!(has_target_route(loaded_enabled_mob, fixture));
    loaded_enabled
        .get_entity()
        .horizontal_collision
        .store(true, Ordering::Relaxed);
    start_goals(loaded_enabled_mob);
    assert!(goal_running::<BreakDoorGoal>(loaded_enabled_mob));
    assert_eq!(goal_count::<BreakDoorGoal>(loaded_enabled_mob), 1);
    assert!(!has_target_route(loaded_disabled_mob, fixture));
    assert_eq!(goal_count::<BreakDoorGoal>(loaded_disabled_mob), 0);
}
