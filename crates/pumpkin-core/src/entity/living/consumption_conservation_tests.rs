use crate::{
    entity::EntityBase,
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::player::player_statistic_increment::PlayerStatisticIncrementEvent,
    },
    server::{Server, combat_test_support},
};
use pumpkin_data::{
    data_component_impl::{ConsumableImpl, PotionContentsImpl, basic::DamageImpl},
    item::Item,
    item_stack::ItemStack,
    potion::Potion,
};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::{codec::var_int::VarInt, java::server::play::SUseItem};
use pumpkin_util::{Hand, math::vector2::Vector2};
use std::sync::Arc;

struct EditHandOnUseStatistic {
    hand: Hand,
}
impl EventHandler<PlayerStatisticIncrementEvent> for EditHandOnUseStatistic {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerStatisticIncrementEvent,
    ) -> BoxFuture<'a, ()> {
        if event.statistic_id == format!("Used:{}", Item::POTION.id) {
            let inventory = event.player.inventory();
            let owner = inventory.slot_transaction(0);
            std::thread::scope(|scope| {
                scope
                    .spawn(move || {
                        assert!(
                            owner.try_lock().is_some(),
                            "consumption callback retained its owner"
                        );
                    })
                    .join()
                    .unwrap();
            });
            let mut current = inventory.get_stack_in_hand(self.hand);
            if current.is_empty() {
                // Remainder delivery must not replace an edit made after the successful claim.
                current = ItemStack::new(1, &Item::STONE);
            } else {
                // The reported exploit retains UID/count while changing unrelated components.
                current.set_data_component(DamageImpl { damage: 7 });
            }
            inventory.set_stack_in_hand(self.hand, current);
        }
        Box::pin(async {})
    }
}

