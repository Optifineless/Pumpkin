use super::LivingEntity;
use crate::{entity::EntityBase, server::Server};
use pumpkin_data::{
    data_component_impl::{ConsumableImpl, FoodImpl, SuspiciousStewEffectsImpl, UseRemainderImpl},
    effect::StatusEffect,
    item::Item,
    item_stack::ItemStack,
    potion::Effect,
    statistic::StatisticCategory,
};
use pumpkin_inventory::screen_handler::InventoryPlayer;
use std::sync::atomic::Ordering::Relaxed;

impl LivingEntity {
    // LivingEntity.updatingUsingItem / updateUsingItem refresh the active hand each tick.
    pub(super) fn updating_using_item(&self, caller: &dyn EntityBase, server: &Server) {
        let hand = *self
            .active_hand
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let saved = self
            .item_in_use
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let (Some(hand), Some(saved)) = (hand, saved) else {
            return;
        };
        let current = self.get_stack_in_hand(caller, hand);
        if current.is_empty() || current.item.id != saved.item.id {
            self.clear_active_hand();
            return;
        }
        *self
            .item_in_use
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(current.clone());
        let remaining = self.item_use_time.load(Relaxed);
        if remaining <= 0 {
            return;
        }
        if let Some(player) = caller.get_player() {
            server
                .item_registry
                .on_use_tick(&current, player, remaining);
        }
        // CrossbowItem.useOnRelease keeps charging active until the release packet.
        if self.item_use_time.fetch_sub(1, Relaxed) == 1 && current.item.id != Item::CROSSBOW.id {
            self.complete_using_item(caller);
        }
    }

    /// Finishes use only if the active hand still contains the validated stack.
    // LivingEntity.completeUsingItem rechecks the hand after onUseTick before effects or consumption.
    pub(crate) fn complete_using_item(&self, caller: &dyn EntityBase) {
        let hand = *self
            .active_hand
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let saved = self
            .item_in_use
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let (Some(hand), Some(saved)) = (hand, saved) {
            let current = self.get_stack_in_hand(caller, hand);
            // Java's ItemStack.equals also requires the same stack instance here.
            if !current.is_empty() && current.uid == saved.uid && current.item.id == saved.item.id {
                self.finish_using_item(caller, current, hand, true);
            }
        }
        self.clear_active_hand();
    }

    // Consumable.startConsuming completes zero-duration uses without touching active-use state.
    pub(crate) fn consume_instantly(
        &self,
        caller: &dyn EntityBase,
        hand: pumpkin_util::Hand,
        item: &ItemStack,
    ) {
        let current = self.get_stack_in_hand(caller, hand);
        if current.uid == item.uid && current.are_equal(item) {
            self.finish_using_item(caller, current, hand, false);
        }
    }

    fn finish_using_item(
        &self,
        caller: &dyn EntityBase,
        mut item: ItemStack,
        hand: pumpkin_util::Hand,
        timed: bool,
    ) {
        let before = item.clone();
        let consumable = before.get_data_component::<ConsumableImpl>();
        if consumable.is_some() {
            if let Some(player) = caller.get_player() {
                item.decrement_unless_creative(player.gamemode.load(), 1);
            } else {
                item.decrement(1);
            }
            // Consumable.onConsume consumes the live stack in one serial operation.
            // Claim first here so callbacks cannot leave effects without payment.
            if !self.set_used_hand_stack(caller, hand, &before, item.clone()) {
                return;
            }
        }
        // ServerPlayer.completeUsingItem only notifies the consuming connection.
        if timed && let Some(player) = caller.get_player() {
            player.try_send_client_packet(
                &pumpkin_protocol::java::client::play::CEntityStatus::new(
                    self.entity.entity_id,
                    pumpkin_data::entity::EntityStatus::UseItemComplete as i8,
                ),
            );
        }
        if let Some(consumable) = consumable {
            self.play_consume_sound(&consumable.sound_event);
            let event = if consumable.animation
                == pumpkin_data::data_component_impl::ConsumeAnimation::Drink
            {
                "drink"
            } else {
                "eat"
            };
            if let Some(player) = caller.get_player() {
                player.increment_stat(StatisticCategory::Used, i32::from(before.item.id), 1);
                player.trigger_advancement(
                    crate::entity::player::advancement::trigger::AdvancementTrigger::ConsumeItem {
                        item_id: format!("minecraft:{}", before.item.registry_key),
                    },
                );
                if let Some(food) = before.get_data_component::<FoodImpl>() {
                    player
                        .hunger_manager
                        .eat(player, food.nutrition as u8, food.saturation);
                    self.entity.world.load().play_bedrock_level_sound(
                        "burp",
                        &self.entity.pos.load(),
                        -1,
                    );
                }
            }
            self.on_consume_listeners(&before);
            self.apply_consumable_effects(caller, &before);
            self.entity
                .world
                .load()
                .emit_game_event(event, self.entity.pos.load());
        }
        self.apply_after_use_component_side_effects(caller, &item, &before, hand);
    }

