//! Adjusted slice 2a enters through real activation, commands and Creative packets.
use super::*;
use crate::entity::passive::villager::VillagerEntity;
use pumpkin_data::{
    data_component::DataComponent,
    data_component_impl::{DamageImpl, VillagerFoodImpl},
    item::Item,
    item_stack::ItemStack,
};
use pumpkin_inventory::screen_handler::ScreenHandler;
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_protocol::{
    codec::item_stack_seralizer::ItemStackSerializer,
    java::{
        client::play::{CSetContainerContent, CSetContainerSlot, CSetEquipment},
        server::play::SSetCreativeSlot,
    },
    packet::MultiVersionJavaPacket,
    ser::NetworkReadExt,
};
use pumpkin_util::GameMode;
use pumpkin_world::chunk::{ChunkEntityData, io::DirtyFlag};
use std::sync::atomic::{AtomicBool, Ordering};

fn stock(item: &str, count: i32, patch: Option<(&str, NbtTag)>) -> NbtTag {
    let mut item_tag = NbtCompound::new();
    item_tag.put_string("id", item.into());
    item_tag.put_int("count", count);
    let mut components = NbtCompound::new();
    if let Some((key, value)) = patch {
        components.put(key, value);
    }
    item_tag.put_compound("components", components);
    NbtTag::Compound(item_tag)
}

fn food(value: Option<NbtTag>, extra: bool) -> NbtTag {
    let mut component = NbtCompound::new();
    if let Some(value) = value {
        component.put("nutrition", value);
    }
    if extra {
        component.put_int("future", 9);
    }
    NbtTag::Compound(component)
}

fn fixture(id: Uuid, inventory: Vec<NbtTag>) -> NbtCompound {
    let mut nbt = NbtCompound::new();
    nbt.put_string("id", "minecraft:villager".into());
    nbt.put_uuid("UUID", id);
    nbt.put_list(
        "Pos",
        vec![
            NbtTag::Double(10.5),
            NbtTag::Double(64.0),
            NbtTag::Double(10.5),
        ],
    );
    nbt.put_bool("NoAI", true);
    nbt.put_bool("PersistenceRequired", true);
    nbt.put_float("Health", 20.0);
    let mut data = NbtCompound::new();
    data.put_string("type", "minecraft:taiga".into());
    data.put_string("profession", "minecraft:librarian".into());
    data.put_int("level", 3);
    nbt.put_compound("VillagerData", data);
    nbt.put_bool("VillagerDataFinalized", true);
    nbt.put_byte("FoodLevel", 9);
    nbt.put_list("Inventory", inventory);
    let mut recipe = NbtCompound::new();
    recipe.put("buy", stock("minecraft:emerald", 7, None));
    recipe.put("sell", stock("minecraft:book", 1, None));
    recipe.put_int("uses", 2);
    recipe.put_int("maxUses", 12);
    recipe.put_bool("rewardExp", false);
    recipe.put_int("xp", 5);
    recipe.put_float("priceMultiplier", 0.05);
    recipe.put_int("specialPrice", -3);
    recipe.put_int("demand", 4);
    let mut offers = NbtCompound::new();
    offers.put_list("Recipes", vec![NbtTag::Compound(recipe)]);
    nbt.put_compound("Offers", offers);
    let mut gossip = NbtCompound::new();
    gossip.put("Target", NbtTag::IntArray(vec![1, 2, 3, 4]));
    gossip.put_string("Type", "major_positive".into());
    gossip.put_int("Value", 64);
    nbt.put_list("Gossips", vec![NbtTag::Compound(gossip)]);
    let mut slept = NbtCompound::new();
    slept.put_long("value", 1234);
    let mut memories = NbtCompound::new();
    memories.put_compound("minecraft:last_slept", slept);
    let mut brain = NbtCompound::new();
    brain.put_compound("memories", memories);
    nbt.put_compound("Brain", brain);
    nbt
}

