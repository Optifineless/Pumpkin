use super::rabbit_goals::{
    RabbitAvoidEntityGoal, RabbitPanicGoal, RabbitPowderSnowGoal, RaidGardenGoal,
};
use super::rabbit_movement::RabbitJumpState;
use super::rabbit_stroll::RabbitStrollGoal;
use crate::entity::ai::control::rabbit_move_control::RabbitMoveControl;
use std::sync::{
    Arc, LazyLock, Weak,
    atomic::{AtomicI32, Ordering},
};

use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::{self, Taggable};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::codec::var_int::VarInt;
use rand::RngExt;

use crate::entity::{
    Entity, EntityBase,
    ageable::{AgeableData, AgeableMob},
    ai::goal::{
        active_target::ActiveTargetGoal, avoid_entity::AvoidEntityGoal, breed::BreedGoal,
        look_at_entity::LookAtEntityGoal, melee_attack::MeleeAttackGoal, swim::SwimGoal,
        tempt::TemptGoal,
    },
    attributes::{Modifier, ModifierOperation},
    mob::{Mob, MobEntity},
    passive::animal::Animal,
    player::Player,
};

// Rabbit's named movement constants.
pub(super) const STROLL_SPEED_MOD: f64 = 0.6;
pub(super) const BREED_SPEED_MOD: f64 = 0.8;
pub(super) const FOLLOW_SPEED_MOD: f64 = 1.0;
pub(super) const FLEE_SPEED_MOD: f64 = 2.2;
pub(super) const ATTACK_SPEED_MOD: f64 = 1.4;
pub(super) const MORE_CARROTS_DELAY: i32 = 40;

// Rabbit.registerGoals uses ItemTags.RABBIT_FOOD for temptation as well as breeding.
static TEMPT_ITEMS: LazyLock<Vec<&'static Item>> = LazyLock::new(|| {
    tag::Item::MINECRAFT_RABBIT_FOOD
        .0
        .iter()
        .filter_map(|name| Item::from_registry_key(name))
        .collect()
});

// Rabbit.setVariant's named constants and transient attribute modifier.
const EVIL_ATTACK_POWER_INCREMENT: f64 = 5.0;
const EVIL_ARMOR_VALUE: f64 = 8.0;
const EVIL_ATTACK_POWER_MODIFIER: &str = "minecraft:evil";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(i32)]
pub enum RabbitVariant {
    #[default]
    Brown = 0,
    White = 1,
    Black = 2,
    WhiteSplotched = 3,
    Gold = 4,
    Salt = 5,
    Evil = 99,
}

impl RabbitVariant {
    #[must_use]
    pub const fn from_id(id: i32) -> Self {
        match id {
            1 => Self::White,
            2 => Self::Black,
            3 => Self::WhiteSplotched,
            4 => Self::Gold,
            5 => Self::Salt,
            99 => Self::Evil,
            _ => Self::Brown,
        }
    }

    #[must_use]
    pub const fn id(self) -> i32 {
        self as i32
    }

    #[must_use]
    pub fn random_variant() -> Self {
        let mut rng = rand::rng();
        match rng.random_range(0..6) {
            1 => Self::White,
            2 => Self::Black,
            3 => Self::WhiteSplotched,
            4 => Self::Gold,
            5 => Self::Salt,
            _ => Self::Brown,
        }
    }
}

pub struct RabbitEntity {
    pub mob_entity: MobEntity,
    pub ageable_data: AgeableData,
    pub variant: AtomicI32,
    pub(super) jump_state: RabbitJumpState,
    pub more_carrot_ticks: AtomicI32,
}

