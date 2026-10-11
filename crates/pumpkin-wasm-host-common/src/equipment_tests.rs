// Compiled by both versioned hosts so these regressions call each actual WIT host implementation.
use super::*;
use pumpkin_core::{
    entity::{Entity, EntityBase, decoration::armor_stand::ArmorStandEntity},
    net::java::combat_test_support::TestPlayer,
    server::{Server, combat_test_support},
};
use pumpkin_data::{
    attributes::Attributes,
    data_component_impl::{AttributeModifiersImpl, EquipmentSlot},
    entity::EntityType,
    item::Item,
    item_stack::ItemStack,
    packet::{CURRENT_MC_VERSION, clientbound::play::SET_EQUIPMENT},
};
use pumpkin_protocol::{ServerPacket, codec::var_int::VarInt, java::client::play::CSetEquipment};
use pumpkin_util::math::vector3::Vector3;

struct Fixture {
    _directory: tempfile::TempDir,
    server: Arc<Server>,
    player: TestPlayer,
    observer: TestPlayer,
    state: PluginHostState,
    entity: Resource<WitLivingEntity>,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        let world = combat_test_support::world(&server, directory.path());
        let player = TestPlayer::new(&world);
        let observer = TestPlayer::new(&world);
        server.worlds.store(Arc::new(vec![world.clone()]));
        world.players.store(Arc::new(vec![
            player.player.clone(),
            observer.player.clone(),
        ]));
        observer
            .player
            .chunk_sender
            .lock()
            .unwrap()
            .mark_sent_out_of_band(player.player.get_entity().chunk_pos.load());
        world
            .entity_tracker
            .get_tracked_entity(player.player.entity_id())
            .unwrap()
            .update_player(&observer.player, &world);
        player.player.inventory.set_selected_slot(3);
        player
            .player
            .inventory
            .set_slot(0, ItemStack::new(1, &Item::STONE));
        let mut state = PluginHostState::new();
        let entity = state
            .add::<WitLivingEntity>(player.player.clone() as Arc<dyn EntityBase>)
            .unwrap();
        Self {
            _directory: directory,
            server,
            player,
            observer,
            state,
            entity,
        }
    }

    fn borrowed(&self) -> Resource<WitLivingEntity> {
        Resource::new_borrow(self.entity.rep())
    }

    async fn set_hand(&mut self, stack: Option<ItemStack>) {
        let stack = stack.map(|stack| {
            self.state
                .add::<WitHostItemStack>(Arc::new(tokio::sync::Mutex::new(stack)))
                .unwrap()
        });
        let entity = self.borrowed();
        HostLivingEntity::set_equipment(&mut self.state, entity, WitEquipmentSlot::MainHand, stack)
            .await
            .unwrap();
    }

    async fn assert_hand(&mut self, expected: &ItemStack) {
        let entity = self.borrowed();
        let stack =
            HostLivingEntity::get_equipment(&mut self.state, entity, WitEquipmentSlot::MainHand)
                .unwrap();
        if let Some(stack) = stack {
            assert!(
                self.state
                    .take(stack)
                    .unwrap()
                    .lock()
                    .await
                    .are_equal(expected)
            );
        } else {
            assert!(expected.is_empty());
        }
        assert!(self.player.player.inventory.held_item().are_equal(expected));
        assert_eq!(self.player.player.inventory.get_slot(0).item, &Item::STONE);
    }

    fn assert_publication(&mut self, expected: &ItemStack) {
        self.player.player.tick(&self.server);
        let updates: Vec<_> = self
            .observer
            .take_packets()
            .into_iter()
            .filter_map(|packet| {
                let mut data = packet.as_ref();
                (VarInt::decode(&mut data).ok()?.0 == SET_EQUIPMENT.0)
                    .then(|| CSetEquipment::read(&mut data, &CURRENT_MC_VERSION).ok())
                    .flatten()
            })
            .filter(|update| update.entity_id.0 == self.player.player.entity_id())
            .flat_map(|update| update.equipment)
            .collect();
        let last = updates
            .iter()
            .rfind(|(slot, _)| *slot == EquipmentSlot::MAIN_HAND.discriminant());
        assert!(last.is_some_and(|(_, stack)| stack.0.are_equal(expected)));
        let modifiers = expected.get_data_component::<AttributeModifiersImpl>();
        for attribute in [&Attributes::ATTACK_DAMAGE, &Attributes::ATTACK_SPEED] {
            let attributes = self.player.player.living_entity.attributes.read().unwrap();
            let actual: Vec<_> = attributes[&attribute.id]
                .modifiers
                .iter()
                .map(|modifier| (modifier.id.as_str(), modifier.amount))
                .collect();
            let expected: Vec<_> = modifiers
                .iter()
                .flat_map(|component| component.attribute_modifiers.iter())
                .filter(|modifier| modifier.r#type == attribute)
                .map(|modifier| (modifier.id, modifier.amount))
                .collect();
            assert_eq!(actual, expected);
        }
    }
}

