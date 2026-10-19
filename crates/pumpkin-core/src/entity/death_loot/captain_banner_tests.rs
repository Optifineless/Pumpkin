#![expect(
    clippy::unwrap_used,
    reason = "Regression fixtures require valid entities, components and locks"
)]

use super::*;
use crate::{
    entity::{
        ai::goal::Goal,
        mob::{
            equipment::DEFAULT_EQUIPMENT_DROP_CHANCE,
            patrol::PatrollingMonster,
            raider::{ObtainRaidLeaderBannerGoal, create_ominous_banner},
        },
        r#type::from_type,
    },
    net::java::combat_test_support::TestPlayer,
    server::{Server, combat_test_support},
    world::{World, raid::Raid},
};
use pumpkin_data::item::Item;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::{
    difficulty::Difficulty,
    math::{position::BlockPos, vector2::Vector2},
};
use std::{cell::RefCell, sync::Arc, time::Duration};

thread_local! {
    static EQUIPMENT_DRAW: RefCell<Option<(f32, usize)>> = const { RefCell::new(None) };
}

pub(super) fn equipment_draw(default: f32) -> f32 {
    EQUIPMENT_DRAW.with(|fixture| {
        let mut fixture = fixture.borrow_mut();
        let Some((draw, remaining)) = fixture.as_mut() else {
            return default;
        };
        assert!(
            *remaining > 0,
            "equipment consumed an unexpected extra draw"
        );
        *remaining -= 1;
        *draw
    })
}

struct EquipmentDraw;

impl EquipmentDraw {
    fn run(expected_draws: usize, action: impl FnOnce()) {
        // Recorded literal 0.5 fails the ordinary 0.085 roll and passes the captain's 2.0.
        assert!(
            EQUIPMENT_DRAW.with(|fixture| fixture.replace(Some((0.5, expected_draws))).is_none())
        );
        let _scope = Self;
        action();
        EQUIPMENT_DRAW.with(|fixture| {
            assert_eq!(fixture.borrow().as_ref().unwrap().1, 0);
        });
    }
}

impl Drop for EquipmentDraw {
    fn drop(&mut self) {
        EQUIPMENT_DRAW.with(RefCell::take);
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    server: Arc<Server>,
    world: Arc<World>,
    client: TestPlayer,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(dir.path());
        let world = combat_test_support::world(&server, dir.path());
        server.worlds.store(Arc::new(vec![world.clone()]));
        combat_test_support::publish_empty_chunk(&world, Vector2::new(0, 0));
        let client = TestPlayer::new(&world);
        Self {
            _dir: dir,
            server,
            world,
            client,
        }
    }

    fn pillager(&self, x: f64) -> Arc<dyn EntityBase> {
        from_type(
            &EntityType::PILLAGER,
            Vector3::new(x, 64.0, 0.0),
            &self.world,
            uuid::Uuid::new_v4(),
        )
    }

    fn patrol_captain(&self) -> Arc<dyn EntityBase> {
        let entity = self.pillager(0.0);
        let patrol = entity.get_mob().unwrap().as_patrolling_monster().unwrap();
        patrol.set_patrol_leader(true);
        patrol.finalize_patrol_spawn(true);
        entity
    }

    fn banner_count(&self) -> u32 {
        self.world
            .entities
            .load()
            .iter()
            .filter_map(|entity| entity.get_item_entity())
            .map(|item| item.get_item_stack().lock().unwrap().clone())
            .filter(|stack| stack.item.id == Item::WHITE_BANNER.id)
            .map(|stack| u32::from(stack.item_count))
            .sum()
    }

    fn kill(&self, victim: &Arc<dyn EntityBase>, player_credit: bool, expected_draws: usize) {
        // Isolate the head roll while retaining the real damage, death and item-spawn paths.
        let mob = victim.get_mob().unwrap().get_mob_entity();
        for slot in equipment_slots_in_vanilla_order() {
            if slot != EquipmentSlot::HEAD {
                mob.set_item_slot(&slot, ItemStack::EMPTY.clone());
            }
        }
        let player = self.client.player.as_ref() as &dyn EntityBase;
        EquipmentDraw::run(expected_draws, || {
            assert!(victim.damage_with_context(
                victim.as_ref(),
                f32::MAX,
                DamageType::GENERIC_KILL,
                None,
                player_credit.then_some(player),
                player_credit.then_some(player),
            ));
        });
        assert!(victim.get_living_entity().unwrap().dead.load(Relaxed));
    }

    fn assert_captain_drop(&self, victim: &Arc<dyn EntityBase>, label: &str) {
        let before = self.banner_count();
        self.kill(victim, true, 1);
        assert_eq!(
            self.banner_count() - before,
            1,
            "{label} captain head banner"
        );
        assert_eq!(
            victim
                .get_mob()
                .unwrap()
                .get_mob_entity()
                .drop_chance(&EquipmentSlot::HEAD),
            2.0,
            "{label} captain head chance"
        );
    }

