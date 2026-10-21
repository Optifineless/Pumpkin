use super::*;
use crate::plugin::{
    BoxFuture, EventHandler, EventPriority, api::events::world::generic_game::GenericGameEvent,
};
use pumpkin_config::AdvancedConfiguration;
use pumpkin_data::{
    attributes::Attributes, data_component_impl::EquipmentSlot, item::Item, item_stack::ItemStack,
};
use pumpkin_nbt::{Nbt, compound::NbtCompound};
use pumpkin_world::data::player_data::PlayerDataStorage;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

const PLAYER: &str = "00000000-0000-4000-8000-000001620003";
const COW: &str = "00000000-0000-4000-8000-000001620004";

fn config(movement: bool) -> AdvancedConfiguration {
    let mut config = AdvancedConfiguration::default();
    config.movement_game_events.enabled = movement;
    config
}

impl Scenario {
    async fn restart_with_movement(self, movement: bool) -> Self {
        self.stop().await;
        let Self {
            server, fixture, ..
        } = self;
        drop(server);
        let scenario = Self {
            server: Self::services_with_advanced(
                fixture.directory.path(),
                false,
                Some(config(movement)),
            ),
            fixture,
            sessions: Rc::default(),
            stopped: Rc::default(),
        };
        ownership::register(&scenario);
        scenario
    }
}

struct Count(Arc<AtomicUsize>);
impl EventHandler<GenericGameEvent> for Count {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut GenericGameEvent,
    ) -> BoxFuture<'a, ()> {
        if event.event_key == "step" {
            self.0.fetch_add(1, Relaxed);
        }
        Box::pin(async {})
    }
}

pub(super) fn capture(s: &Scenario) -> Arc<AtomicUsize> {
    let count = Arc::new(AtomicUsize::new(0));
    s.server.plugin_manager.register::<GenericGameEvent, _>(
        Arc::new(Count(count.clone())),
        EventPriority::Normal,
        true,
    );
    count
}

fn walking(s: &Scenario, player: &Session, dx: f64) {
    let pos = player.player.position() + Vector3::new(dx, 0.0, 0.0);
    player.send(
        &s.server,
        &pumpkin_protocol::java::server::play::SPlayerPosition {
            position: pos,
            collision: 1,
        },
    );
}

fn assert_player(player: &Session, expected: Vector3<f64>) {
    assert_eq!(player.player.position(), expected, "real saved Pos");
    assert_eq!(
        player.player.living_entity.health.load(),
        13.0,
        "real saved Health"
    );
    let stack = player.player.inventory.get_slot(4);
    assert_eq!(stack.item, &Item::EMERALD);
    assert_eq!(stack.item_count, 17, "real saved inventory count");
    assert_eq!(
        pumpkin_util::math::experience::points_to_level(
            player.player.experience_level.load(Relaxed)
        ) + player.player.experience_points.load(Relaxed),
        42,
        "real saved XP"
    );
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &to.join(entry.file_name()));
        } else {
            std::fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
        }
    }
}

fn export(s: &Scenario, cow: NbtCompound) {
    if let Ok(destination) = std::env::var("S3_COMPAT_EXPORT_DIR") {
        let dest = Path::new(&destination);
        copy_tree(s.directory.path(), &dest.join("world"));
        std::fs::write(
            dest.join("entity.nbt"),
            Nbt::new(String::new(), cow).write(),
        )
        .unwrap();
        std::fs::write(dest.join("manifest.json"), format!(
            "{{\"player_uuid\":\"{PLAYER}\",\"player_name\":\"MovementCompat\",\"entity_uuid\":\"{COW}\"}}\n")).unwrap();
    }
}

