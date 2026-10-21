//! Admission-first payment for a single projectile action, with no receipt or refund.

use std::sync::{Arc, atomic::Ordering};

use crate::{
    entity::{EntityBase, player::Player},
    world::World,
};
use pumpkin_data::{data_component::DataComponent, item_stack::ItemStack};
use pumpkin_inventory::screen_handler::InventoryPlayer;
use pumpkin_inventory::{Inventory, player::player_inventory::PlayerInventory};
use pumpkin_util::{GameMode, Hand};

pub struct ProjectileSource {
    pub(crate) source: usize,
    expected: ItemStack,
    replacement: ItemStack,
    weapon: Option<ItemStack>,
    moving_weapon: bool,
    session: ProjectileSession,
    infinite: bool,
    committed: bool,
    stale: bool,
    destination: usize,
}

#[derive(Clone)]
pub struct ProjectileSession {
    world: Arc<World>,
    lifecycle: u64,
    chunk_epoch: u32,
    gamemode: GameMode,
}

impl ProjectileSession {
    pub(crate) fn is_current(&self, player: &Player) -> bool {
        // Membership is an ArcSwap snapshot; tracker lookup can take a registry lock.
        let players = self.world.players.load();
        let mut sessions = players
            .iter()
            .filter(|current| current.gameprofile.id == player.gameprofile.id);
        let session_current = sessions
            .next()
            .is_some_and(|current| std::ptr::eq(current.as_ref(), player))
            && sessions.next().is_none();
        session_current
            && self.gamemode != GameMode::Spectator
            && self.gamemode == player.gamemode.load()
            && self.lifecycle == player.living_entity.damage_lifecycle()
            && self.chunk_epoch == player.chunk_send_epoch.load(Ordering::Relaxed)
            && player.living_entity.is_alive()
            && !player.get_entity().is_removed()
            && !player.client.closed()
            && Arc::ptr_eq(&self.world, &player.world())
    }
}

impl ProjectileSource {
    pub(crate) fn plan_hand(player: &Player, hand: Hand, expected: &ItemStack) -> Option<Self> {
        let source = match hand {
            Hand::Right => usize::from(player.inventory().get_selected_slot()),
            Hand::Left => PlayerInventory::OFF_HAND_SLOT,
        };
        Self::plan(player, source, expected, 1)
    }

    // Block hooks and released tridents have an observed stack but no hand argument.
    pub(crate) fn plan_observed(player: &Player, expected: &ItemStack) -> Option<Self> {
        let inventory = player.inventory();
        let owner = inventory.slot_transaction(0);
        let _guard = owner.lock();
        let mut retained =
            (0..inventory.size()).filter(|slot| inventory.get_stack(*slot).uid == expected.uid);
        let source = retained.next()?;
        if retained.next().is_some() {
            return None;
        }
        Self::plan(player, source, expected, 1)
    }

    pub(crate) fn plan(
        player: &Player,
        source: usize,
        expected: &ItemStack,
        amount: u8,
    ) -> Option<Self> {
        let inventory = player.inventory();
        let owner = inventory.slot_transaction(source);
        let _guard = owner.lock();
        if !owner.is_live()
            || expected.is_empty()
            || !Self::matches(&inventory.get_stack(source), expected)
        {
            return None;
        }
        let infinite = player.has_infinite_materials();
        if !infinite && expected.item_count < amount {
            return None;
        }
        let mut replacement = expected.clone();
        if !infinite {
            replacement.decrement(amount);
            if replacement.is_empty() {
                replacement = ItemStack::EMPTY.clone();
            }
        }
        let plan = Self {
            source,
            expected: expected.clone(),
            replacement,
            weapon: None,
            moving_weapon: false,
            session: ProjectileSession {
                world: player.world(),
                lifecycle: player.living_entity.damage_lifecycle(),
                chunk_epoch: player.chunk_send_epoch.load(Ordering::Relaxed),
                gamemode: player.gamemode.load(),
            },
            infinite,
            committed: false,
            stale: false,
            destination: source,
        };
        plan.session_current(player).then_some(plan)
    }

    pub(crate) fn with_weapon(mut self, weapon: &ItemStack) -> Self {
        self.weapon = Some(weapon.clone());
        self
    }

