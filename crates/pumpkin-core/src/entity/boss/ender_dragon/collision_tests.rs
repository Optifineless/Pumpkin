#![expect(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "The fixture is static test setup and every failure should identify its violated invariant."
)]

use super::{EnderDragonEntity, phase::EnderDragonPhase};
use crate::{
    entity::{
        Entity, EntityBase,
        mob::{creaking::CreakingEntity, zombie::zombie::ZombieEntity},
    },
    net::java::combat_test_support::TestPlayer,
    plugin::{BoxFuture, EventHandler, EventPriority, entity::entity_damage::EntityDamageEvent},
    server::{Server, combat_test_support},
    world::World,
};
use pumpkin_data::{attributes::Attributes, damage::DamageType, entity::EntityType};
use pumpkin_util::math::{position::BlockPos, vector2::Vector2, vector3::Vector3};
use pumpkin_util::{Difficulty, GameMode};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering::Relaxed},
};

struct Fixture {
    _dir: tempfile::TempDir,
    server: Arc<Server>,
    world: Arc<World>,
    player: TestPlayer,
    dragon: Arc<EnderDragonEntity>,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(dir.path());
        let world = combat_test_support::world(&server, dir.path());
        combat_test_support::publish_empty_chunk(&world, Vector2::new(0, 0));
        world.set_difficulty(Difficulty::Normal);

        let player = TestPlayer::new(&world);
        player
            .player
            .get_entity()
            .set_pos(Vector3::new(14.0, 64.0, 8.0));
        // The wing's five-damage hit only admits targets older than two ticks.
        player.player.living_entity.combat_ticks.store(3, Relaxed);

        let dragon = EnderDragonEntity::new(Entity::new(
            world.clone(),
            Vector3::new(8.0, 64.0, 8.0),
            &EntityType::ENDER_DRAGON,
        ));
        dragon.set_phase(EnderDragonPhase::Charging);
        // Start with an established flight history so the collision regressions
        // also exercise the old collision code independently of first-sample filling.
        {
            let mut history = dragon.flight_history.lock().unwrap();
            for _ in 0..64 {
                history.record(64.0, 0.0);
            }
        }
        let dragon_entity: Arc<dyn EntityBase> = dragon.clone();
        assert!(world.spawn_entity(dragon_entity));

        assert!(
            player
                .player
                .get_entity()
                .bounding_box
                .load()
                .intersects(&dragon.get_entity().bounding_box.load())
        );

        Self {
            _dir: dir,
            server,
            world,
            player,
            dragon,
        }
    }

    fn tick_dragon(&self) {
        // EnderDragon.tick -> Mob.mob_tick -> ai_step runs the production collision path.
        self.dragon.tick(self.dragon.as_ref(), &self.server);
    }

    fn zombie(&self, pos: Vector3<f64>) -> Arc<ZombieEntity> {
        let zombie = ZombieEntity::new(Entity::new(self.world.clone(), pos, &EntityType::ZOMBIE));
        let living = zombie.get_living_entity().unwrap();
        living.set_attribute_base(&Attributes::ARMOR, 0.0);
        living.combat_ticks.store(3, Relaxed);
        assert!(
            self.world
                .spawn_entity(zombie.clone() as Arc<dyn EntityBase>)
        );
        zombie
    }

    async fn finish(self) {
        self.world.level.shutdown().await.unwrap();
    }
}

