pub(super) use crate::{entity::EntityBase, net::java::combat_test_support::TestPlayer};
use crate::{
    server::{Server, combat_test_support},
    world::World,
};
use pumpkin_data::{
    damage::DamageType,
    data_component::DataComponent,
    data_component_impl::{
        BannerPatternsImpl, CustomNameImpl, DataComponentImpl, EquipmentSlot, ItemNameImpl, Rarity,
        RarityImpl, TooltipDisplayImpl,
    },
    entity::EntityType,
    packet::{
        CURRENT_MC_VERSION,
        clientbound::play::{
            CONTAINER_SET_CONTENT, CONTAINER_SET_SLOT, SET_CURSOR_ITEM, SET_EQUIPMENT,
        },
    },
};
pub(super) use pumpkin_data::{item::Item, item_stack::ItemStack};
use pumpkin_inventory::Inventory;
pub(super) use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::{
    VarInt,
    codec::data_component::deserialize,
    codec::item_stack_seralizer::OptionalItemStackHash,
    java::server::play::{SClickSlot, SlotActionType},
    ser::{NetworkReadExt, NetworkWriteExt},
};
use pumpkin_util::math::{position::BlockPos, vector2::Vector2, vector3::Vector3};
use std::{
    collections::HashMap,
    sync::{Arc, atomic::Ordering::Relaxed},
    time::Duration,
};

pub(super) const POSITION: BlockPos = BlockPos::new(8, 64, 8);

pub(super) struct Fixture {
    pub server: Arc<Server>,
    pub world: Arc<World>,
    pub client: TestPlayer,
    _directory: tempfile::TempDir,
}

impl Fixture {
    pub fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        let world = combat_test_support::world(&server, directory.path());
        server.worlds.store(Arc::new(vec![world.clone()]));
        combat_test_support::publish_empty_chunk(&world, Vector2::new(0, 0));
        let client = TestPlayer::new(&world);
        client
            .player
            .get_entity()
            .set_pos(Vector3::new(8.0, 64.0, 8.0));
        client
            .player
            .screen_handler_sync_handler
            .store_player(client.player.clone());
        client
            .player
            .on_screen_handler_opened(&client.player.player_screen_handler);
        client
            .player
            .chunk_sender
            .lock()
            .unwrap()
            .mark_sent_out_of_band(Vector2::new(0, 0));
        Self {
            server,
            world,
            client,
            _directory: directory,
        }
    }

    pub fn pillager(&self) -> Arc<dyn EntityBase> {
        let entity = crate::entity::r#type::from_type(
            &EntityType::PILLAGER,
            Vector3::new(8.0, 64.0, 8.0),
            &self.world,
            uuid::Uuid::new_v4(),
        );
        self.world.add_entity_silent(entity.clone());
        entity
    }

    pub fn kill(&self, victim: &Arc<dyn EntityBase>) {
        let mob = victim.get_mob().unwrap().get_mob_entity();
        for slot in [
            EquipmentSlot::MAIN_HAND,
            EquipmentSlot::OFF_HAND,
            EquipmentSlot::CHEST,
            EquipmentSlot::LEGS,
            EquipmentSlot::FEET,
        ] {
            mob.set_item_slot(&slot, ItemStack::EMPTY.clone());
        }
        let player = self.client.player.as_ref() as &dyn EntityBase;
        assert!(victim.damage_with_context(
            victim.as_ref(),
            f32::MAX,
            DamageType::GENERIC_KILL,
            None,
            Some(player),
            Some(player)
        ));
        assert!(victim.get_living_entity().unwrap().dead.load(Relaxed));
    }

    pub fn stacks(&self, item: &Item) -> Vec<ItemStack> {
        self.world
            .entities
            .load()
            .iter()
            .filter_map(|entity| entity.get_item_entity())
            .map(|entity| entity.get_item_stack().lock().unwrap().clone())
            .filter(|stack| !stack.is_empty() && stack.item.id == item.id)
            .collect()
    }

    pub fn release_items(&self) {
        for entity in self.world.entities.load().iter() {
            if let Some(item) = entity.get_item_entity() {
                item.set_pickup_delay(0);
            }
        }
    }

    pub async fn verify(mut self, action: impl FnOnce(&mut Self)) {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| action(&mut self)));
        tokio::time::timeout(Duration::from_secs(15), self.world.level.shutdown())
            .await
            .unwrap()
            .unwrap();
        self.world.players.store(Arc::new(Vec::new()));
        self.world.entities.store(Arc::new(Vec::new()));
        self.world.entity_tracker.entity_map.clear();
        self.world.level.world_portal.store(Arc::new(None));
        self.server.worlds.store(Arc::new(Vec::new()));
        if let Err(panic) = result {
            std::panic::resume_unwind(panic);
        }
    }
}

