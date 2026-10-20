// Compiled by both versioned hosts to exercise their actual inventory resources.
use super::*;
use crate::pumpkin::plugin::inventory::{HostInventory, Inventory as WitInventory};
use pumpkin_core::{
    entity::NBTStorage, net::java::combat_test_support::TestPlayer, server::combat_test_support,
};
use pumpkin_data::item_stack::ItemStack;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::text::TextComponent;
use pumpkin_wasm_host_common::state::InventoryProvider;

struct Fixture {
    _directory: tempfile::TempDir,
    player: TestPlayer,
    host: PluginHostState,
    inventory: Resource<WitInventory>,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        let world = combat_test_support::world(&server, directory.path());
        let player = TestPlayer::new(&world);
        let mut host = PluginHostState::new();
        let inventory = host
            .add::<WitInventory>(InventoryProvider::PlayerMain(player.player.clone()))
            .unwrap();
        Self {
            _directory: directory,
            player,
            host,
            inventory,
        }
    }

    fn load(&mut self, name: &str) -> Resource<ItemStackHandle> {
        let mut components = NbtCompound::new();
        if name == "custom_name" {
            components.put_byte(name, 7);
        } else {
            components.put_string(name, "future format".into());
        }
        components.put_int("example:unrelated", 37);
        let mut stack = NbtCompound::new();
        stack.put_string("id", "minecraft:diamond_sword".into());
        stack.put_int("count", 1);
        stack.put_byte("Slot", 0);
        stack.put_compound("components", components);
        let mut saved = NbtCompound::new();
        saved.put_list("Inventory", vec![NbtTag::Compound(stack)]);
        self.player.player.inventory.read_nbt_non_mut(&saved);
        assert_eq!(
            self.player
                .player
                .inventory
                .get_slot(0)
                .raw_components
                .len(),
            2
        );
        HostInventory::get_item(
            &mut self.host,
            Resource::new_borrow(self.inventory.rep()),
            0,
        )
        .unwrap()
        .unwrap()
    }

    async fn commit_and_reload(&mut self, stack: Resource<ItemStackHandle>) -> ItemStack {
        HostInventory::set_item(
            &mut self.host,
            Resource::new_borrow(self.inventory.rep()),
            0,
            Some(stack),
        )
        .await
        .unwrap();
        let before = self.player.player.inventory.get_slot(0);
        assert_eq!(before.raw_components.len(), 1);
        assert_eq!(&*before.raw_components[0].0, "example:unrelated");
        let mut saved = NbtCompound::new();
        self.player.player.inventory.write_nbt(&mut saved);
        self.player
            .player
            .inventory
            .set_slot(0, ItemStack::EMPTY.clone());
        self.player.player.inventory.read_nbt_non_mut(&saved);
        let after = self.player.player.inventory.get_slot(0);
        assert_eq!(after.raw_components.len(), 1);
        assert_eq!(&*after.raw_components[0].0, "example:unrelated");
        assert!(matches!(after.raw_components[0].1, NbtTag::Int(37)));
        assert!(before.are_equal(&after));
        after
    }
}

#[tokio::test]
async fn raw_components_wasm_inventory_set_survives_save_reload() {
    let mut fixture = Fixture::new();
    for name in ["minecraft:damage", "damage", "!minecraft:damage", "!damage"] {
        let stack = fixture.load(name);
        HostItemStack::set_component(
            &mut fixture.host,
            Resource::new_borrow(stack.rep()),
            WitDataComponent::Damage,
            vec![5],
        )
        .await
        .unwrap();
        let restored = fixture.commit_and_reload(stack).await;
        assert_eq!(restored.get_damage(), 5, "{name}");
    }
}

#[tokio::test]
async fn raw_components_wasm_inventory_remove_resets_default_after_reload() {
    let mut fixture = Fixture::new();
    for name in ["minecraft:damage", "damage", "!minecraft:damage", "!damage"] {
        let stack = fixture.load(name);
        HostItemStack::remove_component(
            &mut fixture.host,
            Resource::new_borrow(stack.rep()),
            WitDataComponent::Damage,
        )
        .await
        .unwrap();
        let restored = fixture.commit_and_reload(stack).await;
        assert_eq!(restored.get_damage(), 0, "{name}");
        assert!(restored.has_data_component(DataComponent::Damage));
        assert!(
            !restored
                .patch
                .iter()
                .any(|(id, _)| *id == DataComponent::Damage)
        );
    }
}

#[tokio::test]
async fn raw_components_wasm_failed_decode_preserves_saved_value() {
    let mut fixture = Fixture::new();
    let stack = fixture.load("minecraft:damage");
    HostItemStack::set_component(
        &mut fixture.host,
        Resource::new_borrow(stack.rep()),
        WitDataComponent::Damage,
        Vec::new(),
    )
    .await
    .unwrap();
    HostInventory::set_item(
        &mut fixture.host,
        Resource::new_borrow(fixture.inventory.rep()),
        0,
        Some(stack),
    )
    .await
    .unwrap();
    let mut saved = NbtCompound::new();
    fixture.player.player.inventory.write_nbt(&mut saved);
    let stack = saved.get_list("Inventory").unwrap()[0]
        .extract_compound()
        .unwrap();
    assert_eq!(
        stack
            .get_compound("components")
            .unwrap()
            .get_string("minecraft:damage"),
        Some("future format")
    );
}

#[tokio::test]
async fn raw_components_wasm_helpers_replace_only_the_edited_component() {
    let mut fixture = Fixture::new();
    let stack = fixture.load("custom_name");
    let name = fixture
        .host
        .add::<WitTextComponent>(TextComponent::text("Edited"))
        .unwrap();
    HostItemStack::set_custom_name(
        &mut fixture.host,
        Resource::new_borrow(stack.rep()),
        Some(name),
    )
    .await
    .unwrap();
    let restored = fixture.commit_and_reload(stack).await;
    assert!(restored.has_data_component(DataComponent::CustomName));

    let stack = fixture.load("!custom_name");
    HostItemStack::set_custom_name(&mut fixture.host, Resource::new_borrow(stack.rep()), None)
        .await
        .unwrap();
    let restored = fixture.commit_and_reload(stack).await;
    assert!(!restored.has_data_component(DataComponent::CustomName));

    let stack = fixture.load("lore");
    let line = fixture
        .host
        .add::<WitTextComponent>(TextComponent::text("Edited lore"))
        .unwrap();
    HostItemStack::add_lore(&mut fixture.host, Resource::new_borrow(stack.rep()), line)
        .await
        .unwrap();
    let restored = fixture.commit_and_reload(stack).await;
    assert_eq!(
        restored
            .get_data_component::<LoreImpl>()
            .unwrap()
            .lines
            .len(),
        1
    );

    let stack = fixture.load("enchantments");
    HostItemStack::add_enchantment(
        &mut fixture.host,
        Resource::new_borrow(stack.rep()),
        to_wit_enchantment(&Enchantment::UNBREAKING),
        2,
    )
    .await
    .unwrap();
    let restored = fixture.commit_and_reload(stack).await;
    assert_eq!(
        restored
            .get_data_component::<EnchantmentsImpl>()
            .unwrap()
            .enchantment
            .len(),
        1
    );

    let stack = fixture.load("!attribute_modifiers");
    HostItemStack::clear_attribute_modifiers(&mut fixture.host, Resource::new_borrow(stack.rep()))
        .await
        .unwrap();
    fixture.commit_and_reload(stack).await;
}