    pub(crate) fn charge(player: &Player, source: usize, weapon: &ItemStack) -> Option<Self> {
        let mut plan = Self::plan(player, source, weapon, 0)?.with_weapon(weapon);
        plan.moving_weapon = true;
        plan.replacement
            .patch
            .retain(|(id, _)| *id != DataComponent::ChargedProjectiles);
        Some(plan)
    }

    pub(crate) fn free_weapon(player: &Player, source: usize, weapon: &ItemStack) -> Option<Self> {
        let mut plan = Self::plan(player, source, weapon, 0)?.with_weapon(weapon);
        plan.moving_weapon = true;
        Some(plan)
    }

    pub(crate) const fn paid(&self) -> bool {
        !self.infinite && self.expected.item_count > self.replacement.item_count
    }

    pub(crate) fn transfer(&self) -> ItemStack {
        if self.expected.item_count == 1 {
            self.expected.clone()
        } else {
            self.expected.copy_with_count(1)
        }
    }

    fn matches(current: &ItemStack, expected: &ItemStack) -> bool {
        current.uid == expected.uid && current.are_equal(expected)
    }

    pub(crate) fn session_current(&self, player: &Player) -> bool {
        self.session.is_current(player)
    }

    pub(crate) fn session(&self) -> ProjectileSession {
        self.session.clone()
    }

    // Only weapons may move. Multiple clones of a retained UID cannot choose a payment source.
    fn retained_weapon(&self, player: &Player, exact: bool) -> Option<usize> {
        let weapon = self.weapon.as_ref()?;
        let inventory = player.inventory();
        let mut retained = (0..inventory.size()).filter(|slot| {
            let current = inventory.get_stack(*slot);
            !current.is_empty() && current.uid == weapon.uid
        });
        let slot = retained.next()?;
        if retained.next().is_some() {
            return None;
        }
        let current = inventory.get_stack(slot);
        (current.item == weapon.item && (!exact || Self::matches(&current, weapon))).then_some(slot)
    }

    pub(crate) fn launch(&mut self, player: &Player, projectile: Arc<dyn EntityBase>) -> bool {
        let Some(prepared) = crate::entity::projectile::launch::prepare_projectile(
            &self.session.world,
            projectile,
            Some(player.get_entity()),
        ) else {
            self.publish(player);
            return false;
        };
        let admitted = self.insert(player, prepared);
        self.publish(player);
        admitted
    }

    pub(crate) fn insert(&mut self, player: &Player, projectile: Arc<dyn EntityBase>) -> bool {
        if self.stale {
            return false;
        }
        let world = self.session.world.clone();
        world.insert_admitted_projectile(projectile, |append| {
            let inventory = player.inventory();
            let owner = inventory.slot_transaction(self.source);
            let _guard = owner.lock();
            if !owner.is_live() || !self.session_current(player) {
                self.stale = true;
                return false;
            }
            let weapon_slot = if self.weapon.is_some() {
                let Some(slot) = self.retained_weapon(player, !self.committed) else {
                    self.stale = true;
                    return false;
                };
                Some(slot)
            } else {
                None
            };
            if self.committed {
                return append();
            }
            let destination = if self.moving_weapon {
                let Some(slot) = weapon_slot else {
                    self.stale = true;
                    return false;
                };
                slot
            } else {
                self.source
            };
            if player.has_infinite_materials() != self.infinite
                || !Self::matches(&inventory.get_stack(destination), &self.expected)
            {
                self.stale = true;
                return false;
            }
            if !append() {
                return false;
            }
            // Append may retry internally; debit exactly once after its successful CAS. Nothing
            // from here to owner release may reject, dispatch a callback or publish a packet.
            inventory.update_slot_storage(destination, &mut |current| {
                let previous = std::mem::replace(current, self.replacement.clone());
                inventory.on_slot_replaced(destination, &previous, current);
            });
            self.destination = destination;
            self.committed = true;
            true
        })
    }

    pub(crate) fn publish(&self, player: &Player) {
        let retained = {
            let owner = player.inventory().slot_transaction(self.source);
            let _guard = owner.lock();
            self.retained_weapon(player, false)
        };
        player.sync_hand_slot(self.source, player.inventory().get_stack(self.source));
        if self.destination != self.source {
            player.sync_hand_slot(
                self.destination,
                player.inventory().get_stack(self.destination),
            );
        }
        if let Some(slot) = retained
            && slot != self.source
            && slot != self.destination
        {
            player.sync_hand_slot(slot, player.inventory().get_stack(slot));
        }
    }
}
