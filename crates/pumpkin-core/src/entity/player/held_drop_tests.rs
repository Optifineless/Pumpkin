#![expect(
    clippy::unwrap_used,
    reason = "Regression fixtures require successful setup"
)]

use super::Player;
use crate::{
    entity::{
        EntityBase,
        attributes::{Modifier, ModifierOperation},
    },
    net::{bedrock::combat_test_support::TestBedrockPlayer, java::combat_test_support::TestPlayer},
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::player::player_drop_item::PlayerDropItemEvent,
    },
    server::{Server, combat_test_support},
};
use pumpkin_data::{
    attributes::Attributes,
    data_component_impl::{EquipmentSlot, MaxStackSizeImpl},
    item::Item,
    item_stack::ItemStack,
    packet::{
        CURRENT_MC_VERSION,
        clientbound::play::{SET_EQUIPMENT, UPDATE_ATTRIBUTES},
    },
};
use pumpkin_inventory::screen_handler::InventoryPlayer;
use pumpkin_protocol::{
    ServerPacket, VarInt,
    bedrock::{
        client::update_attributes::CUpdateAttributes as BedrockAttributes,
        server::player_action::{PlayerActionType as PlayerAction, SPlayerAction as BedrockAction},
    },
    codec::{var_uint::VarUInt, var_ulong::VarULong},
    java::{
        client::play::{CSetEquipment, CUpdateAttributes, Property},
        server::play::{SAttack, SPlayerAction, Status},
    },
    packet::Packet,
    serial::PacketRead,
};
use pumpkin_util::math::{position::BlockPos, vector2::Vector2, vector3::Vector3};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Clone, Copy)]
enum Route {
    JavaSingle,
    JavaStack,
    Bedrock,
}
const ROUTES: [Route; 3] = [Route::JavaSingle, Route::JavaStack, Route::Bedrock];
const COMBAT_ATTRIBUTES: [Attributes; 2] = [Attributes::ATTACK_DAMAGE, Attributes::ATTACK_SPEED];

struct Fixture {
    _directory: tempfile::TempDir,
    server: Arc<Server>,
    java: TestPlayer,
    bedrock: TestBedrockPlayer,
    observer: TestPlayer,
    route: Route,
}

