use pumpkin_data::damage::DamageType;
use pumpkin_data::entity::EntityType;
use pumpkin_data::tag::Taggable;
use pumpkin_data::{Block, BlockStateId, tag};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_world::{chunk::ChunkHeightmapType, world::BlockFlags};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use crate::entity::{
    Entity, EntityBase,
    living::LivingEntity,
    mob::{Mob, MobEntity},
    player::Player,
};

fn body_damage(amount: f32) -> f32 {
    amount / 4.0 + amount.min(1.0)
}

mod collision;
#[cfg(test)]
mod collision_tests;
pub mod flight_history;
mod parts;
pub mod phase;

use flight_history::DragonFlightHistory;
use phase::{EnderDragonPhase, PhaseManager};

pub const NODE_COUNT: usize = 24;
// EnderDragon.subEntities: head, neck, body, three tails and two wings (Java line 101).
pub const PART_COUNT: i32 = 8;
pub const NODE_Y: i32 = 105;
pub const NODE_REACH_SQ: f64 = 64.0;
pub const DEATH_TIMER_MAX: i32 = 200;
const GROWL_COOLDOWN_BASE: i32 = 200;
const FLAP_SPEED: f32 = 0.2;
pub const FLY_ACCEL: f64 = 0.1;
pub const FLY_SPEED: f64 = 0.6;
const ARENA_RADIUS: f64 = 192.0;

const NODE_ADJACENCY: [u32; NODE_COUNT] = [
    6146, 8197, 8202, 16404, 32808, 32848, 65696, 131392, 131712, 263424, 526848, 525313, 1581057,
    3166214, 2138120, 6373424, 4358208, 12910976, 9044480, 9706496, 15216640, 13688832, 11763712,
    8257536,
];

#[derive(Clone, Copy, Default, Debug)]
pub struct DragonNode {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl DragonNode {
    #[must_use]
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    #[inline]
    #[must_use]
    pub fn dist_sq(&self, other: &Self) -> f64 {
        let (dx, dy, dz) = (self.x - other.x, self.y - other.y, self.z - other.z);
        dx * dx + dy * dy + dz * dz
    }

    #[inline]
    #[must_use]
    pub fn dist_sq_vec(&self, v: Vector3<f64>) -> f64 {
        let (dx, dy, dz) = (self.x - v.x, self.y - v.y, self.z - v.z);
        dx * dx + dy * dy + dz * dz
    }

    #[inline]
    #[must_use]
    pub const fn as_vec3(&self) -> Vector3<f64> {
        Vector3::new(self.x, self.y, self.z)
    }
}

#[must_use]
pub fn find_path(
    nodes: &[Option<DragonNode>; NODE_COUNT],
    from: usize,
    to: usize,
    final_node: Option<DragonNode>,
) -> Vec<usize> {
    if from == to && final_node.is_none() {
        return vec![];
    }

    let mut g = [f64::MAX; NODE_COUNT];
    let mut came_from = [NODE_COUNT; NODE_COUNT];
    let mut closed = [false; NODE_COUNT];
    let mut open: std::collections::BinaryHeap<(ordered_float::OrderedFloat<f64>, usize)> =
        std::collections::BinaryHeap::new();

    let h = |n: usize| -> f64 {
        if let Some(target) = final_node
            && let Some(node) = nodes[n]
        {
            return node.dist_sq(&target).sqrt();
        }
        match (nodes[n], nodes[to]) {
            (Some(a), Some(b)) => a.dist_sq(&b).sqrt(),
            _ => 0.0,
        }
    };

    g[from] = 0.0;
    open.push((ordered_float::OrderedFloat(-h(from)), from));

    'outer: while let Some((_, cur)) = open.pop() {
        if closed[cur] {
            continue;
        }
        if cur == to {
            break 'outer;
        }
        closed[cur] = true;

        let adj = NODE_ADJACENCY[cur];
        for nbr in 0..NODE_COUNT {
            if (adj >> nbr) & 1 == 0 || closed[nbr] {
                continue;
            }
            let (Some(cn), Some(nn)) = (nodes[cur], nodes[nbr]) else {
                continue;
            };
            let ng = g[cur] + cn.dist_sq(&nn).sqrt();
            if ng < g[nbr] {
                g[nbr] = ng;
                came_from[nbr] = cur;
                open.push((ordered_float::OrderedFloat(-(ng + h(nbr))), nbr));
            }
        }
    }