fn cow_record(s: &Scenario) -> NbtCompound {
    let world = s.world(&Dimension::OVERWORLD);
    let cow = crate::entity::r#type::from_type(
        &EntityType::COW,
        Vector3::new(10.5, 64.0, 10.5),
        &world,
        Uuid::parse_str(COW).unwrap(),
    );
    let entity = cow.get_entity();
    entity.velocity.store(Vector3::new(0.125, 0.0, -0.25));
    entity.on_ground.store(true, Relaxed);
    cow.get_mob().unwrap().get_mob_entity().set_no_ai(true);
    cow.get_mob().unwrap().as_ageable().unwrap().set_age(123);
    let living = cow.get_living_entity().unwrap();
    living.set_attribute_base(&Attributes::MAX_HEALTH, 30.0);
    living.health.store(7.0);
    let mut sword = ItemStack::new(1, &Item::DIAMOND_SWORD);
    sword.set_damage(7);
    living
        .entity_equipment
        .lock()
        .unwrap()
        .put(&EquipmentSlot::MAIN_HAND, sword);
    let mut nbt = NbtCompound::new();
    cow.write_nbt(&mut nbt);
    assert_eq!(nbt.get_float("Health"), Some(7.0));
    assert_eq!(nbt.get_int("Age"), Some(123));
    assert_eq!(
        nbt.get_compound("equipment")
            .unwrap()
            .get_compound("mainhand")
            .unwrap()
            .get_compound("components")
            .unwrap()
            .get_int("minecraft:damage"),
        Some(7)
    );
    for key in ["moveDist", "flyDist", "nextStep", "movement_game_events"] {
        assert!(!nbt.child_tags.contains_key(key));
    }
    nbt
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s3_real_storage_switch_restarts() {
    run(async {
        let mut s = Scenario::new_with_advanced(false, Some(config(false))).await;
        let id = Uuid::parse_str(PLAYER).unwrap();
        let player = s.join(id, "MovementCompat").await;
        player.player.living_entity.health.store(13.0);
        player
            .player
            .inventory
            .set_slot(4, ItemStack::new(17, &Item::EMERALD));
        player.player.set_experience(4, 2.0 / 15.0, 2);
        let counter = capture(&s);
        walking(&s, &player, 1.75);
        assert_eq!(counter.load(Relaxed), 0, "OFF through actual packet entry");
        assert_player(&player, Vector3::new(5.25, 64.0, 3.5));
        drop(player);
        s = s.restart_with_movement(true).await;
        let player = s.join(id, "MovementCompat").await;
        assert_player(&player, Vector3::new(5.25, 64.0, 3.5));
        let counter = capture(&s);
        walking(&s, &player, 1.5);
        assert_eq!(
            counter.load(Relaxed),
            0,
            "only transient counters reset on restart"
        );
        walking(&s, &player, 0.25);
        assert_eq!(counter.load(Relaxed), 1);
        drop(player);
        s = s.restart_with_movement(false).await;
        let player = s.join(id, "MovementCompat").await;
        assert_player(&player, Vector3::new(7.0, 64.0, 3.5));
        // Export the literal final committed position for the pinned old reader.
        player
            .player
            .request_teleport(Vector3::new(5.25, 64.0, 3.5), 0.0, 0.0);
        player.loaded(&s.server);
        let cow = cow_record(&s);
        s.stop().await;
        export(&s, cow);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s3_old_fork_and_vanilla_input() {
    for legacy in [true, false] {
        run(async move {
            let config_dir = tempfile::tempdir().unwrap();
            std::fs::write(config_dir.path().join("pumpkin.toml"), "").unwrap();
            let config = <pumpkin_config::PumpkinConfig as pumpkin_config::LoadConfiguration>::load(
                config_dir.path(),
            );
            let s = Scenario::new_with_advanced(false, Some(config.advanced)).await;
            let id = Uuid::new_v4();
            let mut nbt = super::old_saves::distinctive_player_save(legacy);
            nbt.put_int("XpTotal", 42);
            let storage = PlayerDataStorage::new(s.directory.path().join("players"), true);
            storage
                .save_snapshot(storage.snapshot(&id, || nbt))
                .unwrap();
            let p = s.join(id, "MovementOldSave").await;
            assert_player(&p, Vector3::new(7.5, 64.0, 9.5));
            let count = capture(&s);
            walking(&s, &p, 1.75);
            assert_eq!(count.load(Relaxed), 1, "omitted config enables real STEP");
            assert_player(&p, Vector3::new(9.25, 64.0, 9.5));
            s.stop().await;
        })
        .await;
    }
}
