use super::*;
use pumpkin_protocol::{packet::MultiVersionJavaPacket, ser::NetworkReadExt};

use crate::{
    entity::{
        death_test_world::DeathTestWorld,
        decoration::leash_knot::{LeashKnotEntity, leash_holder_id},
        shearable::Shearable,
    },
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::entity::entity_transform::EntityTransformEvent,
        api::events::player::player_leash_entity::PlayerLeashEntityEvent,
    },
    server::Server,
};
use pumpkin_data::{
    Block, entity::EntityType, item::Item, item_stack::ItemStack, sound::SoundCategory,
};
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::math::{position::BlockPos, vector2::Vector2};
use pumpkin_world::chunk::ChunkData;
use std::sync::atomic::{AtomicBool, AtomicUsize};

fn make_fence(world: &crate::world::World, pos: BlockPos) {
    world
        .level
        .loaded_chunks
        .insert(Vector2::new(0, 0), Arc::new(ChunkData::empty(0, 0)));
    world
        .level
        .set_block_state(&pos, Block::OAK_FENCE.default_state.id);
}

fn lead_count(world: &crate::world::World) -> u32 {
    world
        .entities
        .load()
        .iter()
        .filter_map(|entity| entity.get_item_entity())
        .map(|item| {
            let stack = item.get_item_stack().lock().unwrap();
            if stack.item == &Item::LEAD {
                u32::from(stack.item_count)
            } else {
                0
            }
        })
        .sum()
}

struct ConversionControl {
    cancel: AtomicBool,
    calls: AtomicUsize,
}
impl EventHandler<EntityTransformEvent> for ConversionControl {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut EntityTransformEvent,
    ) -> BoxFuture<'a, ()> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        event.cancelled = self.cancel.load(Ordering::Relaxed);
        Box::pin(async {})
    }
}

struct RemovePlayerDuringLeash;

