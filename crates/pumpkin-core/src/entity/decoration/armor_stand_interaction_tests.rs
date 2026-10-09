use super::*;
use crate::{
    entity::death_test_world::DeathTestWorld,
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::{
            player::player_interact_at_entity::PlayerInteractAtEntityEvent,
            world::generic_game::GenericGameEvent,
        },
    },
    server::Server,
};
use pumpkin_data::{attributes::Attributes, entity::EntityType};
use pumpkin_protocol::{VarInt, java::server::play::SInteract};
use pumpkin_util::Hand;
use std::sync::Mutex;

fn stand(world: &Arc<crate::world::World>) -> ArmorStandEntity {
    let stand = ArmorStandEntity::new(Entity::new(
        world.clone(),
        Vector3::new(8.5, 64.0, 8.5),
        &EntityType::ARMOR_STAND,
    ));
    stand.set_show_arms(true);
    stand
}

#[tokio::test]
async fn disabled_clicked_slot_falls_back_to_mainhand() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let player = TestPlayer::new(&world);
    let stand = stand(&world);
    stand.set_item_slot(
        &EquipmentSlot::HEAD,
        &ItemStack::new(1, &Item::DIAMOND_HELMET),
    );
    stand.set_item_slot(&EquipmentSlot::MAIN_HAND, &ItemStack::new(1, &Item::STICK));
    stand.set_slot_disabled(&EquipmentSlot::HEAD, true);
    let mut hand = ItemStack::EMPTY.clone();
    assert!(stand.interact_at(&player.player, &mut hand, Vector3::new(0.0, 1.8, 0.0)));
    assert_eq!(hand.item, &Item::STICK);
    let mut helmet = ItemStack::new(1, &Item::DIAMOND_HELMET);
    assert!(!stand.interact(&player.player, &mut helmet));
    assert!(stand.item_in_slot(&EquipmentSlot::MAIN_HAND).is_empty());
    fixture.server.shutdown().await;
}

#[tokio::test]
async fn scaled_armor_stand_click_selects_correct_slot() {
    let fixture = crate::world::spawn_test_support::Fixture::new();
    let stand = stand(&fixture.world);
    stand.set_item_slot(
        &EquipmentSlot::FEET,
        &ItemStack::new(1, &Item::DIAMOND_BOOTS),
    );
    stand.set_item_slot(
        &EquipmentSlot::CHEST,
        &ItemStack::new(1, &Item::DIAMOND_CHESTPLATE),
    );
    stand
        .living_entity
        .set_attribute_base(&Attributes::SCALE, 3.0);
    assert!(stand.get_clicked_slot(Vector3::new(0.0, 1.0, 0.0)) == EquipmentSlot::FEET);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prevent_equipment_drop_survives_break() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let stand = stand(&world);
    let mut helmet = ItemStack::new(1, &Item::DIAMOND_HELMET);
    helmet.add_enchantment(&pumpkin_data::Enchantment::VANISHING_CURSE, 1);
    stand.set_item_slot(&EquipmentSlot::HEAD, &helmet);
    stand.set_item_slot(
        &EquipmentSlot::FEET,
        &ItemStack::new(1, &Item::DIAMOND_BOOTS),
    );
    stand
        .get_entity()
        .set_custom_name(pumpkin_util::text::TextComponent::text("My stand"));
    stand.break_and_drop_items();
    let stacks: Vec<_> = world
        .entities
        .load()
        .iter()
        .filter_map(|e| e.get_item_entity())
        .map(|e| e.get_item_stack().lock().unwrap().clone())
        .collect();
    assert_eq!(stacks.len(), 2);
    assert!(stacks.iter().all(|s| s.item != &Item::DIAMOND_HELMET));
    assert!(
        stacks
            .iter()
            .find(|s| s.item == &Item::ARMOR_STAND)
            .unwrap()
            .get_data_component::<pumpkin_data::data_component_impl::CustomNameImpl>()
            .is_some()
    );
    assert!(
        stand
            .living_entity
            .entity_equipment
            .lock()
            .unwrap()
            .equipment
            .is_empty()
    );
    fixture.server.shutdown().await;
}

