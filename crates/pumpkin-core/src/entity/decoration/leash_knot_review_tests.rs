use super::*;
use crate::{
    entity::death_test_world::DeathTestWorld,
    entity::player::Player,
    net::{ClientPlatform, java::combat_test_support::TestPlayer},
    plugin::{BoxFuture, EventHandler, EventPriority},
    server::Server,
};
use pumpkin_data::{Block, BlockDirection, entity::EntityType, item::Item, item_stack::ItemStack};
use pumpkin_protocol::{
    codec::var_int::VarInt,
    java::server::play::{SInteract, SUseItemOn},
};
use pumpkin_util::math::vector2::Vector2;
use pumpkin_util::{Hand, math::vector3::Vector3};
use pumpkin_world::chunk::ChunkData;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

fn right_click(target_id: i32) -> SInteract {
    SInteract {
        entity_id: VarInt(target_id),
        r#type: VarInt(0),
        target_position: None,
        hand: Some(VarInt(0)),
        sneaking: false,
    }
}

fn use_on_fence(player: &TestPlayer, pos: BlockPos, server: &Arc<Server>) {
    player
        .client()
        .handle_use_item_on(
            &player.player,
            &SUseItemOn {
                hand: VarInt(0),
                position: pos,
                face: VarInt(BlockDirection::Up as i32),
                cursor_pos: Vector3::new(0.5, 0.5, 0.5),
                inside_block: false,
                is_against_world_border: false,
                sequence: VarInt(1),
            },
            server,
        )
        .unwrap();
}

fn player_near_fence(player: &TestPlayer) {
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(0.0, 64.0, 0.0));
}

fn start_fence_tie(
    player: Arc<Player>,
    pos: BlockPos,
    server: Arc<Server>,
) -> tokio::task::JoinHandle<bool> {
    let client = player.client.clone();
    tokio::task::spawn_blocking(move || {
        if let ClientPlatform::Java(client) = client.as_ref() {
            let packet = SUseItemOn {
                hand: VarInt(0),
                position: pos,
                face: VarInt(BlockDirection::Up as i32),
                cursor_pos: Vector3::new(0.5, 0.5, 0.5),
                inside_block: false,
                is_against_world_border: false,
                sequence: VarInt(1),
            };
            client.handle_use_item_on(&player, &packet, &server).is_ok()
        } else {
            false
        }
    })
}

fn make_fence(world: &World, pos: BlockPos) {
    world
        .level
        .loaded_chunks
        .insert(Vector2::new(0, 0), Arc::new(ChunkData::empty(0, 0)));
    world
        .level
        .set_block_state(&pos, Block::OAK_FENCE.default_state.id);
}

fn lead_count(world: &World) -> u32 {
    world
        .entities
        .load()
        .iter()
        .filter_map(|e| e.get_item_entity())
        .filter_map(|i| {
            let stack = i.get_item_stack().lock().unwrap();
            (stack.item == &Item::LEAD).then_some(u32::from(stack.item_count))
        })
        .sum()
}

async fn wait_for_pause_signal(
    receiver: &mut tokio::sync::mpsc::UnboundedReceiver<()>,
    wait: Duration,
) -> bool {
    tokio::time::timeout(wait, receiver.recv())
        .await
        .is_ok_and(|signal| signal.is_some())
}

#[test]
fn public_leash_to_keeps_its_unit_return_signature() {
    let _: fn(&Entity, Arc<dyn EntityBase>) = Entity::leash_to;
}

#[tokio::test]
async fn cleanup_pause_signal_timeout_returns_false() {
    let (_sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    assert!(!wait_for_pause_signal(&mut receiver, Duration::from_millis(1)).await);
}

struct CancelPlayerLeash {
    player_id: i32,
    entity_id: i32,
}

impl EventHandler<crate::plugin::api::events::player::player_leash_entity::PlayerLeashEntityEvent>
    for CancelPlayerLeash
{
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut crate::plugin::api::events::player::player_leash_entity::PlayerLeashEntityEvent,
    ) -> BoxFuture<'a, ()> {
        if event.player.entity_id() == self.player_id && event.entity_id == self.entity_id {
            event.cancelled = true;
        }
        Box::pin(async {})
    }
}

struct PauseKnotSpawn {
    knot_type_id: String,
    entered: tokio::sync::mpsc::UnboundedSender<()>,
    resume: tokio::sync::watch::Receiver<bool>,
}

struct CountKnotSpawns(Arc<AtomicUsize>);