async fn consumption_through_real_entry(instant: bool) {
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    combat_test_support::publish_empty_chunk(&world, Vector2::new(0, 0));
    let fixture = TestPlayer::new(&world);
    let player = &fixture.player;
    let hand = if instant { Hand::Left } else { Hand::Right };
    server
        .plugin_manager
        .register::<PlayerStatisticIncrementEvent, _>(
            Arc::new(EditHandOnUseStatistic { hand }),
            EventPriority::Normal,
            true,
        );
    for count in [1, 2] {
        for slot in 0..player.inventory.size() {
            player.inventory.set_stack(slot, ItemStack::EMPTY.clone());
        }
        player.living_entity.set_health(10.0);
        let mut potion = ItemStack::new(count, &Item::POTION);
        potion.set_data_component(PotionContentsImpl {
            potion_id: Some(i32::from(Potion::HEALING.id)),
            custom_effects: Vec::new(),
            custom_color: None,
            custom_name: None,
        });
        potion
            .get_data_component_mut::<ConsumableImpl>()
            .unwrap()
            .consume_seconds = if instant { 0.0 } else { 0.05 };
        player.inventory.set_stack_in_hand(hand, potion);
        fixture.client().handle_use_item(
            player,
            &SUseItem {
                hand: VarInt(i32::from(hand == Hand::Left)),
                sequence: VarInt(1),
                yaw: 0.0,
                pitch: 0.0,
            },
            &server,
        );
        if !instant {
            // Drive timed completion through the actual living-entity tick.
            player.living_entity.tick(player.as_ref(), &server);
        }
        assert_eq!(
            player.living_entity.health.load(),
            14.0,
            "healing did not run"
        );
        let remaining: u16 = (0..player.inventory.size())
            .map(|i| player.inventory.get_stack(i))
            .filter(|stack| stack.item == &Item::POTION && !stack.is_empty())
            .map(|stack| u16::from(stack.item_count))
            .sum();
        assert_eq!(
            remaining,
            u16::from(count - 1),
            "effect was published without consuming potion"
        );
        assert_eq!(
            (0..player.inventory.size())
                .map(|i| player.inventory.get_stack(i))
                .filter(|stack| stack.item == &Item::GLASS_BOTTLE && !stack.is_empty())
                .map(|stack| u16::from(stack.item_count))
                .sum::<u16>(),
            1
        );
        let hand_stack = player.inventory.get_stack_in_hand(hand);
        if count == 1 {
            assert_eq!(
                hand_stack.item,
                &Item::STONE,
                "remainder overwrote a callback edit"
            );
        } else {
            assert_eq!(
                hand_stack
                    .get_data_component::<DamageImpl>()
                    .unwrap()
                    .damage,
                7
            );
        }
    }
    assert!(world.level.shutdown().await.is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn java_conservation_timed_consumption_claims_before_statistic_edit() {
    consumption_through_real_entry(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn java_conservation_instant_consumption_claims_before_statistic_edit() {
    consumption_through_real_entry(true).await;
}

struct SwapBottleOnUseStatistic;
impl EventHandler<PlayerStatisticIncrementEvent> for SwapBottleOnUseStatistic {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        event: &'a mut PlayerStatisticIncrementEvent,
    ) -> BoxFuture<'a, ()> {
        if event.statistic_id == format!("Used:{}", Item::GLASS_BOTTLE.id) {
            let mut swap = crate::inventory_transaction_tests::click();
            swap.sync_id = VarInt(0);
            swap.slot = 45; // The real player's offhand menu slot.
            swap.mode = pumpkin_protocol::java::server::play::SlotActionType::Swap;
            event.player.on_slot_click(swap, server);
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn java_conservation_dragon_bottle_claim_survives_statistic_hand_swap() {
    use crate::entity::{Entity, area_effect_cloud::AreaEffectCloudEntity, living::LivingEntity};
    use pumpkin_data::entity::EntityType;
    use pumpkin_util::math::vector3::Vector3;
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    combat_test_support::publish_empty_chunk(&world, Vector2::new(0, 0));
    let fixture = TestPlayer::new(&world);
    let player = &fixture.player;
    let pos = Vector3::new(0.5, 64.0, 0.5);
    player.get_entity().set_pos(pos);
    let dragon = Arc::new(LivingEntity::new(Entity::new(
        world.clone(),
        pos,
        &EntityType::ENDER_DRAGON,
    )));
    let cloud = AreaEffectCloudEntity::new(Entity::new(
        world.clone(),
        pos,
        &EntityType::AREA_EFFECT_CLOUD,
    ));
    let cloud_entity = cloud
        .cast_any()
        .downcast_ref::<AreaEffectCloudEntity>()
        .unwrap();
    cloud_entity.set_owner(Some(dragon.as_ref()));
    world.entities.store(Arc::new(vec![dragon, cloud.clone()]));
    server
        .plugin_manager
        .register::<PlayerStatisticIncrementEvent, _>(
            Arc::new(SwapBottleOnUseStatistic),
            EventPriority::Normal,
            true,
        );
    for count in [1, 2] {
        for slot in 0..player.inventory.size() {
            player.inventory.set_stack(slot, ItemStack::EMPTY.clone());
        }
        cloud_entity.set_radius(3.0);
        player
            .inventory
            .set_stack(0, ItemStack::new(1, &Item::STONE));
        player
            .inventory
            .set_stack_in_hand(Hand::Left, ItemStack::new(count, &Item::GLASS_BOTTLE));
        fixture.client().handle_use_item(
            player,
            &SUseItem {
                hand: VarInt(1),
                sequence: VarInt(1),
                yaw: 0.0,
                pitch: 0.0,
            },
            &server,
        );
        assert_eq!(cloud_entity.radius(), 2.5);
        let breath: u16 = (0..player.inventory.size())
            .map(|i| player.inventory.get_stack(i))
            .filter(|stack| stack.item == &Item::DRAGON_BREATH && !stack.is_empty())
            .map(|stack| u16::from(stack.item_count))
            .sum();
        assert_eq!(
            breath, 1,
            "cloud shrank but admitted bottle output was lost"
        );
        assert_eq!(
            player.inventory.get_stack_in_hand(Hand::Left).item,
            &Item::STONE
        );
        let bottles: u16 = (0..player.inventory.size())
            .map(|i| player.inventory.get_stack(i))
            .filter(|stack| stack.item == &Item::GLASS_BOTTLE && !stack.is_empty())
            .map(|stack| u16::from(stack.item_count))
            .sum();
        assert_eq!(bottles, u16::from(count - 1));
    }
    assert!(world.level.shutdown().await.is_ok());
}