    let mut path = Vec::new();
    let mut cur = to;
    while cur != NODE_COUNT {
        path.push(cur);
        cur = came_from[cur];
    }
    path.reverse();
    if path.first() == Some(&from) {
        path.remove(0);
    }
    path
}

pub struct EnderDragonPart {
    pub entity: Entity,
    pub dragon_uuid: uuid::Uuid,
}

impl EnderDragonPart {
    pub const fn new(entity: Entity, dragon_uuid: uuid::Uuid) -> Self {
        Self {
            entity,
            dragon_uuid,
        }
    }
}

impl EntityBase for EnderDragonPart {
    fn get_entity(&self) -> &Entity {
        &self.entity
    }

    fn damage_with_context(
        &self,
        _caller: &dyn EntityBase,
        amount: f32,
        damage_type: DamageType,
        position: Option<Vector3<f64>>,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) -> bool {
        let world = self.entity.world.load();
        let Some(dragon) = world.get_entity_by_uuid(self.dragon_uuid) else {
            return false;
        };
        // EnderDragonPart.hurtServer -> EnderDragon.hurt(level, part, source, damage), lines 452-454.
        let head_or_neck = self.entity.entity_id <= dragon.get_entity().entity_id + 2;
        let damage = if head_or_neck {
            amount
        } else {
            body_damage(amount)
        };
        dragon.damage_with_context(
            dragon.as_ref(),
            damage,
            damage_type,
            position,
            source,
            cause,
        )
    }

    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }

    fn get_living_entity(&self) -> Option<&LivingEntity> {
        None
    }
    fn can_hit(&self) -> bool {
        true
    }
}

pub struct EnderDragonEntity {
    pub mob_entity: MobEntity,
    pub parts: Vec<Arc<EnderDragonPart>>,

    pub flight_history: Mutex<DragonFlightHistory>,
    pub flap_time: Mutex<f32>,
    pub o_flap_time: Mutex<f32>,

    pub in_wall: Mutex<bool>,
    pub dragon_death_time: Mutex<i32>,
    pub sitting_damage_received: Mutex<f32>,

    pub nodes_initialized: Mutex<bool>,

    pub fight_origin: Mutex<BlockPos>,

    pub phase_manager: PhaseManager,
    pub phase: Mutex<EnderDragonPhase>,
    pub growl_time: Mutex<i32>,
    pub yaw_rot_accel: Mutex<f32>,
    pub holding_pattern_clockwise: Mutex<bool>,
    pub target_location: Mutex<Option<Vector3<f64>>>,
    pub fireball_charge: Mutex<i32>,

    pub nodes: Mutex<[Option<DragonNode>; NODE_COUNT]>,
    pub path: Mutex<Vec<usize>>,
    pub target_node: Mutex<usize>,

    pub strafe_target: Mutex<Option<Vector3<f64>>>,
    pub target_player: Mutex<Option<uuid::Uuid>>,
    pub ticks_sitting: Mutex<i32>,
    pub sit_attack_timer: Mutex<i32>,
    pub breathing_timer: Mutex<i32>,
}

