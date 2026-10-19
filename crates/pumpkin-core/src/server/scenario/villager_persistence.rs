//! Vanilla 26.3 villager fixtures enter through chunk activation and leave through shutdown saves.
mod registry;
mod zombie;

use super::*;
use crate::entity::passive::villager::{GossipType, VillagerEntity};
use pumpkin_data::{
    block_properties::{BedPart, WhiteBedLikeProperties},
    villager::{VillagerProfession, VillagerType},
};
use pumpkin_nbt::{
    compound::NbtCompound,
    serializer::{NbtWriteHelper, NbtWriteHelperJava},
    tag::NbtTag,
};
use pumpkin_world::chunk::{ChunkEntityData, io::DirtyFlag};
use std::sync::atomic::{AtomicBool, Ordering};

const HOME: BlockPos = BlockPos::new(8, 64, 8);
const TARGET: Uuid = Uuid::from_u128(0x0123_4567_89ab_cdef_1357_2468_fedc_ba98);

fn stack(item: &str, count: i32) -> NbtTag {
    // ItemStack.CODEC: namespaced id, integer count and component patch.
    let mut tag = NbtCompound::new();
    tag.put_string("id", item.into());
    tag.put_int("count", count);
    tag.put_compound("components", NbtCompound::new());
    NbtTag::Compound(tag)
}

fn gossip(target: Vec<i32>, kind: &str, value: i32) -> NbtTag {
    // GossipContainer.GossipEntry.CODEC: Target, Type and Value.
    let mut tag = NbtCompound::new();
    tag.put("Target", NbtTag::IntArray(target));
    tag.put_string("Type", kind.into());
    tag.put_int("Value", value);
    NbtTag::Compound(tag)
}

fn villager_fixture(id: Uuid, legacy: bool) -> NbtCompound {
    // Villager.addAdditionalSaveData/readAdditionalSaveData and VillagerData.CODEC:
    // VillagerData, VillagerDataFinalized, FoodLevel, Xp, Offers, Inventory and Gossips.
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
    if legacy {
        data.put_int("Type", VillagerType::Taiga as i32);
        data.put_int("Profession", VillagerProfession::Librarian as i32);
        data.put_int("Level", 3);
        nbt.put_int("FoodLevel", 12);
    } else {
        data.put_string("type", "minecraft:taiga".into());
        data.put_string("profession", "minecraft:librarian".into());
        data.put_int("level", 3);
        nbt.put_bool("VillagerDataFinalized", true);
        nbt.put_byte("FoodLevel", 12);
    }
    nbt.put_compound("VillagerData", data);
    nbt.put_int("Xp", 45);
    nbt.put_long("LastRestock", 5_000);
    nbt.put_int("RestocksToday", 1);
    nbt.put_long("LastGossipDecay", 7_000);
    nbt.put_long("LastGossipShare", 4_321);
    nbt.put_int("HomeX", HOME.0.x);
    nbt.put_int("HomeY", HOME.0.y);
    nbt.put_int("HomeZ", HOME.0.z);
    nbt.put_list(
        "Inventory",
        vec![stack("minecraft:bread", 3), stack("minecraft:carrot", 5)],
    );
    nbt.put_list(
        "Gossips",
        vec![gossip(
            vec![
                0x0123_4567,
                0x89ab_cdefu32 as i32,
                0x1357_2468,
                0xfedc_ba98u32 as i32,
            ],
            "major_positive",
            64,
        )],
    );
    let mut recipe = NbtCompound::new();
    recipe.put("buy", stack("minecraft:emerald", 7));
    recipe.put("buyB", stack("minecraft:book", 1));
    recipe.put("sell", stack("minecraft:bookshelf", 1));
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
    // Brain's MemoryValue codecs: LAST_SLEPT and GOLEM_DETECTED_RECENTLY with remaining ttl.
    let mut slept = NbtCompound::new();
    slept.put_long("value", 1_234);
    let mut golem = NbtCompound::new();
    golem.put_bool("value", true);
    golem.put_long("ttl", 476);
    let mut memories = NbtCompound::new();
    memories.put_compound("minecraft:last_slept", slept);
    memories.put_compound("minecraft:golem_detected_recently", golem);
    let mut brain = NbtCompound::new();
    brain.put_compound("memories", memories);
    nbt.put_compound("Brain", brain);
    nbt
}