#[tokio::test]
async fn wasm_main_hand_get_agrees_with_selected_inventory_publication() {
    let mut fixture = Fixture::new();
    // A legacy shadow entry must not override the selected inventory item on reads.
    fixture
        .player
        .player
        .living_entity
        .entity_equipment
        .lock()
        .unwrap()
        .put(&EquipmentSlot::MAIN_HAND, ItemStack::new(1, &Item::STONE));
    let sword = ItemStack::new(1, &Item::DIAMOND_SWORD);
    fixture.player.player.inventory.set_held_item(sword.clone());
    fixture.observer.take_packets();
    fixture
        .player
        .player
        .living_entity
        .send_equipment_changes(&[(EquipmentSlot::MAIN_HAND, sword.clone())]);
    fixture.assert_publication(&sword);
    fixture.assert_hand(&sword).await;
}

#[tokio::test]
async fn wasm_main_hand_set_and_remove_publish_selected_inventory() {
    let mut fixture = Fixture::new();
    fixture.observer.take_packets();
    for item in [Some(&Item::DIAMOND_SWORD), Some(&Item::IRON_SWORD), None] {
        let expected =
            item.map_or_else(|| ItemStack::EMPTY.clone(), |item| ItemStack::new(1, item));
        fixture
            .set_hand(item.map(|item| ItemStack::new(1, item)))
            .await;
        fixture.assert_hand(&expected).await;
        fixture.assert_publication(&expected);
    }
}

#[tokio::test]
async fn wasm_clear_equipment_publishes_empty_selected_hand_without_shadow_entry() {
    let mut fixture = Fixture::new();
    let sword = ItemStack::new(1, &Item::DIAMOND_SWORD);
    fixture.set_hand(Some(sword.clone())).await;
    fixture.assert_publication(&sword);
    fixture
        .player
        .player
        .living_entity
        .entity_equipment
        .lock()
        .unwrap()
        .equipment
        .remove(&EquipmentSlot::MAIN_HAND);
    let entity = fixture.borrowed();
    HostLivingEntity::clear_equipment(&mut fixture.state, entity).unwrap();
    fixture.assert_hand(ItemStack::EMPTY).await;
    fixture.assert_publication(ItemStack::EMPTY);
}

#[tokio::test]
async fn wasm_non_player_main_hand_keeps_entity_equipment_storage() {
    let mut fixture = Fixture::new();
    let stand = Arc::new(ArmorStandEntity::new(Entity::new(
        fixture.player.player.world(),
        Vector3::default(),
        &EntityType::ARMOR_STAND,
    )));
    let entity = fixture
        .state
        .add::<WitLivingEntity>(stand.clone() as Arc<dyn EntityBase>)
        .unwrap();
    let sword = ItemStack::new(1, &Item::IRON_SWORD);
    let stack = fixture
        .state
        .add::<WitHostItemStack>(Arc::new(tokio::sync::Mutex::new(sword.clone())))
        .unwrap();
    HostLivingEntity::set_equipment(
        &mut fixture.state,
        Resource::new_borrow(entity.rep()),
        WitEquipmentSlot::MainHand,
        Some(stack),
    )
    .await
    .unwrap();
    assert!(
        stand
            .get_living_entity()
            .unwrap()
            .entity_equipment
            .lock()
            .unwrap()
            .get(&EquipmentSlot::MAIN_HAND)
            .are_equal(&sword)
    );
    let read = HostLivingEntity::get_equipment(
        &mut fixture.state,
        Resource::new_borrow(entity.rep()),
        WitEquipmentSlot::MainHand,
    )
    .unwrap()
    .unwrap();
    assert!(
        fixture
            .state
            .take(read)
            .unwrap()
            .lock()
            .await
            .are_equal(&sword)
    );
    HostLivingEntity::clear_equipment(&mut fixture.state, Resource::new_borrow(entity.rep()))
        .unwrap();
    assert!(
        stand
            .get_living_entity()
            .unwrap()
            .entity_equipment
            .lock()
            .unwrap()
            .get(&EquipmentSlot::MAIN_HAND)
            .is_empty()
    );
}