    // ItemStack.applyAfterUseComponentSideEffects uses the original components and count.
    fn apply_after_use_component_side_effects(
        &self,
        caller: &dyn EntityBase,
        item: &ItemStack,
        before: &ItemStack,
        hand: pumpkin_util::Hand,
    ) {
        let player = caller.get_player();
        if let Some(remainder) = before.get_data_component::<UseRemainderImpl>()
            && !player.is_some_and(InventoryPlayer::has_infinite_materials)
            && item.item_count < before.item_count
            && let Some(extra) = remainder.create()
        {
            if let Some(player) = player {
                crate::item::item_utils::finish_claimed_result(
                    player,
                    hand,
                    before,
                    &before.copy_with_count(1),
                    extra,
                    false,
                );
            } else if item.is_empty() {
                self.set_used_hand_stack(caller, hand, item, extra);
            }
        }
        if let Some(player) = player
            && let Some(cooldown) = before.get_use_cooldown()
        {
            player.start_cooldown(
                crate::entity::item_use::cooldown_group(before).to_owned(),
                (cooldown.seconds * 20.0) as i32,
            );
        }
    }

    fn set_used_hand_stack(
        &self,
        caller: &dyn EntityBase,
        hand: pumpkin_util::Hand,
        expected: &ItemStack,
        item: ItemStack,
    ) -> bool {
        if let Some(player) = caller.get_player() {
            player
                .inventory()
                .exchange_stack_in_hand(hand, expected, item)
        } else {
            let slot = if hand == pumpkin_util::Hand::Right {
                pumpkin_data::data_component_impl::EquipmentSlot::MAIN_HAND
            } else {
                pumpkin_data::data_component_impl::EquipmentSlot::OFF_HAND
            };
            let mut equipment = self
                .entity_equipment
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let current = equipment.get(&slot);
            if current.uid != expected.uid || !current.are_equal(expected) {
                return false;
            }
            equipment.put(&slot, item);
            true
        }
    }

    pub(super) fn play_consume_sound(
        &self,
        sound: &pumpkin_data::data_component_impl::IdOr<
            pumpkin_data::data_component_impl::basic::SoundEvent,
        >,
    ) {
        self.entity.world.load().play_sound_event(
            sound,
            pumpkin_data::sound::SoundCategory::Players,
            &self.entity.pos.load(),
        );
    }

    fn on_consume_listeners(&self, item: &ItemStack) {
        // PotionContents.onConsume and SuspiciousStewEffects.onConsume are component listeners.
        let effects = crate::item::potion::PotionContents::read_potion_effects(item);
        let duration_scale = item
            .get_data_component::<pumpkin_data::data_component_impl::PotionDurationScaleImpl>()
            .map_or(1.0, |scale| scale.scale);
        crate::item::potion::PotionContents::apply_effects_to(
            self,
            effects,
            duration_scale,
            crate::item::potion::PotionApplicationSource::Normal,
        );
        if let Some(stew) = item.get_data_component::<SuspiciousStewEffectsImpl>() {
            for entry in stew.effects.iter() {
                if let Some(effect_type) = StatusEffect::from_minecraft_name(&entry.effect) {
                    self.add_effect(Effect {
                        effect_type,
                        duration: entry.duration,
                        amplifier: 0,
                        ambient: false,
                        show_particles: true,
                        show_icon: true,
                        blend: false,
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{net::java::combat_test_support::TestPlayer, server::combat_test_support};
    use pumpkin_data::data_component_impl::SuspiciousStewEffect;
    use pumpkin_util::Hand;
    use std::borrow::Cow;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn hand_use_finishing_reads_mutated_components_from_the_same_stack() {
        let dir = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(dir.path());
        let world = combat_test_support::world(&server, dir.path());
        let fixture = TestPlayer::new(&world);
        let player = &fixture.player;
        let mut stew = ItemStack::new(1, &Item::SUSPICIOUS_STEW);
        player
            .inventory()
            .set_stack_in_hand(Hand::Left, stew.clone());
        player
            .living_entity
            .set_active_hand(Hand::Left, stew.clone(), 1);
        // An onUseTick callback can mutate the real stack without replacing it.
        stew.set_data_component(SuspiciousStewEffectsImpl {
            effects: Cow::Owned(vec![SuspiciousStewEffect {
                effect: Cow::Borrowed("minecraft:night_vision"),
                duration: 123,
            }]),
        });
        player.inventory().set_stack_in_hand(Hand::Left, stew);
        player.living_entity.complete_using_item(player.as_ref());
        assert!(player.living_entity.has_effect(&StatusEffect::NIGHT_VISION));
        assert_eq!(player.inventory().off_hand_item().item, &Item::BOWL);
        assert!(world.level.shutdown().await.is_ok());
    }
}

#[cfg(test)]
#[path = "consumption_conservation_tests.rs"]
mod conservation_tests;