    async fn finish(self) {
        tokio::time::timeout(Duration::from_secs(15), self.world.level.shutdown())
            .await
            .unwrap()
            .unwrap();
        self.world.entities.store(Arc::new(Vec::new()));
        self.world.entity_tracker.entity_map.clear();
        self.world.level.world_portal.store(Arc::new(None));
        self.server.worlds.store(Arc::new(Vec::new()));
    }

    async fn verify(self, action: impl FnOnce(&Self)) {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| action(&self)));
        self.finish().await;
        if let Err(panic) = result {
            std::panic::resume_unwind(panic);
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g01_captain_banner_drop_is_guaranteed() {
    let fixture = Fixture::new();
    fixture
        .verify(|fixture| {
            let captain = fixture.patrol_captain();
            fixture.world.add_entity_silent(captain.clone());
            fixture.assert_captain_drop(&captain, "patrol finalization");

            let mut raid = Raid::new(1, BlockPos::new(0, 64, 0), Difficulty::Easy);
            raid.spawn_group(&fixture.world, BlockPos::new(0, 64, 0));
            let leader_uuid = raid.group_to_leader_map[&1];
            let leader = fixture
                .world
                .entities
                .load()
                .iter()
                .find(|entity| entity.get_entity().entity_uuid == leader_uuid)
                .unwrap()
                .clone();
            fixture.assert_captain_drop(&leader, "raid-wave assignment");

            let replacement = fixture.pillager(64.0);
            fixture.world.add_entity_silent(replacement.clone());
            fixture
                .world
                .drop_stack(&BlockPos::new(64, 64, 0), create_ominous_banner());
            ObtainRaidLeaderBannerGoal.tick(replacement.get_mob().unwrap());
            assert!(
                replacement
                    .get_mob()
                    .unwrap()
                    .as_raider()
                    .unwrap()
                    .is_patrol_leader()
            );
            fixture.assert_captain_drop(&replacement, "replacement leadership");

            fixture
                .client
                .player
                .permission_lvl
                .store(pumpkin_util::PermissionLvl::Four);
            let source = crate::command::CommandSender::Player(fixture.client.player.clone())
                .into_source(&fixture.server);
            assert!(
                fixture
                    .server
                    .command_dispatcher
                    .load()
                    .execute_input("raid spawnleader", &source)
                    .is_ok()
            );
            let command_captain = fixture
                .world
                .entities
                .load()
                .iter()
                .find(|entity| {
                    entity
                        .get_mob()
                        .and_then(|mob| mob.as_raider())
                        .is_some_and(PatrollingMonster::is_patrol_leader)
                        && entity
                            .get_living_entity()
                            .is_some_and(|living| !living.dead.load(Relaxed))
                })
                .unwrap()
                .clone();
            fixture.assert_captain_drop(&command_captain, "raid spawnleader command");

            let uncredited = fixture.patrol_captain();
            fixture.world.add_entity_silent(uncredited.clone());
            let before = fixture.banner_count();
            fixture.kill(&uncredited, false, 1);
            assert_eq!(
                fixture.banner_count() - before,
                1,
                "preserved captain gear permits uncredited death"
            );
        })
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g01_ordinary_head_chance_and_drop_gates_are_preserved() {
    let fixture = Fixture::new();
    fixture
        .verify(|fixture| {
            let ordinary = fixture.pillager(0.0);
            let mob = ordinary.get_mob().unwrap().get_mob_entity();
            mob.set_item_slot(&EquipmentSlot::HEAD, create_ominous_banner());
            assert_eq!(
                mob.drop_chance(&EquipmentSlot::HEAD),
                DEFAULT_EQUIPMENT_DROP_CHANCE
            );
            fixture.world.add_entity_silent(ordinary.clone());
            fixture.kill(&ordinary, true, 1);
            assert_eq!(
                fixture.banner_count(),
                0,
                "ordinary head gear obeys literal 0.5"
            );
            assert_eq!(
                mob.drop_chance(&EquipmentSlot::HEAD),
                DEFAULT_EQUIPMENT_DROP_CHANCE
            );

            let uncredited = fixture.pillager(0.0);
            uncredited
                .get_mob()
                .unwrap()
                .get_mob_entity()
                .set_item_slot(&EquipmentSlot::HEAD, create_ominous_banner());
            fixture.world.add_entity_silent(uncredited.clone());
            fixture.kill(&uncredited, false, 0);
            assert_eq!(
                fixture.banner_count(),
                0,
                "ordinary head gear needs player credit"
            );

            fixture.world.level_info.rcu(|info| {
                let mut info = (**info).clone();
                info.game_rules.mob_drops = false;
                info
            });
            let disabled = fixture.patrol_captain();
            fixture.world.add_entity_silent(disabled.clone());
            fixture.kill(&disabled, true, 0);
            assert_eq!(
                fixture.banner_count(),
                0,
                "doMobLoot still gates captain equipment"
            );
        })
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g01_captain_drop_chance_keeps_existing_saved_semantics() {
    let fixture = Fixture::new();
    fixture
        .verify(|fixture| {
            let captain = fixture.patrol_captain();
            let mut saved = NbtCompound::new();
            captain.write_nbt(&mut saved);
            assert_eq!(
                saved
                    .get_compound("drop_chances")
                    .and_then(|chances| chances.get_float("head")),
                Some(2.0),
                "saved captain head chance must preserve the guaranteed banner drop"
            );
            let restored = fixture.pillager(0.0);
            restored.read_nbt_non_mut(&saved);
            assert_eq!(
                restored
                    .get_mob()
                    .unwrap()
                    .get_mob_entity()
                    .drop_chance(&EquipmentSlot::HEAD),
                2.0
            );
            fixture.world.add_entity_silent(restored.clone());
            fixture.assert_captain_drop(&restored, "saved/reloaded captain");

            for (chance, expected) in [(None, DEFAULT_EQUIPMENT_DROP_CHANCE), (Some(0.0), 0.0)] {
                let mut old_save = saved.clone();
                old_save.child_tags.remove("drop_chances");
                if let Some(chance) = chance {
                    let mut chances = NbtCompound::new();
                    chances.put_float("head", chance);
                    old_save.put_compound("drop_chances", chances);
                }
                let old_captain = fixture.pillager(0.0);
                old_captain.read_nbt_non_mut(&old_save);
                assert!(
                    old_captain
                        .get_mob()
                        .unwrap()
                        .as_raider()
                        .unwrap()
                        .is_patrol_leader()
                );
                assert_eq!(
                    old_captain
                        .get_mob()
                        .unwrap()
                        .get_mob_entity()
                        .drop_chance(&EquipmentSlot::HEAD),
                    expected
                );
            }
        })
        .await;
}

struct DropEvents {
    victim: Arc<dyn EntityBase>,
    seen: std::sync::Mutex<Vec<&'static str>>,
}

impl DropEvents {
    fn assert_unlocked(&self) {
        let living = self.victim.get_living_entity().unwrap();
        assert!(living.entity_equipment.try_lock().is_ok());
        assert!(living.equipment_drop_chances.try_lock().is_ok());
        assert!(
            self.victim
                .get_entity()
                .world
                .load()
                .raids
                .try_lock()
                .is_ok()
        );
    }
}

impl crate::plugin::EventHandler<crate::plugin::api::events::entity::entity_death::EntityDeathEvent>
    for DropEvents
{
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut crate::plugin::api::events::entity::entity_death::EntityDeathEvent,
    ) -> crate::plugin::BoxFuture<'a, ()> {
        assert_eq!(event.entity_id, self.victim.get_entity().entity_id);
        self.assert_unlocked();
        self.seen.lock().unwrap().push("death");
        Box::pin(async {})
    }
}

impl crate::plugin::EventHandler<crate::plugin::api::events::entity::item_spawn::ItemSpawnEvent>
    for DropEvents
{
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut crate::plugin::api::events::entity::item_spawn::ItemSpawnEvent,
    ) -> crate::plugin::BoxFuture<'a, ()> {
        self.assert_unlocked();
        if event.item_name == Item::WHITE_BANNER.registry_key {
            self.seen.lock().unwrap().push("banner");
            event.cancelled = true;
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g01_captain_banner_keeps_death_event_order_and_spawn_cancellation() {
    let fixture = Fixture::new();
    fixture
        .verify(|fixture| {
            let captain = fixture.patrol_captain();
            fixture.world.add_entity_silent(captain.clone());
            let events = Arc::new(DropEvents {
                victim: captain.clone(),
                seen: std::sync::Mutex::new(Vec::new()),
            });
            fixture
                .server
                .plugin_manager
                .register::<crate::plugin::api::events::entity::entity_death::EntityDeathEvent, _>(
                events.clone(),
                crate::plugin::EventPriority::Normal,
                true,
            );
            fixture
                .server
                .plugin_manager
                .register::<crate::plugin::api::events::entity::item_spawn::ItemSpawnEvent, _>(
                events.clone(),
                crate::plugin::EventPriority::Normal,
                true,
            );
            fixture.kill(&captain, true, 1);
            assert_eq!(events.seen.lock().unwrap().as_slice(), ["death", "banner"]);
            assert_eq!(
                fixture.banner_count(),
                0,
                "cancelled captain banner stays cancelled"
            );
        })
        .await;
}
