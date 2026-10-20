use std::{cell::RefCell, sync::Arc};

use pumpkin_data::{data_component_impl::UseRemainderImpl, item_stack::ItemStack};
use pumpkin_inventory::screen_handler::InventoryPlayer;
use pumpkin_inventory::{Inventory, player::player_inventory::PlayerInventory};
use pumpkin_util::Hand;

use crate::entity::{EntityBase, player::Player};
use crate::item::item_utils;
use crate::world::World;

struct HandCommit {
    player: usize,
    source: usize,
    before: ItemStack,
    changed: Option<bool>,
}

thread_local! {
    static HAND_COMMITS: RefCell<Vec<HandCommit>> = const { RefCell::new(Vec::new()) };
}

struct CommitScope;

impl Drop for CommitScope {
    fn drop(&mut self) {
        let frame = HAND_COMMITS.with(|frames| frames.borrow_mut().pop());
        drop(frame);
    }
}

/// Returns a receipt when mob use already committed the hand, including through nested callbacks.
/// The scope retains only owned values; no borrow or inventory guard crosses the operation.
pub fn with_hand_commit<R>(
    player: &Player,
    source: usize,
    before: &ItemStack,
    operation: impl FnOnce() -> R,
) -> (R, Option<bool>) {
    HAND_COMMITS.with(|frames| {
        frames.borrow_mut().push(HandCommit {
            player: std::ptr::from_ref(player) as usize,
            source,
            before: before.clone(),
            changed: None,
        });
    });
    let scope = CommitScope;
    let result = operation();
    let committed =
        HAND_COMMITS.with(|frames| frames.borrow().last().and_then(|frame| frame.changed));
    drop(scope);
    (result, committed)
}

/// Retains the physical source of a mob use without holding inventory ownership over callbacks.
pub struct MobInteraction {
    hand: Hand,
    source: usize,
    before: ItemStack,
    world: Arc<World>,
}

impl MobInteraction {
    pub fn new(player: &Player, hand: Hand, stack: &ItemStack) -> Self {
        let retained_source = HAND_COMMITS.with(|frames| {
            frames
                .borrow()
                .last()
                .filter(|frame| {
                    frame.player == std::ptr::from_ref(player) as usize
                        && frame.before.uid == stack.uid
                        && frame.before.are_equal(stack)
                })
                .map(|frame| frame.source)
        });
        Self {
            hand,
            source: retained_source.unwrap_or_else(|| {
                if hand == Hand::Right {
                    player.inventory().get_selected_slot() as usize
                } else {
                    PlayerInventory::OFF_HAND_SLOT
                }
            }),
            before: stack.clone(),
            world: player.world(),
        }
    }

    pub fn is_current(&self, player: &Player) -> bool {
        let inventory = player.inventory();
        let owner = inventory.slot_transaction(self.source);
        let _guard = owner.lock();
        let current = inventory.get_stack(self.source);
        owner.is_live()
            && player.has_client_loaded()
            && Arc::ptr_eq(&self.world, &player.world())
            && self
                .world
                .get_player_by_uuid(player.gameprofile.id)
                .is_some_and(|current| std::ptr::eq(current.as_ref(), player))
            && (self.hand == Hand::Left || inventory.get_selected_slot() as usize == self.source)
            && current.uid == self.before.uid
            && current.are_equal(&self.before)
    }

    pub const fn source_slot(&self) -> usize {
        self.source
    }

    pub const fn observed(&self) -> &ItemStack {
        &self.before
    }

    pub fn refresh(&self, player: &Player, stack: &mut ItemStack) {
        *stack = player.inventory().get_stack(self.source);
    }

    fn record_commit(&self, player: &Player, after: &ItemStack) {
        HAND_COMMITS.with(|frames| {
            if let Some(frame) = frames.borrow_mut().last_mut()
                && frame.player == std::ptr::from_ref(player) as usize
                && frame.source == self.source
                && frame.before.uid == self.before.uid
                && frame.before.are_equal(&self.before)
            {
                frame.changed =
                    Some(frame.changed.unwrap_or(false) || !after.are_equal(&self.before));
            }
        });
    }

    pub fn consume(&self, player: &Player, stack: &mut ItemStack) -> bool {
        let inventory = player.inventory();
        let owner = inventory.slot_transaction(self.source);
        let guard = owner.lock();
        if !self.is_current(player) {
            return false;
        }
        let mut after = self.before.clone();
        after.decrement_unless_creative(player.gamemode.load(), 1);
        if !pumpkin_inventory::inventory::slot_transaction::exchange(
            inventory.as_ref(),
            self.source,
            &self.before,
            after,
        ) {
            return false;
        }
        *stack = inventory.get_stack(self.source);
        self.record_commit(player, stack);
        drop(guard);
        if !stack.are_equal(&self.before) {
            player
                .inventory_changed
                .store(true, std::sync::atomic::Ordering::Relaxed);
            player.sync_hand_slot(self.source, stack.clone());
        }
        *stack = inventory.get_stack(self.source);
        true
    }

    // Mob.usePlayerItem applies the effective remainder before TamableAnimal.feed heals.
    pub fn use_player_item(&self, player: &Player, stack: &mut ItemStack) -> bool {
        if !player.has_infinite_materials()
            && let Some(remainder) = self.before.get_data_component::<UseRemainderImpl>()
            && let Some(remainder) = remainder.create()
        {
            return self.create_filled_result(player, stack, remainder, false);
        }
        self.consume(player, stack)
    }

    pub fn create_filled_result(
        &self,
        player: &Player,
        stack: &mut ItemStack,
        output: ItemStack,
        limit_creative_stack_size: bool,
    ) -> bool {
        let inventory = player.inventory();
        let owner = inventory.slot_transaction(self.source);
        let guard = owner.lock();
        if !self.is_current(player) {
            return false;
        }
        let creative_before =
            (limit_creative_stack_size && player.has_infinite_materials()).then(|| {
                (0..inventory.size())
                    .map(|slot| inventory.get_stack(slot))
                    .collect::<Vec<_>>()
            });
        let Some(result) = item_utils::commit_filled_result(
            stack,
            player,
            output,
            limit_creative_stack_size,
            self.source,
        ) else {
            return false;
        };
        let creative_changes = creative_before.map(|before| {
            before
                .into_iter()
                .enumerate()
                .filter_map(|(slot, previous)| {
                    let current = inventory.get_stack(slot);
                    (!current.are_equal(&previous)).then_some((slot, current))
                })
                .collect::<Vec<_>>()
        });
        self.record_commit(player, stack);
        drop(guard);
        result.publish(player, stack);
        if let Some(changes) = creative_changes
            && !changes.is_empty()
        {
            player
                .inventory_changed
                .store(true, std::sync::atomic::Ordering::Relaxed);
            for (slot, current) in changes {
                player.sync_hand_slot(slot, current);
            }
        }
        *stack = inventory.get_stack(self.source);
        true
    }
}

/// Restricts early hand admission to the three species using the committed interaction path.
pub fn uses_committed_hand(target: &dyn EntityBase) -> bool {
    use crate::entity::passive::{cat::CatEntity, cow::CowEntity, wolf::WolfEntity};
    target.cast_any().is::<CatEntity>()
        || target.cast_any().is::<WolfEntity>()
        || target.cast_any().is::<CowEntity>()
}
