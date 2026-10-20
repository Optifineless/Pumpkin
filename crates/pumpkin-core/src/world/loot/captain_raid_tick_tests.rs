#![expect(
    clippy::unwrap_used,
    reason = "Real raid/death fixtures require valid entities, packets and locks"
)]

use crate::{
    entity::{EntityBase, mob::spawn::finalize_spawn, r#type::from_type},
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::entity::entity_spawn::EntitySpawnEvent, server::packet::PacketSentEvent,
    },
    server::{Server, combat_test_support},
    world::{World, raid::Raid},
};
use pumpkin_data::{
    damage::DamageType, data_component_impl::OminousBottleAmplifierImpl, entity::EntityType,
    item::Item, item_stack::ItemStack, packet::clientbound::play::BOSS_EVENT,
};
use pumpkin_protocol::ser::NetworkReadExt;
use pumpkin_util::{
    difficulty::Difficulty,
    math::{position::BlockPos, vector2::Vector2, vector3::Vector3},
    version::JavaMinecraftVersion,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering::Relaxed},
    },
    time::Duration,
};

struct Fixture {
    server: Arc<Server>,
    world: Arc<World>,
    client: TestPlayer,
    _directory: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        let world = combat_test_support::world(&server, directory.path());
        server.worlds.store(Arc::new(vec![world.clone()]));
        combat_test_support::publish_empty_chunk(&world, Vector2::new(0, 0));
        combat_test_support::publish_empty_chunk(&world, Vector2::new(4, 4));
        let client = TestPlayer::new(&world);
        client.client().set_player(client.player.clone());
        // The native transport's PacketSentEvent observes the actual raid-stop REMOVE.
        client
            .client()
            .version
            .store(JavaMinecraftVersion::V_1_21_11);
        assert!(matches!(
            world.get_loot_table("minecraft:entities/pillager"),
            Some(super::LootTableHandle::Static(table))
                if std::ptr::eq(table, &raw const pumpkin_data::loot_table::ENTITIES_PILLAGER)
        ));
        Self {
            server,
            world,
            client,
            _directory: directory,
        }
    }

    fn captain(&self) -> Arc<dyn EntityBase> {
        let entity = from_type(
            &EntityType::PILLAGER,
            Vector3::new(8.0, 64.0, 8.0),
            &self.world,
            uuid::Uuid::new_v4(),
        );
        finalize_spawn(&entity, &self.world, None);
        let patrol = entity.get_mob().unwrap().as_patrolling_monster().unwrap();
        patrol.set_patrol_leader(true);
        patrol.finalize_patrol_spawn(true);
        self.world.add_entity_silent(entity.clone());
        entity
    }

    async fn verify(self, check: impl FnOnce(&Self)) {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| check(&self)));
        tokio::time::timeout(Duration::from_secs(15), self.world.level.shutdown())
            .await
            .unwrap()
            .unwrap();
        self.world.entities.store(Arc::new(Vec::new()));
        self.world.entity_tracker.entity_map.clear();
        self.world.level.world_portal.store(Arc::new(None));
        self.server.worlds.store(Arc::new(Vec::new()));
        if let Err(payload) = result {
            std::panic::resume_unwind(payload);
        }
    }
}

fn bottles(world: &World) -> Vec<ItemStack> {
    world
        .entities
        .load()
        .iter()
        .filter_map(|entity| entity.get_item_entity())
        .map(|item| item.get_item_stack().lock().unwrap().clone())
        .filter(|stack| stack.item == &Item::OMINOUS_BOTTLE)
        .collect()
}

struct KillDuringRaidCallbacks {
    active_captain: Arc<dyn EntityBase>,
    stopped_captain: Arc<dyn EntityBase>,
    raid_bossbar: uuid::Uuid,
    stage: AtomicUsize,
}

