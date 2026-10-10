use super::{
    Arc, ClientPlatform, EntityBase, GameMode, ItemStack, LivingEntity, Ordering, Player,
    PlayerInventory, RespawnState, ScreenHandler, TextComponent,
};
use pumpkin_inventory::Clearable;

impl Player {
    pub(crate) fn drop_equipment_on_death(&self, lifecycle: u64) {
        // Player.dropEquipment / Inventory.dropAll; the vanishing sweep excludes menus.
        if self.gamemode.load() == GameMode::Spectator
            || !self.living_entity.death_lifecycle_current(lifecycle)
        {
            return;
        }
        let drops = take_inventory_for_death(
            &self.inventory,
            self.world().level_info.load().game_rules.keep_inventory,
        );
        let world = self.world();
        // LivingEntity.createItemStackToDrop uses eye Y minus the Java float 0.3F.
        let mut pos = self.eye_position();
        pos.y -= f64::from(0.3f32);
        // Inventory.dropAll / Player.dropEquipment: finish delivering stacks already taken.
        // A spawn callback may replace this life, but must not delete the remaining old items.
        for stack in drops {
            super::inventory_drop::spawn_death_inventory_stack(&world, pos, stack);
        }
    }

    /// Applies retention and resets the active menu after respawn's menu removal.
    /// `restore_all` retains the real inventory and XP, as for a living dimension transfer.
    pub(crate) fn restore_inventory_after_respawn(&self, restore_all: bool) {
        // ServerPlayer.restoreFrom decides retention in the destination world.
        if !restore_all
            && !self.world().level_info.load().game_rules.keep_inventory
            && self.gamemode.load() != GameMode::Spectator
        {
            self.set_experience(0, 0.0, 0);
            self.inventory.clear();
        }
        *self
            .current_screen_handler
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            self.player_screen_handler.clone();
        self.open_container_pos.store(None);
    }

    /// Removes temporary menu contents in the old world, before a respawn world transfer.
    /// Living transfers return them to inventory; deaths drop them regardless of retention.
    pub(crate) fn remove_respawn_menus(&self, restore_all: bool) {
        // Player.remove / PlayerList.respawn remove menus before copying inventory and XP.
        // A living dimension transfer returns temporary items; KILLED drops them even with retention.
        let reason = if restore_all {
            super::super::RemovalReason::ChangedDimension
        } else {
            super::super::RemovalReason::Killed
        };
        self.get_entity().removal_reason.store(Some(reason));
        self.player_screen_handler
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .on_closed(self);
        let handler = self
            .current_screen_handler
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if Arc::as_ptr(&handler).cast::<()>()
            != Arc::as_ptr(&self.player_screen_handler).cast::<()>()
        {
            handler
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .on_closed(self);
        }
        self.get_entity().removal_reason.store(None);
    }

    pub fn handle_killed(&self, death_msg: &TextComponent) {
        let _owner = self.living_entity.own_damage();
        self.handle_killed_for_life(death_msg, self.living_entity.damage_lifecycle());
    }

    pub(crate) fn handle_killed_for_life(&self, death_msg: &TextComponent, lifecycle: u64) {
        // ServerPlayer.die: revalidate before changing the respawn state after callbacks.
        self.trigger_advancement(
            crate::entity::player::advancement::trigger::AdvancementTrigger::PlayerKilled,
        );
        if !self.living_entity.death_lifecycle_current(lifecycle) {
            return;
        }
        crate::entity::mob::neutral::tell_neutral_mobs_player_died(self, &self.world());
        // Reset air supply & drowning ticks on death
        self.breath_manager.reset(self);

        if matches!(self.client.as_ref(), ClientPlatform::Java(_)) {
            self.set_client_loaded(false);
        }
        self.send_combat_death(death_msg);
        self.send_health();
        self.send_bedrock_respawn_state(RespawnState::SearchingForSpawn);
    }