async fn seed(scenario: &Scenario, records: Vec<NbtCompound>) {
    let world = scenario.world(&Dimension::OVERWORLD);
    let pos = Vector2::new(0, 0);
    let terrain = scenario
        .drive(
            "villager fixture terrain",
            world.level.get_or_fetch_chunk(pos, Clone::clone),
        )
        .await
        .unwrap();
    let mut bed = WhiteBedLikeProperties::default(&Block::WHITE_BED);
    bed.part = BedPart::Head;
    world
        .level
        .set_block_state(&HOME, bed.to_state_id(&Block::WHITE_BED));
    scenario
        .drive(
            "villager fixture bed save",
            world.level.write_chunks(vec![(pos, terrain)]),
        )
        .await;
    let chunk = Arc::new(ChunkEntityData {
        x: 0,
        z: 0,
        data: pumpkin_util::sync::Mutex::new(records),
        live: AtomicBool::new(false),
        dirty: DirtyFlag::new(true),
    });
    scenario
        .drive(
            "villager fixture entity save",
            world.level.write_entity_chunks(vec![(pos, chunk)]),
        )
        .await;
    scenario
        .drive(
            "villager fixture storage flush",
            world.level.drain_entity_storage(),
        )
        .await
        .unwrap();
    assert!(
        world.entities.load().is_empty(),
        "fixture must enter through activation"
    );
}

fn with_villager<T>(scenario: &Scenario, id: Uuid, read: impl FnOnce(&VillagerEntity) -> T) -> T {
    let world = scenario.world(&Dimension::OVERWORLD);
    let entity = world.get_entity_by_uuid(id).unwrap();
    read(entity.cast_any().downcast_ref::<VillagerEntity>().unwrap())
}

fn assert_capacity(scenario: &Scenario, id: Uuid, count: usize) {
    with_villager(scenario, id, |villager| {
        let inventory = villager.inventory.lock().unwrap();
        assert_eq!(
            inventory.len(),
            8,
            "villager inventory must retain eight slots"
        );
        assert_eq!(
            inventory.iter().filter(|stack| !stack.is_empty()).count(),
            count
        );
    });
}

async fn saved_record(scenario: &Scenario, id: Uuid) -> NbtCompound {
    let world = scenario.world(&Dimension::OVERWORLD);
    let storage = scenario
        .drive(
            "saved villager entity chunk read",
            world.level.get_entity_chunk(Vector2::new(0, 0)),
        )
        .await
        .unwrap();
    assert!(
        !storage.live.load(Ordering::Acquire),
        "saved NBT must be checked before activation"
    );
    let data = storage.data.lock().unwrap();
    data.iter()
        .find(|entry| entry.get_uuid("UUID") == Some(id))
        .unwrap()
        .clone()
}

fn assert_vanilla_encoding(nbt: &NbtCompound) {
    let data = nbt.get_compound("VillagerData").unwrap();
    assert_eq!(data.get_string("type"), Some("minecraft:taiga"));
    assert_eq!(data.get_string("profession"), Some("minecraft:librarian"));
    assert_eq!(data.get_int("level"), Some(3));
    for legacy in ["Type", "Profession", "Level"] {
        assert!(
            data.get(legacy).is_none(),
            "legacy VillagerData key {legacy} was written"
        );
    }
    assert_eq!(nbt.get_bool("VillagerDataFinalized"), Some(true));
}

fn assert_unchanged_fields(saved: &NbtCompound, fixture: &NbtCompound) {
    for field in [
        "Offers",
        "Xp",
        "LastRestock",
        "LastGossipDecay",
        "RestocksToday",
        "HomeX",
        "HomeY",
        "HomeZ",
        "Brain",
    ] {
        assert_eq!(
            saved.get(field),
            fixture.get(field),
            "unrelated villager field {field} changed"
        );
    }
}