pub(super) fn head(entity: &Arc<dyn EntityBase>) -> ItemStack {
    entity
        .get_living_entity()
        .unwrap()
        .entity_equipment
        .lock()
        .unwrap()
        .get(&EquipmentSlot::HEAD)
}

pub(super) fn assert_canonical(stack: &ItemStack) {
    assert_eq!(stack.item.id, Item::WHITE_BANNER.id);
    assert_eq!(stack.item_count, 1);
    let patterns = stack.get_data_component::<BannerPatternsImpl>();
    assert!(
        patterns.is_some(),
        "Raid.getBannerComponentPatch must supply BannerPatterns"
    );
    let patterns = patterns.unwrap();
    let actual: Vec<_> = patterns
        .layers
        .iter()
        .map(|layer| (layer.pattern.as_str(), layer.color.id()))
        .collect();
    assert_eq!(
        actual,
        [
            ("minecraft:rhombus", 9),
            ("minecraft:stripe_bottom", 8),
            ("minecraft:stripe_center", 7),
            ("minecraft:border", 8),
            ("minecraft:stripe_middle", 15),
            ("minecraft:half_horizontal", 8),
            ("minecraft:circle", 8),
            ("minecraft:border", 15)
        ],
        "Raid.getBannerComponentPatch eight ordered layers"
    );
    assert_eq!(
        stack.get_data_component::<ItemNameImpl>().unwrap().name,
        "block.minecraft.ominous_banner"
    );
    let tooltip = stack.get_data_component::<TooltipDisplayImpl>().unwrap();
    assert!(!tooltip.hide_tooltip);
    assert!(tooltip.hidden_components == [DataComponent::BannerPatterns]);
    assert_eq!(
        stack.get_data_component::<RarityImpl>().unwrap().rarity,
        Rarity::Uncommon
    );
    assert!(stack.get_data_component::<CustomNameImpl>().is_none());
}

pub(super) fn equipment(client: &mut TestPlayer, entity: i32) -> HashMap<i8, ItemStack> {
    let mut result = HashMap::new();
    for bytes in client.take_packets() {
        let mut data = bytes.as_ref();
        if data.get_var_int().unwrap().0 == SET_EQUIPMENT.0 {
            let packet_entity = data.get_var_int().unwrap().0;
            let mut slots = Vec::new();
            loop {
                let slot = data.get_u8().unwrap();
                slots.push(((slot & 0x7f) as i8, read_published_stack(&mut data)));
                if slot & 0x80 == 0 {
                    break;
                }
            }
            assert!(data.is_empty(), "complete equipment packet consumption");
            if packet_entity == entity {
                if slots.iter().any(|(_, stack)| {
                    stack
                        .get_data_component::<ItemNameImpl>()
                        .is_some_and(|name| name.name == "block.minecraft.ominous_banner")
                }) {
                    // Independent 26.3 synchronized holder IDs + 1 and Java dye IDs.
                    let expected = [74, 8, 24, 9, 32, 8, 33, 7, 2, 8, 37, 15, 18, 8, 4, 8, 2, 15];
                    assert!(
                        bytes
                            .windows(expected.len())
                            .any(|window| window == expected),
                        "equipment contains the independently specified eight holder/color bytes"
                    );
                }
                result.extend(slots);
            }
        }
    }
    result
}

