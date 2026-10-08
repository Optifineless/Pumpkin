use super::LivingEntity;
use crate::entity::EntityBase;
use crate::entity::equipment_damage::{EquippedItem, damage_equipped_item_if};
use crate::entity::player::statistics::StatisticCategory;
use pumpkin_data::damage::DamageType;
use pumpkin_data::data_component_impl::{BlocksAttacksImpl, EquipmentSlot};
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::SoundCategory;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_util::Hand;
use pumpkin_util::math::vector3::Vector3;
use std::sync::atomic::Ordering::Relaxed;

impl LivingEntity {
    /// Returns a snapshot of the active blocking item after its component delay has elapsed.
    // LivingEntity.getItemBlockingWith.
    pub fn get_item_blocking_with(&self) -> Option<ItemStack> {
        if self.livings_flags.load(Relaxed) & Self::USING_ITEM_FLAG == 0 {
            return None;
        }
        let item = self
            .item_in_use
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()?;
        let hand = (*self
            .active_hand
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner))?;
        // Revalidate between ticks too, as LivingEntity.updatingUsingItem does when ticking.
        let live = self
            .entity
            .world
            .load()
            .get_player_by_uuid(self.entity.entity_uuid)
            .map_or_else(
                || self.get_stack_in_hand(&self.entity, hand),
                |player| player.inventory.get_stack_in_hand(hand),
            );
        let item = refresh_used_item(&item, live, || self.clear_active_hand())?;
        self.update_used_item(hand, &item);
        let blocking = item.get_data_component::<BlocksAttacksImpl>()?;
        let elapsed = item.get_max_use_time() - self.item_use_time.load(Relaxed);
        (elapsed >= blocking.block_delay_ticks()).then_some(item)
    }

    /// Resolves blocking before mitigation and returns the amount to subtract from incoming damage.
    /// Applies durability, item-use statistics and melee disable cooldowns; it does not change health,
    /// damage cooldowns or full-hit feedback. Attacker block responses may apply knockback. `source` is the direct attacker/projectile.
    /// All equipment/use mutations run without hand locks.
    pub fn apply_item_blocking(
        &self,
        caller: &dyn EntityBase,
        damage_type: &DamageType,
        damage: f32,
        position: Option<Vector3<f64>>,
        source: Option<&dyn EntityBase>,
    ) -> f32 {
        // LivingEntity.applyItemBlocking.
        let life = self.own_damage();
        if damage <= 0.0 {
            return 0.0;
        }
        let Some(item) = self.get_item_blocking_with() else {
            return 0.0;
        };
        let Some(blocking) = item.get_data_component::<BlocksAttacksImpl>() else {
            return 0.0;
        };
        let pierce_level = source
            .and_then(|source| {
                source
                    .cast_any()
                    .downcast_ref::<crate::entity::projectile::arrow::ArrowEntity>()
            })
            .map_or(0, |arrow| arrow.pierce_level.load(Relaxed));
        if !is_damage_source_blocked(blocking, damage_type, pierce_level) {
            return 0.0;
        }
        let source_position =
            position.or_else(|| source.map(|source| source.get_entity().pos.load()));
        let angle = item_blocking_angle(
            source_position,
            self.entity.pos.load(),
            self.entity.head_yaw.load(),
        );
        let blocked = blocking.resolve_blocked_damage(damage_type, damage, angle);
        // Copy the hand and release its mutex before equipment callbacks or stopUsingItem.
        let hand = *self
            .active_hand
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.hurt_blocking_item(caller, hand, &item, blocking, blocked);
        if !life.is_current_life() {
            return blocked;
        }
        if blocked > 0.0
            && !damage_type.has_tag(&tag::DamageType::MINECRAFT_IS_PROJECTILE)
            && let Some(attacker) = source
            && attacker.get_living_entity().is_some()
        {
            // LivingEntity.blockUsingItem precedes Player's disable hook, even on full blocks.
            self.block_using_item(caller, attacker, blocked >= damage);
            if caller.get_player().is_some() && self.get_item_blocking_with().is_some() {
                self.disable_blocking(
                    caller,
                    &item,
                    blocking,
                    attacker.get_seconds_to_disable_blocking(),
                );
            }
        }
        blocked
    }

    // BlocksAttacks.hurtBlockingItem damages only players' blocking equipment in vanilla.
    fn hurt_blocking_item(
        &self,
        caller: &dyn EntityBase,
        hand: Option<Hand>,
        item: &ItemStack,
        blocking: &BlocksAttacksImpl,
        blocked: f32,
    ) {
        let Some(player) = caller.get_player() else {
            return;
        };
        let life = self.own_damage();
        let equipped = hand.map(|hand| {
            EquippedItem::capture(
                caller,
                &match hand {
                    Hand::Right => EquipmentSlot::MAIN_HAND,
                    Hand::Left => EquipmentSlot::OFF_HAND,
                },
            )
        });
        player.increment_stat(StatisticCategory::Used, i32::from(item.item.id), 1);
        if !life.is_current_life() {
            return;
        }
        let durability = blocking.item_damage.apply(blocked);
        if durability > 0
            && let Some(hand) = hand
            && let Some(equipped) = equipped
            && equipped.stack.uid == item.uid
            && damage_equipped_item_if(caller, &equipped, Some(&life), |_| Some(durability))
        {
            self.update_used_item(hand, &player.inventory.get_stack_in_hand(hand));
        }
    }

    // Vanilla ItemStack mutations update useItem by reference; Pumpkin keeps a copied stack.
    pub(super) fn update_used_item(&self, hand: Hand, updated: &ItemStack) {
        let active = *self
            .active_hand
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if active != Some(hand) {
            return;
        }
        let snapshot = self
            .item_in_use
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(snapshot) = snapshot
            && let Some(updated) =
                refresh_used_item(&snapshot, updated.clone(), || self.clear_active_hand())
        {
            *self
                .item_in_use
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(updated);
        }
    }

    // Player.blockUsingItem / BlocksAttacks.disable.
    fn disable_blocking(
        &self,
        caller: &dyn EntityBase,
        item: &ItemStack,
        blocking: &BlocksAttacksImpl,
        base_seconds: f32,
    ) {
        let ticks = blocking.disable_blocking_for_ticks(base_seconds);
        if ticks <= 0 {
            return;
        }
        if let Some(player) = caller.get_player() {
            player.start_cooldown(
                crate::entity::item_use::cooldown_group(item).to_owned(),
                ticks,
            );
        }
        self.clear_active_hand();
        if let Some(sound) = &blocking.disable_sound {
            self.play_blocking_sound(caller, sound, 0.8);
        }
    }

    /// Plays the configured block sound for a hit that reached the full damage-cooldown path.
    /// Pass the pre-blocking item snapshot because durability or disabling can stop item use.
    pub fn on_item_blocked(&self, caller: &dyn EntityBase, item: &ItemStack) {
        // BlocksAttacks.onBlocked; LivingEntity.hurtServer calls it only on full hits.
        if let Some(blocking) = item.get_data_component::<BlocksAttacksImpl>()
            && let Some(sound) = &blocking.block_sound
        {
            self.play_blocking_sound(caller, sound, 1.0);
        }
    }

    fn play_blocking_sound(
        &self,
        caller: &dyn EntityBase,
        sound: &pumpkin_data::data_component_impl::IdOr<
            pumpkin_data::data_component_impl::SoundEvent,
        >,
        volume: f32,
    ) {
        let packet = pumpkin_protocol::java::client::play::CSoundEffect::new(
            pumpkin_protocol::codec::data_component::data_to_proto_sound(sound),
            self.item_effect_sound_category(caller),
            &self.entity.pos.load(),
            volume,
            0.8 + rand::random::<f32>() * 0.4,
            rand::random(),
        );
        self.entity.world.load().broadcast_packet_all(&packet);
    }

    pub(super) fn item_effect_sound_category(&self, caller: &dyn EntityBase) -> SoundCategory {
        // Entity/Player/Monster/Bat.getSoundSource.
        if caller.get_player().is_some() {
            SoundCategory::Players
        } else if self.entity.entity_type.category == &pumpkin_data::entity::MobCategory::MONSTER {
            SoundCategory::Hostile
        } else if self.entity.entity_type.category == &pumpkin_data::entity::MobCategory::AMBIENT {
            SoundCategory::Ambient
        } else {
            SoundCategory::Neutral
        }
    }
}