impl EventHandler<PlayerLeashEntityEvent> for RemovePlayerDuringLeash {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut PlayerLeashEntityEvent,
    ) -> BoxFuture<'a, ()> {
        let world = event.player.world();
        let player = event.player.clone();
        Box::pin(async move {
            world.remove_player(&player, false).await;
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_mooshroom_conversion_has_no_effects() {
    let fixture = DeathTestWorld::new().await;
    let mob = fixture.mob(&EntityType::MOOSHROOM);
    let mut observer = crate::net::java::combat_test_support::TestPlayer::new(&fixture.world());
    let player = observer.player.clone();
    let control = Arc::new(ConversionControl {
        cancel: AtomicBool::new(true),
        calls: AtomicUsize::new(0),
    });
    fixture
        .server
        .plugin_manager
        .register::<EntityTransformEvent, _>(control.clone(), EventPriority::Normal, true);
    let mut tool = ItemStack::new(1, &Item::SHEARS);
    observer.take_packets();
    assert!(!mob.interact(&player, &mut tool));
    let sound_id = pumpkin_protocol::java::client::play::CSoundEffect::to_id(
        pumpkin_data::packet::CURRENT_MC_VERSION,
    );
    assert!(
        observer
            .take_packets()
            .iter()
            .all(|packet| packet.as_ref().get_var_int().unwrap().0 != sound_id)
    );
    assert!(!mob.get_entity().is_removed());
    assert_eq!(tool.get_damage(), 0);
    assert_eq!(fixture.world().entities.load().len(), 1);
    control.cancel.store(false, Ordering::Relaxed);
    assert!(mob.interact(&player, &mut tool));
    assert_eq!(control.calls.load(Ordering::Relaxed), 2);
    assert!(mob.get_entity().is_removed());
    assert_eq!(tool.get_damage(), 1);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mooshroom_conversion_preserves_common_state_and_mounts() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let mob = fixture.mob(&EntityType::MOOSHROOM);
    let mushroom = mob.cast_any().downcast_ref::<MooshroomEntity>().unwrap();
    let raw = mob.get_entity();
    raw.set_pos(Vector3::new(8.5, 64.0, 8.5));
    raw.set_rotation(30.0, 15.0);
    raw.velocity.store(Vector3::new(0.2, 0.3, 0.4));
    raw.set_fall_flying(true);
    raw.set_custom_data("test", "conversion", pumpkin_nbt::tag::NbtTag::Int(27));
    raw.set_custom_name(pumpkin_util::text::TextComponent::text("Named cow"));
    mushroom.mob_entity.set_left_handed(true);
    mushroom.mob_entity.set_no_ai(true);
    mushroom
        .mob_entity
        .living_entity
        .hurt_cooldown
        .store(17, Ordering::Relaxed);
    let passenger = fixture.mob(&EntityType::SHEEP);
    let vehicle = fixture.mob(&EntityType::COW);
    raw.add_passenger(mob.clone(), passenger.clone());
    vehicle
        .get_entity()
        .add_passenger(vehicle.clone(), mob.clone());
    passenger
        .get_entity()
        .riding_cooldown
        .store(41, Ordering::Relaxed);
    assert!(mushroom.shear(SoundCategory::Players, &ItemStack::new(1, &Item::SHEARS)));
    let entities = world.entities.load_full();
    let cow = entities
        .iter()
        .find(|e| {
            e.get_entity().entity_type == &EntityType::COW
                && e.get_entity().entity_id != vehicle.get_entity().entity_id
        })
        .unwrap();
    let converted = cow.get_mob().unwrap().get_mob_entity();
    assert!(converted.is_left_handed());
    assert!(converted.is_no_ai());
    assert_eq!(
        converted
            .living_entity
            .hurt_cooldown
            .load(Ordering::Relaxed),
        17
    );
    assert!(cow.get_entity().is_fall_flying());
    assert_eq!(
        cow.get_entity().get_custom_data("test", "conversion"),
        Some(pumpkin_nbt::tag::NbtTag::Int(27))
    );
    assert_eq!(
        cow.get_entity().velocity.load(),
        Vector3::new(0.2, 0.3, 0.4)
    );
    assert_eq!(cow.get_entity().yaw.load(), 30.0);
    assert_eq!(
        passenger
            .get_entity()
            .get_vehicle()
            .unwrap()
            .get_entity()
            .entity_id,
        cow.get_entity().entity_id
    );
    assert_eq!(
        cow.get_entity()
            .get_vehicle()
            .unwrap()
            .get_entity()
            .entity_id,
        vehicle.get_entity().entity_id
    );
    assert_eq!(
        passenger
            .get_entity()
            .riding_cooldown
            .load(Ordering::Relaxed),
        0
    );
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mooshroom_conversion_preserves_combat_hurt_animation() {
    let fixture = DeathTestWorld::new().await;
    let mob = fixture.mob(&EntityType::MOOSHROOM);
    mob.get_living_entity()
        .unwrap()
        .hurt_time
        .store(7, Ordering::Relaxed);
    assert!(
        mob.get_mob()
            .unwrap()
            .as_shearable()
            .unwrap()
            .shear(SoundCategory::Players, &ItemStack::new(1, &Item::SHEARS))
    );
    let entities = fixture.world().entities.load_full();
    let cow = entities
        .iter()
        .find(|entity| entity.get_entity().entity_type == &EntityType::COW)
        .unwrap();
    assert_eq!(
        cow.get_living_entity()
            .unwrap()
            .hurt_time
            .load(Ordering::Relaxed),
        7
    );
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mooshroom_conversion_racing_invalid_knot_settles_leash_once() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let pos = BlockPos::new(0, 64, 0);
    make_fence(&world, pos);
    let knot = LeashKnotEntity::create_knot(&world, pos);
    let mooshroom = fixture.mob(&EntityType::MOOSHROOM);
    mooshroom.get_entity().set_pos(Vector3::new(0.5, 64.0, 0.5));
    mooshroom
        .get_entity()
        .leash_to(knot.clone() as Arc<dyn crate::entity::EntityBase>);

    world
        .level
        .set_block_state(&pos, Block::AIR.default_state.id);
    let (entered_tx, mut entered_rx) = tokio::sync::mpsc::unbounded_channel();
    let (resume_tx, resume_rx) = crossbeam::channel::bounded(1);
    knot.set_test_pause(Arc::new(move || {
        let _ = entered_tx.send(());
        let _ = resume_rx.recv_timeout(std::time::Duration::from_secs(30));
    }));
    let cleanup_knot = knot.clone();
    let cleanup_server = fixture.server.clone();
    let cleanup = tokio::task::spawn_blocking(move || {
        cleanup_knot.tick(cleanup_knot.as_ref(), &cleanup_server);
    });

    let reached_invalidation = matches!(
        tokio::time::timeout(std::time::Duration::from_secs(30), entered_rx.recv()).await,
        Ok(Some(()))
    );
    let converted = reached_invalidation
        && mooshroom
            .get_mob()
            .unwrap()
            .as_shearable()
            .unwrap()
            .shear(SoundCategory::Players, &ItemStack::new(1, &Item::SHEARS));
    let _ = resume_tx.send(());
    let cleanup_result = tokio::time::timeout(std::time::Duration::from_secs(30), cleanup).await;

    assert!(
        reached_invalidation,
        "invalid-knot tick did not reach the barrier"
    );
    assert!(
        converted,
        "mooshroom should convert while knot removal is paused"
    );
    cleanup_result
        .expect("invalid-knot tick should finish within its bounded wait")
        .expect("invalid-knot tick should finish");

    let entities = world.entities.load_full();
    let cow = entities
        .iter()
        .find(|entity| entity.get_entity().entity_type == &EntityType::COW)
        .unwrap();
    let cow_has_live_holder = cow
        .get_entity()
        .leashed_to
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|holder| holder.get_entity().is_alive());
    let leads = lead_count(&world);
    assert!(
        (cow_has_live_holder && leads == 0) || (!cow_has_live_holder && leads == 1),
        "conversion must leave a live leash holder or drop exactly one lead"
    );
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mooshroom_conversion_rebinds_removed_knot_to_live_knot() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let pos = BlockPos::new(0, 64, 0);
    make_fence(&world, pos);
    let removed_knot = LeashKnotEntity::create_knot(&world, pos);
    let mooshroom = fixture.mob(&EntityType::MOOSHROOM);
    mooshroom
        .get_entity()
        .leash_to(removed_knot.clone() as Arc<dyn crate::entity::EntityBase>);
    removed_knot.get_entity().remove();

    assert!(
        mooshroom
            .get_mob()
            .unwrap()
            .as_shearable()
            .unwrap()
            .shear(SoundCategory::Players, &ItemStack::new(1, &Item::SHEARS))
    );

    let entities = world.entities.load_full();
    let cow = entities
        .iter()
        .find(|entity| entity.get_entity().entity_type == &EntityType::COW)
        .unwrap();
    let holder = cow.get_entity().leashed_to.lock().unwrap().clone().unwrap();
    assert_ne!(
        holder.get_entity().entity_id,
        removed_knot.get_entity().entity_id
    );
    assert!(holder.get_entity().is_alive());
    assert_eq!(lead_count(&world), 0);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mooshroom_conversion_settles_knot_removed_after_attachment_check() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let pos = BlockPos::new(0, 64, 0);
    make_fence(&world, pos);
    let knot = LeashKnotEntity::create_knot(&world, pos);
    let mooshroom = fixture.mob(&EntityType::MOOSHROOM);
    mooshroom
        .get_entity()
        .leash_to(knot.clone() as Arc<dyn crate::entity::EntityBase>);
    world
        .level
        .set_block_state(&pos, Block::AIR.default_state.id);

    let (entered_tx, mut entered_rx) = tokio::sync::mpsc::unbounded_channel();
    let (resume_tx, resume_rx) = crossbeam::channel::bounded(1);
    knot.set_attachment_check_pause(Arc::new(move || {
        let _ = entered_tx.send(());
        let _ = resume_rx.recv_timeout(std::time::Duration::from_secs(30));
    }));

    let conversion_mooshroom = mooshroom.clone();
    let conversion = tokio::task::spawn_blocking(move || {
        conversion_mooshroom
            .get_mob()
            .unwrap()
            .as_shearable()
            .unwrap()
            .shear(SoundCategory::Players, &ItemStack::new(1, &Item::SHEARS))
    });
    let reached_attachment_check =
        tokio::time::timeout(std::time::Duration::from_secs(30), entered_rx.recv())
            .await
            .is_ok_and(|signal| signal.is_some());
    if reached_attachment_check {
        knot.get_entity().remove();
    }
    let _ = resume_tx.send(());
    let conversion_result = tokio::time::timeout(std::time::Duration::from_secs(30), conversion)
        .await
        .expect("mooshroom conversion should finish within its bounded wait")
        .expect("mooshroom conversion should not panic");

    assert!(
        reached_attachment_check,
        "attachment check seam was not reached"
    );
    assert!(conversion_result, "mooshroom should convert");
    let cow = world
        .entities
        .load_full()
        .iter()
        .find(|entity| entity.get_entity().entity_type == &EntityType::COW)
        .unwrap()
        .clone();
    assert_eq!(
        leash_holder_id(cow.get_entity()),
        Some(knot.get_entity().entity_id)
    );
    assert!(knot.get_entity().is_removed());

    cow.get_entity().tick_leash();
    assert!(!cow.get_entity().is_leashed());
    assert_eq!(lead_count(&world), 1);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mooshroom_conversion_revalidates_player_removed_during_leash_event() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let holder = fixture.player("HolderLeavingDuringConversion");
    let mooshroom = fixture.mob(&EntityType::MOOSHROOM);
    mooshroom
        .get_entity()
        .leash_to(holder.clone() as Arc<dyn crate::entity::EntityBase>);
    fixture
        .server
        .plugin_manager
        .register::<PlayerLeashEntityEvent, _>(
            Arc::new(RemovePlayerDuringLeash),
            EventPriority::Normal,
            true,
        );

    assert!(
        mooshroom
            .get_mob()
            .unwrap()
            .as_shearable()
            .unwrap()
            .shear(SoundCategory::Players, &ItemStack::new(1, &Item::SHEARS))
    );

    assert!(world.get_player_by_uuid(holder.gameprofile.id).is_none());
    let cow = world
        .entities
        .load_full()
        .iter()
        .find(|entity| entity.get_entity().entity_type == &EntityType::COW)
        .unwrap()
        .clone();
    assert!(!cow.get_entity().is_leashed());
    assert_eq!(lead_count(&world), 1);
    cow.get_entity().tick_leash();
    assert_eq!(lead_count(&world), 1);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mooshroom_conversion_unkept_leash_drops_nothing_without_entity_drops() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let holder = fixture.player("HolderLeavingWithoutDrops");
    let mooshroom = fixture.mob(&EntityType::MOOSHROOM);
    mooshroom
        .get_entity()
        .leash_to(holder.clone() as Arc<dyn crate::entity::EntityBase>);
    fixture
        .server
        .plugin_manager
        .register::<PlayerLeashEntityEvent, _>(
            Arc::new(RemovePlayerDuringLeash),
            EventPriority::Normal,
            true,
        );
    fixture.server.level_info.rcu(|info| {
        let mut next = (**info).clone();
        next.game_rules.entity_drops = false;
        next
    });

    assert!(
        mooshroom
            .get_mob()
            .unwrap()
            .as_shearable()
            .unwrap()
            .shear(SoundCategory::Players, &ItemStack::new(1, &Item::SHEARS))
    );

    let cow = world
        .entities
        .load_full()
        .iter()
        .find(|entity| entity.get_entity().entity_type == &EntityType::COW)
        .unwrap()
        .clone();
    assert!(!cow.get_entity().is_leashed());
    assert_eq!(lead_count(&world), 0);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn player_logout_drops_one_lead_on_the_leashee_tick() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let holder = fixture.player("LoggedOutHolder");
    let cow = fixture.mob(&EntityType::COW);
    holder.get_entity().set_pos(cow.get_entity().pos.load());
    cow.get_entity()
        .leash_to(holder.clone() as Arc<dyn crate::entity::EntityBase>);

    holder.remove().await;
    assert!(world.get_player_by_uuid(holder.gameprofile.id).is_none());
    cow.get_entity().tick_leash();
    assert!(!cow.get_entity().is_leashed());
    assert_eq!(lead_count(&world), 1);
    cow.get_entity().tick_leash();
    assert_eq!(lead_count(&world), 1);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dying_leashee_settles_one_lead_and_death_removal_does_not_duplicate_it() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let holder = fixture.player("DyingLeasheeHolder");
    let cow = fixture.mob(&EntityType::COW);
    holder.get_entity().set_pos(cow.get_entity().pos.load());
    cow.get_entity()
        .leash_to(holder.clone() as Arc<dyn crate::entity::EntityBase>);

    holder.remove().await;
    let living = cow.get_living_entity().unwrap();
    living.set_health(0.0);
    living.on_death(pumpkin_data::damage::DamageType::GENERIC, None, None);
    cow.get_entity().tick_leash();
    assert!(!cow.get_entity().is_leashed());
    assert_eq!(lead_count(&world), 1);

    for _ in 0..20 {
        living.tick(&*cow, &fixture.server);
    }
    assert!(cow.get_entity().is_removed());
    cow.get_entity().tick_leash();
    assert_eq!(lead_count(&world), 1);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dead_living_holder_settles_one_lead_on_the_leashee_tick() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let holder = fixture.mob(&EntityType::MOOSHROOM);
    let cow = fixture.mob(&EntityType::COW);
    cow.get_entity()
        .leash_to(holder.clone() as Arc<dyn crate::entity::EntityBase>);

    let living_holder = holder.get_living_entity().unwrap();
    living_holder.set_health(0.0);
    living_holder.on_death(pumpkin_data::damage::DamageType::GENERIC, None, None);
    cow.get_entity().tick_leash();

    assert!(!cow.get_entity().is_leashed());
    assert_eq!(lead_count(&world), 1);
    cow.get_entity().tick_leash();
    assert_eq!(lead_count(&world), 1);
    fixture.server.shutdown().await;
}
