use super::{
    horse_spawn_stats_test_rng::{NaturalScope, Script},
    horse_spawn_stats_test_support::*,
};
use crate::entity::EntityBase;
use pumpkin_data::entity::{EntityType, MobCategory};
use pumpkin_util::math::{position::BlockPos, vector2::Vector2, vector3::Vector3};

#[tokio::test]
async fn feat_horses_s1_real_births_use_individual_stats() {
    tokio::time::timeout(std::time::Duration::from_secs(90), async {
        let mut fixture = Fixture::new(true);
        let script = Script::new([0, 0], [0.0; 6], 1);
        let horse = fixture.summon("horse 8.5 64 8.5");
        assert_stats(
            horse.as_ref(),
            [15.0, 0.112_499_997_019_767_76, 0.400_000_005_960_464_5],
            15.0,
        );
        assert_observer(
            &fixture.actor.take_packets(),
            horse.as_ref(),
            [15.0, 0.11249999701976776, 0.4000000059604645],
            15.0,
        );
        script.consumed();
        let script = Script::new([7, 8], [0.25, 0.5, 0.75, 0.125, 0.25, 0.375], 1);
        let horse = fixture.egg(horse_egg());
        assert_stats(horse.as_ref(), [30.0, INTERNAL[1], INTERNAL[2]], 30.0);
        assert_observer(
            &fixture.actor.take_packets(),
            horse.as_ref(),
            [30.0, INTERNAL[1], INTERNAL[2]],
            30.0,
        );
        script.consumed();
        fixture.finish().await;
        natural_birth().await;
    })
    .await
    .unwrap();
}

async fn natural_birth() {
    let mut fixture = Fixture::new(true);
    // A literal seed makes placement/species selection repeatable; horse stats
    // still use independently scripted bounded integer and double draws.
    let natural = NaturalScope::new(12);
    let script = Script::new([3, 4], [0.25, 0.5, 0.75, 0.125, 0.25, 0.375], 18);
    fixture
        .actor
        .player
        .get_entity()
        .set_pos(Vector3::new(8.5, 64.0, -32.0));
    fixture.server.level_info.rcu(|info| {
        let mut info = (**info).clone();
        info.spawn_x = -1000;
        info
    });
    let state = crate::world::natural_spawner::SpawnState::new(
        289,
        &fixture.world.entities,
        &fixture.world,
    );
    crate::world::natural_spawner::spawn_category_for_position(
        &MobCategory::CREATURE,
        &fixture.world,
        BlockPos::new(8, 64, 8),
        &Vector2::new(0, 0),
        &state,
        false,
    );
    let horses: Vec<_> = fixture
        .world
        .entities
        .load()
        .iter()
        .filter(|e| e.get_entity().entity_type == &EntityType::HORSE)
        .cloned()
        .collect();
    assert!(
        !horses.is_empty(),
        "seeded natural horse birth was not admitted"
    );
    let packets = fixture.actor.take_packets();
    for horse in &horses {
        assert_observer(&packets, horse.as_ref(), INTERNAL, 22.0);
        assert_stats(horse.as_ref(), INTERNAL, 22.0);
    }
    assert_eq!(
        Script::remaining(),
        (18 - horses.len()) * 8,
        "natural births did not draw once each"
    );
    drop(script);
    drop(natural);
    fixture.finish().await;
}

#[tokio::test]
async fn feat_horses_s1_observers_receive_rolled_attributes_and_health() {
    tokio::time::timeout(std::time::Duration::from_secs(90), async {
        let mut fixture = Fixture::new(true);
        let script = Script::new([3, 4], [0.25, 0.5, 0.75, 0.125, 0.25, 0.375], 1);
        let horse = fixture.summon("horse 8.5 64 8.5");
        script.consumed();
        assert_observer(
            &fixture.actor.take_packets(),
            horse.as_ref(),
            INTERNAL,
            22.0,
        );
        fixture.finish().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn feat_horses_s1_non_horse_controls_keep_defaults() {
    tokio::time::timeout(std::time::Duration::from_secs(90), async {
        let fixture = Fixture::new(true);
        let script = Script::new([0, 0], [0.0; 6], 0);
        for ty in [
            &EntityType::DONKEY,
            &EntityType::MULE,
            &EntityType::SKELETON_HORSE,
            &EntityType::ZOMBIE_HORSE,
            &EntityType::COW,
        ] {
            let mob = fixture.summon(&format!("{} 8.5 64 8.5", ty.resource_name));
            assert_eq!(bases(mob.as_ref()), default_bases(ty));
            assert_eq!(
                mob.get_living_entity().unwrap().health.load(),
                default_bases(ty)[0] as f32
            );
        }
        script.consumed();
        fixture.finish().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn feat_horses_s1_config_old_toml_and_disabled_birth() {
    tokio::time::timeout(std::time::Duration::from_secs(90), async {
        for enabled in [false, true] {
            let mut fixture = Fixture::new(enabled);
            let script = Script::new(
                [3, 4],
                [0.25, 0.5, 0.75, 0.125, 0.25, 0.375],
                usize::from(enabled) * 2,
            );
            let expected = if enabled {
                INTERNAL
            } else {
                default_bases(&EntityType::HORSE)
            };
            let summoned = fixture.summon("horse 8.5 64 8.5");
            assert_stats(summoned.as_ref(), expected, expected[0] as f32);
            if enabled {
                assert_observer(
                    &fixture.actor.take_packets(),
                    summoned.as_ref(),
                    expected,
                    expected[0] as f32,
                );
            }
            let egg = fixture.egg(horse_egg());
            assert_stats(egg.as_ref(), expected, expected[0] as f32);
            if enabled {
                assert_observer(
                    &fixture.actor.take_packets(),
                    egg.as_ref(),
                    expected,
                    expected[0] as f32,
                );
            }
            script.consumed();
            fixture.finish().await;
        }
    })
    .await
    .unwrap();
}