impl EventHandler<crate::plugin::api::events::entity::entity_spawn::EntitySpawnEvent>
    for CountKnotSpawns
{
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut crate::plugin::api::events::entity::entity_spawn::EntitySpawnEvent,
    ) -> BoxFuture<'a, ()> {
        if event.entity_type == EntityType::LEASH_KNOT.id.to_string() {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
        Box::pin(async {})
    }
}

fn track_knot_spawns(server: &Arc<Server>) -> Arc<AtomicUsize> {
    let count = Arc::new(AtomicUsize::new(0));
    server
        .plugin_manager
        .register::<crate::plugin::api::events::entity::entity_spawn::EntitySpawnEvent, _>(
            Arc::new(CountKnotSpawns(count.clone())),
            EventPriority::Normal,
            true,
        );
    count
}

impl EventHandler<crate::plugin::api::events::entity::entity_spawn::EntitySpawnEvent>
    for PauseKnotSpawn
{
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut crate::plugin::api::events::entity::entity_spawn::EntitySpawnEvent,
    ) -> BoxFuture<'a, ()> {
        if event.entity_type == self.knot_type_id {
            let entered = self.entered.clone();
            let mut resume = self.resume.clone();
            Box::pin(async move {
                let _ = entered.send(());
                if !*resume.borrow() {
                    let _ = tokio::time::timeout(Duration::from_secs(30), resume.changed()).await;
                }
            })
        } else {
            Box::pin(async {})
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn knot_interaction_finds_player_leashed_mob_below_fence() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let player = TestPlayer::new(&world);
    player_near_fence(&player);
    let pos = BlockPos::new(0, 64, 0);
    make_fence(&world, pos);
    let knot = LeashKnotEntity::create_knot(&world, pos);
    let cow = fixture.mob(&EntityType::COW);
    cow.get_entity().set_pos(Vector3::new(0.0, 62.0, 0.0));
    cow.get_entity()
        .leash_to(player.player.clone() as Arc<dyn EntityBase>);

    player.client().handle_interact(
        &player.player,
        &right_click(knot.get_entity().entity_id),
        &fixture.server,
    );

    assert_eq!(
        leash_holder_id(cow.get_entity()),
        Some(knot.get_entity().entity_id)
    );
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lead_item_rebinds_player_leashed_mob_below_fence() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let player = TestPlayer::new(&world);
    player_near_fence(&player);
    player
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::new(1, &Item::LEAD));
    let pos = BlockPos::new(0, 64, 0);
    make_fence(&world, pos);
    let knot_spawns = track_knot_spawns(&fixture.server);
    let cow = fixture.mob(&EntityType::COW);
    cow.get_entity().set_pos(Vector3::new(0.0, 62.0, 0.0));
    cow.get_entity()
        .leash_to(player.player.clone() as Arc<dyn EntityBase>);

    use_on_fence(&player, pos, &fixture.server);

    let knot = LeashKnotEntity::get_knot(&world, pos).expect("using a lead ties a knot");
    assert_eq!(
        leash_holder_id(cow.get_entity()),
        Some(knot.get_entity().entity_id)
    );
    assert_eq!(player.player.inventory().held_item().item_count, 1);
    assert_eq!(knot_spawns.load(Ordering::Relaxed), 1);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lower_leash_survives_transfer_of_another_mob() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let player = TestPlayer::new(&world);
    player_near_fence(&player);
    let pos = BlockPos::new(0, 64, 0);
    make_fence(&world, pos);
    let lower = fixture.mob(&EntityType::COW);
    lower.get_entity().set_pos(Vector3::new(0.0, 62.0, 0.0));
    let upper = fixture.mob(&EntityType::SHEEP);
    upper.get_entity().set_pos(Vector3::new(0.0, 64.0, 0.0));
    let knot = LeashKnotEntity::create_knot(&world, pos);
    lower
        .get_entity()
        .leash_to(knot.clone() as Arc<dyn EntityBase>);
    upper
        .get_entity()
        .leash_to(knot.clone() as Arc<dyn EntityBase>);
    fixture
        .server
        .plugin_manager
        .register::<
            crate::plugin::api::events::player::player_leash_entity::PlayerLeashEntityEvent,
            _,
        >(
            Arc::new(CancelPlayerLeash {
                player_id: player.player.entity_id(),
                entity_id: lower.get_entity().entity_id,
            }),
            EventPriority::Normal,
            true,
        );

    player.client().handle_interact(
        &player.player,
        &right_click(knot.get_entity().entity_id),
        &fixture.server,
    );
    knot.tick(knot.as_ref(), &fixture.server);

    assert_eq!(
        leash_holder_id(lower.get_entity()),
        Some(knot.get_entity().entity_id)
    );
    assert_eq!(
        leash_holder_id(upper.get_entity()),
        Some(player.player.get_entity().entity_id)
    );
    assert!(!knot.get_entity().is_removed());
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lead_item_leaves_mob_with_player_when_fence_is_out_of_snap_range() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let player = TestPlayer::new(&world);
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(4.0, 64.0, 0.0));
    player
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::new(1, &Item::LEAD));
    let pos = BlockPos::new(0, 64, 0);
    make_fence(&world, pos);
    let knot_spawns = track_knot_spawns(&fixture.server);
    let cow = fixture.mob(&EntityType::COW);
    cow.get_entity().set_pos(Vector3::new(14.0, 64.0, 0.0));
    cow.get_entity()
        .leash_to(player.player.clone() as Arc<dyn EntityBase>);

    use_on_fence(&player, pos, &fixture.server);

    assert_eq!(
        leash_holder_id(cow.get_entity()),
        Some(player.player.entity_id())
    );
    assert!(LeashKnotEntity::get_knot(&world, pos).is_none());
    assert_eq!(knot_spawns.load(Ordering::Relaxed), 0);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn knot_interaction_respects_happy_ghasts_extended_leash_range() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let player = TestPlayer::new(&world);
    player_near_fence(&player);
    let pos = BlockPos::new(0, 64, 0);
    make_fence(&world, pos);
    let knot = LeashKnotEntity::create_knot(&world, pos);
    let ghast = fixture.mob(&EntityType::HAPPY_GHAST);
    ghast.get_entity().set_pos(Vector3::new(13.0, 64.0, 0.0));
    ghast
        .get_entity()
        .leash_to(player.player.clone() as Arc<dyn EntityBase>);

    player.client().handle_interact(
        &player.player,
        &right_click(knot.get_entity().entity_id),
        &fixture.server,
    );
    ghast
        .get_entity()
        .tick_leash_with_living(ghast.get_living_entity().unwrap());

    assert_eq!(
        leash_holder_id(ghast.get_entity()),
        Some(knot.get_entity().entity_id)
    );
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn happy_ghast_player_leash_uses_extended_range() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let player = TestPlayer::new(&world);
    player_near_fence(&player);
    player
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::new(1, &Item::LEAD));
    let ghast = fixture.mob(&EntityType::HAPPY_GHAST);
    ghast.get_entity().set_pos(Vector3::new(13.0, 64.0, 0.0));

    player.client().handle_interact(
        &player.player,
        &right_click(ghast.get_entity().entity_id),
        &fixture.server,
    );
    ghast
        .get_entity()
        .tick_leash_with_living(ghast.get_living_entity().unwrap());

    assert_eq!(
        leash_holder_id(ghast.get_entity()),
        Some(player.player.entity_id())
    );
    assert_eq!(player.player.inventory().held_item().item_count, 0);
    assert!((ghast.get_entity().velocity.load().x + 0.335).abs() < 0.002);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cow_snaps_at_13_blocks_and_uses_elastic_6_at_8_blocks() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let snap_holder = TestPlayer::new(&world);
    snap_holder
        .player
        .get_entity()
        .set_pos(Vector3::new(0.0, 64.0, 20.0));
    let snapping_cow = fixture.mob(&EntityType::COW);
    snapping_cow
        .get_entity()
        .set_pos(Vector3::new(13.0, 64.0, 20.0));
    snapping_cow.get_entity().velocity.store(Vector3::default());
    snapping_cow
        .get_entity()
        .leash_to(snap_holder.player.clone() as Arc<dyn EntityBase>);
    snapping_cow
        .get_entity()
        .tick_leash_with_living(snapping_cow.get_living_entity().unwrap());

    assert_eq!(leash_holder_id(snapping_cow.get_entity()), None);
    assert_eq!(lead_count(&world), 1);

    let elastic_holder = TestPlayer::new(&world);
    elastic_holder
        .player
        .get_entity()
        .set_pos(Vector3::new(0.0, 64.0, 40.0));
    let elastic_cow = fixture.mob(&EntityType::COW);
    elastic_cow
        .get_entity()
        .set_pos(Vector3::new(8.0, 64.0, 40.0));
    elastic_cow.get_entity().velocity.store(Vector3::default());
    elastic_cow
        .get_entity()
        .leash_to(elastic_holder.player.clone() as Arc<dyn EntityBase>);
    elastic_cow
        .get_entity()
        .tick_leash_with_living(elastic_cow.get_living_entity().unwrap());

    assert_eq!(
        leash_holder_id(elastic_cow.get_entity()),
        Some(elastic_holder.player.entity_id())
    );
    assert_eq!(lead_count(&world), 1);
    assert!((elastic_cow.get_entity().velocity.load().x + 0.2203).abs() < 0.001);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shears_on_knot_cut_leashes_and_notify_empty_cleanup() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let player = TestPlayer::new(&world);
    player_near_fence(&player);
    player
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::new(1, &Item::SHEARS));
    let pos = BlockPos::new(0, 64, 0);
    make_fence(&world, pos);
    let cow = fixture.mob(&EntityType::COW);
    cow.get_entity().set_pos(Vector3::new(0.0, 64.0, 0.0));
    let knot = LeashKnotEntity::create_knot(&world, pos);
    cow.get_entity()
        .leash_to(knot.clone() as Arc<dyn EntityBase>);

    player.client().handle_interact(
        &player.player,
        &right_click(knot.get_entity().entity_id),
        &fixture.server,
    );
    assert!(!cow.get_entity().is_leashed());
    knot.tick(knot.as_ref(), &fixture.server);
    assert!(knot.get_entity().is_removed());
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn simultaneous_first_fence_ties_publish_one_knot() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let player_a = fixture.player("FirstTieA");
    let player_b = fixture.player("FirstTieB");
    player_a.get_entity().set_pos(Vector3::new(0.0, 64.0, 0.0));
    player_b.get_entity().set_pos(Vector3::new(0.0, 64.0, 0.0));
    for player in [&player_a, &player_b] {
        player
            .inventory()
            .set_stack_in_hand(Hand::Right, ItemStack::new(1, &Item::LEAD));
    }
    let pos = BlockPos::new(0, 64, 0);
    make_fence(&world, pos);
    let cow_a = fixture.mob(&EntityType::COW);
    cow_a.get_entity().set_pos(Vector3::new(-1.0, 64.0, 0.0));
    let cow_b = fixture.mob(&EntityType::COW);
    cow_b.get_entity().set_pos(Vector3::new(1.0, 64.0, 0.0));
    cow_a
        .get_entity()
        .leash_to(player_a.clone() as Arc<dyn EntityBase>);
    cow_b
        .get_entity()
        .leash_to(player_b.clone() as Arc<dyn EntityBase>);

    let (entered_tx, mut entered_rx) = tokio::sync::mpsc::unbounded_channel();
    let (resume_tx, resume_rx) = tokio::sync::watch::channel(false);
    fixture
        .server
        .plugin_manager
        .register::<crate::plugin::api::events::entity::entity_spawn::EntitySpawnEvent, _>(
            Arc::new(PauseKnotSpawn {
                knot_type_id: EntityType::LEASH_KNOT.id.to_string(),
                entered: entered_tx,
                resume: resume_rx,
            }),
            EventPriority::Normal,
            true,
        );

    let worker_a = start_fence_tie(player_a, pos, fixture.server.clone());
    let worker_b = start_fence_tie(player_b, pos, fixture.server.clone());
    let first_spawn_paused = matches!(
        tokio::time::timeout(Duration::from_secs(30), entered_rx.recv()).await,
        Ok(Some(()))
    );
    let second_spawn_paused = matches!(
        tokio::time::timeout(Duration::from_secs(30), entered_rx.recv()).await,
        Ok(Some(()))
    );
    let _ = resume_tx.send(true);
    let result_a = tokio::time::timeout(Duration::from_secs(30), worker_a)
        .await
        .expect("first fence tie worker should finish after the spawn pause is released")
        .expect("first fence tie worker should not panic");
    let result_b = tokio::time::timeout(Duration::from_secs(30), worker_b)
        .await
        .expect("second fence tie worker should finish after the spawn pause is released")
        .expect("second fence tie worker should not panic");

    assert!(first_spawn_paused && second_spawn_paused);
    assert!(result_a && result_b);
    let knot = LeashKnotEntity::get_knot(&world, pos).expect("concurrent ties publish a knot");
    assert_eq!(
        leash_holder_id(cow_a.get_entity()),
        Some(knot.get_entity().entity_id)
    );
    assert_eq!(
        leash_holder_id(cow_b.get_entity()),
        Some(knot.get_entity().entity_id)
    );
    let knot_count = world
        .get_entities_at_box(&leash_search_box(Vector3::new(0.5, 64.5, 0.5)))
        .iter()
        .filter(|entity| {
            entity.get_entity().entity_type == &EntityType::LEASH_KNOT
                && entity
                    .cast_any()
                    .downcast_ref::<LeashKnotEntity>()
                    .is_some_and(|candidate| candidate.block_pos() == pos)
        })
        .count();
    assert_eq!(knot_count, 1);
    fixture.server.shutdown().await;
}

#[path = "leash_knot_followup_tests.rs"]
mod followup;
