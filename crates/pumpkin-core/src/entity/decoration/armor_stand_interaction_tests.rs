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
        ItemStack::new(1, &Item::DIAMOND_HELMET),
    );
    stand.set_item_slot(&EquipmentSlot::MAIN_HAND, ItemStack::new(1, &Item::STICK));
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
        ItemStack::new(1, &Item::DIAMOND_BOOTS),
    );
    stand.set_item_slot(
        &EquipmentSlot::CHEST,
        ItemStack::new(1, &Item::DIAMOND_CHESTPLATE),
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
    stand.set_item_slot(&EquipmentSlot::HEAD, helmet);
    stand.set_item_slot(
        &EquipmentSlot::FEET,
        ItemStack::new(1, &Item::DIAMOND_BOOTS),
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
        ItemStack::new(1, &Item::DIAMOND_HELMET),
    );
    assert!(events.0.lock().unwrap().is_empty());
    stand.living_entity.combat_ticks.store(1, Ordering::Relaxed);
    stand.set_item_slot(&EquipmentSlot::HEAD, ItemStack::new(1, &Item::IRON_HELMET));
    stand.set_item_slot(&EquipmentSlot::HEAD, ItemStack::new(1, &Item::IRON_HELMET));
    stand.set_item_slot(&EquipmentSlot::HEAD, ItemStack::EMPTY.clone());
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
        ItemStack::new(1, &Item::DIAMOND_HELMET),
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
