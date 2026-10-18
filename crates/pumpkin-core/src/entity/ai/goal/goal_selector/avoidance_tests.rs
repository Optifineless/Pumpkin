use super::*;
use crate::{
    entity::{
        Entity, EntityBase,
        ai::{goal::avoid_entity::AvoidEntityGoal, pathfinder::Navigator},
        passive::{
            cat::CatEntity, llama::LlamaEntity, ocelot::OcelotEntity, tamable::TamableAnimal,
            trader_llama::TraderLlamaEntity, villager::VillagerEntity,
            wandering_trader::WanderingTraderEntity, wolf::WolfEntity,
        },
        r#type::from_type,
    },
    net::java::combat_test_support::TestPlayer,
    server::{Server, combat_test_support},
    world::World,
};
use pumpkin_data::{Block, entity::EntityType};
use pumpkin_util::math::{vector2::Vector2, vector3::Vector3};
use pumpkin_world::chunk::ChunkData;
use std::sync::{Arc, atomic::Ordering::Relaxed};

struct Fixture {
    world: Arc<World>,
    _server: Arc<Server>,
    _directory: tempfile::TempDir,
}

impl Fixture {
    async fn finish(self) {
        self.world.level.shutdown().await.unwrap();
    }
}

fn flat_world() -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(directory.path());
    let fixture = Fixture {
        world: combat_test_support::world(&server, directory.path()),
        _server: server,
        _directory: directory,
    };
    for cx in -1..=1 {
        for cz in -1..=1 {
            let chunk = ChunkData::empty_sync(cx, cz);
            for x in 0..16 {
                for z in 0..16 {
                    chunk.set_block_absolute_y(x, 63, z, Block::STONE.default_state.id);
                }
            }
            fixture
                .world
                .level
                .loaded_chunks
                .insert(Vector2::new(cx, cz), chunk);
        }
    }
    fixture
}

fn installed_goals(mob: &dyn Mob) -> GoalSelector {
    mob.get_entity().on_ground.store(true, Relaxed);
    std::mem::take(&mut *mob.get_mob_entity().goals_selector.lock().unwrap())
}

fn starts(selector: &mut GoalSelector, mob: &dyn Mob) -> bool {
    (0..256).any(|_| {
        selector.tick(mob);
        selector.goals.iter().any(|goal| goal.running)
    })
}

fn stop_goals(selector: &mut GoalSelector, mob: &dyn Mob) {
    for goal in &mut selector.goals {
        goal.stop(mob);
    }
}

#[tokio::test]
async fn villager_zombie_goal_selects_husk_and_excludes_nearer_zombified_piglin() {
    let fixture = flat_world();
    let villager = VillagerEntity::new(Entity::new(
        fixture.world.clone(),
        Vector3::new(8.0, 64.0, 8.0),
        &EntityType::VILLAGER,
    ));
    let piglin = from_type(
        &EntityType::ZOMBIFIED_PIGLIN,
        Vector3::new(6.0, 64.0, 8.0),
        &fixture.world,
        uuid::Uuid::new_v4(),
    );
    let husk = from_type(
        &EntityType::HUSK,
        Vector3::new(12.0, 64.0, 8.0),
        &fixture.world,
        uuid::Uuid::new_v4(),
    );
    fixture
        .world
        .entities
        .store(Arc::new(vec![piglin.clone(), husk]));
    let mut selector = installed_goals(villager.as_ref());
    // Exercise the installed Zombie goal itself, rather than a later exact-Husk fallback.
    let zombie_goal = selector
        .goals
        .iter()
        .position(|goal| goal.type_id == TypeId::of::<AvoidEntityGoal>())
        .unwrap();
    selector.goals = vec![selector.goals.remove(zombie_goal)];
    assert!(
        starts(&mut selector, villager.as_ref()),
        "Zombie.class must include husks"
    );
    let target = villager
        .mob_entity
        .navigator
        .lock()
        .unwrap()
        .get_path()
        .unwrap()
        .get_target();
    assert!(
        target.0.x < 8,
        "flee from the husk to the east, not the nearer piglin to the west: {target:?}"
    );
    stop_goals(&mut selector, villager.as_ref());
    fixture.world.entities.store(Arc::new(vec![piglin]));
    for _ in 0..256 {
        selector.tick(villager.as_ref());
        assert!(
            !selector.goals[0].running,
            "zombified piglins are not villager hostiles"
        );
    }
    fixture.finish().await;
}

