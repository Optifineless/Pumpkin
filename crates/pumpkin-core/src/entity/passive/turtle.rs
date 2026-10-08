use crate::entity::ai::{control::turtle_move_control::TurtleMoveControl, pathfinder::Navigator};
use crossbeam::atomic::AtomicCell;
use pumpkin_util::math::position::BlockPos;
use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, Ordering},
};

use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::Sound;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_nbt::compound::NbtCompound;

use crate::entity::{
    Entity, EntityBase,
    ageable::{AgeableData, AgeableMob},
    ai::goal::{
        breed::BreedGoal, escape_danger::EscapeDangerGoal, look_around::RandomLookAroundGoal,
        look_at_entity::LookAtEntityGoal, tempt::TemptGoal, try_find_water::TryFindWaterGoal,
        wander_around::WanderAroundGoal,
    },
    mob::{Mob, MobEntity},
    passive::animal::Animal,
    player::Player,
};

const TEMPT_ITEMS: &[&Item] = &[&Item::SEAGRASS];

pub struct TurtleEntity {
    pub mob_entity: MobEntity,
    pub ageable_data: AgeableData,
    /// Spawn beach, restored from NBT on load.
    pub home_pos: AtomicCell<BlockPos>,
    /// Stub for `TurtleGoHomeGoal`, not yet registered; suppresses sinking near home.
    pub going_home: AtomicBool,
    /// Stub for `TurtleTravelGoal`, not yet registered; restricts destinations to water.
    pub travelling: Arc<AtomicBool>,
    pub has_egg: AtomicBool,
    pub laying_egg: AtomicBool,
}

impl TurtleEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        // Turtle.createNavigation uses AmphibiousPathNavigation with a water-only travel target.
        let travelling = Arc::new(AtomicBool::new(false));
        let navigation = Navigator::turtle(travelling.clone());
        mob_entity.configure_movement(navigation, TurtleMoveControl::default());
        // Turtle.finalizeSpawn supplies the home; direct construction also avoids an unset sentinel.
        let home = mob_entity.living_entity.entity.block_pos.load();
        let turtle = Self {
            mob_entity,
            ageable_data: AgeableData::default(),
            home_pos: AtomicCell::new(home),
            going_home: AtomicBool::new(false),
            travelling,
            has_egg: AtomicBool::new(false),
            laying_egg: AtomicBool::new(false),
        };
        let mob_arc = Arc::new(turtle);
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

            goal_selector.add_goal(0, Box::new(TryFindWaterGoal));

            goal_selector.add_goal(2, EscapeDangerGoal::new(1.2));
            goal_selector.add_goal(3, BreedGoal::new(1.0));
            goal_selector.add_goal(4, Box::new(TemptGoal::new(1.1, TEMPT_ITEMS, false)));
            goal_selector.add_goal(5, Box::new(WanderAroundGoal::new(1.0)));
            goal_selector.add_goal(
                6,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 6.0),
            );
            goal_selector.add_goal(7, Box::new(RandomLookAroundGoal::default()));
        };

        mob_arc
    }

    /// `TurtleMoveControl` / `travelInWater` home-distance predicate.
    pub fn close_to_home(&self, distance: f64) -> bool {
        let home = self.home_pos.load().0;
        let center = pumpkin_util::math::vector3::Vector3::new(
            f64::from(home.x) + 0.5,
            f64::from(home.y) + 0.5,
            f64::from(home.z) + 0.5,
        );
        (self.get_entity().pos.load() - center).length_squared() < distance * distance
    }

    #[must_use]
    pub fn has_egg(&self) -> bool {
        self.has_egg.load(Ordering::Relaxed)
    }

    pub fn set_has_egg(&self, has_egg: bool) {
        self.has_egg.store(has_egg, Ordering::Relaxed);
        let entity = self.get_entity();
        entity.set_synced_data(pumpkin_data::tracked_data::turtle::HAS_EGG, has_egg);
    }

    #[must_use]
    pub fn is_laying_egg(&self) -> bool {
        self.laying_egg.load(Ordering::Relaxed)
    }

    pub fn set_laying_egg(&self, laying_egg: bool) {
        self.laying_egg.store(laying_egg, Ordering::Relaxed);
        let entity = self.get_entity();
        entity.set_synced_data(pumpkin_data::tracked_data::turtle::LAYING_EGG, laying_egg);
    }
}

impl AgeableMob for TurtleEntity {
    fn get_ageable_data(&self) -> &AgeableData {
        &self.ageable_data
    }
}

impl Animal for TurtleEntity {
    fn is_food(&self, item_stack: &ItemStack) -> bool {
        item_stack.item.has_tag(&tag::Item::MINECRAFT_TURTLE_FOOD)
            || item_stack.item == &Item::SEAGRASS
    }
}

impl Mob for TurtleEntity {
    fn finalize_spawn(
        &self,
        world: &Arc<crate::world::World>,
        view: &crate::world::spawn_view::SpawnView<'_>,
        group: Option<crate::entity::mob::spawn::SpawnGroupData>,
    ) -> Option<crate::entity::mob::spawn::SpawnGroupData> {
        // Turtle.finalizeSpawn sets the home before AgeableMob.finalizeSpawn.
        self.home_pos.store(self.get_entity().block_pos.load());
        crate::entity::mob::movement::finalize_spawn_after_species(self, world, view, group)
    }

    fn mob_is_pushed_by_fluids(&self) -> bool {
        false
    }

    // Turtle.travelInWater.
    fn custom_travel(&self, caller: &dyn EntityBase) -> bool {
        let sink = self.mob_entity.get_target().is_none()
            && (!self.going_home.load(Ordering::Relaxed) || !self.close_to_home(20.0));
        crate::entity::mob::movement::travel_in_water(self, caller, 0.1, sink)
    }

    fn as_ageable(&self) -> Option<&dyn AgeableMob> {
        Some(self)
    }

    fn as_animal(&self) -> Option<&dyn Animal> {
        Some(self)
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        self.write_ageable_nbt(nbt);
        nbt.put_bool("has_egg", self.has_egg());
        // Turtle.addAdditionalSaveData: BlockPos.CODEC is an int array.
        let home = self.home_pos.load().0;
        nbt.put(
            "home_pos",
            pumpkin_nbt::tag::NbtTag::IntArray(vec![home.x, home.y, home.z]),
        );
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.read_ageable_nbt(nbt);
        // Turtle.readAdditionalSaveData defaults missing homes to the current position.
        let home = match nbt.get_int_array("home_pos") {
            Some([x, y, z]) => BlockPos::new(*x, *y, *z),
            _ => self.get_entity().block_pos.load(),
        };
        self.home_pos.store(home);
        // Turtle.readAdditionalSaveData; accept Pumpkin's old key for existing saves.
        self.set_has_egg(
            nbt.get_bool("has_egg")
                .or_else(|| nbt.get_bool("HasEgg"))
                .unwrap_or(false),
        );
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
            entity.set_synced_data(pumpkin_data::tracked_data::turtle::DATA_BABY_ID, true);
        }
        entity.set_synced_data(pumpkin_data::tracked_data::turtle::HAS_EGG, self.has_egg());
        entity.set_synced_data(
            pumpkin_data::tracked_data::turtle::LAYING_EGG,
            self.is_laying_egg(),
        );
    }

    fn mob_interact(&self, player: &Arc<Player>, item_stack: &mut ItemStack) -> bool {
        self.animal_interact(player, item_stack, Sound::EntityTurtleAmbientLand)
    }
}
