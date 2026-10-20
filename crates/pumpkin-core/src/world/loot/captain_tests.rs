mod review;

use crate::{
    entity::{
        EntityBase,
        mob::{raider::create_ominous_banner, spawn::finalize_spawn},
    },
    server::{Server, combat_test_support},
    world::{World, raid::Raid},
};
use pumpkin_data::{
    damage::DamageType,
    data_component_impl::{
        CustomNameImpl, DataComponentImpl, EquipmentSlot, ItemNameImpl, OminousBottleAmplifierImpl,
    },
    entity::EntityType,
    item::Item,
    item_stack::ItemStack,
};
use pumpkin_util::{
    identifier::Identifier,
    math::{position::BlockPos, vector2::Vector2, vector3::Vector3},
};
use serde_json::json;
use std::{
    any::Any,
    sync::{
        Arc, Weak,
        atomic::{AtomicUsize, Ordering::Relaxed},
    },
    time::Duration,
};

struct Fixture {
    server: Arc<Server>,
    world: Arc<World>,
    _directory: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        let world = combat_test_support::world(&server, directory.path());
        server.worlds.store(Arc::new(vec![world.clone()]));
        combat_test_support::publish_empty_chunk(&world, Vector2::new(0, 0));
        assert!(matches!(
            world.get_loot_table("minecraft:entities/pillager"),
            Some(super::LootTableHandle::Static(table))
                if std::ptr::eq(table, &raw const pumpkin_data::loot_table::ENTITIES_PILLAGER)
        ));
        Self {
            server,
            world,
            _directory: directory,
        }
    }

    fn pillager(&self, captain: bool) -> Arc<dyn EntityBase> {
        let entity = self.mob(&EntityType::PILLAGER);
        let patrol = entity.get_mob().unwrap().as_patrolling_monster().unwrap();
        patrol.set_patrol_leader(captain);
        // The existing patrol finalizer is separate from generic mob finalization on this base.
        patrol.finalize_patrol_spawn(true);
        entity
    }

    fn mob(&self, kind: &'static EntityType) -> Arc<dyn EntityBase> {
        let entity = crate::entity::r#type::from_type(
            kind,
            Vector3::new(8.0, 64.0, 8.0),
            &self.world,
            uuid::Uuid::new_v4(),
        );
        finalize_spawn(&entity, &self.world, None);
        self.world.add_entity_silent(entity.clone());
        entity
    }

    fn install(&self, kind: &str, table: &serde_json::Value) {
        let mut unsupported = std::collections::BTreeSet::new();
        let table =
            pumpkin_util::loot_table::parse_loot_table(&table.to_string(), &mut unsupported)
                .unwrap();
        self.server
            .datapack_manager
            .insert_loot_table(format!("minecraft:entities/{kind}"), Arc::new(table));
    }

    fn bottles(&self) -> Vec<ItemStack> {
        self.world
            .entities
            .load()
            .iter()
            .filter_map(|entity| entity.get_item_entity())
            .map(|item| item.get_item_stack().lock().unwrap().clone())
            .filter(|stack| stack.item == &Item::OMINOUS_BOTTLE)
            .collect()
    }

    fn kill(&self, entity: &Arc<dyn EntityBase>) -> Vec<ItemStack> {
        let before = self.bottles().len();
        assert!(entity.damage_with_context(
            entity.as_ref(),
            f32::MAX,
            DamageType::GENERIC_KILL,
            None,
            None,
            None,
        ));
        assert!(
            entity
                .get_living_entity()
                .unwrap()
                .dead
                .load(std::sync::atomic::Ordering::Relaxed)
        );
        self.bottles().into_iter().skip(before).collect()
    }

    fn stream(&self) -> [i64; 2] {
        self.server
            .random_sequences
            .lock()
            .unwrap()
            .get_or_create(
                &Identifier::parse("minecraft:entities/pillager").unwrap(),
                0,
            )
            .random()
            .state()
    }

    async fn finish(self) {
        tokio::time::timeout(Duration::from_secs(15), self.world.level.shutdown())
            .await
            .unwrap()
            .unwrap();
        self.world.players.store(Arc::new(Vec::new()));
        self.world.entities.store(Arc::new(Vec::new()));
        self.world.entity_tracker.entity_map.clear();
        self.world.level.world_portal.store(Arc::new(None));
        self.server.worlds.store(Arc::new(Vec::new()));
    }

    async fn verify(self, check: impl FnOnce(&Self)) {
        // Negative controls must drain the fixture even when the intended assertion fails.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| check(&self)));
        self.finish().await;
        if let Err(payload) = result {
            std::panic::resume_unwind(payload);
        }
    }
}