impl KillDuringRaidCallbacks {
    fn kill(victim: &Arc<dyn EntityBase>, world: &World) {
        let living = victim.get_living_entity().unwrap();
        let raids = world.raids.try_lock();
        assert!(raids.is_ok(), "raid guard spans callback");
        let raids = raids.unwrap();
        assert!(
            raids.get(7).is_none(),
            "callback must run while the tick owns raid 7"
        );
        drop(raids);
        assert!(
            living.entity_equipment.try_lock().is_ok(),
            "equipment guard spans callback"
        );
        assert!(
            living.equipment_drop_chances.try_lock().is_ok(),
            "drop chance guard spans callback"
        );
        assert!(
            !victim
                .get_mob()
                .unwrap()
                .as_raider()
                .unwrap()
                .has_active_raid(),
            "callback victim must be unjoined so the position lookup decides eligibility"
        );
        assert!(victim.damage_with_context(
            victim.as_ref(),
            f32::MAX,
            DamageType::GENERIC_KILL,
            None,
            None,
            None,
        ));
        assert!(living.dead.load(Relaxed));
    }
}

impl EventHandler<EntitySpawnEvent> for KillDuringRaidCallbacks {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut EntitySpawnEvent,
    ) -> BoxFuture<'a, ()> {
        if event.entity_type == EntityType::PILLAGER.id.to_string()
            && self.stage.compare_exchange(0, 1, Relaxed, Relaxed).is_ok()
        {
            // This callback runs inside the real Raid::spawn_group before tick state returns.
            Self::kill(&self.active_captain, &event.world);
            assert!(
                bottles(&event.world).is_empty(),
                "active raid during wave-spawn callback suppresses unjoined captain bottle"
            );
            let mut raids = event.world.raids.lock().unwrap();
            raids.raid_map.insert(
                8,
                Raid::new(8, BlockPos::new(256, 64, 256), Difficulty::Normal),
            );
            raids.next_id = 8;
        }
        Box::pin(async {})
    }
}

impl EventHandler<PacketSentEvent> for KillDuringRaidCallbacks {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut PacketSentEvent,
    ) -> BoxFuture<'a, ()> {
        if event.packet_id != BOSS_EVENT.0 {
            return Box::pin(async {});
        }
        let mut payload = event.payload.as_ref();
        let uuid = payload.get_uuid().unwrap();
        let action = payload.get_var_int().unwrap().0;
        if uuid == self.raid_bossbar
            && action == 1
            && self.stage.compare_exchange(1, 2, Relaxed, Relaxed).is_ok()
        {
            // Raid.stop must clear active visibility before removing its player's bar.
            let world = self.stopped_captain.get_entity().world.load_full();
            Self::kill(&self.stopped_captain, &world);
            let bottles = bottles(&world);
            assert_eq!(
                bottles.len(),
                1,
                "stopped raid callback permits captain bottle"
            );
            let amplifier = bottles[0]
                .get_data_component::<OminousBottleAmplifierImpl>()
                .unwrap()
                .amplifier;
            assert!((0..=4).contains(&amplifier));
        }
        Box::pin(async {})
    }
}

pub(super) async fn check_raid_tick_death() {
    Fixture::new()
        .verify(|fixture| {
            let active_captain = fixture.captain();
            let stopped_captain = fixture.captain();
            let mut raid = Raid::new(7, BlockPos::new(8, 64, 8), Difficulty::Normal);
            raid.raid_cooldown_ticks = 0;
            // Keep spawned raiders outside the inherited banner goal's item-pickup range.
            raid.wave_spawn_pos = Some(BlockPos::new(64, 64, 64));
            raid.players_in_raid
                .insert(fixture.client.player.gameprofile.id);
            let callbacks = Arc::new(KillDuringRaidCallbacks {
                active_captain,
                stopped_captain,
                raid_bossbar: raid.bossbar.uuid,
                stage: AtomicUsize::new(0),
            });
            fixture.world.raids.lock().unwrap().raid_map.insert(7, raid);
            fixture
                .server
                .plugin_manager
                .register::<EntitySpawnEvent, _>(callbacks.clone(), EventPriority::Normal, true);
            fixture
                .server
                .plugin_manager
                .register::<PacketSentEvent, _>(callbacks.clone(), EventPriority::Normal, true);
            fixture.world.tick(&fixture.server);
            assert_eq!(
                callbacks.stage.load(Relaxed),
                1,
                "raid spawn callback must run once"
            );
            assert!(bottles(&fixture.world).is_empty());

            let mut raids = fixture.world.raids.lock().unwrap();
            assert!(
                raids.loot_tick_view.is_empty(),
                "raid tick projection must clear"
            );
            assert!(
                raids
                    .raid_map
                    .values()
                    .all(|raid| raid.loot_tick_active.is_none())
            );
            assert!(
                raids.get(8).is_some(),
                "callback-created raid survives tick merge"
            );
            assert_eq!(
                raids.next_id, 8,
                "callback-created raid ID progress survives tick merge"
            );
            let raid = raids.get_mut(7).unwrap();
            raid.ticks_active = 47_999;
            drop(raids);
            fixture.world.tick(&fixture.server);
            assert_eq!(
                callbacks.stage.load(Relaxed),
                2,
                "raid stop REMOVE callback must run once"
            );
            assert!(fixture.world.raids.lock().unwrap().get(7).is_none());
            assert!(
                fixture
                    .world
                    .raids
                    .lock()
                    .unwrap()
                    .loot_tick_view
                    .is_empty()
            );
            assert_eq!(bottles(&fixture.world).len(), 1);
        })
        .await;
}