impl Fixture {
    async fn new(route: Route) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        let world = combat_test_support::world(&server, directory.path());
        combat_test_support::publish_empty_chunk(&world, Vector2::new(0, 0));
        let java = TestPlayer::new(&world);
        let bedrock = TestBedrockPlayer::new(&world).await;
        let observer = TestPlayer::new(&world);
        world.players.store(Arc::new(vec![
            java.player.clone(),
            bedrock.player.clone(),
            observer.player.clone(),
        ]));
        server.worlds.store(Arc::new(vec![world.clone()]));
        for player in [&java.player, &bedrock.player] {
            player.get_entity().set_pos(Vector3::new(1.0, 64.0, 1.0));
            observer
                .player
                .chunk_sender
                .lock()
                .unwrap()
                .mark_sent_out_of_band(player.get_entity().chunk_pos.load());
            let tracked = world
                .entity_tracker
                .get_tracked_entity(player.entity_id())
                .unwrap();
            tracked.update_player(&observer.player, &world);
            assert!(tracked.seen_by.contains(&observer.player.gameprofile.id));
        }
        let fixture = Self {
            _directory: directory,
            server,
            java,
            bedrock,
            observer,
            route,
        };
        let player = fixture.player();
        player
            .screen_handler_sync_handler
            .store_player(player.clone());
        player.on_screen_handler_opened(&player.player_screen_handler);
        player.inventory.set_selected_slot(3);
        // Independently installed modifiers must survive removal of the weapon's slot modifiers.
        for attribute in COMBAT_ATTRIBUTES {
            player
                .living_entity
                .attributes
                .write()
                .unwrap()
                .get_mut(&attribute.id)
                .unwrap()
                .add_or_replace_modifier(Modifier {
                    id: "test:independent".into(),
                    amount: 0.25,
                    operation: ModifierOperation::Add,
                    permanent: true,
                });
        }
        fixture
    }

    fn player(&self) -> &Arc<Player> {
        if matches!(self.route, Route::Bedrock) {
            &self.bedrock.player
        } else {
            &self.java.player
        }
    }

    fn equip(&mut self, stack: &ItemStack) {
        let player = self.player();
        player.inventory.set_held_item(stack.clone());
        player.enqueue_equipment_change(&EquipmentSlot::MAIN_HAND, stack);
        player.tick(&self.server);
        self.observer.take_packets();
        self.java.take_packets();
        self.bedrock.take_packets();
    }

    fn drop_packet(&self) {
        let player = self.player();
        let position = BlockPos::new(0, 0, 0);
        match self.route {
            Route::JavaSingle | Route::JavaStack => self.java.client().handle_player_action(
                player,
                &SPlayerAction {
                    status: VarInt(if matches!(self.route, Route::JavaStack) {
                        Status::DropItemStack
                    } else {
                        Status::DropItem
                    } as i32),
                    position,
                    face: 0,
                    sequence: VarInt(0),
                },
                &self.server,
            ),
            Route::Bedrock => self.bedrock.client().handle_player_action(
                player,
                &self.server,
                &BedrockAction {
                    player_runtime_id: VarULong(player.entity_id() as u64),
                    action: PlayerAction::DropItem,
                    block_position: position,
                    result_pos: position,
                    face: VarInt(0),
                },
            ),
        }
    }

    fn take_equipment(&mut self) -> Vec<ItemStack> {
        let entity_id = self.player().entity_id();
        self.observer
            .take_packets()
            .into_iter()
            .filter_map(|packet| {
                let mut data = packet.as_ref();
                if VarInt::decode(&mut data).unwrap().0 != SET_EQUIPMENT.0 {
                    return None;
                }
                Some(CSetEquipment::read(&mut data, &CURRENT_MC_VERSION).unwrap())
            })
            .filter(|packet| packet.entity_id.0 == entity_id)
            .flat_map(|packet| packet.equipment)
            .filter(|(slot, _)| *slot == EquipmentSlot::MAIN_HAND.discriminant())
            .map(|(_, stack)| stack.0.into_owned())
            .collect()
    }

    fn ground(&self) -> Vec<ItemStack> {
        let player = self.player();
        player
            .world()
            .entities
            .load()
            .iter()
            .filter_map(|entity| {
                let item = entity.get_item_entity()?;
                assert_eq!(item.get_pickup_delay(), 40);
                item.on_player_collision(player);
                assert!(!item.get_entity().is_removed());
                Some(item.get_item_stack().lock().unwrap().clone())
            })
            .collect()
    }

    async fn close(&self) {
        tokio::time::timeout(std::time::Duration::from_secs(5), self.bedrock.close())
            .await
            .unwrap();
    }
}

fn combat_values(player: &Player) -> [f64; 2] {
    COMBAT_ATTRIBUTES.map(|attribute| player.living_entity.get_attribute_value(&attribute))
}

fn weapon(count: u8, item: &'static Item) -> ItemStack {
    let mut stack = ItemStack::new(count, item);
    if count > 1 {
        stack.set_data_component(MaxStackSizeImpl { size: 16 });
    }
    stack
}