#[tokio::test]
async fn wandering_trader_zombie_goal_selects_nearest_subclass() {
    let fixture = flat_world();
    let trader = WanderingTraderEntity::new(Entity::new(
        fixture.world.clone(),
        Vector3::new(8.0, 64.0, 8.0),
        &EntityType::WANDERING_TRADER,
    ));
    let zombie = from_type(
        &EntityType::ZOMBIE,
        Vector3::new(1.0, 64.0, 8.0),
        &fixture.world,
        uuid::Uuid::new_v4(),
    );
    let husk = from_type(
        &EntityType::HUSK,
        Vector3::new(11.0, 64.0, 8.0),
        &fixture.world,
        uuid::Uuid::new_v4(),
    );
    fixture.world.entities.store(Arc::new(vec![zombie, husk]));
    let mut selector = installed_goals(trader.as_ref());
    selector
        .goals
        .retain(|goal| goal.type_id == TypeId::of::<AvoidEntityGoal>());
    assert!(starts(&mut selector, trader.as_ref()));
    let target = trader
        .mob_entity
        .navigator
        .lock()
        .unwrap()
        .get_path()
        .unwrap()
        .get_target();
    assert!(
        target.0.x < 8,
        "the nearer husk to the east wins the shared Zombie-class search: {target:?}"
    );
    fixture.finish().await;
}

#[tokio::test]
async fn cat_avoidance_stops_on_taming() {
    let fixture = flat_world();
    let witness = TestPlayer::new(&fixture.world);
    witness
        .player
        .get_entity()
        .set_pos(Vector3::new(10.0, 64.0, 8.0));
    let cat = CatEntity::new(Entity::new(
        fixture.world.clone(),
        Vector3::new(8.0, 64.0, 8.0),
        &EntityType::CAT,
    ));
    let mut selector = installed_goals(cat.as_ref());
    selector
        .goals
        .retain(|goal| goal.priority == 4 && goal.controls().get(Controls::MOVE));
    assert!(starts(&mut selector, cat.as_ref()));
    cat.set_tame(true, None);
    selector.tick(cat.as_ref());
    assert!(
        selector.goals.iter().all(|goal| !goal.running),
        "a newly tamed cat stops fleeing"
    );
    for _ in 0..32 {
        selector.tick(cat.as_ref());
        assert!(selector.goals.iter().all(|goal| !goal.running));
    }

    fixture.finish().await;
}

#[tokio::test]
async fn ocelot_avoidance_stops_on_trust() {
    let fixture = flat_world();
    let witness = TestPlayer::new(&fixture.world);
    witness
        .player
        .get_entity()
        .set_pos(Vector3::new(10.0, 64.0, 8.0));
    let ocelot = OcelotEntity::new(Entity::new(
        fixture.world.clone(),
        Vector3::new(8.0, 64.0, 8.0),
        &EntityType::OCELOT,
    ));
    let mut selector = installed_goals(ocelot.as_ref());
    selector.goals.retain(|goal| goal.priority == 4);
    assert!(starts(&mut selector, ocelot.as_ref()));
    ocelot.set_trusting(true);
    selector.tick(ocelot.as_ref());
    assert!(
        selector.goals.iter().all(|goal| !goal.running),
        "a newly trusting ocelot stops fleeing"
    );
    for _ in 0..32 {
        selector.tick(ocelot.as_ref());
        assert!(selector.goals.iter().all(|goal| !goal.running));
    }
    fixture.finish().await;
}

