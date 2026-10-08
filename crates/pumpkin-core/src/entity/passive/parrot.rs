use super::{
    animal::Animal,
    tamable::{TamableAnimal, TamableData},
};
use crate::entity::ai::goal::{
    follow_owner::FollowOwnerGoal, sit_when_ordered_to::SitWhenOrderedToGoal,
};
use crate::entity::ai::{control::flying_move_control::FlyingMoveControl, pathfinder::Navigator};
use pumpkin_nbt::compound::NbtCompound;
use std::sync::{Arc, Weak};

use pumpkin_data::damage::DamageType;
use pumpkin_data::effect::StatusEffect;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::tag::{self, Taggable};

use crate::entity::{
    Entity, EntityBase,
    ai::goal::{
        look_around::RandomLookAroundGoal, look_at_entity::LookAtEntityGoal, swim::SwimGoal,
        water_avoiding_random_flying::WaterAvoidingRandomFlyingGoal,
    },
    mob::{Mob, MobEntity},
    player::Player,
};

/// Duration in ticks of the poison a parrot gets from eating a cookie, matching
/// vanilla `Parrot.mobInteract`.
const COOKIE_POISON_DURATION: i32 = 900;

/// Represents a Parrot, a passive flying mob that can mimic nearby mob sounds.
///
/// Wiki: <https://minecraft.wiki/w/Parrot>
pub struct ParrotEntity {
    pub mob_entity: MobEntity,
    pub tamable_data: TamableData,
}

impl ParrotEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        // Parrot constructor / createNavigation.
        let mut navigation = Navigator::flying();
        navigation.set_can_open_doors(false);
        navigation.set_can_float(true);
        navigation.set_can_pass_doors(true);
        mob_entity.configure_movement(navigation, FlyingMoveControl::new(10, false));

        let parrot = Self {
            mob_entity,
            tamable_data: TamableData::default(),
        };
        let mob_arc = Arc::new(parrot);
        let mob_weak: Weak<dyn Mob> = {
            let mob_arc: Arc<dyn Mob> = mob_arc.clone();
            Arc::downgrade(&mob_arc)
        };

        {
            let mut goal_selector = mob_arc
                .mob_entity
                .goals_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);

            goal_selector.add_goal(0, Box::new(SwimGoal::default()));
            goal_selector.add_goal(2, Box::new(SitWhenOrderedToGoal::new()));
            goal_selector.add_goal(2, FollowOwnerGoal::new(1.0, 5.0, 1.0));
            goal_selector.add_goal(2, Box::new(WaterAvoidingRandomFlyingGoal::new(1.0)));
            goal_selector.add_goal(
                2,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 6.0),
            );
            goal_selector.add_goal(3, Box::new(RandomLookAroundGoal::default()));
        };

        mob_arc
    }

    /// Feeds the parrot a cookie: it is poisoned and then killed, as in vanilla
    /// `Parrot.mobInteract`.
    fn eat_cookie(&self, player: &Arc<Player>, item_stack: &mut ItemStack) {
        item_stack.decrement_unless_creative(player.gamemode.load(), 1);

        self.mob_entity
            .living_entity
            .add_effect(pumpkin_data::potion::Effect {
                effect_type: &StatusEffect::POISON,
                duration: COOKIE_POISON_DURATION,
                amplifier: 0,
                ambient: false,
                show_particles: true,
                show_icon: true,
                blend: true,
            });

        // Vanilla guards this call with `player.isCreative() || !this.isInvulnerable()`,
        // but `hurt` re-checks invulnerability itself and `player_attack` doesn't bypass
        // it, so the guard only skips a call that would do nothing anyway.
        self.damage_with_context(
            self,
            f32::MAX,
            DamageType::PLAYER_ATTACK,
            None,
            Some(player.as_ref()),
            Some(player.as_ref()),
        );
    }
}

impl Animal for ParrotEntity {
    fn is_food(&self, _item: &ItemStack) -> bool {
        false
    }
}
impl TamableAnimal for ParrotEntity {
    fn get_tamable_data(&self) -> &TamableData {
        &self.tamable_data
    }
}
impl Mob for ParrotEntity {
    fn get_base_experience_reward(&self) -> u32 {
        // Animal.getBaseExperienceReward, inherited through ShoulderRidingEntity.
        rand::random_range(1..=3)
    }