impl RabbitEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        let variant = RabbitVariant::random_variant();
        let rabbit = Self {
            mob_entity,
            ageable_data: AgeableData::default(),
            variant: AtomicI32::new(variant.id()),
            jump_state: RabbitJumpState::default(),
            more_carrot_ticks: AtomicI32::new(0),
        };
        let mob_arc = Arc::new(rabbit);
        // Rabbit constructor installs its hopping control; a weak owner avoids a reference cycle.
        *mob_arc
            .mob_entity
            .move_control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Box::new(RabbitMoveControl::new(Arc::downgrade(&mob_arc)));
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

            goal_selector.add_goal(1, Box::new(SwimGoal::default()));
            goal_selector.add_goal(1, Box::new(RabbitPowderSnowGoal));
            goal_selector.add_goal(1, Box::new(RabbitPanicGoal::new()));
            goal_selector.add_goal(2, BreedGoal::new(BREED_SPEED_MOD));
            goal_selector.add_goal(
                3,
                Box::new(TemptGoal::new(FOLLOW_SPEED_MOD, &TEMPT_ITEMS, false)),
            );
            goal_selector.add_goal(
                4,
                Box::new(RabbitAvoidEntityGoal::new(AvoidEntityGoal::new(
                    &EntityType::PLAYER,
                    8.0,
                    FLEE_SPEED_MOD,
                    FLEE_SPEED_MOD,
                ))),
            );
            goal_selector.add_goal(
                4,
                Box::new(RabbitAvoidEntityGoal::new(AvoidEntityGoal::new(
                    &EntityType::WOLF,
                    10.0,
                    FLEE_SPEED_MOD,
                    FLEE_SPEED_MOD,
                ))),
            );
            goal_selector.add_goal(4, Box::new(RabbitAvoidEntityGoal::monsters()));
            goal_selector.add_goal(5, RaidGardenGoal::new());
            goal_selector.add_goal(6, Box::new(RabbitStrollGoal::default()));
            goal_selector.add_goal(
                11,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 10.0),
            );
        };

        mob_arc
    }

    #[must_use]
    pub fn get_variant(&self) -> RabbitVariant {
        RabbitVariant::from_id(self.variant.load(Ordering::Relaxed))
    }

    pub fn set_variant(&self, variant: RabbitVariant) {
        self.variant.store(variant.id(), Ordering::Relaxed);
        let entity = self.get_entity();
        entity.set_synced_data(
            pumpkin_data::tracked_data::rabbit::DATA_TYPE_ID,
            VarInt(variant.id()),
        );

        if variant == RabbitVariant::Evil {
            self.mob_entity.living_entity.set_attribute_base(
                &pumpkin_data::attributes::Attributes::ARMOR,
                EVIL_ARMOR_VALUE,
            );
            self.mob_entity.living_entity.update_attribute(
                &pumpkin_data::attributes::Attributes::ATTACK_DAMAGE,
                |attribute| {
                    attribute.add_or_replace_modifier(Modifier {
                        id: EVIL_ATTACK_POWER_MODIFIER.into(),
                        amount: EVIL_ATTACK_POWER_INCREMENT,
                        operation: ModifierOperation::Add,
                        permanent: false,
                    });
                },
            );
            if entity.custom_name.load().is_none() {
                entity.set_custom_name(pumpkin_util::text::TextComponent::translate(
                    "entity.minecraft.killer_bunny",
                    &[],
                ));
            }
            let mut goal_selector = self
                .mob_entity
                .goals_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            goal_selector.add_goal(4, Box::new(MeleeAttackGoal::new(ATTACK_SPEED_MOD, true)));

            let mut target_selector = self
                .mob_entity
                .target_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            target_selector.add_goal(
                1,
                Box::new(crate::entity::ai::goal::revenge::RevengeGoal::new(true)),
            );
            target_selector.add_goal(
                2,
                ActiveTargetGoal::with_default(&self.mob_entity, &EntityType::PLAYER, true),
            );
            target_selector.add_goal(
                2,
                ActiveTargetGoal::with_default(&self.mob_entity, &EntityType::WOLF, true),
            );
        } else {
            self.mob_entity.living_entity.update_attribute(
                &pumpkin_data::attributes::Attributes::ATTACK_DAMAGE,
                |attribute| attribute.remove_modifier(EVIL_ATTACK_POWER_MODIFIER),
            );
        }
    }
}

impl AgeableMob for RabbitEntity {
    fn get_ageable_data(&self) -> &AgeableData {
        &self.ageable_data
    }
}

impl Animal for RabbitEntity {
    fn is_food(&self, item_stack: &ItemStack) -> bool {
        item_stack.item.has_tag(&tag::Item::MINECRAFT_RABBIT_FOOD)
    }
}