async fn seed(scenario: &Scenario, records: Vec<NbtCompound>) {
    let world = scenario.world(&Dimension::OVERWORLD);
    let pos = Vector2::new(0, 0);
    let chunk = Arc::new(ChunkEntityData {
        x: 0,
        z: 0,
        data: pumpkin_util::sync::Mutex::new(records),
        live: AtomicBool::new(false),
        dirty: DirtyFlag::new(true),
    });
    scenario
        .drive(
            "food fixture save",
            world.level.write_entity_chunks(vec![(pos, chunk)]),
        )
        .await;
    scenario
        .drive("food fixture flush", world.level.drain_entity_storage())
        .await
        .unwrap();
    assert!(world.entities.load().is_empty());
}

async fn saved(scenario: &Scenario, id: Uuid) -> NbtCompound {
    let world = scenario.world(&Dimension::OVERWORLD);
    let storage = scenario
        .drive(
            "food saved record",
            world.level.get_entity_chunk(Vector2::new(0, 0)),
        )
        .await
        .unwrap();
    assert!(!storage.live.load(Ordering::Acquire));
    let records = storage.data.lock().unwrap();
    records
        .iter()
        .find(|entry| entry.get_uuid("UUID") == Some(id))
        .unwrap()
        .clone()
}

fn loaded(scenario: &Scenario, id: Uuid) -> Vec<ItemStack> {
    let world = scenario.world(&Dimension::OVERWORLD);
    let entity = world.get_entity_by_uuid(id).unwrap();
    let villager = entity.cast_any().downcast_ref::<VillagerEntity>().unwrap();
    assert_eq!(villager.food_level.load(Ordering::Relaxed), 9);
    let inventory = villager.inventory.lock().unwrap();
    assert_eq!(inventory.len(), 8);
    inventory.to_vec()
}

fn assert_stock(stack: &ItemStack, item: &Item, count: u8, nutrition: Option<i32>) {
    assert_eq!(stack.item.id, item.id);
    assert_eq!(stack.item_count, count);
    assert_eq!(
        stack
            .get_data_component::<VillagerFoodImpl>()
            .map(|value| value.nutrition),
        nutrition
    );
}

fn owner_stacks(session: &Session) -> Vec<ItemStack> {
    let mut result = Vec::new();
    for packet in session.packets() {
        let mut bytes = packet.as_ref();
        let id = bytes.get_var_int().unwrap().0;
        if id == CSetContainerSlot::to_id(CURRENT_MC_VERSION) {
            let decoded = CSetContainerSlot::read(&mut bytes, &CURRENT_MC_VERSION).unwrap();
            result.push(decoded.slot_data.0.as_ref().clone());
        } else if id == CSetContainerContent::to_id(CURRENT_MC_VERSION) {
            let decoded = CSetContainerContent::read(&mut bytes, &CURRENT_MC_VERSION).unwrap();
            result.extend(
                decoded
                    .slot_data
                    .iter()
                    .map(|stack| stack.0.as_ref().clone()),
            );
        }
    }
    result
}

fn import_fixtures(ids: [Uuid; 3]) -> Vec<NbtCompound> {
    let canonical = vec![
        stock(
            "minecraft:stone",
            2,
            Some(("minecraft:villager_food", food(Some(NbtTag::Int(7)), false))),
        ),
        stock("minecraft:bread", 3, None),
        stock(
            "minecraft:bread",
            2,
            Some((
                "!minecraft:villager_food",
                NbtTag::Compound(NbtCompound::new()),
            )),
        ),
        stock("minecraft:wheat", 5, None),
        stock("minecraft:carrot", 4, None),
        stock("minecraft:potato", 6, None),
        stock("minecraft:beetroot", 7, None),
        stock(
            "minecraft:iron_pickaxe",
            1,
            Some(("minecraft:damage", NbtTag::Int(2))),
        ),
    ];
    let mut invalid: Vec<_> = [
        Some(NbtTag::Int(0)),
        Some(NbtTag::Int(-1)),
        None,
        Some(NbtTag::Byte(7)),
    ]
    .into_iter()
    .map(|value| {
        stock(
            "minecraft:stone",
            1,
            Some(("minecraft:villager_food", food(value, false))),
        )
    })
    .collect();
    invalid.push(stock(
        "minecraft:stone",
        1,
        Some(("minecraft:villager_food", food(Some(NbtTag::Int(7)), true))),
    ));
    invalid.push(stock(
        "minecraft:stone",
        1,
        Some(("future:payload", NbtTag::Int(7))),
    ));
    invalid.push(stock("minecraft:wheat", 3, None));
    let overflow = (1..=10)
        .map(|count| stock("minecraft:bread", count, None))
        .collect();
    [canonical, invalid, overflow]
        .into_iter()
        .zip(ids)
        .map(|(items, id)| fixture(id, items))
        .collect::<Vec<_>>()
}