fn amplifier(stack: &ItemStack) -> i32 {
    stack
        .get_data_component::<OminousBottleAmplifierImpl>()
        .unwrap()
        .amplifier
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g01_patrol_captain_death_emits_bottle() {
    Fixture::new()
        .verify(|fixture| {
            check_captain_death(fixture);
            check_equality_guards(fixture);
        })
        .await;
    super::captain_raid_tick_tests::check_raid_tick_death().await;
}

fn check_captain_death(fixture: &Fixture) {
    let initial_stream = fixture.stream();
    let noncaptain = fixture.pillager(false);
    noncaptain
        .get_living_entity()
        .unwrap()
        .entity_equipment
        .lock()
        .unwrap()
        .put(&EquipmentSlot::HEAD, create_ominous_banner());
    assert!(
        fixture.kill(&noncaptain).is_empty(),
        "banner alone is not a captain"
    );

    for replacement in [ItemStack::new(1, &Item::WHITE_BANNER), {
        let mut banner = create_ominous_banner();
        banner.item_count = 2;
        banner
    }] {
        let impostor = fixture.pillager(true);
        impostor
            .get_living_entity()
            .unwrap()
            .entity_equipment
            .lock()
            .unwrap()
            .put(&EquipmentSlot::HEAD, replacement);
        assert!(
            fixture.kill(&impostor).is_empty(),
            "captain head must match the canonical stack"
        );
    }

    fixture.server.level_info.rcu(|info| {
        let mut info = (**info).clone();
        info.game_rules.mob_drops = false;
        info
    });
    assert!(
        fixture.kill(&fixture.pillager(true)).is_empty(),
        "doMobLoot=false"
    );
    fixture.server.level_info.rcu(|info| {
        let mut info = (**info).clone();
        info.game_rules.mob_drops = true;
        info
    });

    fixture.world.raids.lock().unwrap().raid_map.insert(
        8,
        Raid::new(
            8,
            BlockPos::new(8, 64, 8),
            pumpkin_util::difficulty::Difficulty::Normal,
        ),
    );
    assert!(
        fixture.kill(&fixture.pillager(true)).is_empty(),
        "a raid at the position suppresses an unjoined captain's bottle"
    );
    fixture.world.raids.lock().unwrap().raid_map.clear();
    assert_eq!(
        fixture.stream(),
        initial_stream,
        "ineligible deaths consume no bottle draw"
    );

    let bottles = fixture.kill(&fixture.pillager(true));
    assert_eq!(bottles.len(), 1, "outside-raid captain bottle");
    assert_eq!(bottles[0].item_count, 1);
    assert!((0..=4).contains(&amplifier(&bottles[0])));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g01_bottle_amplifier_uses_named_loot_rng() {
    Fixture::new().verify(check_amplifier).await;
}

fn check_amplifier(fixture: &Fixture) {
    let mut table: serde_json::Value =
        serde_json::from_str(pumpkin_data::loot_table::ENTITIES_PILLAGER.json()).unwrap();
    // Isolate modifier/RNG behavior from the separately tested raider predicate gate.
    table["pools"][0]
        .as_object_mut()
        .unwrap()
        .remove("condition");
    fixture.install("pillager", &table);
    assert_eq!(
        fixture.stream(),
        [-676_395_180_996_242_682, 3_572_350_221_058_679_314]
    );
    let mut recorded = Vec::new();
    for _ in 0..8 {
        let bottles = fixture.kill(&fixture.pillager(true));
        assert_eq!(bottles.len(), 1);
        assert_eq!(bottles[0].item_count, 1);
        recorded.push(amplifier(&bottles[0]));
    }
    let after_named = fixture.stream();
    let mut endpoints = Vec::new();
    for provider in [
        json!({"type":"minecraft:constant","value":-9}),
        json!({"type":"minecraft:uniform","min":12,"max":12}),
    ] {
        table["pools"][0]["entries"][0]["modifier"] = json!([
            {"type":"minecraft:set_count","count":2},
            {"type":"minecraft:set_ominous_bottle_amplifier","amplifier":provider},
            {"type":"minecraft:set_count","count":3}
        ]);
        fixture.install("pillager", &table);
        let bottles = fixture.kill(&fixture.pillager(true));
        assert_eq!(bottles.len(), 1);
        assert_eq!(
            bottles[0].item_count, 3,
            "ordered count modifiers remain active"
        );
        endpoints.push(amplifier(&bottles[0]));
    }
    assert_eq!(
        fixture.stream(),
        after_named,
        "constant endpoints consume no random draw"
    );
    assert_eq!(
        recorded,
        [2, 3, 2, 4, 4, 0, 1, 4],
        "recorded named pillager stream amplifiers"
    );
    assert_eq!(
        after_named,
        [-2_292_644_840_545_207_373, -5_111_486_850_812_869_335]
    );
    assert_eq!(endpoints, [0, 4], "custom providers are clamped");
}

#[derive(Clone)]
struct EqualityProbe<T> {
    inner: T,
    entity: Weak<dyn EntityBase>,
    calls: Arc<AtomicUsize>,
}

impl<T: DataComponentImpl + Clone + 'static> DataComponentImpl for EqualityProbe<T> {
    fn write_data(&self) -> pumpkin_nbt::tag::NbtTag {
        self.inner.write_data()
    }

    fn get_hash(&self) -> i32 {
        self.inner.get_hash()
    }

    fn equal(&self, other: &dyn DataComponentImpl) -> bool {
        let entity = self.entity.upgrade().unwrap();
        let living = entity.get_living_entity().unwrap();
        assert!(
            living.entity_equipment.try_lock().is_ok(),
            "equality held equipment"
        );
        assert!(
            living.equipment_drop_chances.try_lock().is_ok(),
            "equality held drop chances"
        );
        assert!(
            entity.get_entity().world.load().raids.try_lock().is_ok(),
            "equality held raids"
        );
        self.calls.fetch_add(1, Relaxed);
        self.inner.equal(other)
    }

    fn get_enum() -> pumpkin_data::data_component::DataComponent {
        T::get_enum()
    }

    fn get_self_enum(&self) -> pumpkin_data::data_component::DataComponent {
        self.inner.get_self_enum()
    }

    fn to_dyn(self) -> Box<dyn DataComponentImpl> {
        Box::new(self)
    }

    fn clone_dyn(&self) -> Box<dyn DataComponentImpl> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn Any {
        self.inner.as_any()
    }

    fn as_mut_any(&mut self) -> &mut dyn Any {
        self.inner.as_mut_any()
    }
}

fn check_equality_guards(fixture: &Fixture) {
    let captain = fixture.pillager(true);
    let calls = Arc::new(AtomicUsize::new(0));
    let mut head = create_ominous_banner();
    if head.get_data_component::<CustomNameImpl>().is_some() {
        probe_name::<CustomNameImpl>(&mut head, &captain, &calls);
    } else {
        probe_name::<ItemNameImpl>(&mut head, &captain, &calls);
    }
    captain
        .get_living_entity()
        .unwrap()
        .entity_equipment
        .lock()
        .unwrap()
        .put(&EquipmentSlot::HEAD, head);
    assert_eq!(fixture.kill(&captain).len(), 1);
    assert!(
        calls.load(Relaxed) > 0,
        "captain identity compared component values"
    );
}

fn probe_name<T: DataComponentImpl + Clone + 'static>(
    head: &mut ItemStack,
    captain: &Arc<dyn EntityBase>,
    calls: &Arc<AtomicUsize>,
) {
    let component = head.get_data_component::<T>().unwrap().clone();
    head.set_data_component(EqualityProbe {
        inner: component,
        entity: Arc::downgrade(captain),
        calls: calls.clone(),
    });
}