    fn as_tamable(&self) -> Option<&dyn TamableAnimal> {
        Some(self)
    }
    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        self.write_tamable_nbt(nbt);
    }
    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.read_tamable_nbt(nbt);
    }

    // Parrot.aiStep calls calculateFlapping after LivingEntity.aiStep movement.
    fn post_tick(&self) {
        let entity = self.get_entity();
        let mut velocity = entity.velocity.load();
        velocity.y = flapping_descent(
            velocity.y,
            entity.on_ground.load(std::sync::atomic::Ordering::Relaxed),
        );
        entity.velocity.store(velocity);
    }

    fn omnidirectional_air_mover(&self) -> bool {
        true
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn mob_interact(&self, player: &Arc<Player>, item_stack: &mut ItemStack) -> bool {
        // Parrot.mobInteract: ownership and sitting are prerequisites for FollowOwnerGoal.
        if !self.is_tame() && item_stack.item.has_tag(&tag::Item::MINECRAFT_PARROT_FOOD) {
            item_stack.decrement_unless_creative(player.gamemode.load(), 1);
            let entity = self.get_entity();
            let world = entity.world.load();
            if !entity.is_silent() {
                world.play_sound_fine(
                    pumpkin_data::sound::Sound::EntityParrotEat,
                    pumpkin_data::sound::SoundCategory::Neutral,
                    &entity.pos.load(),
                    1.0,
                    1.0 + (rand::random::<f32>() - rand::random::<f32>()) * 0.2,
                );
            }
            let mut success = rand::random_range(0..10) == 0;
            if success {
                let mut event =
                    crate::plugin::api::events::entity::entity_tame::EntityTameEvent::new(
                        entity.entity_id,
                        player.clone(),
                    );
                if let Some(server) = world.server.upgrade() {
                    server.plugin_manager.fire_blocking(&server, &mut event);
                }
                success = !event.cancelled;
                if success {
                    self.tamable_data.owner.store(Some(player.gameprofile.id));
                    self.set_tame(true);
                }
            }
            world.send_entity_status(
                entity,
                if success {
                    pumpkin_data::entity::EntityStatus::TamingSucceeded
                } else {
                    pumpkin_data::entity::EntityStatus::TamingFailed
                },
                None,
            );
            return true;
        }
        if !item_stack
            .get_item()
            .has_tag(&tag::Item::MINECRAFT_PARROT_POISONOUS_FOOD)
        {
            if self
                .get_entity()
                .on_ground
                .load(std::sync::atomic::Ordering::Relaxed)
                && self.is_tame()
                && self.get_owner() == Some(player.gameprofile.id)
            {
                self.set_ordered_to_sit(!self.is_ordered_to_sit());
                return true;
            }
            return self.mob_entity.mob_interact(player, item_stack);
        }

        self.eat_cookie(player, item_stack);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::COOKIE_POISON_DURATION;
    use pumpkin_data::item::Item;
    use pumpkin_data::tag::{self, Taggable};

    /// The interaction is gated on the vanilla `parrot_poisonous_food` tag rather than
    /// on a hardcoded cookie id, so check the tag actually resolves the way the
    /// interaction assumes.
    #[test]
    fn cookie_is_poisonous_parrot_food() {
        assert!(Item::COOKIE.has_tag(&tag::Item::MINECRAFT_PARROT_POISONOUS_FOOD));
    }

    /// Seeds tame a parrot in vanilla and must not reach the poison branch.
    #[test]
    fn parrot_food_is_not_poisonous() {
        assert!(!Item::WHEAT_SEEDS.has_tag(&tag::Item::MINECRAFT_PARROT_POISONOUS_FOOD));
        assert!(!Item::COOKED_CHICKEN.has_tag(&tag::Item::MINECRAFT_PARROT_POISONOUS_FOOD));
    }

    #[test]
    fn poison_lasts_45_seconds() {
        assert_eq!(COOKIE_POISON_DURATION, 900);
    }
}

// Parrot.calculateFlapping only damps downward airborne motion.
fn flapping_descent(y: f64, on_ground: bool) -> f64 {
    if !on_ground && y < 0.0 { y * 0.6 } else { y }
}

#[cfg(test)]
mod movement_tests {
    use super::flapping_descent;

    #[test]
    fn parrot_damps_only_airborne_descent() {
        assert_eq!(flapping_descent(-0.5, false), -0.3);
        assert_eq!(flapping_descent(0.5, false), 0.5);
        assert_eq!(flapping_descent(-0.5, true), -0.5);
    }
}