    pub(super) fn death_experience_reward(&self, killer: Option<&dyn EntityBase>) -> u32 {
        let amount = player_death_experience(
            self.experience_level.load(Ordering::Relaxed),
            self.world().level_info.load().game_rules.keep_inventory,
            self.gamemode.load() == GameMode::Spectator,
        );
        LivingEntity::process_mob_experience(killer, amount)
    }
}

fn take_inventory_for_death(inventory: &PlayerInventory, keep_inventory: bool) -> Vec<ItemStack> {
    if keep_inventory {
        return Vec::new();
    }
    let mut drops = Vec::new();
    let mut main = inventory
        .main_inventory
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for item in main.iter_mut() {
        let stack = std::mem::replace(item, ItemStack::EMPTY.clone());
        if !stack.is_empty() && !crate::entity::death_loot::prevents_equipment_drop(&stack) {
            drops.push(stack);
        }
    }
    drop(main);
    let mut equipment = inventory
        .entity_equipment
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for (_, stack) in equipment.equipment.drain() {
        if !stack.is_empty() && !crate::entity::death_loot::prevents_equipment_drop(&stack) {
            drops.push(stack);
        }
    }
    drops
}

fn player_death_experience(level: i32, keep_inventory: bool, spectator: bool) -> u32 {
    // Player.getBaseExperienceReward: the Java constants are 7 and 100.
    if keep_inventory || spectator {
        0
    } else {
        level.saturating_mul(7).clamp(0, 100) as u32
    }
}

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    reason = "Regression tests require successful fixture locks"
)]
mod tests {
    use super::*;
    use crate::server::Server;
    use pumpkin_data::{damage::DamageType, entity::EntityType, item::Item};
    use std::{
        sync::Mutex,
        time::{Duration, Instant},
    };
    #[test]
    fn death_inventory_drops_main_armor_and_offhand_without_changing_durability() {
        use super::take_inventory_for_death;
        use pumpkin_data::{
            Enchantment, data_component_impl::EquipmentSlot, item::Item, item_stack::ItemStack,
        };
        use pumpkin_inventory::{
            build_equipment_slots, entity_equipment::EntityEquipment,
            player::player_inventory::PlayerInventory,
        };
        use std::sync::{Arc, Mutex};
        let inventory = PlayerInventory::new(
            Arc::new(Mutex::new(EntityEquipment::new())),
            Arc::new(build_equipment_slots()),
        );
        inventory
            .main_inventory
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)[35] =
            ItemStack::new(64, &Item::STONE);
        let mut helmet = ItemStack::new(1, &Item::IRON_HELMET);
        helmet.set_damage(17);
        let mut cursed = ItemStack::new(1, &Item::IRON_BOOTS);
        cursed.add_enchantment(&Enchantment::VANISHING_CURSE, 1);
        {
            let mut equipment = inventory
                .entity_equipment
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            equipment.put(&EquipmentSlot::HEAD, helmet.clone());
            equipment.put(&EquipmentSlot::OFF_HAND, ItemStack::new(1, &Item::SHIELD));
            equipment.put(&EquipmentSlot::FEET, cursed)
        };
        assert!(take_inventory_for_death(&inventory, true).is_empty());
        assert_eq!(inventory.get_slot(35).item_count, 64);
        assert_eq!(inventory.get_slot(36).item, &Item::IRON_BOOTS);
        assert_eq!(inventory.get_slot(39).get_damage(), 17);
        let drops = take_inventory_for_death(&inventory, false);
        assert_eq!(drops.len(), 3);
        assert!(
            drops
                .iter()
                .any(|stack| stack.item == &Item::IRON_HELMET && stack.get_damage() == 17)
        );
        assert!(
            drops
                .iter()
                .any(|stack| stack.item == &Item::STONE && stack.item_count == 64)
        );
        assert!(drops.iter().any(|stack| stack.item == &Item::SHIELD));
        assert!(!drops.iter().any(|stack| stack.item == &Item::IRON_BOOTS));
        assert!(
            inventory
                .entity_equipment
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_empty()
        );
        assert!(take_inventory_for_death(&inventory, false).is_empty());
    }

    #[test]
    fn death_player_experience_respects_retention_spectator_and_cap() {
        use super::player_death_experience;
        assert_eq!(player_death_experience(10, false, false), 70);
        assert_eq!(player_death_experience(100, false, false), 100);
        assert_eq!(player_death_experience(10, true, false), 0);
        assert_eq!(player_death_experience(10, false, true), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn death_crafting_result_clears_after_a_real_recipe_is_drained() {
        use crate::entity::death_test_world::DeathTestWorld;
        use pumpkin_inventory::crafting::crafting_screen_handler::CraftingTableScreenHandler;
        let fixture = DeathTestWorld::new().await;
        let player = fixture.player("Crafter");
        let table = Arc::new(Mutex::new(CraftingTableScreenHandler::new(
            1,
            &player.inventory,
            None,
        )));
        *player.current_screen_handler.lock().unwrap() = table.clone();
        for (handler, width) in [
            (
                player.player_screen_handler.clone() as Arc<Mutex<dyn ScreenHandler>>,
                2,
            ),
            (table.clone() as Arc<Mutex<dyn ScreenHandler>>, 3),
        ] {
            let handler = handler.lock().unwrap();
            let slots = &handler.get_behaviour().slots;
            slots[1].set_stack(ItemStack::new(2, &Item::OAK_PLANKS));
            slots[1 + width].set_stack(ItemStack::new(3, &Item::OAK_PLANKS));
            slots[0].set_stack(ItemStack::EMPTY.clone());
            assert_eq!(slots[0].get_stack().item, &Item::STICK);
            assert_eq!(slots[0].get_stack().item_count, 4);
        }
        player
            .living_entity
            .damage(&*player, f32::MAX, DamageType::GENERIC_KILL);
        player.remove_respawn_menus(false);
        player.restore_inventory_after_respawn(false);
        for handler in [
            player.player_screen_handler.clone() as Arc<Mutex<dyn ScreenHandler>>,
            table,
        ] {
            let handler = handler.lock().unwrap();
            assert!(handler.get_behaviour().slots[0].get_stack().is_empty());
            assert!(handler.get_behaviour().slots[1].get_stack().is_empty());
        }
        let drops = fixture.world().entities.load_full();
        let planks: u32 = drops
            .iter()
            .filter_map(|entity| entity.get_item_entity())
            .map(|item| item.get_item_stack().lock().unwrap().clone())
            .filter(|stack| stack.item == &Item::OAK_PLANKS)
            .map(|stack| u32::from(stack.item_count))
            .sum();
        assert_eq!(planks, 10);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn death_temporary_vanishing_items_drop_with_retention_and_menu_resets() {
        use crate::entity::death_test_world::DeathTestWorld;
        use pumpkin_inventory::crafting::crafting_screen_handler::CraftingTableScreenHandler;
        let fixture = DeathTestWorld::new().await;
        fixture.keep_inventory(true);
        let player = fixture.player("Retained");
        let table = Arc::new(Mutex::new(CraftingTableScreenHandler::new(
            1,
            &player.inventory,
            None,
        )));
        let mut cursed = ItemStack::new(1, &Item::IRON_SWORD);
        cursed.add_enchantment(&pumpkin_data::Enchantment::VANISHING_CURSE, 1);
        player.inventory.set_slot(0, cursed.clone());
        player
            .player_screen_handler
            .lock()
            .unwrap()
            .get_behaviour()
            .slots[1]
            .set_stack(cursed.clone());
        let handler = table.lock().unwrap();
        handler.get_behaviour().slots[1].set_stack(cursed.clone());
        *handler.get_behaviour().cursor_stack.lock().unwrap() = cursed;
        drop(handler);
        *player.current_screen_handler.lock().unwrap() = table.clone();
        player
            .living_entity
            .damage(&*player, f32::MAX, DamageType::GENERIC_KILL);
        assert_eq!(player.inventory.get_slot(0).item, &Item::IRON_SWORD);
        assert!(
            fixture
                .world()
                .entities
                .load()
                .iter()
                .all(|entity| entity.get_item_entity().is_none())
        );
        player.remove_respawn_menus(false);
        player.restore_inventory_after_respawn(false);
        assert_eq!(player.inventory.get_slot(0).item, &Item::IRON_SWORD);
        let entities = fixture.world().entities.load_full();
        let drops = entities
            .iter()
            .filter_map(|entity| entity.get_item_entity())
            .filter(|item| item.get_item_stack().lock().unwrap().item == &Item::IRON_SWORD)
            .count();
        assert_eq!(drops, 3);
        assert!(
            table
                .lock()
                .unwrap()
                .get_behaviour()
                .cursor_stack
                .lock()
                .unwrap()
                .is_empty()
        );
        let current = player.current_screen_handler.lock().unwrap().clone();
        assert_eq!(
            Arc::as_ptr(&current).cast::<()>(),
            Arc::as_ptr(&player.player_screen_handler).cast::<()>()
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn death_respawn_drops_temporary_items_in_the_death_dimension() {
        use crate::{
            entity::death_test_world::DeathTestWorld,
            plugin::api::events::player::player_respawn::PlayerRespawnEvent,
            plugin::{BoxFuture, EventHandler, EventPriority},
        };
        use pumpkin_data::dimension::Dimension;
        use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
        use tokio::sync::Notify;

        struct RespawnReached(Arc<Notify>);
        impl EventHandler<PlayerRespawnEvent> for RespawnReached {
            fn handle<'a>(
                &'a self,
                _server: &'a Arc<Server>,
                _event: &'a PlayerRespawnEvent,
            ) -> BoxFuture<'a, ()> {
                Box::pin(async move { self.0.notify_one() })
            }
        }
        let fixture = DeathTestWorld::new().await;
        fixture.keep_inventory(true);
        let player = fixture.player("Traveller");
        let old_world = player.world();
        let destination = fixture
            .server
            .get_world_from_dimension(&Dimension::THE_NETHER);
        // This test checks death drops, so its forced respawn must not wait for terrain generation.
        crate::server::combat_test_support::publish_empty_chunk(
            &destination,
            pumpkin_util::math::vector2::Vector2::new(0, 0),
        );
        player.set_respawn_point(
            Dimension::THE_NETHER,
            BlockPos(Vector3::new(8, 200, 8)),
            0.0,
            0.0,
            true,
        );
        player
            .player_screen_handler
            .lock()
            .unwrap()
            .get_behaviour()
            .slots[1]
            .set_stack(ItemStack::new(1, &Item::DIAMOND));
        player
            .living_entity
            .damage(&*player, f32::MAX, DamageType::GENERIC_KILL);
        let reached = Arc::new(Notify::new());
        fixture.server.plugin_manager.register(
            Arc::new(RespawnReached(reached.clone())),
            EventPriority::Normal,
            false,
        );
        // Stop at the production respawn event, before waiting for a nonexistent network writer.
        tokio::time::timeout(Duration::from_secs(60), async {
            tokio::select! {
                () = old_world.respawn_player(&player, false) => {}
                () = reached.notified() => {}
            }
        })
        .await
        .unwrap();
        assert_eq!(player.world().uuid, destination.uuid);
        let drops = old_world.entities.load_full();
        assert!(
            drops
                .iter()
                .filter_map(|entity| entity.get_item_entity())
                .any(|item| item.get_item_stack().lock().unwrap().item == &Item::DIAMOND)
        );
        assert!(
            destination
                .entities
                .load()
                .iter()
                .all(|entity| entity.get_item_entity().is_none())
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn death_experience_retention_is_decided_at_respawn() {
        use crate::entity::death_test_world::DeathTestWorld;
        let fixture = DeathTestWorld::new().await;
        let player = fixture.player("XP");
        player.set_experience(10, 0.5, 167);
        player
            .living_entity
            .damage(&*player, f32::MAX, DamageType::GENERIC_KILL);
        assert!(
            fixture
                .world()
                .entities
                .load()
                .iter()
                .any(|entity| entity.get_entity().entity_type == &EntityType::EXPERIENCE_ORB)
        );
        assert_eq!(player.experience_level.load(Ordering::Relaxed), 10);
        fixture.keep_inventory(true);
        player.restore_inventory_after_respawn(false);
        assert_eq!(player.experience_level.load(Ordering::Relaxed), 10);
        assert_eq!(player.experience_progress.load(), 0.5);
        assert_eq!(player.experience_points.load(Ordering::Relaxed), 167);
        fixture.keep_inventory(false);
        player.restore_inventory_after_respawn(true);
        assert_eq!(player.experience_level.load(Ordering::Relaxed), 10);
        player.gamemode.store(GameMode::Spectator);
        player
            .inventory
            .set_slot(0, ItemStack::new(1, &Item::DIAMOND));
        player.restore_inventory_after_respawn(false);
        assert_eq!(player.inventory.get_slot(0).item, &Item::DIAMOND);
        assert_eq!(player.experience_level.load(Ordering::Relaxed), 10);
        player.gamemode.store(GameMode::Survival);
        player.restore_inventory_after_respawn(false);
        assert_eq!(player.experience_level.load(Ordering::Relaxed), 0);
        assert!(player.inventory.get_slot(0).is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn death_event_cancellation_precedes_inventory_and_experience_drops() {
        use crate::{
            entity::death_test_world::DeathTestWorld,
            plugin::api::events::entity::entity_death::PlayerDeathEvent,
            plugin::{BoxFuture, EventHandler, EventPriority},
        };
        struct CancelDeath;
        impl EventHandler<PlayerDeathEvent> for CancelDeath {
            fn handle_blocking<'a>(
                &'a self,
                _server: &'a Arc<Server>,
                event: &'a mut PlayerDeathEvent,
            ) -> BoxFuture<'a, ()> {
                Box::pin(async move {
                    assert_eq!(event.player.inventory.get_slot(0).item, &Item::DIAMOND);
                    event.cancelled = true;
                })
            }
        }
        let fixture = DeathTestWorld::new().await;
        fixture
            .server
            .plugin_manager
            .register(Arc::new(CancelDeath), EventPriority::Normal, true);
        let player = fixture.player("Cancelled");
        player
            .inventory
            .set_slot(0, ItemStack::new(1, &Item::DIAMOND));
        player.set_experience(10, 0.0, 160);
        player
            .living_entity
            .damage(&*player, f32::MAX, DamageType::GENERIC_KILL);
        assert_eq!(player.inventory.get_slot(0).item, &Item::DIAMOND);
        assert_eq!(player.experience_level.load(Ordering::Relaxed), 10);
        assert!(fixture.world().entities.load().is_empty());
    }

    #[test]
    fn death_inventory_locks_release_main_before_waiting_for_equipment() {
        use pumpkin_inventory::{build_equipment_slots, entity_equipment::EntityEquipment};
        for clearing in [false, true] {
            let inventory = Arc::new(PlayerInventory::new(
                Arc::new(Mutex::new(EntityEquipment::new())),
                Arc::new(build_equipment_slots()),
            ));
            inventory.set_slot(0, ItemStack::new(1, &Item::DIAMOND));
            let equipment = inventory.entity_equipment.lock().unwrap();
            let worker_inventory = inventory.clone();
            let worker = std::thread::spawn(move || {
                if clearing {
                    worker_inventory.clear();
                } else {
                    let _ = take_inventory_for_death(&worker_inventory, false);
                }
            });
            let deadline = Instant::now() + Duration::from_secs(5);
            let unlocked = loop {
                if inventory
                    .main_inventory
                    .try_read()
                    .is_ok_and(|main| main[0].is_empty())
                {
                    break true;
                }
                if Instant::now() >= deadline {
                    break false;
                }
                std::thread::sleep(Duration::from_millis(1));
            };
            drop(equipment);
            worker.join().unwrap();
            assert!(
                unlocked,
                "main inventory remained locked while waiting for equipment"
            );
        }
    }
}