async fn assert_no_contact_effects(mode: GameMode) {
    let fixture = Fixture::new();
    fixture.player.player.set_gamemode(mode);
    fixture.player.player.living_entity.set_health(5.0);

    fixture.tick_dragon();
    fixture
        .player
        .player
        .get_entity()
        .set_pos(Vector3::new(8.0, 64.0, 0.25));
    fixture.tick_dragon();

    assert_eq!(fixture.player.player.living_entity.health.load(), 5.0);
    assert!(
        !fixture
            .player
            .player
            .living_entity
            .dead
            .load(std::sync::atomic::Ordering::Relaxed)
    );
    assert_eq!(
        fixture.player.player.get_entity().velocity.load(),
        Vector3::default()
    );
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dragon_tick_does_not_hurt_or_push_creative_player() {
    assert_no_contact_effects(GameMode::Creative).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dragon_tick_does_not_hurt_or_push_spectator_player() {
    assert_no_contact_effects(GameMode::Spectator).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dragon_tick_damages_survival_player_with_dragon_as_attacker() {
    let fixture = Fixture::new();
    fixture.player.player.living_entity.set_health(20.0);

    fixture.tick_dragon();

    assert_eq!(fixture.player.player.living_entity.health.load(), 15.0);
    {
        let combat = fixture
            .player
            .player
            .living_entity
            .combat_tracker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let hit = combat
            .get_killer_entry()
            .expect("dragon hit should be recorded");
        assert_eq!(hit.damage_type, DamageType::MOB_ATTACK);
        assert_eq!(hit.attacker_id, Some(fixture.dragon.get_entity().entity_id));
        assert_eq!(hit.source_id, Some(fixture.dragon.get_entity().entity_id));
    };

    let death_message = crate::entity::living::LivingEntity::get_death_message(
        fixture.player.player.as_ref(),
        DamageType::MOB_ATTACK,
        None,
        None,
    );
    let with = match death_message.0.content.as_ref() {
        pumpkin_util::text::TextContent::Translate { with, .. } => with.as_slice(),
        _ => &[],
    };
    assert_eq!(
        with.len(),
        2,
        "death.attack.mob needs victim and attacker names"
    );
    assert_eq!(
        with[1],
        fixture.dragon.as_ref().get_display_name().0,
        "the second death-message argument should name the dragon"
    );
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ender_dragon_tick_hits_head_and_wing_zombies_but_leaves_distant_zombie_untouched() {
    let fixture = Fixture::new();
    fixture.player.player.set_gamemode(GameMode::Creative);
    // Prime the previous head/neck positions through a real tick (aiStep:283-291).
    fixture.tick_dragon();
    let head = fixture.zombie(Vector3::new(8.0, 64.0, 0.25));
    let wing = fixture.zombie(Vector3::new(17.0, 65.0, 9.0));
    let far = fixture.zombie(Vector3::new(30.0, 64.0, 8.0));
    let above = fixture.zombie(Vector3::new(8.0, 70.0, 8.0));
    let frozen = CreakingEntity::new(Entity::new(
        fixture.world.clone(),
        Vector3::new(17.0, 65.0, 9.0),
        &EntityType::CREAKING,
    ));
    frozen.set_transient(BlockPos::new(17, 64, 9));
    frozen.set_can_move(false);
    assert!(
        fixture
            .world
            .spawn_entity(frozen.clone() as Arc<dyn EntityBase>)
    );

    fixture.tick_dragon();

    assert_eq!(head.get_living_entity().unwrap().health.load(), 10.0);
    assert_eq!(
        head.get_living_entity().unwrap().last_damage_taken.load(),
        10.0
    );
    {
        let hit = head
            .get_living_entity()
            .unwrap()
            .combat_tracker
            .lock()
            .unwrap();
        let entry = hit.get_killer_entry().unwrap();
        assert_eq!(entry.damage_type, DamageType::MOB_ATTACK);
        assert_eq!(
            entry.attacker_id,
            Some(fixture.dragon.get_entity().entity_id)
        );
    };
    assert_eq!(wing.get_living_entity().unwrap().health.load(), 15.0);
    let velocity = wing.get_entity().velocity.load();
    assert!(velocity.x > 0.0, "wing push points away from the body");
    assert!(
        velocity.y >= f64::from(0.2f32),
        "wing push adds vertical motion"
    );
    assert_eq!(far.get_living_entity().unwrap().health.load(), 20.0);
    assert_eq!(far.get_entity().velocity.load(), Vector3::default());
    assert_eq!(above.get_living_entity().unwrap().health.load(), 20.0);
    assert_eq!(above.get_entity().velocity.load(), Vector3::default());
    assert_eq!(frozen.get_entity().velocity.load(), Vector3::default());
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ender_dragon_tick_sitting_still_pushes_wings_and_hurts_head() {
    let fixture = Fixture::new();
    fixture.player.player.set_gamemode(GameMode::Creative);
    fixture.tick_dragon();
    fixture.dragon.set_phase(EnderDragonPhase::SitAttacking);
    let head = fixture.zombie(Vector3::new(8.0, 64.0, 0.25));
    let wing = fixture.zombie(Vector3::new(17.0, 65.0, 9.0));

    fixture.tick_dragon();

    assert_eq!(head.get_living_entity().unwrap().health.load(), 10.0);
    assert_eq!(wing.get_living_entity().unwrap().health.load(), 20.0);
    assert!(wing.get_entity().velocity.load().x > 0.0);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ender_dragon_tick_suppresses_contact_while_hurt_then_resumes() {
    let fixture = Fixture::new();
    fixture.player.player.set_gamemode(GameMode::Creative);
    fixture.tick_dragon();
    let head = fixture.zombie(Vector3::new(8.0, 64.0, 0.25));
    let wing = fixture.zombie(Vector3::new(17.0, 65.0, 9.0));
    fixture
        .dragon
        .get_living_entity()
        .unwrap()
        .hurt_time
        .store(2, Relaxed);

    fixture.tick_dragon();

    assert_eq!(head.get_living_entity().unwrap().health.load(), 20.0);
    assert_eq!(wing.get_entity().velocity.load(), Vector3::default());
    fixture.tick_dragon();
    assert_eq!(head.get_living_entity().unwrap().health.load(), 10.0);
    assert!(wing.get_entity().velocity.load().x > 0.0);
    fixture.finish().await;
}

struct DamageCallback<F>(F);

impl<F: Fn(&mut EntityDamageEvent) + Send + Sync> EventHandler<EntityDamageEvent>
    for DamageCallback<F>
{
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut EntityDamageEvent,
    ) -> BoxFuture<'a, ()> {
        (self.0)(event);
        Box::pin(async {})
    }
}

async fn assert_stale_head_target_skipped(remove: bool) {
    let fixture = Fixture::new();
    fixture.player.player.set_gamemode(GameMode::Creative);
    fixture.tick_dragon();
    fixture.player.player.set_gamemode(GameMode::Survival);
    let other_dir = tempfile::tempdir().unwrap();
    let other_world = combat_test_support::world(&fixture.server, other_dir.path());
    let target_fixture = TestPlayer::new(&fixture.world);
    let target = target_fixture.player.clone();
    target.get_entity().set_pos(Vector3::new(8.0, 64.0, 0.25));
    fixture.world.players.store(Arc::new(vec![
        fixture.player.player.clone(),
        target.clone(),
    ]));
    assert!(
        target.get_entity().bounding_box.load().intersects(
            &fixture.dragon.parts[0]
                .entity
                .bounding_box
                .load()
                .expand_all(1.0)
        )
    );
    let reached = Arc::new(AtomicBool::new(false));
    let called = reached.clone();
    let trigger_id = fixture.player.player.get_entity().entity_id;
    let moved = target.clone();
    let destination = other_world.clone();
    let dragon = fixture.dragon.clone();
    fixture
        .server
        .plugin_manager
        .register::<EntityDamageEvent, _>(
            Arc::new(DamageCallback(move |event: &mut EntityDamageEvent| {
                if event.entity_id == trigger_id && !called.swap(true, Relaxed) {
                    assert!(dragon.phase.try_lock().is_ok());
                    assert!(dragon.flight_history.try_lock().is_ok());
                    if remove {
                        moved.get_entity().remove();
                    } else {
                        // The live world writer can run before the old roster is republished.
                        moved.get_entity().set_world(destination.clone());
                    }
                }
            })),
            EventPriority::Normal,
            true,
        );

    fixture.tick_dragon();

    assert!(
        reached.load(Relaxed),
        "a real wing-damage callback must run"
    );
    assert_eq!(target.living_entity.health.load(), 20.0);
    assert_eq!(target.get_entity().velocity.load(), Vector3::default());
    other_world.level.shutdown().await.unwrap();
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ender_dragon_tick_skips_head_player_moved_to_another_world_by_callback() {
    assert_stale_head_target_skipped(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ender_dragon_tick_skips_head_player_removed_by_callback() {
    assert_stale_head_target_skipped(true).await;
}