// LivingEntity.updatingUsingItem refreshes same-item stacks and stops on a different item.
fn refresh_used_item(
    snapshot: &ItemStack,
    live: ItemStack,
    stop: impl FnOnce(),
) -> Option<ItemStack> {
    if live.is_empty() || live.item != snapshot.item {
        stop();
        None
    } else {
        Some(live)
    }
}

// LivingEntity.applyItemBlocking: a component bypass or piercing arrow skips all block effects.
fn is_damage_source_blocked(
    blocking: &BlocksAttacksImpl,
    damage_type: &DamageType,
    pierce_level: u8,
) -> bool {
    pierce_level == 0
        && !blocking.bypassed_by.as_ref().is_some_and(|types| {
            pumpkin_data::data_component_impl::combat::damage_type_set_contains(types, damage_type)
        })
}

pub fn seconds_to_disable_blocking<T: EntityBase + ?Sized>(caller: &T) -> f32 {
    if let Some(seconds) = caller
        .get_mob()
        .and_then(crate::entity::mob::Mob::blocking_disable_seconds_override)
    {
        return seconds;
    }
    let Some(living) = caller.get_living_entity() else {
        return 0.0;
    };
    let using_hand = (living.livings_flags.load(Relaxed) & LivingEntity::USING_ITEM_FLAG != 0)
        .then(|| {
            *living
                .active_hand
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        })
        .flatten();
    let weapon = caller.get_player().map_or_else(
        || living.held_item(&living.entity),
        |player| player.inventory.held_item(),
    );
    weapon_disable_seconds(&weapon, using_hand, caller.is_spectator())
}