fn assert_loaded_vanilla(scenario: &Scenario, id: Uuid) {
    with_villager(scenario, id, |villager| {
        let data = *villager.villager_data.lock().unwrap();
        assert_eq!(data.r#type.0, VillagerType::Taiga as i32);
        assert_eq!(data.profession.0, VillagerProfession::Librarian as i32);
        assert_eq!(data.level.0, 3);
        assert_eq!(villager.food_level.load(Ordering::Relaxed), 12);
        assert_eq!(
            villager.last_gossip_share_time.load(Ordering::Relaxed),
            4_321
        );
        assert_eq!(
            villager
                .gossips
                .lock()
                .unwrap()
                .get(&TARGET)
                .and_then(|entries| entries.get(&GossipType::MajorPositive))
                .copied(),
            Some(64)
        );
        let inventory = villager.inventory.lock().unwrap();
        assert_eq!(inventory[0].item_count, 3);
        assert_eq!(inventory[0].item.registry_key, "bread");
        assert_eq!(inventory[1].item_count, 5);
        assert_eq!(inventory[1].item.registry_key, "carrot");
    });
    assert_capacity(scenario, id, 2);
}

#[tokio::test]
async fn villager_a_vanilla_survives_two_restarts() {
    run(async {
        let mut scenario = Scenario::new().await;
        let id = Uuid::new_v4();
        let fixture = villager_fixture(id, false);
        seed(&scenario, vec![fixture.clone()]).await;
        scenario.join(Uuid::new_v4(), "VanillaVillager").await;
        assert_loaded_vanilla(&scenario, id);
        for _ in 0..2 {
            scenario = scenario.restart().await;
            let saved = saved_record(&scenario, id).await;
            assert_vanilla_encoding(&saved);
            assert_eq!(saved.get("FoodLevel"), Some(&NbtTag::Byte(12)));
            assert_eq!(saved.get_list("Inventory").unwrap().len(), 2);
            assert_eq!(saved.get("Gossips"), fixture.get("Gossips"));
            assert_eq!(saved.get_long("LastGossipShare"), Some(4_321));
            assert_unchanged_fields(&saved, &fixture);
            scenario.join(Uuid::new_v4(), "VanillaVillager").await;
            assert_loaded_vanilla(&scenario, id);
        }
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn villager_b_legacy_migrates_once_across_two_restarts() {
    run(async {
        let mut scenario = Scenario::new().await;
        let id = Uuid::new_v4();
        let mut fixture = villager_fixture(id, true);
        let mut inventory = vec![stack("minecraft:air", 0); 8];
        inventory[2] = stack("minecraft:bread", 3);
        inventory[6] = stack("minecraft:carrot", 5);
        fixture.put_list("Inventory", inventory);
        // Keep this case focused on legacy data, food and empty-slot migration.
        fixture.put_list("Gossips", Vec::new());
        seed(&scenario, vec![fixture.clone()]).await;
        scenario.join(Uuid::new_v4(), "LegacyVillager").await;
        assert_capacity(&scenario, id, 2);
        let mut migrated = None;
        for _ in 0..2 {
            scenario = scenario.restart().await;
            let saved = saved_record(&scenario, id).await;
            assert_vanilla_encoding(&saved);
            assert_eq!(saved.get("FoodLevel"), Some(&NbtTag::Byte(12)));
            let inventory = saved.get("Inventory").cloned().unwrap();
            assert_eq!(
                inventory,
                NbtTag::List(vec![
                    stack("minecraft:bread", 3),
                    stack("minecraft:carrot", 5)
                ])
            );
            assert_unchanged_fields(&saved, &fixture);
            let relevant = [
                "VillagerData",
                "VillagerDataFinalized",
                "FoodLevel",
                "Inventory",
            ]
            .map(|key| saved.get(key).cloned());
            if let Some(previous) = &migrated {
                assert_eq!(
                    &relevant, previous,
                    "second restart must be a migration no-op"
                );
            }
            migrated = Some(relevant);
            scenario.join(Uuid::new_v4(), "LegacyVillager").await;
            assert_capacity(&scenario, id, 2);
        }
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn villager_c_empty_inventory_keeps_eight_slots_after_two_restarts() {
    run(async {
        let mut scenario = Scenario::new().await;
        let id = Uuid::new_v4();
        let mut fixture = villager_fixture(id, false);
        fixture.put_list("Inventory", Vec::new());
        seed(&scenario, vec![fixture]).await;
        scenario.join(Uuid::new_v4(), "EmptyVillager").await;
        assert_capacity(&scenario, id, 0);
        for _ in 0..2 {
            scenario = scenario.restart().await;
            let saved = saved_record(&scenario, id).await;
            assert!(saved.get_list("Inventory").is_none_or(<[NbtTag]>::is_empty));
            scenario.join(Uuid::new_v4(), "EmptyVillager").await;
            assert_capacity(&scenario, id, 0);
        }
        scenario.stop().await;
    })
    .await;
}

// NbtCompound uses a HashMap, so disk parsing can change compound key order. Compare
// canonical NBT bytes (keys sorted, all types/payloads/list order retained) for raw entries.
fn canonical_bytes(tag: &NbtTag) -> Vec<u8> {
    fn payload(tag: &NbtTag, writer: &mut NbtWriteHelperJava<&mut Vec<u8>>) {
        match tag {
            NbtTag::Compound(compound) => {
                let mut entries: Vec<_> = compound.child_tags.iter().collect();
                entries.sort_unstable_by_key(|(name, _)| *name);
                for (name, child) in entries {
                    writer.write_u8(child.get_type_id()).unwrap();
                    writer.write_string(name).unwrap();
                    payload(child, writer);
                }
                writer.write_u8(0).unwrap();
            }
            NbtTag::List(entries) => {
                writer
                    .write_u8(entries.first().map_or(0, NbtTag::get_type_id))
                    .unwrap();
                writer
                    .write_i32(i32::try_from(entries.len()).unwrap())
                    .unwrap();
                for child in entries {
                    payload(child, writer);
                }
            }
            _ => tag.clone().serialize_data(writer).unwrap(),
        }
    }
    let mut bytes = Vec::new();
    let mut writer = NbtWriteHelperJava::new(&mut bytes);
    writer.write_u8(tag.get_type_id()).unwrap();
    payload(tag, &mut writer);
    bytes
}

#[tokio::test]
async fn villager_d_lossy_inventory_entries_survive_two_restarts_byte_for_byte() {
    run(async {
        let mut scenario = Scenario::new().await;
        let id = Uuid::new_v4();
        let mut fixture = villager_fixture(id, false);
        let mut inventory: Vec<_> = (1..=12)
            .map(|count| stack("minecraft:bread", count))
            .collect();
        let NbtTag::Compound(noncompound_components) = &mut inventory[2] else {
            panic!("fixture item is not a compound")
        };
        noncompound_components.put("components", NbtTag::String("future payload".into()));
        let NbtTag::Compound(unknown_field) = &mut inventory[4] else {
            panic!("fixture item is not a compound")
        };
        unknown_field.put_long("retained_unknown_field", 404);
        let mut undecodable = stack("minecraft:stone", 2)
            .extract_compound()
            .unwrap()
            .clone();
        let mut bad_components = NbtCompound::new();
        bad_components.put_string("minecraft:damage", "invalid component".into());
        undecodable.put_compound("components", bad_components);
        inventory.insert(6, NbtTag::Compound(undecodable));
        let expected: Vec<_> = inventory.iter().map(canonical_bytes).collect();
        fixture.put_list("Inventory", inventory);
        seed(&scenario, vec![fixture]).await;
        scenario
            .join(Uuid::new_v4(), "LossyInventoryVillager")
            .await;
        assert_capacity(&scenario, id, 8);
        for _ in 0..2 {
            scenario = scenario.restart().await;
            let saved = saved_record(&scenario, id).await;
            let inventory = saved.get_list("Inventory").unwrap();
            assert_eq!(
                inventory.iter().map(canonical_bytes).collect::<Vec<_>>(),
                expected
            );
            scenario
                .join(Uuid::new_v4(), "LossyInventoryVillager")
                .await;
            assert_capacity(&scenario, id, 8);
        }
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn villager_g_unknown_zero_count_inventory_entry_survives_two_restarts() {
    run(async {
        let mut scenario = Scenario::new().await;
        let id = Uuid::new_v4();
        let mut fixture = villager_fixture(id, false);
        let mut unknown = NbtCompound::new();
        unknown.put_string("id", "future:artifact".into());
        unknown.put_int("count", 0);
        let mut components = NbtCompound::new();
        components.put_string("future:payload", "keep this value".into());
        unknown.put_compound("components", components);
        unknown.put_long("retained_unknown_field", 505);
        let raw = NbtTag::Compound(unknown);
        let expected = canonical_bytes(&raw);
        fixture.put_list("Inventory", vec![raw]);
        seed(&scenario, vec![fixture]).await;
        scenario
            .join(Uuid::new_v4(), "ZeroCountUnknownVillager")
            .await;
        assert_capacity(&scenario, id, 0);
        for _ in 0..2 {
            scenario = scenario.restart().await;
            let saved = saved_record(&scenario, id).await;
            let inventory = saved.get_list("Inventory").unwrap();
            assert_eq!(inventory.len(), 1);
            assert_eq!(canonical_bytes(&inventory[0]), expected);
            scenario
                .join(Uuid::new_v4(), "ZeroCountUnknownVillager")
                .await;
            assert_capacity(&scenario, id, 0);
        }
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn villager_e_short_gossip_target_is_skipped_without_losing_valid_entries() {
    run(async {
        let mut scenario = Scenario::new().await;
        let id = Uuid::new_v4();
        let mut fixture = villager_fixture(id, false);
        let valid = gossip(vec![1, 2, 3, 4], "trading", 16);
        fixture.put_list(
            "Gossips",
            vec![gossip(vec![1, 2, 3], "major_positive", 64), valid.clone()],
        );
        seed(&scenario, vec![fixture]).await;
        scenario.join(Uuid::new_v4(), "ShortTargetVillager").await;
        let target = Uuid::from_u128(0x0000_0001_0000_0002_0000_0003_0000_0004);
        for cycle in 0..=2 {
            with_villager(&scenario, id, |villager| {
                let gossips = villager.gossips.lock().unwrap();
                assert_eq!(gossips.len(), 1);
                assert_eq!(
                    gossips
                        .get(&target)
                        .and_then(|entries| entries.get(&GossipType::Trading))
                        .copied(),
                    Some(16)
                );
            });
            if cycle < 2 {
                scenario = scenario.restart().await;
                let saved = saved_record(&scenario, id).await;
                assert_eq!(
                    saved.get_list("Gossips"),
                    Some(std::slice::from_ref(&valid))
                );
                scenario.join(Uuid::new_v4(), "ShortTargetVillager").await;
            }
        }
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn villager_f_out_of_range_food_is_preserved_while_ordinary_legacy_food_migrates() {
    run(async {
        let mut scenario = Scenario::new().await;
        let ids = [Uuid::new_v4(), Uuid::new_v4()];
        let values = [300, 12];
        let records = ids
            .iter()
            .zip(values)
            .map(|(id, value)| {
                let mut fixture = villager_fixture(*id, true);
                fixture.put_int("FoodLevel", value);
                fixture
            })
            .collect();
        seed(&scenario, records).await;
        scenario.join(Uuid::new_v4(), "FoodVillagers").await;
        for cycle in 0..=2 {
            for (id, value) in ids.iter().zip(values) {
                with_villager(&scenario, *id, |villager| {
                    assert_eq!(villager.food_level.load(Ordering::Relaxed), value);
                });
            }
            if cycle < 2 {
                scenario = scenario.restart().await;
                let outlier = saved_record(&scenario, ids[0]).await;
                assert_eq!(outlier.get("FoodLevel"), Some(&NbtTag::Int(300)));
                let ordinary = saved_record(&scenario, ids[1]).await;
                assert_eq!(
                    ordinary.get("FoodLevel"),
                    Some(&NbtTag::Byte(12)),
                    "ordinary legacy food must migrate while the outlier stays an int"
                );
                scenario.join(Uuid::new_v4(), "FoodVillagers").await;
            }
        }
        scenario.stop().await;
    })
    .await;
}
