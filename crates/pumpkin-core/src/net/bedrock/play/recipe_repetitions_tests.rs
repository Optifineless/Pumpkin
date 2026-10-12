use super::created_output_tests::slot;
use crate::{
    entity::death_test_world::DeathTestWorld, net::bedrock::combat_test_support::TestBedrockPlayer,
};
use pumpkin_data::{item::Item, item_stack::ItemStack};
use pumpkin_inventory::{
    Inventory, inventory::slot_transaction, screen_handler::InventoryPlayer, slot::Slot,
};
use pumpkin_protocol::{
    bedrock::{
        network_item::ContainerName,
        server::item_stack_request::{ItemStackRequest, ItemStackRequestAction, SItemStackRequest},
    },
    codec::{var_int::VarInt, var_uint::VarUInt},
};
use std::sync::Arc;

// Force a retained inventory writer after the real result's capacity read, before payment.
struct MoveInputAfterCapacityRead {
    result: Arc<dyn Slot>,
    input: Arc<dyn Slot>,
    player: Arc<dyn Inventory>,
}
impl Slot for MoveInputAfterCapacityRead {
    fn get_inventory(&self) -> Arc<dyn Inventory> {
        self.result.get_inventory()
    }
    fn get_index(&self) -> usize {
        self.result.get_index()
    }
    fn set_id(&self, index: usize) {
        self.result.set_id(index);
    }
    fn mark_dirty(&self) {
        self.result.mark_dirty();
    }
    fn get_stack(&self) -> ItemStack {
        self.result.get_stack()
    }
    fn set_stack(&self, stack: ItemStack) {
        self.result.set_stack(stack);
    }
    fn can_insert(&self, stack: &ItemStack) -> bool {
        self.result.can_insert(stack)
    }
    fn on_quick_move_crafted(&self, stack: ItemStack, previous: ItemStack) {
        self.result.on_quick_move_crafted(stack, previous);
    }
    fn on_take_item(&self, player: &dyn InventoryPlayer, stack: &ItemStack) {
        self.result.on_take_item(player, stack);
    }
    fn craft_into(
        &self,
        output: &ItemStack,
        repetitions: u8,
        destination: &dyn Slot,
        expected: &ItemStack,
        replacement: &ItemStack,
    ) -> Option<pumpkin_inventory::crafting::CraftingCommit> {
        self.result
            .craft_into(output, repetitions, destination, expected, replacement)
    }
    fn get_max_item_count(&self) -> u8 {
        let capacity = self.result.get_max_item_count();
        std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    let grid = self.input.get_inventory();
                    let owners =
                        slot_transaction::ordered_owners(&[grid.as_ref(), self.player.as_ref()]);
                    let guards = slot_transaction::lock_owners(&owners);
                    let mut moved = grid.remove_stack_specific(self.input.get_index(), 1);
                    slot_transaction::deposit(self.player.as_ref(), 10, &mut moved, u8::MAX);
                    assert!(moved.is_empty());
                    drop(guards);
                })
                .join()
                .unwrap();
        });
        capacity
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bedrock_conservation_repetitions_two_cannot_take_eighteen_from_one_input() {
    for competing_writer in [false, true] {
        let fixture = DeathTestWorld::new().await;
        let mut bedrock = TestBedrockPlayer::new(&fixture.world()).await;
        // ResultSlot.onTake pays each nine-diamond batch with its own diamond block.
        let screen = bedrock
            .player
            .current_screen_handler
            .lock()
            .unwrap()
            .clone();
        {
            let mut handler = screen.lock().unwrap();
            let count = if competing_writer { 2 } else { 1 };
            handler.get_behaviour().slots[1].set_stack(ItemStack::new(count, &Item::DIAMOND_BLOCK));
            handler.get_behaviour().slots[0]
                .on_quick_move_crafted(ItemStack::EMPTY.clone(), ItemStack::EMPTY.clone());
            assert_eq!(
                handler.get_behaviour().slots[0]
                    .get_cloned_stack()
                    .item_count,
                9
            );
            if competing_writer {
                handler.get_behaviour_mut().slots[0] = Arc::new(MoveInputAfterCapacityRead {
                    result: handler.get_behaviour().slots[0].clone(),
                    input: handler.get_behaviour().slots[1].clone(),
                    player: bedrock.player.inventory.clone(),
                });
            }
        }
        bedrock.take_packets();
        bedrock.client().handle_item_stack_request(
            &bedrock.player,
            SItemStackRequest {
                requests: vec![ItemStackRequest {
                    request_id: VarInt(-1),
                    actions: vec![
                        ItemStackRequestAction::CraftRecipe {
                            recipe_id: VarUInt(1),
                            repetitions: 2,
                        },
                        ItemStackRequestAction::Take {
                            count: 18,
                            source: slot(ContainerName::CreatedOutput, 0),
                            destination: slot(ContainerName::Inventory, 9),
                        },
                    ],
                    filter_strings: Vec::new(),
                    filter_cause: 0,
                }],
            },
        );
        super::created_output_conservation_tests::assert_response(&mut bedrock, 1);
        let input = screen.lock().unwrap().get_behaviour().slots[1].get_cloned_stack();
        assert_eq!((input.item, input.item_count), (&Item::DIAMOND_BLOCK, 1));
        for index in 0..bedrock.player.inventory.size() {
            let owned = bedrock.player.inventory.get_stack(index);
            if competing_writer && index == 10 {
                assert_eq!((owned.item, owned.item_count), (&Item::DIAMOND_BLOCK, 1));
            } else {
                assert!(owned.is_empty(), "unpaid repetition delivered diamonds");
            }
        }
        assert!(
            screen
                .lock()
                .unwrap()
                .get_behaviour()
                .cursor_stack
                .lock()
                .unwrap()
                .is_empty()
        );
        bedrock.close().await;
        fixture.server.shutdown().await;
    }
}