// LivingEntity.getSecondsToDisableBlocking / getActiveItem.
fn weapon_disable_seconds(
    weapon_item: &ItemStack,
    using_hand: Option<Hand>,
    spectator: bool,
) -> f32 {
    if spectator || using_hand == Some(Hand::Left) {
        0.0
    } else {
        weapon_item
            .get_data_component::<pumpkin_data::data_component_impl::WeaponImpl>()
            .map_or(0.0, |weapon| weapon.disable_blocking_for_seconds)
    }
}

// LivingEntity.applyItemBlocking normalizes after flattening to the horizontal plane.
fn item_blocking_angle(source: Option<Vector3<f64>>, position: Vector3<f64>, yaw: f32) -> f64 {
    source.map_or(f64::from(std::f32::consts::PI), |source| {
        let delta = source - position;
        let horizontal = Vector3::new(delta.x, 0.0, delta.z);
        // Vec3.normalize (vanilla Vec3.java:89) uses the float constant 1.0E-5F.
        let direction = if horizontal.length() < f64::from(1.0e-5f32) {
            Vector3::new(0.0, 0.0, 0.0)
        } else {
            horizontal.normalize()
        };
        // Entity.calculateViewVector uses float radians and Mth's sine table.
        let radians = -yaw * (std::f64::consts::PI / 180.0) as f32;
        let view = Vector3::new(
            f64::from(pumpkin_util::math::sin(radians)),
            0.0,
            f64::from(pumpkin_util::math::cos(radians)),
        );
        direction.dot(&view).acos()
    })
}

#[cfg(test)]
mod tests {
    use super::{is_damage_source_blocked, item_blocking_angle, weapon_disable_seconds};
    use pumpkin_data::{
        damage::DamageType, data_component_impl::BlocksAttacksImpl, item::Item,
        item_stack::ItemStack,
    };
    use pumpkin_util::Hand;
    use pumpkin_util::math::vector3::Vector3;

