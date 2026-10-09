use super::*;
use crate::{
    entity::{EntityBase, death_test_world::DeathTestWorld},
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::player::{
            player_item_break::PlayerItemBreakEvent, player_item_damage::PlayerItemDamageEvent,
            player_shear_entity::PlayerShearEntityEvent,
        },
    },
    server::Server,
};
use pumpkin_data::{item::Item, statistic::StatisticCategory};
use pumpkin_protocol::{codec::var_int::VarInt, java::server::play::SInteract};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering::Relaxed},
};

struct DamageControl {
    cancelled: AtomicBool,
    amount: AtomicI32,
    calls: AtomicUsize,
    breaks: AtomicUsize,
    hands: Mutex<Vec<u8>>,
}
impl EventHandler<PlayerItemDamageEvent> for DamageControl {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerItemDamageEvent,
    ) -> BoxFuture<'a, ()> {
        self.calls.fetch_add(1, Relaxed);
        event.cancelled = self.cancelled.load(Relaxed);
        event.damage = self.amount.load(Relaxed);
        Box::pin(async {})
    }
}
impl EventHandler<PlayerItemBreakEvent> for DamageControl {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        _: &'a mut PlayerItemBreakEvent,
    ) -> BoxFuture<'a, ()> {
        self.breaks.fetch_add(1, Relaxed);
        Box::pin(async {})
    }
}
impl EventHandler<PlayerShearEntityEvent> for DamageControl {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerShearEntityEvent,
    ) -> BoxFuture<'a, ()> {
        self.hands.lock().unwrap().push(event.hand);
        Box::pin(async {})
    }
}
fn click(player: &TestPlayer, target: &dyn EntityBase, server: &Arc<Server>) {
    player.client().handle_interact(
        &player.player,
        &SInteract {
            entity_id: VarInt(target.get_entity().entity_id),
            r#type: VarInt(0),
            target_position: None,
            hand: Some(VarInt(1)),
            sneaking: false,
        },
        server,
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn offhand_shear_respects_damage_event_and_claims_once() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let mut player = TestPlayer::new(&world);
    let control = Arc::new(DamageControl {
        cancelled: AtomicBool::new(true),
        amount: AtomicI32::new(5),
        calls: AtomicUsize::new(0),
        breaks: AtomicUsize::new(0),
        hands: Mutex::default(),
    });
    let manager = &fixture.server.plugin_manager;
    manager.register::<PlayerItemDamageEvent, _>(control.clone(), EventPriority::Normal, true);
    manager.register::<PlayerItemBreakEvent, _>(control.clone(), EventPriority::Normal, true);
    manager.register::<PlayerShearEntityEvent, _>(control.clone(), EventPriority::Normal, true);
    for hand in [Hand::Right, Hand::Left] {
        player
            .player
            .inventory()
            .set_stack_in_hand(hand, ItemStack::new(1, &Item::SHEARS));
    }
    let first = fixture.mob(&pumpkin_data::entity::EntityType::SHEEP);
    click(&player, first.as_ref(), &fixture.server);
    click(&player, first.as_ref(), &fixture.server);
    assert_eq!(*control.hands.lock().unwrap(), [1]);
    assert_eq!(control.calls.load(Relaxed), 1);
    assert_eq!(
        player
            .player
            .inventory()
            .get_stack_in_hand(Hand::Left)
            .get_damage(),
        0
    );
    control.cancelled.store(false, Relaxed);
    let second = fixture.mob(&pumpkin_data::entity::EntityType::SHEEP);
    click(&player, second.as_ref(), &fixture.server);
    assert_eq!(
        player
            .player
            .inventory()
            .get_stack_in_hand(Hand::Left)
            .get_damage(),
        5
    );
    assert_eq!(
        player
            .player
            .inventory()
            .get_stack_in_hand(Hand::Right)
            .get_damage(),
        0
    );
    check_last_shear_break(&fixture, &mut player, &control);
    fixture.server.shutdown().await;
}

fn check_last_shear_break(
    fixture: &DeathTestWorld,
    player: &mut TestPlayer,
    control: &DamageControl,
) {
    let mut last = player.player.inventory().get_stack_in_hand(Hand::Left);
    last.set_damage(last.get_max_damage().unwrap() - 1);
    player
        .player
        .inventory()
        .set_stack_in_hand(Hand::Left, last);
    player.take_packets();
    let third = fixture.mob(&pumpkin_data::entity::EntityType::SHEEP);
    click(player, third.as_ref(), &fixture.server);
    assert!(
        player
            .player
            .inventory()
            .get_stack_in_hand(Hand::Left)
            .is_empty()
    );
    assert_eq!(control.breaks.load(Relaxed), 1);
    assert_eq!(
        player
            .player
            .stats
            .lock()
            .unwrap()
            .get(StatisticCategory::Broken, i32::from(Item::SHEARS.id)),
        1
    );
    let status = player
        .client()
        .serialize_packet(&pumpkin_protocol::java::client::play::CEntityStatus::new(
            player.player.entity_id(),
            crate::entity::equipment_break_status(
                &pumpkin_data::data_component_impl::EquipmentSlot::OFF_HAND,
            ) as i8,
        ))
        .unwrap();
    assert_eq!(
        player
            .take_packets()
            .iter()
            .filter(|packet| **packet == status)
            .count(),
        1
    );
}

async fn independent_damage_contract(cancelled: bool, amount: i32, breaking: bool) {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let mut player = TestPlayer::new(&world);
    let control = Arc::new(DamageControl {
        cancelled: AtomicBool::new(cancelled),
        amount: AtomicI32::new(amount),
        calls: AtomicUsize::new(0),
        breaks: AtomicUsize::new(0),
        hands: Mutex::default(),
    });
    fixture
        .server
        .plugin_manager
        .register::<PlayerItemDamageEvent, _>(control.clone(), EventPriority::Normal, true);
    fixture
        .server
        .plugin_manager
        .register::<PlayerItemBreakEvent, _>(control.clone(), EventPriority::Normal, true);
    let mut tool = ItemStack::new(1, &Item::SHEARS);
    if breaking {
        tool.set_damage(tool.get_max_damage().unwrap() - 1);
    }
    player
        .player
        .inventory()
        .set_stack_in_hand(Hand::Left, tool);
    player.take_packets();
    if breaking {
        check_last_shear_break(&fixture, &mut player, &control);
    } else {
        let sheep = fixture.mob(&pumpkin_data::entity::EntityType::SHEEP);
        click(&player, sheep.as_ref(), &fixture.server);
        assert_eq!(
            player
                .player
                .inventory()
                .get_stack_in_hand(Hand::Left)
                .get_damage(),
            if cancelled { 0 } else { amount }
        );
        assert_eq!(control.breaks.load(Relaxed), 0);
    }
    assert_eq!(control.calls.load(Relaxed), 1);
    fixture.server.shutdown().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_shear_durability_keeps_tool_undamaged() {
    independent_damage_contract(true, 5, false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn adjusted_shear_durability_uses_event_amount() {
    independent_damage_contract(false, 5, false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn final_shear_sends_exactly_one_break_event_stat_and_status() {
    independent_damage_contract(false, 1, true).await;
}
