use super::*;
use crate::plugin::api::events::inventory::inventory_open::InventoryOpenEvent;

struct ChangeOnOpen {
    command: Option<String>,
    called: AtomicBool,
}

impl EventHandler<InventoryOpenEvent> for ChangeOnOpen {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        event: &'a mut InventoryOpenEvent,
    ) -> BoxFuture<'a, ()> {
        if !self.called.swap(true, Ordering::Relaxed) {
            if let Some(command) = &self.command {
                let source = CommandSender::Console.into_source(server).with_silent();
                assert_eq!(
                    server
                        .command_dispatcher
                        .load()
                        .execute_input(command, &source),
                    Ok(1),
                );
            } else {
                event.cancelled = true;
            }
        }
        Box::pin(async {})
    }
}

fn install_pair(fixture: &Fixture) {
    fixture.command("setblock 8 64 8 chest[facing=north,type=left] strict");
    fixture.command("setblock 9 64 8 chest[facing=north,type=right] strict");
    fixture
        .world
        .get_block_entity(&LEFT)
        .unwrap()
        .get_inventory()
        .unwrap()
        .set_stack(0, ItemStack::new(64, &Item::DIAMOND));
    let second = fixture
        .world
        .get_block_entity(&RIGHT)
        .unwrap()
        .get_inventory()
        .unwrap();
    for slot in 0..second.size() {
        second.set_stack(slot, ItemStack::new(64, &Item::STONE));
    }
}

fn register_change(fixture: &Fixture, command: Option<String>) -> Arc<ChangeOnOpen> {
    let callback = Arc::new(ChangeOnOpen {
        command,
        called: AtomicBool::new(false),
    });
    fixture
        .server
        .plugin_manager
        .register::<InventoryOpenEvent, _>(callback.clone(), EventPriority::Normal, true);
    callback
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g10_inventory_open_revalidates_changed_chest_partner() {
    for replacement in ["chest[facing=north,type=single]", "barrel"] {
        let fixture = Fixture::new();
        install_pair(&fixture);
        assert_eq!(fixture.open(LEFT).unwrap().size(), 54);
        assert_eq!(fixture.comparator(), 8);
        let data = if replacement == "barrel" {
            r#"{Items:[{Slot:0b,id:"minecraft:emerald",count:19}],lock:{"minecraft:custom_name":"vault key"}}"#
        } else {
            r#"{LootTable:"minecraft:chests/simple_dungeon",LootTableSeed:352L,lock:{"minecraft:custom_name":"vault key"}}"#
        };
        let callback = register_change(
            &fixture,
            Some(format!("setblock 9 64 8 {replacement}{data} strict")),
        );

        let inventory = fixture.open(LEFT).unwrap();

        assert!(callback.called.load(Ordering::Relaxed));
        assert_eq!(
            inventory.size(),
            27,
            "InventoryOpenEvent invalidation must select the current single chest"
        );
        assert_eq!(inventory.get_stack(0).item, &Item::DIAMOND);
        assert_eq!(inventory.get_stack(0).item_count, 64);
        assert_eq!(fixture.comparator(), 1);
        let unrelated = fixture.world.get_block_entity(&RIGHT).unwrap();
        assert!(unrelated.container_lock().is_some());
        let unrelated_inventory = unrelated.clone().get_inventory().unwrap();
        assert_eq!(unrelated_inventory.size(), 27);
        if replacement == "barrel" {
            let stack = unrelated_inventory.get_stack(0);
            assert_eq!(stack.item, &Item::EMERALD);
            assert_eq!(stack.item_count, 19);
        } else {
            assert!(
                unrelated.has_loot_table(),
                "the callback's unrelated deferred loot must remain pending"
            );
        }
        fixture.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g10_inventory_open_rejects_detached_chest_source() {
    for replacement in [
        "barrel",
        "chest[facing=north,type=left]{Items:[{Slot:0b,id:\"minecraft:emerald\",count:19}]}",
    ] {
        let fixture = Fixture::new();
        install_pair(&fixture);
        let original = fixture.world.get_block_entity(&LEFT).unwrap();
        let callback = register_change(
            &fixture,
            Some(format!("setblock 8 64 8 {replacement} strict")),
        );

        assert!(
            fixture.open(LEFT).is_none(),
            "a source replaced in InventoryOpenEvent cannot open a detached inventory"
        );

        assert!(callback.called.load(Ordering::Relaxed));
        assert!(!Arc::ptr_eq(
            &original,
            &fixture.world.get_block_entity(&LEFT).unwrap()
        ));
        fixture.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g10_inventory_open_cancellation_keeps_existing_loot_order() {
    let fixture = Fixture::new();
    fixture.command("setblock 8 64 8 chest{LootTable:\"minecraft:chests/simple_dungeon\",LootTableSeed:352L} strict");
    let source = fixture.world.get_block_entity(&LEFT).unwrap();
    assert!(source.has_loot_table());
    let callback = register_change(&fixture, None);

    assert!(fixture.open(LEFT).is_none());

    assert!(callback.called.load(Ordering::Relaxed));
    assert!(
        !source.has_loot_table(),
        "initial loot resolution remains before the existing cancellable open event"
    );
    fixture.finish().await;
}