    #[test]
    fn horizontal_blocking_ignores_source_height_and_rejects_rear_hits() {
        let position = Vector3::new(0.0, 0.0, 0.0);
        let above_front = Vector3::new(0.0, 100.0, 2.0);
        assert_eq!(item_blocking_angle(Some(above_front), position, 0.0), 0.0);
        let behind = Vector3::new(0.0, -100.0, -2.0);
        assert_eq!(
            item_blocking_angle(Some(behind), position, 0.0),
            std::f64::consts::PI
        );
        assert_eq!(
            item_blocking_angle(None, position, 0.0),
            f64::from(std::f32::consts::PI)
        );
    }

    #[test]
    fn piercing_and_component_bypasses_skip_blocking() {
        let item = ItemStack::new(1, &Item::SHIELD);
        let blocking = item.get_data_component::<BlocksAttacksImpl>().unwrap();
        assert!(is_damage_source_blocked(blocking, &DamageType::ARROW, 0));
        assert!(!is_damage_source_blocked(blocking, &DamageType::ARROW, 1));
        assert!(!is_damage_source_blocked(blocking, &DamageType::FALL, 0));
        let mut custom = blocking.clone();
        custom.bypassed_by = None;
        assert!(is_damage_source_blocked(&custom, &DamageType::FALL, 0));
    }

    #[test]
    fn only_the_active_main_hand_weapon_disables_blocking() {
        let axe = ItemStack::new(1, &Item::DIAMOND_AXE);
        assert_eq!(weapon_disable_seconds(&axe, None, false), 5.0);
        assert_eq!(weapon_disable_seconds(&axe, Some(Hand::Right), false), 5.0);
        assert_eq!(weapon_disable_seconds(&axe, Some(Hand::Left), false), 0.0);
        assert_eq!(weapon_disable_seconds(&axe, None, true), 0.0);
        let sword = ItemStack::new(1, &Item::DIAMOND_SWORD);
        assert_eq!(weapon_disable_seconds(&sword, None, false), 0.0);
    }
    #[test]
    fn rotated_and_oblique_hits_resolve_in_the_defenders_frame() {
        let origin = Vector3::new(4.0, 3.0, -2.0);
        let front = origin + Vector3::new(-2.0, 30.0, 0.0);
        let rear = origin + Vector3::new(2.0, 0.0, 0.0);
        assert!(item_blocking_angle(Some(front), origin, 90.0).abs() < 0.001);
        assert!(
            (item_blocking_angle(Some(rear), origin, 90.0) - std::f64::consts::PI).abs() < 0.001
        );
        let diagonal = origin + Vector3::new(-2.0, 0.0, 2.0);
        assert!(
            (item_blocking_angle(Some(diagonal), origin, 90.0) - std::f64::consts::FRAC_PI_4).abs()
                < 0.001
        );
        assert!((item_blocking_angle(Some(diagonal), origin, 45.0)).abs() < 0.02);
        let other = origin + Vector3::new(2.0, 0.0, -2.0);
        assert!((item_blocking_angle(Some(other), origin, -135.0)).abs() < 0.02);
    }

    #[test]
    fn a_changed_hand_stops_use_and_same_item_refreshes_the_snapshot() {
        use std::cell::Cell;
        let snapshot = ItemStack::new(1, &Item::SHIELD);
        let stopped = Cell::new(false);
        assert!(
            super::refresh_used_item(&snapshot, ItemStack::new(1, &Item::STONE), || stopped
                .set(true))
            .is_none()
        );
        assert!(stopped.get());
        let mut live = snapshot.clone();
        live.set_damage(50);
        let refreshed = super::refresh_used_item(&snapshot, live, || stopped.set(false)).unwrap();
        assert_eq!(refreshed.get_damage(), 50);
        assert!(stopped.get());
    }
}
