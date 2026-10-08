//! `Rabbit.RabbitPanicGoal`, `RabbitAvoidEntityGoal` and `RaidGardenGoal`.
use super::rabbit::{FLEE_SPEED_MOD, MORE_CARROTS_DELAY, RabbitEntity, RabbitVariant};
use crate::{
    entity::{
        ai::goal::{
            Controls, Goal, ParentHandle,
            avoid_entity::AvoidEntityGoal,
            escape_danger::EscapeDangerGoal,
            move_to_target_pos::{MoveToTargetPos, MoveToTargetPosGoal},
        },
        mob::Mob,
    },
    world::World,
};
use pumpkin_data::{
    Block,
    block_properties::WheatLikeProperties,
    tag::{self, Taggable},
};
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering::Relaxed},
};

// CarrotBlock inherits CropBlock.MAX_AGE in vanilla 26.3.
const MAX_CARROT_AGE: u8 = 7;

pub struct RabbitPanicGoal {
    inner: Box<EscapeDangerGoal>,
}

/// Rabbit.registerGoals installs `ClimbOnTopOfPowderSnowGoal` alongside `FloatGoal`.
pub struct RabbitPowderSnowGoal;

impl Goal for RabbitPowderSnowGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        // ClimbOnTopOfPowderSnowGoal.canUse.
        let entity = mob.get_entity();
        if !(entity.was_in_powder_snow.load(Relaxed) || entity.is_in_powder_snow.load(Relaxed))
            || !entity
                .entity_type
                .has_tag(&tag::EntityType::MINECRAFT_POWDER_SNOW_WALKABLE_MOBS)
        {
            return false;
        }
        let above = entity.block_pos.load().up();
        let (block, state) = entity.world.load().get_block_and_state(&above);
        block == &Block::POWDER_SNOW || state.get_block_collision_shapes_at(&above).next().is_none()
    }

    fn tick(&mut self, mob: &dyn Mob) {
        mob.get_mob_entity()
            .jump_control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .jump();
    }

    fn should_run_every_tick(&self) -> bool {
        true
    }

    fn controls(&self) -> Controls {
        Controls::JUMP
    }
}
impl RabbitPanicGoal {
    pub fn new() -> Self {
        Self {
            inner: EscapeDangerGoal::new(FLEE_SPEED_MOD),
        }
    }
}
impl Goal for RabbitPanicGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        self.inner.can_start(mob)
    }
    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.inner.should_continue(mob)
    }
    fn start(&mut self, mob: &dyn Mob) {
        self.inner.start(mob);
    }
    fn stop(&mut self, mob: &dyn Mob) {
        self.inner.stop(mob);
    }
    fn tick(&mut self, mob: &dyn Mob) {
        self.inner.tick(mob);
        let m = mob.get_mob_entity();
        m.navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .set_speed(FLEE_SPEED_MOD);
        let mut control = m
            .move_control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (wanted, _) = control.wanted_position();
        control.set_wanted_position(wanted.x, wanted.y, wanted.z, FLEE_SPEED_MOD);
    }
    fn controls(&self) -> Controls {
        self.inner.controls()
    }
}

pub struct RabbitAvoidEntityGoal {
    inner: Option<AvoidEntityGoal>,
    monsters: bool,
}
impl RabbitAvoidEntityGoal {
    pub const fn new(inner: AvoidEntityGoal) -> Self {
        Self {
            inner: Some(inner),
            monsters: false,
        }
    }
}
impl RabbitAvoidEntityGoal {
    pub const fn monsters() -> Self {
        Self {
            inner: None,
            monsters: true,
        }
    }
}
impl Goal for RabbitAvoidEntityGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        if !mob
            .cast_any()
            .downcast_ref::<RabbitEntity>()
            .is_some_and(|r| r.get_variant() != RabbitVariant::Evil)
        {
            return false;
        }
        if self.monsters {
            let target = AvoidEntityGoal::find_threat(mob, 4.0, |candidate| {
                is_monster(candidate.get_entity().entity_type)
            });
            self.inner = target.map(|target| {
                AvoidEntityGoal::new(
                    target.get_entity().entity_type,
                    4.0,
                    FLEE_SPEED_MOD,
                    FLEE_SPEED_MOD,
                )
            });
        }
        self.inner.as_mut().is_some_and(|goal| goal.can_start(mob))
    }
    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.inner
            .as_mut()
            .is_some_and(|goal| goal.should_continue(mob))
    }
    fn start(&mut self, mob: &dyn Mob) {
        if let Some(goal) = &mut self.inner {
            goal.start(mob);
        }
    }
    fn stop(&mut self, mob: &dyn Mob) {
        if let Some(goal) = &mut self.inner {
            goal.stop(mob);
        }
    }
    fn tick(&mut self, mob: &dyn Mob) {
        if let Some(goal) = &mut self.inner {
            goal.tick(mob);
        }
    }
    fn controls(&self) -> Controls {
        Controls::MOVE
    }
}