async fn accepted_drop(route: Route, count: u8, item: &'static Item) {
    let mut fixture = Fixture::new(route).await;
    let player = fixture.player().clone();
    let baseline = combat_values(&player);
    let stack = weapon(count, item);
    fixture.equip(&stack);
    let equipped = combat_values(&player);
    assert!(equipped[0] > baseline[0]);
    assert!(equipped[1] < baseline[1]);
    fixture.drop_packet();
    let withdrawn = if matches!(route, Route::JavaStack) {
        count
    } else {
        1
    };
    let remaining = count - withdrawn;
    assert_eq!(player.inventory.held_item().item_count, remaining);
    assert!(
        fixture.take_equipment().is_empty(),
        "equipment must wait for normal publication"
    );
    let ground = fixture.ground();
    assert_eq!(
        ground.len(),
        1,
        "accepted withdrawal must spawn exactly once"
    );
    assert_eq!(ground[0].item_count, withdrawn);
    assert!(ground[0].are_items_and_components_equal(&stack));
    player.tick(&fixture.server);
    assert_eq!(
        combat_values(&player),
        if remaining == 0 { baseline } else { equipped },
        "accepted held drop left stale combat attributes"
    );
    let updates = fixture.take_equipment();
    assert_eq!(
        updates.len(),
        1,
        "observer missed the committed main-hand update"
    );
    assert!(updates[0].are_equal(&player.inventory.held_item()));
    if remaining == 0 {
        if matches!(route, Route::Bedrock) {
            assert_bedrock_owner_damage(&mut fixture.bedrock, baseline[0]);
        } else {
            assert_owner_attributes(&mut fixture.java, baseline);
        }
    }
    player.tick(&fixture.server);
    assert!(
        fixture.take_equipment().is_empty(),
        "drop was published twice"
    );
    assert_eq!(
        fixture.ground()[0].item_count,
        withdrawn,
        "pickup bypassed drop delay"
    );
    assert_eq!(player.inventory.held_item().item_count, remaining);
    fixture.close().await;
}

fn assert_owner_attributes(owner: &mut TestPlayer, baseline: [f64; 2]) {
    let properties: Vec<Property> = owner
        .take_packets()
        .into_iter()
        .filter_map(|packet| {
            let mut data = packet.as_ref();
            if VarInt::decode(&mut data).unwrap().0 != UPDATE_ATTRIBUTES.0 {
                return None;
            }
            Some(CUpdateAttributes::read(&mut data, &CURRENT_MC_VERSION).unwrap())
        })
        .flat_map(|packet| packet.properties)
        .collect();
    for (attribute, expected) in COMBAT_ATTRIBUTES.into_iter().zip(baseline) {
        let property = properties
            .iter()
            .rev()
            .find(|property| property.id.0 == i32::from(attribute.id));
        assert!(property.is_some(), "missing combat attribute packet");
        let property = property.unwrap();
        assert_eq!(
            property.modifiers.len(),
            1,
            "owner retained a weapon modifier"
        );
        assert_eq!(property.modifiers[0].id, "test:independent");
        assert_eq!(property.value + property.modifiers[0].amount, expected);
    }
}

fn assert_bedrock_owner_damage(owner: &mut TestBedrockPlayer, expected: f64) {
    let mut damage = None;
    for packet in owner.take_packets() {
        // The transport fixture captures uncompressed Bedrock batch frames.
        assert_eq!(packet[0], pumpkin_protocol::bedrock::BEDROCK_GAME_PACKET);
        let mut data = &packet[1..];
        let length = VarUInt::decode(&mut data).unwrap().0 as usize;
        assert_eq!(length, data.len());
        if VarUInt::decode(&mut data).unwrap().0 & 0x3ff != BedrockAttributes::PACKET_ID as u32 {
            continue;
        }
        assert_eq!(
            VarULong::read(&mut data).unwrap().0,
            owner.player.entity_id() as u64
        );
        let count = VarUInt::decode(&mut data).unwrap().0;
        assert!(count <= 64);
        for _ in 0..count {
            let values: [f32; 6] = std::array::from_fn(|_| f32::read(&mut data).unwrap());
            let name = String::read(&mut data).unwrap();
            assert_eq!(VarUInt::decode(&mut data).unwrap().0, 0);
            if name == "minecraft:attack_damage" {
                damage = Some(values[2]);
            }
        }
    }
    assert_eq!(
        damage,
        Some(expected as f32),
        "Bedrock owner retained weapon damage"
    );
}

