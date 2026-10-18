use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering::SeqCst},
};

use pumpkin_data::{
    data_component_impl::{ChargedProjectilesImpl, CustomDataImpl, IntangibleProjectileImpl},
    item::Item,
    item_stack::ItemStack,
};
use pumpkin_inventory::Inventory;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::GameMode;

use crate::{
    entity::player::Player,
    item::{
        ItemBehaviour,
        items::{
            crossbow::CrossbowItem, firework_rocket::FireworkRocketItem, trident::TridentItem,
        },
    },
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        entity::{entity_spawn::EntitySpawnEvent, projectile_launch::ProjectileLaunchEvent},
    },
    projectile_item_event_tests::{cancel, run, use_packet},
    server::Server,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn projectile_item_events_throwable_handlers_refund_both_vetoes() {
    for launch in [false, true] {
        for cancelled in [false, true] {
            run(|fixture| {
                let world = fixture.world();
                let player = TestPlayer::new(&world);
                let calls = cancelled.then(|| cancel(fixture, &player.player, launch));
                for item in [
                    &Item::EGG,
                    &Item::ENDER_PEARL,
                    &Item::SPLASH_POTION,
                    &Item::LINGERING_POTION,
                    &Item::EXPERIENCE_BOTTLE,
                    &Item::WIND_CHARGE,
                ] {
                    player
                        .player
                        .inventory
                        .set_stack(0, ItemStack::new(1, item));
                    let before = world.entities.load().len();
                    use_packet(&player, fixture);
                    assert_eq!(
                        world.entities.load().len() - before,
                        usize::from(!cancelled),
                        "{} entity admission",
                        item.registry_key
                    );
                    assert_eq!(
                        player.player.inventory.count(item),
                        u8::from(cancelled),
                        "{} payment",
                        item.registry_key
                    );
                }
                if let Some(calls) = calls {
                    assert_eq!(calls.load(SeqCst), 6);
                }
            })
            .await;
        }
    }
}

struct MoveRocket {
    player: Arc<Player>,
    cancel: bool,
}

struct GrantRocket(Arc<Player>);

impl EventHandler<ProjectileLaunchEvent> for GrantRocket {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        _: &'a mut ProjectileLaunchEvent,
    ) -> BoxFuture<'a, ()> {
        let mut stack = self.0.inventory().get_stack(0);
        stack.increment(1);
        self.0.inventory().set_stack(0, stack);
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn projectile_item_events_block_firework_packet_preserves_callback_grant() {
    use crate::entity::EntityBase;
    use pumpkin_data::{Block, biome::Biome};
    use pumpkin_protocol::{VarInt, java::server::play::SUseItemOn};
    use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
    use pumpkin_world::world::BlockFlags;
    run(|fixture| {
        let world = fixture.world();
        crate::world::spawn_test_support::publish(
            &world,
            crate::world::spawn_test_support::proto(&Biome::PLAINS, &Block::STONE),
        );
        let position = BlockPos::new(8, 64, 8);
        world.set_block_state(
            &position,
            Block::STONE.default_state.id,
            BlockFlags::FORCE_STATE,
        );
        let player = TestPlayer::new(&world);
        player
            .player
            .get_entity()
            .set_pos(Vector3::new(8.5, 65.0, 8.5));
        player
            .player
            .inventory
            .set_stack(0, ItemStack::new(3, &Item::FIREWORK_ROCKET));
        fixture
            .server
            .plugin_manager
            .register::<ProjectileLaunchEvent, _>(
                Arc::new(GrantRocket(player.player.clone())),
                EventPriority::Normal,
                true,
            );
        player
            .client()
            .handle_use_item_on(
                &player.player,
                &SUseItemOn {
                    hand: VarInt(0),
                    position,
                    face: VarInt(1),
                    cursor_pos: Vector3::new(0.5, 1.0, 0.5),
                    inside_block: false,
                    is_against_world_border: false,
                    sequence: VarInt(1),
                },
                &fixture.server,
            )
            .unwrap();
        assert_eq!(world.entities.load().len(), 1);
        assert_eq!(
            player.player.inventory.count(&Item::FIREWORK_ROCKET),
            3,
            "one paid, one granted, no second clone payment"
        );
    })
    .await;
}