impl EnderDragonEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        entity.no_physics.store(true, Ordering::Relaxed);
        let base_id = entity.entity_id;
        let dragon_uuid = entity.entity_uuid;
        let world = entity.world.load();

        let mut parts = Vec::new();
        // EnderDragon.recreateFromPacket uses the eight IDs reserved by from_type.
        for (index, (width, height)) in parts::PART_SIZES.into_iter().enumerate() {
            let part_entity = Entity::from_uuid_with_id(
                base_id + index as i32 + 1,
                uuid::Uuid::new_v4(),
                world.clone(),
                Vector3::default(),
                &EntityType::ENDER_DRAGON,
            );
            parts::set_dimensions(&part_entity, width, height);
            let part = Arc::new(EnderDragonPart::new(part_entity, dragon_uuid));
            parts.push(part);
        }

        Arc::new(Self {
            mob_entity: MobEntity::new(entity),
            parts,
            flight_history: Mutex::new(DragonFlightHistory::default()),
            flap_time: Mutex::new(0.0),
            o_flap_time: Mutex::new(0.0),
            in_wall: Mutex::new(false),
            dragon_death_time: Mutex::new(0),
            sitting_damage_received: Mutex::new(0.0),
            nodes_initialized: Mutex::new(false),
            fight_origin: Mutex::new(BlockPos::new(0, 128, 0)),
            phase_manager: PhaseManager::new(),
            phase: Mutex::new(EnderDragonPhase::Circling),
            growl_time: Mutex::new(GROWL_COOLDOWN_BASE),
            yaw_rot_accel: Mutex::new(0.0),
            holding_pattern_clockwise: Mutex::new(true),
            target_location: Mutex::new(None),
            fireball_charge: Mutex::new(0),
            nodes: Mutex::new([None; NODE_COUNT]),
            path: Mutex::new(Vec::new()),
            target_node: Mutex::new(0),
            strafe_target: Mutex::new(None),
            target_player: Mutex::new(None),
            ticks_sitting: Mutex::new(0),
            sit_attack_timer: Mutex::new(0),
            breathing_timer: Mutex::new(0),
        })
    }

    pub fn set_fight_origin(&self, pos: BlockPos) {
        if let (Ok(mut initialized), Ok(mut origin)) = (
            self.nodes_initialized.try_lock(),
            self.fight_origin.try_lock(),
        ) && *origin != pos
        {
            *origin = pos;
            *initialized = false;
        }
    }

    pub fn set_phase(&self, phase_type: EnderDragonPhase) {
        let mut phase_lock = self
            .phase
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *phase_lock == phase_type {
            return;
        }

        let old_phase = self.phase_manager.get_phase(*phase_lock);
        old_phase.end(self);

        *phase_lock = phase_type;

        let new_phase = self.phase_manager.get_phase(phase_type);
        new_phase.begin(self);
    }

    fn ensure_nodes_initialized(&self) {
        let mut initialized = self
            .nodes_initialized
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *initialized {
            return;
        }

        let world = self.mob_entity.living_entity.entity.world.load();
        let fight_origin = self
            .fight_origin
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let mut nodes = self
            .nodes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for i in 0..NODE_COUNT {
            let mut y_adjustment = 5;
            let node_x;
            let node_z;

            if i < 12 {
                node_x = fight_origin.0.x
                    + (60.0 * ((i as f32) * std::f32::consts::TAU / 12.0).cos()) as i32;
                node_z = fight_origin.0.z
                    + (60.0 * ((i as f32) * std::f32::consts::TAU / 12.0).sin()) as i32;
            } else if i < 20 {
                let multiplier = i - 12;
                node_x = fight_origin.0.x
                    + (40.0 * ((multiplier as f32) * std::f32::consts::TAU / 8.0).cos()) as i32;
                node_z = fight_origin.0.z
                    + (40.0 * ((multiplier as f32) * std::f32::consts::TAU / 8.0).sin()) as i32;
                y_adjustment += 10;
            } else {
                let multiplier = i - 20;
                node_x = fight_origin.0.x
                    + (20.0 * ((multiplier as f32) * std::f32::consts::TAU / 4.0).cos()) as i32;
                node_z = fight_origin.0.z
                    + (20.0 * ((multiplier as f32) * std::f32::consts::TAU / 4.0).sin()) as i32;
            }

            let height =
                world.get_heightmap_height(ChunkHeightmapType::MotionBlocking, node_x, node_z);
            let node_y = 73.max(height + y_adjustment);
            nodes[i] = Some(DragonNode::new(node_x as f64, node_y as f64, node_z as f64));
        }

        let pos = self.mob_entity.living_entity.entity.pos.load();
        let nearest = Self::nearest_node_in(&nodes, pos);
        let dest = Self::random_node_idx();
        let new_path = find_path(&nodes, nearest, dest, None);
        drop(nodes);

        *self
            .target_node
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = dest;
        *self
            .path
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = new_path;
        *initialized = true;
    }

    pub fn find_closest_node(&self) -> usize {
        let pos = self.mob_entity.living_entity.entity.pos.load();
        self.find_closest_node_to(pos)
    }

    pub fn find_closest_node_to(&self, pos: Vector3<f64>) -> usize {
        self.ensure_nodes_initialized();
        let nodes = self
            .nodes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Self::nearest_node_in(&nodes, pos)
    }

    fn nearest_node_in(nodes: &[Option<DragonNode>; NODE_COUNT], pos: Vector3<f64>) -> usize {
        nodes
            .iter()
            .enumerate()
            .filter_map(|(i, n)| n.map(|n| (i, n.dist_sq_vec(pos))))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map_or(0, |(i, _)| i)
    }

    fn random_node_idx() -> usize {
        rand::random_range(0..NODE_COUNT)
    }

    #[inline]
    #[must_use]
    pub fn yaw_toward(from: Vector3<f64>, to: Vector3<f64>) -> f32 {
        (-(to.z - from.z)).atan2(to.x - from.x).to_degrees() as f32 - 90.0
    }

    pub fn move_relative(&self, speed: f32, relative_movement: Vector3<f64>) {
        let yaw = self.mob_entity.living_entity.entity.yaw.load();
        let movement = Self::get_relative_movement(relative_movement, speed, yaw);
        let entity = &self.mob_entity.living_entity.entity;
        let vel = entity.velocity.load();
        entity.velocity.store(vel + movement);
    }

    fn get_relative_movement(
        relative_movement: Vector3<f64>,
        speed: f32,
        yaw: f32,
    ) -> Vector3<f64> {
        let dist = relative_movement.length_squared();
        if dist < 1.0E-7 {
            return Vector3::new(0.0, 0.0, 0.0);
        }
        let vec = if dist > 1.0 {
            relative_movement.normalize()
        } else {
            relative_movement
        } * (speed as f64);
        let sin = (yaw * (std::f32::consts::PI / 180.0)).sin() as f64;
        let cos = (yaw * (std::f32::consts::PI / 180.0)).cos() as f64;
        Vector3::new(vec.x * cos - vec.z * sin, vec.y, vec.z * cos + vec.x * sin)
    }

    pub fn steer_toward(
        &self,
        pos: Vector3<f64>,
        target: Vector3<f64>,
        fly_speed: f32,
        turn_speed: f32,
    ) {
        let xdd = target.x - pos.x;
        let mut ydd = target.y - pos.y;
        let zdd = target.z - pos.z;
        let dist_sq = xdd * xdd + ydd * ydd + zdd * zdd;

        let horizontal_dist = xdd.hypot(zdd);
        if horizontal_dist > 0.0 {
            ydd = (ydd / horizontal_dist).clamp(-(fly_speed as f64), fly_speed as f64);
        }

        let entity = &self.mob_entity.living_entity.entity;
        let mut vel = entity.velocity.load();
        vel.y += ydd * 0.01;
        entity.velocity.store(vel);

        let yaw = entity.yaw.load();
        let aim_diff = target - pos;
        let aim = if aim_diff.length_squared() > 1e-6 {
            aim_diff.normalize()
        } else {
            Vector3::new(0.0, 0.0, 0.0)
        };

        let dir_vec = Vector3::new(
            (yaw * (std::f32::consts::PI / 180.0)).sin() as f64,
            vel.y,
            -(yaw * (std::f32::consts::PI / 180.0)).cos() as f64,
        );
        let dir = if dir_vec.length_squared() > 1e-6 {
            dir_vec.normalize()
        } else {
            Vector3::new(0.0, 0.0, 0.0)
        };

        let dot = (dir.dot(&aim) as f32 + 0.5).max(0.0) / 1.5;
        if xdd.abs() > 1.0E-5 || zdd.abs() > 1.0E-5 {
            let mut y_rot_d =
                (180.0 - (xdd.atan2(zdd).to_degrees() as f32) - yaw).rem_euclid(360.0);
            if y_rot_d > 180.0 {
                y_rot_d -= 360.0;
            }
            y_rot_d = y_rot_d.clamp(-50.0, 50.0);

            let mut y_rot_a = self
                .yaw_rot_accel
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *y_rot_a *= 0.8;
            *y_rot_a += y_rot_d * turn_speed;
            entity.yaw.store(yaw + *y_rot_a * 0.1);
        }

        let span = (2.0 / (dist_sq + 1.0)) as f32;
        self.move_relative(
            0.06 * (dot * span + (1.0 - span)),
            Vector3::new(0.0, 0.0, -1.0),
        );

        let actual_vel = entity.velocity.load();
        let actual_dir = if actual_vel.length_squared() > 1e-6 {
            actual_vel.normalize()
        } else {
            Vector3::new(0.0, 0.0, 0.0)
        };
        let slide = 0.8 + 0.15 * (actual_dir.dot(&dir) + 1.0) / 2.0;
        entity.velocity.store(Vector3::new(
            actual_vel.x * slide,
            actual_vel.y * 0.91,
            actual_vel.z * slide,
        ));
    }

    fn update_flap_time(&self) {
        let sitting = self
            .phase
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_sitting();
        let in_wall = *self
            .in_wall
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut flap = self
            .flap_time
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut o_flap = self
            .o_flap_time
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        *o_flap = *flap;
        *flap += if sitting {
            0.1
        } else if in_wall {
            FLAP_SPEED * 0.5
        } else {
            FLAP_SPEED
        };
        if *flap > std::f32::consts::TAU {
            *flap -= std::f32::consts::TAU;
        }
    }

    fn tick_growl(&self) {
        if self
            .phase
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_sitting()
        {
            return;
        }
        let mut t = self
            .growl_time
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *t > 0 {
            *t -= 1;
        } else {
            *t = GROWL_COOLDOWN_BASE + rand::random_range(0..100);
        }
    }

    fn find_nearest_player(&self) -> Option<Arc<Player>> {
        let world = self.mob_entity.living_entity.entity.world.load();
        let pos = self.mob_entity.living_entity.entity.pos.load();
        world
            .players
            .load()
            .iter()
            .filter(|p| {
                let p_pos = p.living_entity.entity.pos.load();
                p_pos.distance_squared(pos) < ARENA_RADIUS * ARENA_RADIUS
            })
            .min_by(|a, b| {
                let a_pos = a.living_entity.entity.pos.load();
                let b_pos = b.living_entity.entity.pos.load();
                a_pos
                    .distance_squared(pos)
                    .total_cmp(&b_pos.distance_squared(pos))
            })
            .cloned()
    }

    fn tick_crystal_healing(&self) {
        let world = self.mob_entity.living_entity.entity.world.load();
        let pos = self.mob_entity.living_entity.entity.pos.load();

        let mut nearest_crystal = None;
        let mut min_dist_sq = 1024.0; // 32 blocks

        for entity in world.entities.load().iter() {
            if entity.get_entity().entity_type == &EntityType::END_CRYSTAL {
                let crystal_pos = entity.get_entity().pos.load();
                let dist_sq = pos.distance_squared(crystal_pos);
                if dist_sq < min_dist_sq {
                    min_dist_sq = dist_sq;
                    nearest_crystal = Some(entity.clone());
                }
            }
        }

        if let Some(_crystal) = nearest_crystal {
            let living = &self.mob_entity.living_entity;
            if living.health.load() < living.get_max_health() {
                living.heal(1.0);
            }
        }
    }

    fn tick_block_breaking(&self) {
        let phase_type = *self
            .phase
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if phase_type.is_sitting() || phase_type == EnderDragonPhase::Dying {
            return;
        }

        let world = self.mob_entity.living_entity.entity.world.load();
        let bbox = self.mob_entity.living_entity.entity.bounding_box.load();

        let min = bbox.min_block_pos();
        let max = bbox.max_block_pos();

        for pos in BlockPos::iterate(min, max) {
            let block = world.get_block(&pos);
            if block != &Block::BEDROCK
                && block != &Block::END_STONE
                && block != &Block::OBSIDIAN
                && block != &Block::IRON_BARS
                && block != &Block::END_PORTAL
                && block != &Block::END_PORTAL_FRAME
            {
                world.set_block_state(&pos, BlockStateId::AIR, BlockFlags::NOTIFY_ALL);
            }
        }
    }

    pub fn ai_step(&self) {
        if self.mob_entity.living_entity.entity.is_removed() {
            return;
        }
        self.mob_entity.living_entity.entity.update_last_pos();
        self.ensure_nodes_initialized();
        self.update_flap_time();

        {
            let y = self.mob_entity.living_entity.entity.pos.load().y;
            let yaw = self.mob_entity.living_entity.entity.yaw.load();
            self.flight_history
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .record(y, yaw);
        };

        self.tick_growl();
        self.tick_crystal_healing();

        {
            let world = self.mob_entity.living_entity.entity.world.load();
            if let Some(ref fight_mutex) = world.dragon_fight {
                let living = &self.mob_entity.living_entity;
                crate::world::dragon_fight::DragonFight::with_fight(fight_mutex, &world, |fight| {
                    let custom_name = living.entity.custom_name.load();
                    fight.update_dragon(
                        &world,
                        living.entity.entity_uuid,
                        living.health.load(),
                        living.get_max_health(),
                        custom_name.as_ref().as_ref(),
                    );
                });
            }
        }

        let phase_type: EnderDragonPhase = *self
            .phase
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let phase = self.phase_manager.get_phase(phase_type);

        if phase_type.is_sitting() {
            *self
                .ticks_sitting
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
        } else {
            self.tick_block_breaking();
        }

        phase.tick(self);

        if self.mob_entity.living_entity.entity.is_removed() {
            return;
        }

        if phase_type != EnderDragonPhase::Dying {
            let target_location = *self
                .target_location
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(target) = target_location {
                let pos = self.mob_entity.living_entity.entity.pos.load();
                self.steer_toward(pos, target, phase.get_fly_speed(), phase.get_turn_speed());
            }
        }
    }

    pub fn hurt(&self, damage: f32) {
        let phase_type: EnderDragonPhase = *self
            .phase
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if phase_type.is_sitting() {
            *self
                .sitting_damage_received
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) += damage;
        }
    }
}