#[tokio::test]
async fn wolf_avoidance_selects_nearest_trader_llama_and_respects_taming() {
    let fixture = flat_world();
    let wolf = WolfEntity::new(Entity::new(
        fixture.world.clone(),
        Vector3::new(8.0, 64.0, 8.0),
        &EntityType::WOLF,
    ));
    let trader_llama = TraderLlamaEntity::new(Entity::new(
        fixture.world.clone(),
        Vector3::new(11.0, 64.0, 8.0),
        &EntityType::TRADER_LLAMA,
    ));
    trader_llama.set_strength(5);
    let llama = LlamaEntity::new(Entity::new(
        fixture.world.clone(),
        Vector3::new(1.0, 64.0, 8.0),
        &EntityType::LLAMA,
    ));
    llama.set_strength(5);
    fixture
        .world
        .entities
        .store(Arc::new(vec![llama.clone(), trader_llama]));
    wolf.mob_entity.set_target(Some(llama));
    let mut selector = installed_goals(wolf.as_ref());
    selector.goals.retain(|goal| goal.priority == 3);
    assert!(starts(&mut selector, wolf.as_ref()));
    let target = wolf
        .mob_entity
        .navigator
        .lock()
        .unwrap()
        .get_path()
        .unwrap()
        .get_target();
    assert!(
        target.0.x < 8,
        "the nearer trader llama to the east wins selection: {target:?}"
    );
    assert!(
        wolf.mob_entity.get_target().is_none(),
        "wolf avoidance clears its combat target"
    );
    stop_goals(&mut selector, wolf.as_ref());
    wolf.set_tame(true);
    for _ in 0..256 {
        selector.tick(wolf.as_ref());
        assert!(
            !selector.goals[0].running,
            "tamed wolves do not flee from llamas"
        );
    }
    fixture.finish().await;
}

#[tokio::test]
async fn wolf_avoidance_failed_nearest_strength_roll_does_not_choose_farther_llama() {
    let fixture = flat_world();
    let wolf = WolfEntity::new(Entity::new(
        fixture.world.clone(),
        Vector3::new(8.0, 64.0, 8.0),
        &EntityType::WOLF,
    ));
    let nearest = LlamaEntity::new(Entity::new(
        fixture.world.clone(),
        Vector3::new(11.0, 64.0, 8.0),
        &EntityType::LLAMA,
    ));
    nearest.set_strength(1);
    let farther = LlamaEntity::new(Entity::new(
        fixture.world.clone(),
        Vector3::new(1.0, 64.0, 8.0),
        &EntityType::LLAMA,
    ));
    farther.set_strength(5);
    fixture
        .world
        .entities
        .store(Arc::new(vec![farther, nearest]));
    let mut selector = installed_goals(wolf.as_ref());
    selector.goals.retain(|goal| goal.priority == 3);
    let mut rejected_admitted_path = false;
    // ThreadRng has no seed API. A fresh navigator separates an unsuccessful random
    // path sample from a strength rejection after create_path admitted a route.
    for _ in 0..256 {
        stop_goals(&mut selector, wolf.as_ref());
        *wolf.mob_entity.navigator.lock().unwrap() = Navigator::ground();
        selector.tick(wolf.as_ref());
        let computed_target = wolf.mob_entity.navigator.lock().unwrap().get_target_pos();
        if let Some(target) = computed_target {
            assert!(
                target.0.x < 8,
                "a failed nearest roll must not select the farther strong llama: {target:?}"
            );
            if !selector.goals[0].running {
                rejected_admitted_path = true;
                break;
            }
        }
    }
    assert!(
        rejected_admitted_path,
        "strength 1 must sometimes reject an otherwise admitted escape path"
    );
    fixture.finish().await;
}