impl EventHandler<ProjectileLaunchEvent> for MoveRocket {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut ProjectileLaunchEvent,
    ) -> BoxFuture<'a, ()> {
        let inventory = self.player.inventory();
        inventory.set_stack(1, inventory.get_stack(0));
        inventory.set_stack(0, ItemStack::new(1, &Item::DIAMOND));
        event.cancelled = self.cancel;
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn projectile_item_events_block_firework_claims_before_callback_replacement() {
    use pumpkin_data::{Block, BlockDirection};
    use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
    for veto in [None, Some(false), Some(true)] {
        run(|fixture| {
            let world = fixture.world();
            let player = TestPlayer::new(&world);
            let mut item = ItemStack::new(1, &Item::FIREWORK_ROCKET);
            player.player.inventory.set_stack(0, item.clone());
            if let Some(cancel) = veto {
                fixture
                    .server
                    .plugin_manager
                    .register::<ProjectileLaunchEvent, _>(
                        Arc::new(MoveRocket {
                            player: player.player.clone(),
                            cancel,
                        }),
                        EventPriority::Normal,
                        true,
                    );
            }
            let result = FireworkRocketItem.use_on_block(
                &mut item,
                &player.player,
                BlockPos::new(0, 64, 0),
                BlockDirection::Up,
                Vector3::new(0.5, 1.0, 0.5),
                &Block::STONE,
                &fixture.server,
            );
            let rejected = veto == Some(true);
            assert_eq!(result.consumes_action(), !rejected);
            assert_eq!(world.entities.load().len(), usize::from(!rejected));
            assert_eq!(
                player.player.inventory.count(&Item::FIREWORK_ROCKET),
                u8::from(rejected)
            );
            if veto.is_some() {
                assert_eq!(player.player.inventory.get_stack(0).item, &Item::DIAMOND);
            }
        })
        .await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn projectile_item_events_trident_refunds_only_paid_stock() {
    for creative in [false, true] {
        for veto in [None, Some(false), Some(true)] {
            run(|fixture| {
                let world = fixture.world();
                let player = TestPlayer::new(&world);
                if creative {
                    player.player.gamemode.store(GameMode::Creative);
                }
                let trident = ItemStack::new(1, &Item::TRIDENT);
                player.player.inventory.set_stack(0, trident.clone());
                let calls = veto.map(|launch| cancel(fixture, &player.player, launch));
                TridentItem.normal_use(&Item::TRIDENT, &player.player);
                player
                    .player
                    .living_entity
                    .item_use_time
                    .store(72000 - 20, SeqCst);
                TridentItem.on_stopped_using(&trident, &player.player);
                assert_eq!(world.entities.load().len(), usize::from(veto.is_none()));
                assert_eq!(
                    player.player.inventory.count(&Item::TRIDENT),
                    u8::from(creative || veto.is_some())
                );
                if veto.is_none() {
                    let entities = world.entities.load();
                    let thrown = entities[0]
                        .cast_any()
                        .downcast_ref::<crate::entity::projectile::trident::TridentEntity>()
                        .unwrap();
                    assert_eq!(
                        thrown.item_stack.lock().unwrap().get_damage(),
                        i32::from(!creative)
                    );
                }
                if veto.is_some() {
                    assert!(player.player.inventory.get_stack(0).are_equal(&trident));
                }
                if let Some(calls) = calls {
                    assert_eq!(calls.load(SeqCst), 1);
                }
            })
            .await;
        }
    }
}

struct RejectRound {
    player: Arc<Player>,
    reject: usize,
    replace: bool,
    calls: Arc<AtomicUsize>,
}

impl EventHandler<EntitySpawnEvent> for RejectRound {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut EntitySpawnEvent,
    ) -> BoxFuture<'a, ()> {
        let round = self.calls.fetch_add(1, SeqCst);
        if self.replace {
            self.player
                .inventory
                .set_stack(0, ItemStack::new(1, &Item::DIAMOND));
        } else {
            let mut weapon = self.player.inventory.get_stack(0);
            let mut nbt = NbtCompound::new();
            nbt.put_int("callback", 7);
            weapon.set_data_component(CustomDataImpl::new(nbt));
            self.player.inventory.set_stack(0, weapon);
        }
        event.cancelled = self.replace || round == self.reject;
        Box::pin(async {})
    }
}