// Rabbit.registerGoals tests Monster.class, which differs from MobCategory.MONSTER
// (ghasts, phantoms, hoglins, slimes and shulkers are not subclasses of Monster).
fn is_monster(kind: &pumpkin_data::entity::EntityType) -> bool {
    matches!(
        kind.resource_name,
        "blaze"
            | "breeze"
            | "creaking"
            | "creeper"
            | "enderman"
            | "endermite"
            | "giant"
            | "guardian"
            | "elder_guardian"
            | "silverfish"
            | "spider"
            | "cave_spider"
            | "vex"
            | "warden"
            | "wither"
            | "zoglin"
            | "piglin"
            | "piglin_brute"
            | "zombie"
            | "husk"
            | "drowned"
            | "zombie_villager"
            | "zombified_piglin"
            | "skeleton"
            | "stray"
            | "bogged"
            | "parched"
            | "wither_skeleton"
            | "pillager"
            | "vindicator"
            | "evoker"
            | "illusioner"
            | "witch"
            | "ravager"
    )
}

pub struct RaidGardenGoal {
    inner: MoveToTargetPosGoal<Self>,
    wants_to_raid: AtomicBool,
    can_raid: AtomicBool,
}
impl RaidGardenGoal {
    #[expect(
        clippy::unnecessary_box_returns,
        reason = "ParentHandle requires a stable heap address"
    )]
    pub fn new() -> Box<Self> {
        let mut goal = Box::new(Self {
            inner: MoveToTargetPosGoal::with_default(ParentHandle::none(), f64::from(0.7f32), 16),
            wants_to_raid: AtomicBool::new(false),
            can_raid: AtomicBool::new(false),
        });
        // SAFETY: the boxed goal has a stable address for the lifetime of its child.
        goal.inner.move_to_target_pos = unsafe { ParentHandle::new(&goal) };
        goal
    }
}
impl MoveToTargetPos for RaidGardenGoal {
    fn is_target_pos(&self, world: Arc<World>, pos: BlockPos) -> bool {
        if !world
            .get_block(&pos)
            .has_tag(&tag::Block::MINECRAFT_SUPPORTS_CROPS)
            || !self.wants_to_raid.load(Relaxed)
            || self.can_raid.load(Relaxed)
        {
            return false;
        }
        let (block, state) = world.get_block_and_state_id(&pos.up());
        let valid = block == &Block::CARROTS
            && WheatLikeProperties::from_state_id(state).age == MAX_CARROT_AGE;
        if valid {
            self.can_raid.store(true, Relaxed);
        }
        valid
    }
}
impl Goal for RaidGardenGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let Some(rabbit) = mob.cast_any().downcast_ref::<RabbitEntity>() else {
            return false;
        };
        if self.inner.cooldown <= 0 {
            if !mob
                .get_entity()
                .world
                .load()
                .level_info
                .load()
                .game_rules
                .mob_griefing
            {
                return false;
            }
            self.can_raid.store(false, Relaxed);
            self.wants_to_raid
                .store(rabbit.more_carrot_ticks.load(Relaxed) <= 0, Relaxed);
        }
        self.inner.can_start(mob)
    }
    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.can_raid.load(Relaxed) && self.inner.should_continue(mob)
    }
    fn start(&mut self, mob: &dyn Mob) {
        self.inner.start(mob);
    }
    fn tick(&mut self, mob: &dyn Mob) {
        self.inner.tick(mob);
        let pos = self.inner.target_pos;
        mob.get_mob_entity()
            .look_control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .look_at_with_range(
                f64::from(pos.0.x) + 0.5,
                f64::from(pos.0.y) + 1.0,
                f64::from(pos.0.z) + 0.5,
                10.0,
                mob.get_max_look_pitch_change(),
            );
        if !self.inner.is_reached_target() {
            return;
        }
        let Some(rabbit) = mob.cast_any().downcast_ref::<RabbitEntity>() else {
            return;
        };
        let world = mob.get_entity().world.load();
        let crops = pos.up();
        let (block, state) = world.get_block_and_state_id(&crops);
        if self.can_raid.load(Relaxed)
            && block == &Block::CARROTS
            && world.level_info.load().game_rules.mob_griefing
        {
            let mut properties = WheatLikeProperties::from_state_id(state);
            let age = properties.age;
            let next = if age == 0 {
                Block::AIR.default_state.id
            } else {
                properties.age -= 1;
                properties.to_state_id(block)
            };
            let new_state = if age == 0 {
                "minecraft:air".into()
            } else {
                format!("minecraft:carrots[age={}]", age - 1)
            };
            let mut event = crate::plugin::api::events::entity::entity_change_block::EntityChangeBlockEvent::new(
                mob.get_entity().entity_id,
                crops,
                new_state,
            );
            if let Some(server) = world.server.upgrade() {
                server.plugin_manager.fire_blocking(&server, &mut event);
            }
            if !event.cancelled {
                world.set_block_state(&crops, next, BlockFlags::NOTIFY_LISTENERS);
                if age > 0 {
                    world.emit_game_event("minecraft:block_change", crops.to_f64());
                    world.sync_world_event(
                        pumpkin_data::world::WorldEvent::ParticlesDestroyBlock,
                        crops,
                        i32::from(state.as_u16()),
                    );
                }
                rabbit.more_carrot_ticks.store(MORE_CARROTS_DELAY, Relaxed);
            }
        }
        self.can_raid.store(false, Relaxed);
        self.inner.cooldown = 10;
    }
    fn should_run_every_tick(&self) -> bool {
        self.inner.should_run_every_tick()
    }
    fn controls(&self) -> Controls {
        self.inner.controls()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        entity::Entity,
        plugin::{
            BoxFuture, EventHandler, EventPriority,
            api::events::entity::entity_change_block::EntityChangeBlockEvent,
        },
        server::{Server, combat_test_support},
    };
    use pumpkin_data::entity::EntityType;
    use pumpkin_util::math::{vector2::Vector2, vector3::Vector3};
    use pumpkin_world::chunk::ChunkData;

    struct CropHandler(AtomicBool);

    #[tokio::test]
    async fn rabbit_review_home_before_crop_validation() {
        let directory = tempfile::tempdir().unwrap();
        let world = crate::entity::living::test_support::armor_test_world(directory.path());
        let chunk = ChunkData::empty_sync(0, 0);
        let excluded = BlockPos::new(4, 63, 4);
        let eligible = BlockPos::new(5, 63, 4);
        for soil in [excluded, eligible] {
            chunk.set_block_absolute_y(
                soil.0.x as usize,
                63,
                soil.0.z as usize,
                Block::FARMLAND.default_state.id,
            );
            chunk.set_block_absolute_y(
                soil.0.x as usize,
                64,
                soil.0.z as usize,
                WheatLikeProperties { age: 7 }.to_state_id(&Block::CARROTS),
            );
        }
        world.level.loaded_chunks.insert(Vector2::new(0, 0), chunk);
        let rabbit = RabbitEntity::new(Entity::new(
            world,
            Vector3::new(4.5, 64.0, 4.5),
            &EntityType::RABBIT,
        ));
        rabbit
            .mob_entity
            .position_target
            .store(BlockPos::new(6, 63, 4));
        rabbit.mob_entity.position_target_range.store(2, Relaxed);
        assert!(!rabbit.mob_entity.is_in_position_target_range_pos(&excluded));
        let mut goal = RaidGardenGoal::new();
        assert!(goal.can_start(rabbit.as_ref()));
        assert_eq!(goal.inner.target_pos, eligible);
    }

    impl EventHandler<EntityChangeBlockEvent> for CropHandler {
        fn handle_blocking<'a>(
            &'a self,
            _: &'a Arc<Server>,
            event: &'a mut EntityChangeBlockEvent,
        ) -> BoxFuture<'a, ()> {
            assert_eq!(event.new_block, "minecraft:carrots[age=6]");
            event.cancelled = self.0.load(Relaxed);
            Box::pin(async {})
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancelled_carrot_raid_preserves_crop_and_appetite() -> Result<(), std::io::Error> {
        let directory = tempfile::tempdir()?;
        let server = combat_test_support::server(directory.path());
        let world = combat_test_support::world(&server, directory.path());
        let chunk = ChunkData::empty_sync(0, 0);
        let soil = BlockPos::new(4, 63, 4);
        let mature = WheatLikeProperties { age: 7 }.to_state_id(&Block::CARROTS);
        chunk.set_block_absolute_y(4, 63, 4, Block::FARMLAND.default_state.id);
        chunk.set_block_absolute_y(4, 64, 4, mature);
        world.level.loaded_chunks.insert(Vector2::new(0, 0), chunk);
        let rabbit = RabbitEntity::new(Entity::new(
            world.clone(),
            Vector3::new(4.5, 64.0, 4.5),
            &EntityType::RABBIT,
        ));
        let handler = Arc::new(CropHandler(AtomicBool::new(true)));
        server.plugin_manager.register::<EntityChangeBlockEvent, _>(
            handler.clone(),
            EventPriority::Normal,
            true,
        );
        let mut goal = RaidGardenGoal::new();
        goal.inner.target_pos = soil;
        goal.wants_to_raid.store(true, Relaxed);
        assert!(goal.is_target_pos(world.clone(), soil));
        goal.tick(rabbit.as_ref());
        assert_eq!(world.get_block_state_id(&soil.up()), mature);
        assert_eq!(rabbit.more_carrot_ticks.load(Relaxed), 0);

        handler.0.store(false, Relaxed);
        assert!(goal.is_target_pos(world.clone(), soil));
        goal.tick(rabbit.as_ref());
        assert_eq!(
            WheatLikeProperties::from_state_id(world.get_block_state_id(&soil.up())).age,
            6
        );
        assert_eq!(rabbit.more_carrot_ticks.load(Relaxed), 40);
        assert!(!goal.is_target_pos(world, soil));
        Ok(())
    }
}