struct LoadMemberDuringRaidTick {
    captain: Arc<dyn EntityBase>,
    saved: pumpkin_nbt::compound::NbtCompound,
    bossbar: uuid::Uuid,
    calls: AtomicUsize,
}

impl EventHandler<PacketSentEvent> for LoadMemberDuringRaidTick {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut PacketSentEvent,
    ) -> BoxFuture<'a, ()> {
        if event.packet_id == BOSS_EVENT.0 {
            let mut payload = event.payload.as_ref();
            let uuid = payload.get_uuid().unwrap();
            let action = payload.get_var_int().unwrap().0;
            if uuid == self.bossbar
                && action == 2
                && self.calls.compare_exchange(0, 1, Relaxed, Relaxed).is_ok()
            {
                let world = self.captain.get_entity().world.load_full();
                assert!(world.raids.try_lock().is_ok());
                assert!(world.raids.lock().unwrap().get(7).is_none());
                self.captain.read_nbt_non_mut(&self.saved);
            }
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g01_r2_tick_callback_load_precedes_victory_and_death() {
    Fixture::new()
        .verify(|fixture| {
            let captain = fixture.captain();
            captain.get_mob().unwrap().get_mob_entity().set_no_ai(true);
            let mut saved = pumpkin_nbt::compound::NbtCompound::new();
            captain.write_nbt(&mut saved);
            saved.put_int("RaidId", 7);
            saved.put_int("Wave", 2);
            saved.child_tags.remove("drop_chances");
            let mut raid = Raid::new(7, BlockPos::new(8, 64, 8), Difficulty::Normal);
            raid.started = true;
            raid.groups_spawned = raid.num_groups;
            raid.ticks_active = 19;
            raid.post_raid_ticks = 40;
            raid.players_in_raid
                .insert(fixture.client.player.gameprofile.id);
            let callback = Arc::new(LoadMemberDuringRaidTick {
                captain: captain.clone(),
                saved,
                bossbar: raid.bossbar.uuid,
                calls: AtomicUsize::new(0),
            });
            fixture.world.raids.lock().unwrap().raid_map.insert(7, raid);
            fixture
                .server
                .plugin_manager
                .register::<PacketSentEvent, _>(callback.clone(), EventPriority::Normal, true);
            fixture.world.tick(&fixture.server);
            assert_eq!(callback.calls.load(Relaxed), 1);
            let raids = fixture.world.raids.lock().unwrap();
            let raid = raids.get(7).unwrap();
            assert_eq!(
                raid.status,
                crate::world::raid::RaidStatus::Ongoing,
                "loaded living member prevents victory"
            );
            assert!(raid.group_raider_map[&2].contains(&captain.get_entity().entity_uuid));
            drop(raids);
            assert_eq!(
                captain
                    .get_living_entity()
                    .unwrap()
                    .equipment_drop_chances
                    .lock()
                    .unwrap()[&pumpkin_data::data_component_impl::EquipmentSlot::HEAD],
                2.0,
                "Raid.setLeader restores old-save guarantee"
            );
            captain
                .get_entity()
                .set_pos(Vector3::new(1000.0, 64.0, 1000.0));
            assert!(captain.damage_with_context(
                captain.as_ref(),
                f32::MAX,
                DamageType::GENERIC_KILL,
                None,
                None,
                None
            ));
            assert_eq!(
                bottles(&fixture.world).len(),
                1,
                "death removes callback-loaded membership before loot"
            );
        })
        .await;
}