impl Mob for EnderDragonEntity {
    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn mob_tick(&self, _caller: &dyn EntityBase) {
        self.ai_step();
    }

    fn pre_damage(
        &self,
        damage_type: DamageType,
        _source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) -> bool {
        // EnderDragon.hurtServer accepts a Player cause or the ALWAYS_HURTS_ENDER_DRAGONS tag.
        cause.is_some_and(|cause| cause.get_player().is_some())
            || damage_type.has_tag(&tag::DamageType::MINECRAFT_ALWAYS_HURTS_ENDER_DRAGONS)
    }

    fn mob_kill(&self, _caller: &dyn EntityBase) {
        // EnderDragon.kill removes the dragon directly, then settles EnderDragonFight.
        let entity = self.get_entity();
        let world = entity.world.load();
        let dragon_uuid = entity.entity_uuid;
        entity.remove();

        if let Some(fight_mutex) = &world.dragon_fight {
            crate::world::dragon_fight::DragonFight::with_fight(fight_mutex, &world, |fight| {
                fight.set_dragon_killed(&world, dragon_uuid);
            });
        }
    }

    fn modify_incoming_damage(&self, amount: f32, damage_type: DamageType) -> f32 {
        if damage_type == DamageType::THORNS {
            // Reflected Thorns targets the root, which vanilla routes through the body.
            body_damage(amount)
        } else {
            amount
        }
    }

    fn post_tick(&self) {
        // Vanilla aiStep skips part positioning and contact in the NoAI branch.
        // Pumpkin's shared LivingEntity travel finishes after mob_tick returns.
        if !self.mob_entity.living_entity.entity.is_removed() && !self.mob_entity.is_no_ai() {
            self.tick_parts();
        }
    }

    fn on_damage(
        &self,
        _damage_type: DamageType,
        _source: Option<&dyn EntityBase>,
        _cause: Option<&dyn EntityBase>,
    ) {
        let living = &self.mob_entity.living_entity;
        if living.health.load() <= 0.0 {
            self.set_phase(EnderDragonPhase::Dying);
        }
    }

    fn get_mob_gravity(&self) -> f64 {
        0.0
    }
}

pub trait Vector3Ext {
    fn distance_squared(self, other: Self) -> f64;
}

impl Vector3Ext for Vector3<f64> {
    fn distance_squared(self, other: Self) -> f64 {
        let (dx, dy, dz) = (self.x - other.x, self.y - other.y, self.z - other.z);
        dx * dx + dy * dy + dz * dz
    }
}