fn weapon_with_rounds(item: &'static Item, rounds: usize) -> ItemStack {
    let mut weapon = ItemStack::new(1, &Item::CROSSBOW);
    weapon.set_data_component(ChargedProjectilesImpl {
        projectiles: (0..rounds)
            .map(|index| {
                let mut round = ItemStack::new(1, item);
                if index > 0 {
                    round.set_data_component(IntangibleProjectileImpl);
                }
                let mut nbt = NbtCompound::new();
                round.write_item_stack(&mut nbt);
                nbt
            })
            .collect(),
    });
    weapon
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn projectile_item_events_crossbow_partial_multishot_keeps_only_rejected_round() {
    for rejected in [0, 1] {
        run(|fixture| {
            let world = fixture.world();
            let player = TestPlayer::new(&world);
            player
                .player
                .inventory
                .set_stack(0, weapon_with_rounds(&Item::ARROW, 3));
            let calls = Arc::new(AtomicUsize::new(0));
            fixture
                .server
                .plugin_manager
                .register::<EntitySpawnEvent, _>(
                    Arc::new(RejectRound {
                        player: player.player.clone(),
                        reject: rejected,
                        replace: false,
                        calls: calls.clone(),
                    }),
                    EventPriority::Normal,
                    true,
                );
            CrossbowItem.normal_use(&Item::CROSSBOW, &player.player);
            assert_eq!(world.entities.load().len(), 2);
            assert_eq!(calls.load(SeqCst), 3);
            let actual = player.player.inventory.get_stack(0);
            assert_eq!(actual.get_damage(), 2);
            assert_eq!(
                actual
                    .get_data_component::<CustomDataImpl>()
                    .unwrap()
                    .data
                    .get_int("callback"),
                Some(7)
            );
            let charge = actual
                .get_data_component::<ChargedProjectilesImpl>()
                .unwrap();
            assert_eq!(charge.projectiles.len(), 1);
            let round = ItemStack::read_item_stack(&charge.projectiles[0]).unwrap();
            assert_eq!(
                round
                    .get_data_component::<IntangibleProjectileImpl>()
                    .is_some(),
                rejected > 0
            );
            // Retrying fires the retained round once, without replaying the admitted pair.
            CrossbowItem.normal_use(&Item::CROSSBOW, &player.player);
            assert_eq!(world.entities.load().len(), 3);
            assert_eq!(player.player.inventory.get_stack(0).get_damage(), 3);
        })
        .await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn projectile_item_events_crossbow_replaced_weapon_refunds_no_free_copies() {
    run(|fixture| {
        let world = fixture.world();
        let player = TestPlayer::new(&world);
        player
            .player
            .inventory
            .set_stack(0, weapon_with_rounds(&Item::ARROW, 3));
        fixture
            .server
            .plugin_manager
            .register::<EntitySpawnEvent, _>(
                Arc::new(RejectRound {
                    player: player.player.clone(),
                    reject: 0,
                    replace: true,
                    calls: Arc::new(AtomicUsize::new(0)),
                }),
                EventPriority::Normal,
                true,
            );
        CrossbowItem.normal_use(&Item::CROSSBOW, &player.player);
        assert_eq!(world.entities.load().len(), 0);
        assert_eq!(player.player.inventory.get_stack(0).item, &Item::DIAMOND);
        assert_eq!(player.player.inventory.count(&Item::ARROW), 1);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn projectile_item_events_crossbow_firework_payment_and_wear_follow_admission() {
    for veto in [None, Some(false), Some(true)] {
        run(|fixture| {
            let world = fixture.world();
            let player = TestPlayer::new(&world);
            let weapon = weapon_with_rounds(&Item::FIREWORK_ROCKET, 1);
            player.player.inventory.set_stack(0, weapon.clone());
            let calls = veto.map(|launch| cancel(fixture, &player.player, launch));
            CrossbowItem.normal_use(&Item::CROSSBOW, &player.player);
            assert_eq!(world.entities.load().len(), usize::from(veto.is_none()));
            let actual = player.player.inventory.get_stack(0);
            assert_eq!(actual.get_damage(), if veto.is_none() { 3 } else { 0 });
            if veto.is_some() {
                assert!(actual.are_equal(&weapon));
            }
            if let Some(calls) = calls {
                assert_eq!(calls.load(SeqCst), 1);
            }
        })
        .await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn projectile_item_events_fishing_bobber_rejection_keeps_rod_and_clears_owner() {
    for veto in [None, Some(false), Some(true)] {
        run(|fixture| {
            let world = fixture.world();
            let player = TestPlayer::new(&world);
            let rod = ItemStack::new(1, &Item::FISHING_ROD);
            player.player.inventory.set_stack(0, rod.clone());
            let calls = veto.map(|launch| cancel(fixture, &player.player, launch));
            use_packet(&player, fixture);
            assert_eq!(world.entities.load().len(), usize::from(veto.is_none()));
            assert!(player.player.inventory.get_stack(0).are_equal(&rod));
            assert_eq!(
                player.player.fishing_bobber.load(SeqCst) == -1,
                veto.is_some()
            );
            if let Some(calls) = calls {
                assert_eq!(calls.load(SeqCst), 1);
            }
        })
        .await;
    }
}
