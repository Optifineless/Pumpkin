use super::dolphin_movement::{DolphinBreathAirGoal, DolphinJumpGoal, MAX_AIR_SUPPLY};
use crate::entity::ai::control::smooth_swimming_look_control::SmoothSwimmingLookControl;
use crate::entity::ai::{
    control::smooth_swimming_move_control::SmoothSwimmingMoveControl, pathfinder::Navigator,
};
use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, AtomicI32, Ordering},
};

use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::particle::Particle;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::vector3::Vector3;

use crate::entity::{
    Entity, EntityBase,
    ai::goal::{
        escape_danger::EscapeDangerGoal, look_around::RandomLookAroundGoal,
        look_at_entity::LookAtEntityGoal, melee_attack::MeleeAttackGoal, revenge::RevengeGoal,
        tempt::TemptGoal, try_find_water::TryFindWaterGoal, wander_around::WanderAroundGoal,
    },
    mob::{Mob, MobEntity},
    player::Player,
};

const TEMPT_ITEMS: &[&Item] = &[&Item::COD, &Item::SALMON, &Item::TROPICAL_FISH];

pub struct DolphinEntity {
    pub mob_entity: MobEntity,
    pub air_supply: AtomicI32,
    pub got_fish: AtomicBool,
    pub moistness_level: AtomicI32,
}

impl DolphinEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        // Dolphin constructor / createNavigation (26.3).
        mob_entity.configure_movement(
            Navigator::water_bound(true),
            SmoothSwimmingMoveControl::new(85, 10, 0.02, 0.1, true),
        );
        mob_entity.configure_look(SmoothSwimmingLookControl::new(10));

        let dolphin = Self {
            mob_entity,
            air_supply: AtomicI32::new(MAX_AIR_SUPPLY),
            got_fish: AtomicBool::new(false),
            moistness_level: AtomicI32::new(2400),
        };
        let mob_arc = Arc::new(dolphin);
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

            goal_selector.add_goal(0, Box::new(DolphinBreathAirGoal));
            goal_selector.add_goal(0, Box::new(TryFindWaterGoal));
            goal_selector.add_goal(5, Box::new(DolphinJumpGoal::default()));

            goal_selector.add_goal(1, EscapeDangerGoal::new(1.6));
            goal_selector.add_goal(2, Box::new(MeleeAttackGoal::new(1.2, true)));
            goal_selector.add_goal(3, Box::new(TemptGoal::new(1.2, TEMPT_ITEMS, false)));
            // Dolphin.registerGoals: RandomSwimmingGoal(this, 1.0, 10).
            goal_selector.add_goal(4, Box::new(WanderAroundGoal::swimming(1.0, 10)));
            goal_selector.add_goal(
                5,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 6.0),
            );
            goal_selector.add_goal(6, Box::new(RandomLookAroundGoal::default()));
        };

        {
            let mut target_selector = mob_arc
                .mob_entity
                .target_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);

            target_selector.add_goal(1, Box::new(RevengeGoal::new(true)));
        };

        mob_arc
    }

    #[must_use]
    pub fn got_fish(&self) -> bool {
        self.got_fish.load(Ordering::Relaxed)
    }

    pub fn set_got_fish(&self, val: bool) {
        self.got_fish.store(val, Ordering::Relaxed);
    }
}

impl Mob for DolphinEntity {
    fn mob_init_data_tracker(&self) {
        // Entity.defineSynchedData uses Dolphin.getMaxAirSupply, including before the first tick.
        self.get_entity().set_synced_data(
            pumpkin_data::tracked_data::entity::DATA_AIR_SUPPLY_ID,
            pumpkin_protocol::codec::var_int::VarInt(self.air_supply.load(Ordering::Relaxed)),
        );
    }

    // Dolphin.getMaxHeadYRot.
    fn get_max_head_rotation(&self) -> f32 {
        1.0
    }

    // Dolphin.getMaxHeadXRot.
    fn get_max_look_pitch_change(&self) -> f32 {
        1.0
    }

    // Dolphin.tick performs dry-land flopping after superclass movement.
    fn post_tick(&self) {
        if self.mob_entity.is_no_ai() {
            return;
        }
        let entity = self.get_entity();
        let world = entity.world.load();
        // Entity.isInRain, used by Dolphin.aiStep's isInWaterOrRain.
        let feet = entity.block_pos.load();
        let head = pumpkin_util::math::position::BlockPos::new(
            feet.0.x,
            entity.bounding_box.load().max.y.floor() as i32,
            feet.0.z,
        );
        if entity.touching_water.load(Ordering::Relaxed)
            || world.is_raining_at(&feet)
            || world.is_raining_at(&head)
        {
            self.moistness_level.store(2400, Ordering::Relaxed);
        } else {
            if self.moistness_level.fetch_sub(1, Ordering::Relaxed) <= 1 {
                self.damage(self, 1.0, pumpkin_data::damage::DamageType::DRY_OUT);
            }
            crate::entity::mob::movement::flop_on_land(self, 0.2);
        }
    }

    fn mob_tick(&self, _caller: &dyn EntityBase) {
        self.tick_air_supply();
    }

    fn mob_is_pushed_by_fluids(&self) -> bool {
        false
    }

    // Dolphin.travelInWater.
    fn custom_travel(&self, caller: &dyn EntityBase) -> bool {
        crate::entity::mob::movement::travel_in_water(
            self,
            caller,
            self.mob_entity.movement_speed.load(),
            self.mob_entity.get_target().is_none(),
        )
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put_bool("GotFish", self.got_fish());
        nbt.put_short("Air", self.air_supply.load(Ordering::Relaxed) as i16);
        nbt.put_int("Moistness", self.moistness_level.load(Ordering::Relaxed));
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.air_supply.store(
            nbt.get_short("Air")
                .map_or(super::dolphin_movement::MAX_AIR_SUPPLY, i32::from),
            Ordering::Relaxed,
        );
        if let Some(got_fish) = nbt.get_bool("GotFish") {
            self.set_got_fish(got_fish);
        }
        if let Some(moistness) = nbt.get_int("Moistness") {
            self.moistness_level.store(moistness, Ordering::Relaxed);
        }
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn mob_interact(&self, player: &Arc<Player>, item_stack: &mut ItemStack) -> bool {
        let item = item_stack.get_item();
        if TEMPT_ITEMS.iter().any(|i| i.id == item.id) {
            item_stack.decrement_unless_creative(player.gamemode.load(), 1);
            self.set_got_fish(true);
            self.mob_entity.living_entity.heal(2.0);

            let entity = self.get_entity();
            let world = entity.world.load();
            let pos = entity.pos.load();
            world.spawn_particle(
                pos + Vector3::new(0.0, f64::from(entity.height()), 0.0),
                Vector3::new(0.5, 0.5, 0.5),
                1.0,
                7,
                Particle::HappyVillager,
            );
            world.play_sound(Sound::EntityDolphinEat, SoundCategory::Neutral, &pos);
            return true;
        }

        false
    }
}