impl Mob for RabbitEntity {
    fn play_attack_sound(&self) {
        // Rabbit.playAttackSound and getSoundSource.
        let entity = self.get_entity();
        if self.get_variant() == RabbitVariant::Evil && !entity.is_silent() {
            let pitch = (rand::random::<f32>() - rand::random::<f32>()) * 0.2 + 1.0;
            entity.world.load().play_sound_fine(
                Sound::EntityRabbitAttack,
                SoundCategory::Hostile,
                &entity.pos.load(),
                1.0,
                pitch,
            );
        }
    }

    fn custom_server_ai_step(&self, _caller: &dyn EntityBase) {
        self.tick_hopping();
    }
    fn jump_power_scale(&self) -> f64 {
        self.rabbit_jump_power_scale()
    }
    fn after_jump(&self) {
        self.rabbit_after_jump();
    }
    fn post_tick(&self) {
        self.tick_jump_animation();
    }
    fn tick_jump_control(&self) {
        // RabbitJumpControl.tick does not clear jumping when there is no new request.
        let requested = self
            .mob_entity
            .jump_control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take_request();
        if requested {
            self.start_jumping();
        }
    }

    fn as_ageable(&self) -> Option<&dyn AgeableMob> {
        Some(self)
    }

    fn as_animal(&self) -> Option<&dyn Animal> {
        Some(self)
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        self.write_ageable_nbt(nbt);
        nbt.put_int("RabbitType", self.get_variant().id());
        nbt.put_int(
            "MoreCarrotTicks",
            self.more_carrot_ticks.load(Ordering::Relaxed),
        );
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.read_ageable_nbt(nbt);
        if let Some(rabbit_type) = nbt.get_int("RabbitType") {
            self.set_variant(RabbitVariant::from_id(rabbit_type));
        }
        if let Some(ticks) = nbt.get_int("MoreCarrotTicks") {
            self.more_carrot_ticks.store(ticks, Ordering::Relaxed);
        }
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn mob_tick(&self, _caller: &dyn EntityBase) {
        self.ageable_ai_step();
    }

    fn mob_init_data_tracker(&self) {
        let entity = self.get_entity();
        let is_baby = entity.age.load(Ordering::Relaxed) < 0;
        if is_baby {
            entity.set_synced_data(pumpkin_data::tracked_data::rabbit::DATA_BABY_ID, true);
        }
        entity.set_synced_data(
            pumpkin_data::tracked_data::rabbit::DATA_TYPE_ID,
            VarInt(self.get_variant().id()),
        );
    }

    fn mob_interact(&self, player: &Arc<Player>, item_stack: &mut ItemStack) -> bool {
        self.animal_interact(player, item_stack, Sound::EntityRabbitAmbient)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::living::test_support::armor_test_world;
    use pumpkin_data::attributes::Attributes;
    use pumpkin_util::math::vector3::Vector3;

    #[tokio::test]
    async fn killer_bunny_attributes_follow_variant_changes() -> Result<(), std::io::Error> {
        let directory = tempfile::tempdir()?;
        let rabbit = RabbitEntity::new(Entity::new(
            armor_test_world(directory.path()),
            Vector3::new(8.0, 64.0, 8.0),
            &EntityType::RABBIT,
        ));
        let living = &rabbit.mob_entity.living_entity;
        rabbit.set_variant(RabbitVariant::Evil);
        assert_eq!(living.get_attribute_value(&Attributes::ATTACK_DAMAGE), 8.0);
        assert_eq!(living.get_attribute_value(&Attributes::ARMOR), 8.0);
        rabbit.set_variant(RabbitVariant::Evil);
        assert_eq!(living.get_attribute_value(&Attributes::ATTACK_DAMAGE), 8.0);
        rabbit.set_variant(RabbitVariant::Brown);
        assert_eq!(living.get_attribute_value(&Attributes::ATTACK_DAMAGE), 3.0);
        // Rabbit.setVariant removes the evil damage modifier but leaves the armor base.
        assert_eq!(living.get_attribute_value(&Attributes::ARMOR), 8.0);
        Ok(())
    }
}