#[tokio::test]
async fn b1019_g24_attributes_reach_owner_and_observer() {
    let mut fixture = Fixture::new(Route::JavaSingle).await;
    let baseline = combat_values(fixture.player());
    fixture.equip(&weapon(1, &Item::DIAMOND_SPEAR));
    fixture.drop_packet();
    fixture.player().tick(&fixture.server);
    assert_owner_attributes(&mut fixture.java, baseline);
    assert_owner_attributes(&mut fixture.observer, baseline);
    fixture.close().await;
}

#[tokio::test]
async fn b1019_g24_java_single_spear() {
    accepted_drop(Route::JavaSingle, 1, &Item::DIAMOND_SPEAR).await;
}
#[tokio::test]
async fn b1019_g24_java_whole_spear() {
    accepted_drop(Route::JavaStack, 1, &Item::DIAMOND_SPEAR).await;
}
#[tokio::test]
async fn b1019_g24_bedrock_single_spear() {
    accepted_drop(Route::Bedrock, 1, &Item::DIAMOND_SPEAR).await;
}
#[tokio::test]
async fn b1019_g24_java_partial_spear() {
    accepted_drop(Route::JavaSingle, 3, &Item::DIAMOND_SPEAR).await;
}
#[tokio::test]
async fn b1019_g24_java_whole_spear_stack() {
    accepted_drop(Route::JavaStack, 3, &Item::DIAMOND_SPEAR).await;
}
#[tokio::test]
async fn b1019_g24_bedrock_partial_spear() {
    accepted_drop(Route::Bedrock, 3, &Item::DIAMOND_SPEAR).await;
}
#[tokio::test]
async fn b1019_g24_java_other_weapon() {
    accepted_drop(Route::JavaSingle, 1, &Item::IRON_SWORD).await;
}

enum Callback {
    Cancel,
    Replace,
    Select,
}
struct DropCallback {
    action: Callback,
    calls: Arc<AtomicUsize>,
    count: u8,
}
impl EventHandler<PlayerDropItemEvent> for DropCallback {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerDropItemEvent,
    ) -> BoxFuture<'a, ()> {
        pumpkin_util::sync::assert_no_guards_held("held-drop callback");
        assert_eq!(event.item_name, "diamond_spear");
        assert_eq!(event.count, self.count);
        self.calls.fetch_add(1, Ordering::Relaxed);
        match self.action {
            Callback::Cancel => event.cancelled = true,
            Callback::Replace => event
                .player
                .inventory
                .set_held_item(weapon(self.count, &Item::DIAMOND_SPEAR)),
            Callback::Select => event.player.inventory.set_selected_slot(4),
        }
        Box::pin(async {})
    }
}

