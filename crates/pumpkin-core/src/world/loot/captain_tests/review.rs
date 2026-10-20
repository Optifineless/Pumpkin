use super::Fixture;
use crate::{
    entity::{EntityBase, mob::raider::create_ominous_banner},
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::player::player_statistic_increment::PlayerStatisticIncrementEvent,
    },
    server::Server,
    world::raid::Raid,
};
use pumpkin_data::{
    damage::DamageType,
    effect::StatusEffect,
    entity::EntityType,
    item::Item,
    potion::Effect,
    statistic::{CustomStatistic, StatisticCategory},
};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::{
    difficulty::Difficulty,
    math::{position::BlockPos, vector3::Vector3},
};
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering::Relaxed},
};

struct KillOnRaidTrigger {
    captain: Arc<dyn EntityBase>,
    ordinary: Arc<dyn EntityBase>,
    calls: AtomicUsize,
}

impl EventHandler<PlayerStatisticIncrementEvent> for KillOnRaidTrigger {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut PlayerStatisticIncrementEvent,
    ) -> BoxFuture<'a, ()> {
        if event.statistic_id
            == format!(
                "{:?}:{}",
                StatisticCategory::Custom,
                CustomStatistic::RaidTrigger as i32
            )
        {
            let world = event.player.world();
            // Fail immediately on the old lock scope instead of entering an unbounded deadlock.
            assert!(
                world.raids.try_lock().is_ok(),
                "Raid Omen guard spans statistic callback"
            );
            assert!(event.player.living_entity.active_effects.try_lock().is_ok());
            for victim in [&self.captain, &self.ordinary] {
                assert_eq!(
                    victim
                        .get_mob()
                        .unwrap()
                        .as_raider()
                        .unwrap()
                        .get_raider_data()
                        .raid_id
                        .load(),
                    None
                );
                assert!(victim.damage_with_context(
                    victim.as_ref(),
                    f32::MAX,
                    DamageType::GENERIC_KILL,
                    None,
                    None,
                    None
                ));
                assert!(victim.get_living_entity().unwrap().dead.load(Relaxed));
            }
            self.calls.fetch_add(1, Relaxed);
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g01_r2_raid_omen_expiry_callback_death() {
    Fixture::new()
        .verify(|fixture| {
            let client = TestPlayer::new(&fixture.world);
            client.client().set_player(client.player.clone());
            let callback = Arc::new(KillOnRaidTrigger {
                captain: fixture.pillager(true),
                ordinary: fixture.pillager(false),
                calls: AtomicUsize::new(0),
            });
            fixture
                .server
                .plugin_manager
                .register::<PlayerStatisticIncrementEvent, _>(
                    callback.clone(),
                    EventPriority::Normal,
                    true,
                );
            client
                .player
                .set_raid_omen_position(BlockPos::new(1000, 64, 1000));
            client.player.add_effect(Effect {
                effect_type: &StatusEffect::RAID_OMEN,
                duration: 1,
                amplifier: 0,
                ambient: false,
                show_particles: false,
                show_icon: false,
                blend: false,
            });
            client
                .player
                .living_entity
                .tick(client.player.as_ref(), &fixture.server);
            assert_eq!(callback.calls.load(Relaxed), 1);
            assert_eq!(
                fixture.bottles().len(),
                1,
                "only the outside-raid captain drops a bottle"
            );
            assert!(
                !client
                    .player
                    .living_entity
                    .has_effect(&StatusEffect::RAID_OMEN)
            );
            assert!(client.player.get_raid_omen_position().is_none());
        })
        .await;
}

fn ai(entity: &Arc<dyn EntityBase>) {
    entity.get_entity().age.store(0, Relaxed);
    let mob = entity.get_mob().unwrap();
    mob.get_mob_entity().server_ai_step(mob, entity.as_ref());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g01_r2_nearby_pillager_preserves_captain_bottle() {
    Fixture::new()
        .verify(|fixture| {
            let survivor = fixture.pillager(false);
            survivor.get_entity().set_pos(Vector3::new(8.5, 64.0, 8.5));
            assert_eq!(fixture.kill(&fixture.pillager(true)).len(), 1);
            let bottle = fixture
                .world
                .entities
                .load()
                .iter()
                .find(|e| {
                    e.get_item_entity().is_some_and(|i| {
                        i.get_item_stack().lock().unwrap().item == &Item::OMINOUS_BOTTLE
                    })
                })
                .unwrap()
                .clone();
            // No earlier junk is present: the bottle is the first ground item, before the banner.
            ai(&survivor);
            assert!(
                !bottle.get_entity().removed.load(Relaxed),
                "nearby AI consumed captain bottle"
            );
            assert_eq!(fixture.bottles().len(), 1);
            assert!(
                !survivor
                    .get_mob()
                    .unwrap()
                    .as_raider()
                    .unwrap()
                    .is_patrol_leader()
            );
            bottle.get_item_entity().unwrap().set_pickup_delay(0);
            check_banner_pickup(fixture, &survivor);
        })
        .await;
}

fn check_banner_pickup(fixture: &Fixture, survivor: &Arc<dyn EntityBase>) {
    // Start an actual raid membership only after the outside-raid bottle survives.
    let raider = survivor.get_mob().unwrap().as_raider().unwrap();
    raider.get_raider_data().raid_id.store(Some(7));
    let mut raid = Raid::new(7, BlockPos::new(8, 64, 8), Difficulty::Normal);
    raid.join_raid(0, survivor.get_entity().entity_uuid, survivor);
    fixture.world.raids.lock().unwrap().raid_map.insert(7, raid);
    let banner = fixture
        .world
        .entities
        .load()
        .iter()
        .find(|e| {
            e.get_item_entity().is_some_and(|i| {
                i.get_item_stack()
                    .lock()
                    .unwrap()
                    .are_equal(&create_ominous_banner())
            })
        })
        .unwrap()
        .clone();
    let canonical = create_ominous_banner();
    for wrong in [
        pumpkin_data::item_stack::ItemStack::new(1, &Item::WHITE_BANNER),
        {
            let mut doubled = canonical.clone();
            doubled.item_count = 2;
            doubled
        },
    ] {
        *banner
            .get_item_entity()
            .unwrap()
            .get_item_stack()
            .lock()
            .unwrap() = wrong;
        banner.get_item_entity().unwrap().set_pickup_delay(0);
        ai(survivor);
        assert!(!raider.is_patrol_leader());
        assert!(!banner.get_entity().removed.load(Relaxed));
        assert_eq!(fixture.bottles().len(), 1);
    }
    *banner
        .get_item_entity()
        .unwrap()
        .get_item_stack()
        .lock()
        .unwrap() = canonical;
    banner.get_item_entity().unwrap().set_pickup_delay(10);
    // Delayed canonical banners and arbitrary items remain ineligible inside a raid too.
    ai(survivor);
    assert!(!raider.is_patrol_leader());
    assert!(!banner.get_entity().removed.load(Relaxed));
    banner.get_item_entity().unwrap().set_pickup_delay(0);
    let equipment_guard = survivor
        .get_living_entity()
        .unwrap()
        .entity_equipment
        .lock()
        .unwrap();
    ai(survivor);
    assert!(!banner.get_entity().removed.load(Relaxed));
    assert!(!raider.is_patrol_leader());
    drop(equipment_guard);
    ai(survivor);
    assert!(raider.is_patrol_leader());
    assert!(banner.get_entity().removed.load(Relaxed));
    assert_eq!(fixture.bottles().len(), 1);
    assert!(
        fixture.kill(survivor).is_empty(),
        "positional raid still suppresses successor bottle"
    );
}

fn loaded_captain(fixture: &Fixture, raid_id: i32) -> Arc<dyn EntityBase> {
    let source = fixture.pillager(true);
    let mut saved = NbtCompound::new();
    source.write_nbt(&mut saved);
    saved.child_tags.remove("drop_chances");
    saved.put_int("RaidId", raid_id);
    saved.put_int("Wave", 2);
    let loaded = fixture.pillager(false);
    loaded.read_nbt_non_mut(&saved);
    loaded
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g01_r2_saved_raid_membership_death_order() {
    Fixture::new()
        .verify(|fixture| {
            let orphan = loaded_captain(fixture, 7);
            assert_eq!(
                fixture.kill(&orphan).len(),
                1,
                "orphaned saved RaidId is not membership"
            );
            fixture.world.raids.lock().unwrap().raid_map.insert(
                7,
                Raid::new(7, BlockPos::new(1000, 64, 1000), Difficulty::Normal),
            );
            let remote = loaded_captain(fixture, 7);
            let raids = fixture.world.raids.lock().unwrap();
            assert!(
                raids.get(7).unwrap().group_raider_map[&2]
                    .contains(&remote.get_entity().entity_uuid)
            );
            drop(raids);
            assert_eq!(
                fixture.kill(&remote).len(),
                1,
                "Raider.die removes remote membership before loot"
            );
            assert_eq!(
                remote
                    .get_mob()
                    .unwrap()
                    .as_raider()
                    .unwrap()
                    .get_raider_data()
                    .raid_id
                    .load(),
                None
            );
            assert!(
                !fixture
                    .world
                    .raids
                    .lock()
                    .unwrap()
                    .get(7)
                    .unwrap()
                    .group_raider_map[&2]
                    .contains(&remote.get_entity().entity_uuid)
            );
            fixture
                .world
                .raids
                .lock()
                .unwrap()
                .raid_map
                .insert(7, Raid::new(7, BlockPos::new(8, 64, 8), Difficulty::Normal));
            assert!(
                fixture.kill(&loaded_captain(fixture, 7)).is_empty(),
                "raid at the position still suppresses bottle"
            );
        })
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g01_r2_raider_predicate_defaults_death() {
    Fixture::new().verify(|fixture| {
        for (fields, expected) in [(json!({}), [1,0,0]), (json!({"has_raid":false,"is_captain":false}), [1,0,0]), (json!({"has_raid":false,"is_captain":true}), [0,1,0])] {
            let table = json!({"type":"minecraft:entity", "pools":[{"rolls":1,"condition":{"type":"minecraft:entity_properties","entity":"this","predicate":{"minecraft:type_specific/raider":fields}},"entries":[{"type":"minecraft:item","name":"minecraft:emerald"}]}]});
            fixture.install("pillager", &table);
            fixture.install("cow", &table);
            let count = || fixture.world.entities.load().iter().filter_map(|e| e.get_item_entity()).map(|i| i.get_item_stack().lock().unwrap().clone()).filter(|s| s.item == &Item::EMERALD).map(|s| usize::from(s.item_count)).sum::<usize>();
            for (victim, expected) in [fixture.pillager(false), fixture.pillager(true), fixture.mob(&EntityType::COW)].iter().zip(expected) {
                let before = count();
                fixture.kill(victim);
                assert_eq!(count() - before, expected, "codec fields={fields}, type={:?}", victim.get_entity().entity_type);
            }
        }
    }).await;
}