struct EquipEvents(Mutex<Vec<String>>);
impl EventHandler<GenericGameEvent> for EquipEvents {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut GenericGameEvent,
    ) -> BoxFuture<'a, ()> {
        if matches!(event.event_key.as_str(), "equip" | "unequip") {
            self.0.lock().unwrap().push(event.event_key.clone());
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn armor_stand_equipment_emits_one_event_per_change() {
    let fixture = DeathTestWorld::new().await;
    let stand = stand(&fixture.world());
    let events = Arc::new(EquipEvents(Mutex::default()));
    fixture
        .server
        .plugin_manager
        .register::<GenericGameEvent, _>(events.clone(), EventPriority::Normal, true);
    stand.set_item_slot(
        &EquipmentSlot::HEAD,
        &ItemStack::new(1, &Item::DIAMOND_HELMET),
    );
    assert!(events.0.lock().unwrap().is_empty());
    stand.living_entity.combat_ticks.store(1, Ordering::Relaxed);
    stand.set_item_slot(&EquipmentSlot::HEAD, &ItemStack::new(1, &Item::IRON_HELMET));
    stand.set_item_slot(&EquipmentSlot::HEAD, &ItemStack::new(1, &Item::IRON_HELMET));
    stand.set_item_slot(&EquipmentSlot::HEAD, &ItemStack::EMPTY.clone());
    assert_eq!(*events.0.lock().unwrap(), ["equip", "unequip"]);
    fixture.server.shutdown().await;
}

struct AdjustHit {
    target: i32,
}
impl EventHandler<PlayerInteractAtEntityEvent> for AdjustHit {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerInteractAtEntityEvent,
    ) -> BoxFuture<'a, ()> {
        event.entity_id = self.target;
        event.clicked_y = 1.8;
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interact_at_honors_plugin_adjusted_position_and_skips_item_use_stats() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let player = TestPlayer::new(&world);
    let original = Arc::new(stand(&world));
    let target = Arc::new(stand(&world));
    target.set_item_slot(
        &EquipmentSlot::HEAD,
        &ItemStack::new(1, &Item::DIAMOND_HELMET),
    );
    world.add_entity_silent(original.clone());
    world.add_entity_silent(target.clone());
    fixture
        .server
        .plugin_manager
        .register::<PlayerInteractAtEntityEvent, _>(
            Arc::new(AdjustHit {
                target: target.get_entity().entity_id,
            }),
            EventPriority::Normal,
            true,
        );
    player.client().handle_interact(
        &player.player,
        &SInteract {
            entity_id: VarInt(original.get_entity().entity_id),
            r#type: VarInt(2),
            target_position: Some(Vector3::new(0.0, 0.1, 0.0)),
            hand: Some(VarInt(1)),
            sneaking: false,
        },
        &fixture.server,
    );
    assert_eq!(
        player.player.inventory().get_stack_in_hand(Hand::Left).item,
        &Item::DIAMOND_HELMET
    );
    assert!(target.item_in_slot(&EquipmentSlot::HEAD).is_empty());
    assert_eq!(
        player.player.stats.lock().unwrap().get(
            pumpkin_data::statistic::StatisticCategory::Used,
            i32::from(Item::AIR.id)
        ),
        0
    );
    assert_eq!(
        player.player.stats.lock().unwrap().get(
            pumpkin_data::statistic::StatisticCategory::Used,
            i32::from(Item::DIAMOND_HELMET.id)
        ),
        0
    );
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn java_equipment_transfer_never_records_a_durability_break() {
    use pumpkin_data::statistic::StatisticCategory;
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let mut player = TestPlayer::new(&world);
    let stand = Arc::new(stand(&world));
    world.add_entity_silent(stand.clone());
    let helmet = ItemStack::new(1, &Item::DIAMOND_HELMET);
    player
        .player
        .inventory()
        .set_stack_in_hand(Hand::Left, helmet.clone());
    player.take_packets();
    player.client().handle_interact(
        &player.player,
        &SInteract {
            entity_id: VarInt(stand.get_entity().entity_id),
            r#type: VarInt(0),
            target_position: None,
            hand: Some(VarInt(1)),
            sneaking: false,
        },
        &fixture.server,
    );
    assert!(stand.item_in_slot(&EquipmentSlot::HEAD).are_equal(&helmet));
    assert!(
        player
            .player
            .inventory()
            .get_stack_in_hand(Hand::Left)
            .is_empty()
    );
    assert_eq!(
        player.player.stats.lock().unwrap().get(
            StatisticCategory::Broken,
            i32::from(Item::DIAMOND_HELMET.id)
        ),
        0
    );
    let status = player
        .client()
        .serialize_packet(&pumpkin_protocol::java::client::play::CEntityStatus::new(
            player.player.entity_id(),
            crate::entity::equipment_break_status(&EquipmentSlot::OFF_HAND) as i8,
        ))
        .unwrap();
    assert!(!player.take_packets().contains(&status));
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bedrock_equipment_transfer_never_records_a_durability_break() {
    use crate::net::bedrock::combat_test_support::TestBedrockPlayer;
    use pumpkin_data::statistic::StatisticCategory;
    use pumpkin_protocol::{
        bedrock::{
            network_item::NetworkItemDescriptor,
            server::inventory_transaction::{
                SInventoryTransaction, TransactionData, UseItemOnEntityTransactionData,
            },
        },
        codec::{var_uint::VarUInt, var_ulong::VarULong},
    };
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let mut player = TestBedrockPlayer::new(&world).await;
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(8.5, 64.0, 8.5));
    // A Java observer captures the status broadcast that Bedrock's old bookkeeping also sent.
    let mut observer = TestPlayer::new(&world);
    world.players.store(Arc::new(vec![
        player.player.clone(),
        observer.player.clone(),
    ]));
    world
        .entity_tracker
        .add_entity(&(player.player.clone() as Arc<dyn EntityBase>), &world);
    let stand = Arc::new(stand(&world));
    world.add_entity_silent(stand.clone());
    let helmet = ItemStack::new(1, &Item::DIAMOND_HELMET);
    player
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, helmet.clone());
    observer.take_packets();
    player.take_packets();
    player.client().handle_inventory_action(
        &player.player,
        SInventoryTransaction {
            legacy_request_id: VarInt(0),
            legacy_set_item_slots: Vec::new(),
            has_value: false,
            actions: Vec::new(),
            transaction_type: VarUInt(3),
            transaction_data: TransactionData::UseItemOnEntity(UseItemOnEntityTransactionData {
                target_entity_runtime_id: VarULong(stand.get_entity().entity_id as u64),
                action_type: VarInt(0),
                hot_bar_slot: VarInt(0),
                item_in_hand: NetworkItemDescriptor::default(),
                player_position: Vector3::new(0.0, 64.0, 0.0),
                click_position: Vector3::new(0.0, 0.0, 0.0),
            }),
        },
    );
    assert!(stand.item_in_slot(&EquipmentSlot::HEAD).are_equal(&helmet));
    assert!(player.player.inventory().held_item().is_empty());
    assert_eq!(
        player.player.stats.lock().unwrap().get(
            StatisticCategory::Broken,
            i32::from(Item::DIAMOND_HELMET.id)
        ),
        0
    );
    let status = observer
        .client()
        .serialize_packet(&pumpkin_protocol::java::client::play::CEntityStatus::new(
            player.player.entity_id(),
            crate::entity::equipment_break_status(&EquipmentSlot::MAIN_HAND) as i8,
        ))
        .unwrap();
    assert!(!observer.take_packets().contains(&status));
    player.close().await;
    fixture.server.shutdown().await;
}