fn register_callback(fixture: &Fixture, action: Callback, count: u8) -> Arc<AtomicUsize> {
    let calls = Arc::new(AtomicUsize::new(0));
    fixture
        .server
        .plugin_manager
        .register::<PlayerDropItemEvent, _>(
            Arc::new(DropCallback {
                action,
                calls: calls.clone(),
                count,
            }),
            EventPriority::Normal,
            true,
        );
    calls
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g24_cancelled_drop_control() {
    for route in ROUTES {
        let mut fixture = Fixture::new(route).await;
        let stack = weapon(1, &Item::DIAMOND_SPEAR);
        fixture.equip(&stack);
        let player = fixture.player().clone();
        let equipped = combat_values(&player);
        let calls = register_callback(&fixture, Callback::Cancel, 1);
        fixture.drop_packet();
        player.tick(&fixture.server);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(player.inventory.held_item().uid, stack.uid);
        assert_eq!(combat_values(&player), equipped);
        assert!(fixture.ground().is_empty());
        assert!(
            fixture.take_equipment().is_empty(),
            "cancelled drop published a hand change"
        );
        fixture.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g24_replaced_source_control() {
    for route in ROUTES {
        let mut fixture = Fixture::new(route).await;
        let stack = weapon(1, &Item::DIAMOND_SPEAR);
        fixture.equip(&stack);
        let player = fixture.player().clone();
        let equipped = combat_values(&player);
        let calls = register_callback(&fixture, Callback::Replace, 1);
        fixture.drop_packet();
        player.tick(&fixture.server);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_ne!(player.inventory.held_item().uid, stack.uid);
        assert!(player.inventory.held_item().are_equal(&stack));
        assert_eq!(combat_values(&player), equipped);
        assert!(fixture.ground().is_empty());
        assert!(
            fixture.take_equipment().is_empty(),
            "failed withdrawal published a false empty hand"
        );
        fixture.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g24_callback_selection_publishes_current_hand() {
    for route in ROUTES {
        let mut fixture = Fixture::new(route).await;
        let player = fixture.player().clone();
        let sword = weapon(1, &Item::IRON_SWORD);
        fixture.equip(&sword);
        let sword_values = combat_values(&player);
        fixture.equip(&weapon(1, &Item::DIAMOND_SPEAR));
        player.inventory.set_slot(4, sword.clone());
        let calls = register_callback(&fixture, Callback::Select, 1);
        fixture.drop_packet();
        player.tick(&fixture.server);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert!(player.inventory.get_slot(3).is_empty());
        assert_eq!(player.inventory.get_selected_slot(), 4);
        assert_eq!(
            combat_values(&player),
            sword_values,
            "drop published the old source instead of the selected hand"
        );
        let updates = fixture.take_equipment();
        assert_eq!(updates.len(), 1);
        assert!(updates[0].are_equal(&sword));
        assert_eq!(fixture.ground().len(), 1);
        fixture.close().await;
    }
}

#[tokio::test]
async fn b1019_g24_drop_then_real_attack_uses_fist_damage_and_recharge() {
    let mut fixture = Fixture::new(Route::JavaSingle).await;
    let player = fixture.player().clone();
    let baseline = combat_values(&player);
    fixture.equip(&weapon(1, &Item::DIAMOND_SPEAR));
    player.last_attacked_ticks.store(2, Ordering::Relaxed);
    fixture.drop_packet();
    assert_eq!(
        player.last_attacked_ticks.load(Ordering::Relaxed),
        2,
        "drop reset elapsed attack recharge"
    );
    player.tick(&fixture.server);
    player.last_attacked_ticks.store(2, Ordering::Relaxed);
    let charge = (2.5 / (20.0 / baseline[1]) as f32).min(1.0);
    assert_eq!(
        player.get_attack_strength_scale(0.5),
        charge,
        "post-drop recharge retained spear speed"
    );
    let victim = fixture.observer.player.clone();
    player.get_entity().on_ground.store(true, Ordering::Relaxed);
    victim.get_entity().set_pos(Vector3::new(2.0, 64.0, 1.0));
    assert_eq!(player.living_entity.fall_distance.load(), 0.0);
    assert!(!player.get_entity().is_sprinting());
    assert_eq!(
        victim.living_entity.get_attribute_value(&Attributes::ARMOR),
        0.0
    );
    let before = victim.living_entity.health.load();
    fixture.java.client().handle_attack(
        &player,
        &SAttack {
            entity_id: victim.entity_id().into(),
        },
        &fixture.server,
    );
    let expected = baseline[0] as f32 * (0.2 + charge * charge * 0.8);
    let damage = before - victim.living_entity.health.load();
    assert!(damage > 0.0, "real attack did not reach the victim");
    assert!(
        (damage - expected).abs() < 0.00001,
        "post-drop attack dealt {damage}, expected unarmed {expected}"
    );
    assert_eq!(player.last_attacked_ticks.load(Ordering::Relaxed), 0);
    fixture.close().await;
}