fn read_published_stack(data: &mut &[u8]) -> ItemStack {
    let count = data.get_var_int().unwrap().0;
    if count == 0 {
        return ItemStack::EMPTY.clone();
    }
    let item = Item::from_id(u16::try_from(data.get_var_int().unwrap().0).unwrap()).unwrap();
    let added = data.get_var_int().unwrap().0;
    let removed = data.get_var_int().unwrap().0;
    let mut patch = Vec::new();
    for _ in 0..added {
        let id = DataComponent::try_from_id(u8::try_from(data.get_var_int().unwrap().0).unwrap())
            .unwrap();
        // Clientbound ItemName is native text NBT; its unrelated generic reader is out of scope.
        let value = if id == DataComponent::ItemName {
            let nbt = data
                .get_nbt_with_version(&CURRENT_MC_VERSION)
                .unwrap()
                .unwrap();
            ItemNameImpl::read_data(&nbt).unwrap().to_dyn()
        } else {
            let value = deserialize(id, data);
            assert!(
                value.is_ok(),
                "published native banner/tooltip components must decode"
            );
            value.unwrap()
        };
        patch.push((id, Some(value)));
    }
    for _ in 0..removed {
        let id = DataComponent::try_from_id(u8::try_from(data.get_var_int().unwrap().0).unwrap())
            .unwrap();
        patch.push((id, None));
    }
    ItemStack::new_with_component(u8::try_from(count).unwrap(), item, patch)
}

pub(super) fn click(fixture: &mut Fixture, stack: &ItemStack, hashes: &[(u8, u32)]) {
    let corrections = click_claim(fixture, stack, hashes);
    assert!(
        corrections.is_empty(),
        "matching full client projection must not need corrections: {corrections:?}"
    );
}

pub(super) fn mismatched_click(fixture: &mut Fixture, stack: &ItemStack, hashes: &[(u8, u32)]) {
    let corrections = click_claim(fixture, stack, hashes);
    assert!(
        corrections.contains(&SET_CURSOR_ITEM.0),
        "mismatched client claim must emit a real cursor correction"
    );
}

fn click_claim(fixture: &mut Fixture, stack: &ItemStack, hashes: &[(u8, u32)]) -> Vec<i32> {
    let handler = fixture
        .client
        .player
        .current_screen_handler
        .lock()
        .unwrap()
        .clone();
    let revision = {
        let mut screen = handler.lock().unwrap();
        screen.get_behaviour().slots[36].set_stack(stack.clone());
        screen.set_received_stack(36, stack.clone());
        screen.get_behaviour().revision.load(Relaxed)
    };
    fixture.client.take_packets();
    let mut claim = vec![1];
    claim
        .write_var_int(&VarInt(i32::from(stack.item.id)))
        .unwrap();
    claim
        .write_var_int(&VarInt(i32::from(stack.item_count)))
        .unwrap();
    claim.write_var_int(&VarInt(hashes.len() as i32)).unwrap();
    for (id, hash) in hashes {
        claim.push(*id);
        claim.extend_from_slice(&hash.to_be_bytes());
    }
    claim.push(0);
    let mut raw = claim.as_slice();
    let carried_item = OptionalItemStackHash::read(&mut raw).unwrap();
    assert!(raw.is_empty());
    fixture.client.player.on_slot_click(
        SClickSlot {
            sync_id: VarInt(0),
            revision: VarInt(revision as i32),
            slot: 36,
            button: 0,
            mode: SlotActionType::Pickup,
            length_of_array: VarInt(1),
            array_of_changed_slots: vec![(36, OptionalItemStackHash(None))],
            carried_item,
        },
        &fixture.server,
    );
    assert!(
        fixture.client.player.inventory.get_stack(0).is_empty(),
        "authoritative source emptied"
    );
    let cursor = handler
        .lock()
        .unwrap()
        .get_behaviour()
        .cursor_stack
        .lock()
        .unwrap()
        .clone();
    assert!(
        cursor.are_equal(stack),
        "authoritative full stored stack moved, including opaque layers"
    );
    let corrections: Vec<_> = fixture
        .client
        .take_packets()
        .into_iter()
        .filter_map(|bytes| {
            let id = bytes.as_ref().get_var_int().unwrap().0;
            [
                CONTAINER_SET_SLOT.0,
                CONTAINER_SET_CONTENT.0,
                SET_CURSOR_ITEM.0,
            ]
            .contains(&id)
            .then_some(id)
        })
        .collect();
    *handler
        .lock()
        .unwrap()
        .get_behaviour()
        .cursor_stack
        .lock()
        .unwrap() = ItemStack::EMPTY.clone();
    corrections
}