#[tokio::test]
async fn villager_s2_component_import_promotes_once_across_two_restarts() {
    tokio::time::timeout(
        Duration::from_secs(300),
        run(async {
            let mut scenario = Scenario::new().await;
            let ids = [Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
            let fixtures = import_fixtures(ids);
            seed(&scenario, fixtures.clone()).await;
            scenario.join(Uuid::new_v4(), "FoodWatcher").await;
            let mut canonical_slots = Vec::new();
            for cycle in 0..=2 {
                let raw_control = loaded(&scenario, ids[1]);
                assert_stock(&raw_control[0], &Item::WHEAT, 3, None);
                assert_eq!(
                    raw_control.iter().filter(|stack| !stack.is_empty()).count(),
                    1
                );
                let capacity = loaded(&scenario, ids[2]);
                assert_eq!(
                    capacity
                        .iter()
                        .map(|stack| stack.item_count)
                        .collect::<Vec<_>>(),
                    vec![1, 2, 3, 4, 5, 6, 7, 8]
                );
                canonical_slots.push(loaded(&scenario, ids[0]));
                let world = scenario.world(&Dimension::OVERWORLD);
                assert_eq!(
                    world.entities.load().len(),
                    3,
                    "no baby or dropped item may appear"
                );
                if cycle < 2 {
                    scenario = scenario.restart().await;
                    for (id, original) in ids.into_iter().zip(&fixtures) {
                        let record = saved(&scenario, id).await;
                        for field in [
                            "Inventory",
                            "FoodLevel",
                            "VillagerData",
                            "Offers",
                            "Gossips",
                            "Brain",
                        ] {
                            assert_eq!(
                                record.get(field),
                                original.get(field),
                                "{field} changed on restart {cycle}"
                            );
                        }
                    }
                    scenario.join(Uuid::new_v4(), "FoodWatcher").await;
                }
            }
            for slots in canonical_slots {
                assert_stock(&slots[0], &Item::STONE, 2, Some(7));
                assert_stock(&slots[1], &Item::BREAD, 3, Some(4));
                assert_stock(&slots[2], &Item::BREAD, 2, None);
                assert_stock(&slots[3], &Item::WHEAT, 5, None);
                assert_stock(&slots[4], &Item::CARROT, 4, Some(1));
                assert_stock(&slots[5], &Item::POTATO, 6, Some(1));
                assert_stock(&slots[6], &Item::BEETROOT, 7, Some(1));
                assert_stock(&slots[7], &Item::IRON_PICKAXE, 1, None);
                assert_eq!(
                    slots[7]
                        .get_data_component::<DamageImpl>()
                        .map(|value| value.damage),
                    Some(2)
                );
                assert_eq!(slots.iter().filter(|stack| !stack.is_empty()).count(), 8);
            }
            scenario.stop().await;
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn villager_s2_give_component_reaches_player_and_wire() {
    tokio::time::timeout(
        Duration::from_secs(300),
        run(async {
            let scenario = Scenario::new().await;
            let player = scenario.join(Uuid::new_v4(), "FoodPolicy").await;
            player.packets();
            scenario.command(
                "give FoodPolicy minecraft:stone[minecraft:villager_food={nutrition:7}] 2",
            );
            scenario.command("give FoodPolicy minecraft:bread 3");
            scenario.command("give FoodPolicy minecraft:wheat 5");
            scenario.ticks(1).await;
            assert_eq!(player.player.inventory.get_slot(1).item_count, 3);
            assert_stock(&player.player.inventory.get_slot(2), &Item::WHEAT, 5, None);
            assert_stock(
                &player.player.inventory.get_slot(0),
                &Item::STONE,
                2,
                Some(7),
            );
            assert_stock(
                &player.player.inventory.get_slot(1),
                &Item::BREAD,
                3,
                Some(4),
            );
            let packets = owner_stacks(&player);
            assert!(
                packets.iter().any(|stack| stack.item == &Item::STONE
                    && stack.item_count == 2
                    && stack
                        .get_data_component::<VillagerFoodImpl>()
                        .is_some_and(|food| food.nutrition == 7)),
                "custom give patch absent from owner wire"
            );
            scenario.stop().await;
        }),
    )
    .await
    .unwrap();
}

fn assert_observer(session: &Session, entity_id: i32, expected: &ItemStack) {
    let mut found = false;
    for packet in session.packets() {
        let mut bytes = packet.as_ref();
        if bytes.get_var_int().unwrap().0 != CSetEquipment::to_id(CURRENT_MC_VERSION) {
            continue;
        }
        let decoded = CSetEquipment::read(&mut bytes, &CURRENT_MC_VERSION).unwrap();
        if decoded.entity_id.0 == entity_id {
            found |= decoded
                .equipment
                .iter()
                .any(|(slot, stack)| *slot == 0 && stack.0.are_equal(expected));
        }
    }
    assert!(
        found,
        "equipped component/removal absent from observer wire"
    );
}

fn enqueue_creative(session: &Session, expected: ItemStack) {
    let packet = SSetCreativeSlot::new(36, ItemStackSerializer::from(expected));
    let mut payload = Vec::new();
    packet
        .write_packet_data(&mut payload, &CURRENT_MC_VERSION)
        .unwrap();
    session.player.inbound_packets.push(RawPacket {
        id: SSetCreativeSlot::to_id(CURRENT_MC_VERSION),
        payload: payload.into(),
    });
}

#[tokio::test]
async fn villager_s2_26_3_stack_and_creative_dispatch() {
    tokio::time::timeout(
        Duration::from_secs(300),
        run(async {
            let scenario = Scenario::new().await;
            let player = scenario.join(Uuid::new_v4(), "FoodCreative").await;
            let observer = scenario.join(Uuid::new_v4(), "FoodObserver").await;
            scenario.command("gamemode creative FoodCreative");
            assert_eq!(player.player.gamemode.load(), GameMode::Creative);
            let mut patched = ItemStack::new(2, &Item::STONE);
            patched.set_data_component(VillagerFoodImpl { nutrition: 300 });
            let mut removed = ItemStack::new(3, &Item::BREAD);
            removed.remove_data_component(DataComponent::VillagerFood);
            for expected in [patched, removed] {
                player.packets();
                observer.packets();
                enqueue_creative(&player, expected.clone());
                scenario.ticks(1).await;
                assert!(player.player.inbound_packets.is_empty());
                let stored = player.player.inventory.get_slot(0);
                assert!(stored.are_equal(&expected));
                // Accepted Creative predictions suppress corrections; request the authoritative menu snapshot.
                player
                    .player
                    .player_screen_handler
                    .lock()
                    .unwrap()
                    .sync_state();
                assert!(
                    owner_stacks(&player)
                        .iter()
                        .any(|stack| stack.are_equal(&expected)),
                    "Creative patch/removal absent from owner wire"
                );
                assert_observer(&observer, player.player.entity_id(), &expected);
            }
            for _ in 0..2 {
                enqueue_creative(&player, ItemStack::new(4, &Item::STONE));
                scenario.ticks(1).await;
                assert!(player.player.inbound_packets.is_empty());
                assert_stock(&player.player.inventory.get_slot(0), &Item::STONE, 4, None);
            }
            scenario.command("gamemode survival FoodCreative");
            enqueue_creative(&player, ItemStack::new(9, &Item::DIAMOND));
            scenario.ticks(1).await;
            assert!(player.player.inbound_packets.is_empty());
            assert_stock(&player.player.inventory.get_slot(0), &Item::STONE, 4, None);
            scenario.stop().await;
        }),
    )
    .await
    .unwrap();
}
