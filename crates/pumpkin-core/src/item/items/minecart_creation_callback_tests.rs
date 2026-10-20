use super::*;
use crate::{
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::{
            entity::entity_spawn::EntitySpawnEvent,
            player::player_interact_event::PlayerInteractEvent,
        },
    },
    server::Server,
};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering::Relaxed};

struct Events {
    actor: Arc<crate::entity::player::Player>,
    action: AtomicU8,
    cancel_interaction: AtomicBool,
    births: AtomicUsize,
}
impl Events {
    fn register(f: &Fixture) -> Arc<Self> {
        let events = Arc::new(Self {
            actor: f.actor.player.clone(),
            action: AtomicU8::new(0),
            cancel_interaction: AtomicBool::new(false),
            births: AtomicUsize::new(0),
        });
        f.context
            .server
            .plugin_manager
            .register::<EntitySpawnEvent, _>(events.clone(), EventPriority::Normal, true);
        f.context
            .server
            .plugin_manager
            .register::<PlayerInteractEvent, _>(events.clone(), EventPriority::Normal, true);
        events
    }
}
impl EventHandler<EntitySpawnEvent> for Events {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        event: &'a mut EntitySpawnEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.births.fetch_add(1, Relaxed);
            let inventory = self.actor.inventory();
            assert!(
                std::thread::scope(|scope| scope
                    .spawn(|| inventory.slot_transaction(0).try_lock().is_some())
                    .join()
                    .unwrap()),
                "spawn callback must not retain the inventory owner"
            );
            match self.action.load(Relaxed) {
                1 => event.cancelled = true,
                2 => inventory.set_stack(0, ItemStack::new(3, &Item::DIAMOND)),
                3 => inventory.set_selected_slot(1),
                4 => {
                    self.actor.world().set_block_state(
                        &RAIL,
                        Block::STONE.default_state.id,
                        BlockFlags::FORCE_STATE,
                    );
                }
                5 => {
                    let _replacement = TestPlayer::with_profile(
                        &self.actor.world(),
                        self.actor.gameprofile.clone(),
                    );
                }
                6 => {
                    self.actor.set_client_loaded(false);
                }
                7 => {
                    self.actor.living_entity.set_health(0.0);
                }
                8 => {
                    let world = self.actor.world();
                    let state = world.get_block_state_id(&RAIL);
                    combat_test_support::publish_empty_chunk(&world, RAIL.chunk_position());
                    world.set_block_state(
                        &RAIL,
                        state,
                        BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
                    );
                }
                9 => {
                    self.actor
                        .get_entity()
                        .set_pos(Vector3::new(100.5, 64.0, 100.5));
                }
                10 => inventory.set_stack(0, ItemStack::new(2, &Item::MINECART)),
                11 => self.actor.gamemode.store(GameMode::Creative),
                12 => {
                    server.level_info.rcu(|info| {
                        let mut info = (**info).clone();
                        info.data_packs
                            .enabled
                            .retain(|pack| pack != "minecart_improvements");
                        info
                    });
                }
                13 => {
                    let world = self.actor.world();
                    world.add_entity_silent(Arc::new(MinecartEntity::new(Entity::new(
                        world.clone(),
                        Vector3::new(10.5, 64.1, 20.5),
                        &EntityType::MINECART,
                    ))));
                }
                14 => self.actor.get_entity().world.store(
                    server
                        .get_world_from_dimension(&pumpkin_data::dimension::Dimension::THE_NETHER),
                ),
                _ => {}
            }
        })
    }
}
impl EventHandler<PlayerInteractEvent> for Events {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerInteractEvent,
    ) -> BoxFuture<'a, ()> {
        event.cancelled = self.cancel_interaction.load(Relaxed);
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g12_cancelled_spawn_interaction_and_stale_source_controls() {
    for action in 1..=14 {
        let mut f = Fixture::new(true).await;
        f.rail(RAIL, RailShape::NorthSouth, &Block::RAIL);
        f.equip(RAIL, &Item::MINECART, Hand::Right);
        let source = f.actor.player.inventory().held_item();
        let events = Events::register(&f);
        events.action.store(action, Relaxed);
        f.use_on(RAIL, Hand::Right);
        assert_eq!(events.births.load(Relaxed), 1, "action {action}");
        assert_eq!(
            f.carts().len(),
            usize::from(action == 13),
            "stale/cancelled creation action {action}"
        );
        let remaining = f.actor.player.inventory().get_stack(0);
        if action == 2 {
            assert_eq!((remaining.item, remaining.item_count), (&Item::DIAMOND, 3));
        } else if action == 10 {
            assert!(remaining.are_equal(&source));
            assert_ne!(remaining.uid, source.uid);
        } else {
            assert!(
                remaining.are_equal(&source),
                "no stock loss for action {action}"
            );
            assert_eq!(remaining.uid, source.uid);
        }
        assert_eq!(
            f.packets().0.len(),
            usize::from(action == 13),
            "no outer spawn on action {action}"
        );
        f.close().await;
    }
    let mut f = Fixture::new(true).await;
    f.rail(RAIL, RailShape::NorthSouth, &Block::RAIL);
    f.equip(RAIL, &Item::MINECART, Hand::Right);
    let events = Events::register(&f);
    events.cancel_interaction.store(true, Relaxed);
    f.use_on(RAIL, Hand::Right);
    assert_eq!(events.births.load(Relaxed), 0);
    assert!(f.carts().is_empty());
    assert_eq!(f.hand_count(Hand::Right), 2);
    assert!(f.packets().0.is_empty());
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g12_unloaded_and_out_of_reach_controls() {
    let mut f = Fixture::new(true).await;
    f.equip(RAIL, &Item::MINECART, Hand::Right);
    f.use_on(RAIL, Hand::Right);
    assert!(f.carts().is_empty());
    assert_eq!(f.hand_count(Hand::Right), 2);
    f.rail(RAIL, RailShape::NorthSouth, &Block::RAIL);
    f.actor
        .player
        .get_entity()
        .set_pos(Vector3::new(100.5, 64.0, 100.5));
    let result = f.actor.client().handle_use_item_on(
        &f.actor.player,
        &SUseItemOn {
            hand: VarInt(0),
            position: RAIL,
            face: VarInt(1),
            cursor_pos: Vector3::new(0.5, 1.0, 0.5),
            inside_block: false,
            is_against_world_border: false,
            sequence: VarInt(2),
        },
        &f.context.server,
    );
    assert!(result.is_err());
    assert!(f.carts().is_empty());
    assert_eq!(f.hand_count(Hand::Right), 2);
    f.close().await;
}

struct SpawnGate {
    entered: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    release: std::sync::Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
}
impl EventHandler<EntitySpawnEvent> for SpawnGate {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        _: &'a mut EntitySpawnEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let entered = self.entered.lock().unwrap().take();
            let release = self.release.lock().unwrap().take();
            if let Some(entered) = entered {
                entered.send(()).unwrap();
            }
            if let Some(release) = release {
                tokio::time::timeout(std::time::Duration::from_secs(10), release)
                    .await
                    .unwrap()
                    .unwrap();
            }
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g12_concurrent_placement_reserves_aligned_space_without_callback_lock() {
    let mut f = Fixture::new(true).await;
    f.rail(RAIL, RailShape::NorthSouth, &Block::RAIL);
    f.equip(RAIL, &Item::MINECART, Hand::Right);
    let other = TestPlayer::new(&f.world);
    other
        .player
        .permission_lvl
        .store(pumpkin_util::permission::PermissionLvl::Four);
    other
        .player
        .get_entity()
        .set_pos(Vector3::new(10.5, 65.0, 20.5));
    other
        .player
        .inventory()
        .set_stack(0, ItemStack::new(2, &Item::MINECART));
    f.world
        .players
        .store(Arc::new(vec![f.actor.player.clone(), other.player.clone()]));
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    f.context
        .server
        .plugin_manager
        .register::<EntitySpawnEvent, _>(
            Arc::new(SpawnGate {
                entered: std::sync::Mutex::new(Some(entered_tx)),
                release: std::sync::Mutex::new(Some(release_rx)),
            }),
            EventPriority::Normal,
            true,
        );
    let player = f.actor.player.clone();
    let server = f.context.server.clone();
    let first = tokio::task::spawn_blocking(move || {
        player
            .client
            .java()
            .unwrap()
            .handle_use_item_on(
                &player,
                &SUseItemOn {
                    hand: VarInt(0),
                    position: RAIL,
                    face: VarInt(1),
                    cursor_pos: Vector3::new(0.5, 1.0, 0.5),
                    inside_block: false,
                    is_against_world_border: false,
                    sequence: VarInt(1),
                },
                &server,
            )
            .unwrap();
    });
    tokio::time::timeout(std::time::Duration::from_secs(10), entered_rx)
        .await
        .unwrap()
        .unwrap();
    other
        .client()
        .handle_use_item_on(
            &other.player,
            &SUseItemOn {
                hand: VarInt(0),
                position: RAIL,
                face: VarInt(1),
                cursor_pos: Vector3::new(0.5, 1.0, 0.5),
                inside_block: false,
                is_against_world_border: false,
                sequence: VarInt(1),
            },
            &f.context.server,
        )
        .unwrap();
    let pending_hand = other.player.inventory().held_item().item_count;
    release_tx.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), first)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        pending_hand, 2,
        "pending aligned cart reserves the overlap before another hand pays"
    );
    assert_eq!(f.carts().len(), 1);
    assert_eq!(f.hand_count(Hand::Right), 1);
    f.close().await;
}
