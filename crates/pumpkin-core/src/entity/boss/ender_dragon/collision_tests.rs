#![expect(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "The fixture is static test setup and every failure should identify its violated invariant."
)]

use super::{EnderDragonEntity, phase::EnderDragonPhase};
use crate::{
    entity::{Entity, EntityBase},
    net::java::combat_test_support::TestPlayer,
    server::{Server, combat_test_support},
    world::World,
};
use pumpkin_data::{damage::DamageType, entity::EntityType};
use pumpkin_util::math::{vector2::Vector2, vector3::Vector3};
use pumpkin_util::{Difficulty, GameMode};
use std::sync::Arc;

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
            .set_pos(Vector3::new(9.0, 64.0, 8.0));

        let dragon = EnderDragonEntity::new(Entity::new(
            world.clone(),
            Vector3::new(8.0, 64.0, 8.0),
            &EntityType::ENDER_DRAGON,
        ));
        dragon.set_phase(EnderDragonPhase::Hovering);
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

    async fn finish(self) {
        self.world.level.shutdown().await.unwrap();
    }
}

async fn assert_no_contact_effects(mode: GameMode) {
    let fixture = Fixture::new();
    fixture.player.player.set_gamemode(mode);
    fixture.player.player.living_entity.set_health(5.0);

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
