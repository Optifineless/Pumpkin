use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

use crate::entity::EntityBase;
use crate::entity::living::LivingEntity;
use crate::entity::mob::MobEntity;
use crate::world::World;

use crate::entity::ai::pathfinder::amphibious_node_evaluator::AmphibiousNodeEvaluator;
use crate::entity::ai::pathfinder::binary_heap::BinaryHeap;
use crate::entity::ai::pathfinder::fly_node_evaluator::FlyNodeEvaluator;
use crate::entity::ai::pathfinder::node::Node;
use crate::entity::ai::pathfinder::node::PathType;
use crate::entity::ai::pathfinder::node_evaluator::{MobData, NodeEvaluator};
use crate::entity::ai::pathfinder::path::Path;
use crate::entity::ai::pathfinder::pathfinding_context::PathfindingContext;
use crate::entity::ai::pathfinder::swim_node_evaluator::SwimNodeEvaluator;
use crate::entity::ai::pathfinder::walk_node_evaluator::WalkNodeEvaluator;
use pumpkin_data::attributes::Attributes;
use rustc_hash::{FxHashMap, FxHashSet};
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicBool, Ordering};

pub mod amphibious_node_evaluator;
pub mod binary_heap;
pub mod fly_node_evaluator;
mod mob_malus;
mod navigation_following;
pub(crate) mod navigation_geometry;
#[cfg(test)]
mod navigation_replacement_tests;
mod navigation_snapshot;
pub mod node;
pub mod node_evaluator;
mod passive_malus;
pub mod path;
pub mod path_type_cache;
pub mod pathfinding_context;
pub mod swim_node_evaluator;
pub mod walk_node_evaluator;

const MAX_ITERS: usize = 560;
const TARGET_DISTANCE_MULTIPLIER: f32 = 1.5;

pub struct PathFinder {
    max_visited_nodes: usize,
    open_set: BinaryHeap,
    neighbors_buf: Vec<Node>,
    all_nodes: FxHashMap<Vector3<i32>, Node>,
}

impl PathFinder {
    #[must_use]
    pub fn new(max_visited_nodes: usize) -> Self {
        Self {
            max_visited_nodes,
            open_set: BinaryHeap::new(),
            neighbors_buf: Vec::with_capacity(32),
            all_nodes: FxHashMap::default(),
        }
    }

    pub const fn set_max_visited_nodes(&mut self, max_visited_nodes: usize) {
        self.max_visited_nodes = max_visited_nodes;
    }

    #[must_use]
    pub const fn get_max_visited_nodes(&self) -> usize {
        self.max_visited_nodes
    }

    pub fn find_path_single(
        &mut self,
        evaluator: &mut EvaluatorKind,
        target: BlockPos,
        max_path_length: f32,
        reach_range: i32,
        max_visited_nodes_multiplier: f32,
    ) -> Option<Path> {
        self.find_path(
            evaluator,
            &[target],
            max_path_length,
            reach_range,
            max_visited_nodes_multiplier,
        )
    }

    #[expect(clippy::too_many_lines)]
    pub fn find_path(
        &mut self,
        evaluator: &mut EvaluatorKind,
        targets: &[BlockPos],
        max_path_length: f32,
        reach_range: i32,
        max_visited_nodes_multiplier: f32,
    ) -> Option<Path> {
        if targets.is_empty() {
            return None;
        }

        let mut from = evaluator.get_start()?;

        let mut target_entries: Vec<(crate::entity::ai::pathfinder::node::Target, BlockPos)> =
            targets
                .iter()
                .map(|&pos| (evaluator.get_target(pos), pos))
                .collect();

        self.all_nodes.clear();
        self.open_set.clear();

        from.g = 0.0;
        from.h = Self::get_best_h(&from, &mut target_entries);
        from.f = from.h;
        from.walked_dist = 0.0;
        from.came_from = None;
        from.closed = false;

        self.all_nodes.insert(from.pos.0, from);
        self.open_set.insert(from);

        let mut count = 0usize;
        let mut reached_targets: Vec<usize> = Vec::new();
        let max_visited_nodes_adjusted =
            (self.max_visited_nodes as f32 * max_visited_nodes_multiplier) as usize;

        while !self.open_set.is_empty() {
            count += 1;
            if count >= max_visited_nodes_adjusted {
                break;
            }

            let Some(mut current) = self.open_set.pop() else {
                break;
            };

            current.closed = true;
            self.all_nodes.insert(current.pos.0, current);

            for (idx, (target, _)) in target_entries.iter_mut().enumerate() {
                if current.distance_manhattan_node(&target.node) <= reach_range as f32 {
                    target.set_reached();
                    target.update_best(0.0, &current);
                    if !reached_targets.contains(&idx) {
                        reached_targets.push(idx);
                    }
                }
            }

            if !reached_targets.is_empty() {
                break;
            }

            if current.distance_to_node(&from) < max_path_length {
                self.neighbors_buf.clear();
                evaluator.get_neighbors(&current, &mut self.neighbors_buf);

                for mut neighbor in self.neighbors_buf.drain(..) {
                    let distance = current.distance_to_node(&neighbor);
                    neighbor.walked_dist = current.walked_dist + distance;
                    let tentative_g = current.g + distance + neighbor.cost_malus;

                    let in_open = self.open_set.contains(&neighbor);
                    let is_better = if in_open {
                        self.open_set
                            .get_node(&neighbor)
                            .is_some_and(|existing| tentative_g < existing.g)
                    } else {
                        true
                    };

                    if neighbor.walked_dist < max_path_length && is_better {
                        neighbor.came_from = Some(current.pos.0);
                        neighbor.g = tentative_g;
                        neighbor.h = Self::get_best_h(&neighbor, &mut target_entries) * 1.5;
                        neighbor.f = neighbor.g + neighbor.h;

                        if in_open {
                            self.open_set.update_node(&neighbor, neighbor);
                        } else {
                            self.open_set.insert(neighbor);
                        }
                        self.all_nodes.insert(neighbor.pos.0, neighbor);
                    }
                }
            }
        }

        for node in self.open_set.drain() {
            self.all_nodes.entry(node.pos.0).or_insert(node);
        }

        if reached_targets.is_empty() {
            target_entries
                .into_iter()
                .filter_map(|(target, target_pos)| {
                    target.get_best_node().map(|best| {
                        Self::reconstruct_path(&best, target_pos, false, &self.all_nodes)
                    })
                })
                .min_by(|a, b| {
                    a.get_dist_to_target()
                        .total_cmp(&b.get_dist_to_target())
                        .then_with(|| a.get_node_count().cmp(&b.get_node_count()))
                })
        } else {
            reached_targets
                .into_iter()
                .filter_map(|idx| {
                    let (target, target_pos) = &target_entries[idx];
                    target.get_best_node().map(|best| {
                        Self::reconstruct_path(&best, *target_pos, true, &self.all_nodes)
                    })
                })
                .min_by_key(Path::get_node_count)
        }
    }

    fn reconstruct_path(
        closest: &Node,
        target: BlockPos,
        reached: bool,
        all_nodes: &FxHashMap<Vector3<i32>, Node>,
    ) -> Path {
        let mut nodes = Vec::new();
        let mut current = *closest;
        nodes.push(current);
        let mut visited = FxHashSet::default();
        visited.insert(current.pos.0);

        while let Some(prev_pos) = current.came_from {
            if prev_pos == current.pos.0 || !visited.insert(prev_pos) {
                break;
            }
            if let Some(prev_node) = all_nodes.get(&prev_pos) {
                nodes.push(*prev_node);
                current = *prev_node;
            } else {
                break;
            }
        }

        nodes.reverse();
        Path::new(nodes, target, reached)
    }

    fn get_best_h(
        from: &Node,
        targets: &mut [(crate::entity::ai::pathfinder::node::Target, BlockPos)],
    ) -> f32 {
        let mut best_h = f32::MAX;
        for (target, _) in targets.iter_mut() {
            let h = from.distance_to_node(&target.node);
            target.update_best(h, from);
            best_h = best_h.min(h);
        }
        best_h
    }
}

#[derive(Clone, Copy, Debug)]
pub struct NavigatorGoal {
    pub current_progress: Vector3<f64>,
    pub destination: Vector3<f64>,
    pub speed: f64,
}

impl NavigatorGoal {
    #[must_use]
    pub const fn new(
        current_progress: Vector3<f64>,
        destination: Vector3<f64>,
        speed: f64,
    ) -> Self {
        Self {
            current_progress,
            destination,
            speed,
        }
    }
}

pub enum EvaluatorKind {
    Walk(WalkNodeEvaluator),
    Fly(FlyNodeEvaluator),
    Swim(SwimNodeEvaluator),
    Amphibious(AmphibiousNodeEvaluator),
}

impl EvaluatorKind {
    pub fn prepare(&mut self, context: PathfindingContext, mob_data: MobData) {
        match self {
            Self::Walk(e) => e.prepare(context, mob_data),
            Self::Fly(e) => e.prepare(context, mob_data),
            Self::Swim(e) => e.prepare(context, mob_data),
            Self::Amphibious(e) => e.prepare(context, mob_data),
        }
    }

    pub fn done(&mut self) {
        match self {
            Self::Walk(e) => e.done(),
            Self::Fly(e) => e.done(),
            Self::Swim(e) => e.done(),
            Self::Amphibious(e) => e.done(),
        }
    }

    pub fn get_start(&mut self) -> Option<Node> {
        match self {
            Self::Walk(e) => e.get_start(),
            Self::Fly(e) => e.get_start(),
            Self::Swim(e) => e.get_start(),
            Self::Amphibious(e) => e.get_start(),
        }
    }

    pub fn get_target(&mut self, pos: BlockPos) -> crate::entity::ai::pathfinder::node::Target {
        match self {
            Self::Walk(e) => e.get_target(pos),
            Self::Fly(e) => e.get_target(pos),
            Self::Swim(e) => e.get_target(pos),
            Self::Amphibious(e) => e.get_target(pos),
        }
    }

    pub fn get_neighbors(&mut self, current: &Node, out: &mut Vec<Node>) {
        match self {
            Self::Walk(e) => e.get_neighbors(current, out),
            Self::Fly(e) => e.get_neighbors(current, out),
            Self::Swim(e) => e.get_neighbors(current, out),
            Self::Amphibious(e) => e.get_neighbors(current, out),
        }
    }

    pub fn close_node(&mut self, node: Node) {
        match self {
            Self::Walk(e) => e.base.close_node(node),
            Self::Fly(e) => e.walk.base.close_node(node),
            Self::Swim(e) => e.base.close_node(node),
            Self::Amphibious(e) => e.walk.base.close_node(node),
        }
    }

    #[must_use]
    pub fn closed_node(&self, pos: &Vector3<i32>) -> Option<Node> {
        match self {
            Self::Walk(e) => e.base.closed_node(pos),
            Self::Fly(e) => e.walk.base.closed_node(pos),
            Self::Swim(e) => e.base.closed_node(pos),
            Self::Amphibious(e) => e.walk.base.closed_node(pos),
        }
    }

    pub fn set_can_float(&mut self, can_float: bool) {
        match self {
            Self::Walk(e) => e.set_can_float(can_float),
            Self::Fly(e) => e.set_can_float(can_float),
            Self::Swim(e) => e.set_can_float(can_float),
            Self::Amphibious(e) => e.set_can_float(can_float),
        }
    }

    pub fn set_can_open_doors(&mut self, can_open: bool) {
        match self {
            Self::Walk(e) => e.set_can_open_doors(can_open),
            Self::Fly(e) => e.set_can_open_doors(can_open),
            Self::Swim(e) => e.set_can_open_doors(can_open),
            Self::Amphibious(e) => e.set_can_open_doors(can_open),
        }
    }

    pub fn set_can_pass_doors(&mut self, can_pass: bool) {
        match self {
            Self::Walk(e) => e.set_can_pass_doors(can_pass),
            Self::Fly(e) => e.set_can_pass_doors(can_pass),
            Self::Swim(e) => e.set_can_pass_doors(can_pass),
            Self::Amphibious(e) => e.set_can_pass_doors(can_pass),
        }
    }

    pub fn set_can_walk_over_fences(&mut self, can_walk: bool) {
        match self {
            Self::Walk(e) => e.set_can_walk_over_fences(can_walk),
            Self::Fly(e) => e.set_can_walk_over_fences(can_walk),
            Self::Swim(e) => e.set_can_walk_over_fences(can_walk),
            Self::Amphibious(e) => e.set_can_walk_over_fences(can_walk),
        }
    }
}

pub trait PathNavigationTrait: Send + Sync {
    fn set_progress(&mut self, goal: NavigatorGoal);
    fn set_speed(&mut self, speed: f64);
    fn stop(&mut self);
    fn is_idle(&self) -> bool;
    fn is_done(&self) -> bool;
    fn is_in_progress(&self) -> bool;
    fn is_stuck(&self) -> bool;
    fn get_path(&self) -> Option<&Path>;
    fn get_path_mut(&mut self) -> Option<&mut Path>;
    fn set_pathfinding_malus(&mut self, path_type: PathType, malus: f32);
    fn get_pathfinding_malus(&self, path_type: PathType) -> f32;
    fn set_mob_dimensions(&mut self, width: f32, height: f32);
    fn can_reach_within(
        &mut self,
        mob: &MobEntity,
        destination: Vector3<f64>,
        distance: f32,
    ) -> bool;
    /// Ticks navigation with its owning entity, preserving species collision context for rays.
    fn tick(&mut self, mob: &MobEntity, caller: &dyn EntityBase);

    /// Evaluates a block with this navigator's evaluator for MoveControl.isWalkable.
    fn path_type_at(&mut self, _entity: &LivingEntity, _pos: BlockPos) -> PathType {
        // Custom navigators without an evaluator retain MoveControl.isWalkable's permissive fallback.
        PathType::Walkable
    }

    /// Waypoint passed to `MoveControl`, as in vanilla `PathNavigation.tick`.
    fn next_move_target(&self) -> Option<(Vector3<f64>, f64)> {
        None
    }
    fn move_to_coords(&mut self, x: f64, y: f64, z: f64, speed: f64, entity: &LivingEntity)
    -> bool;
    fn move_to_pos(&mut self, pos: BlockPos, speed: f64, entity: &LivingEntity) -> bool;
    fn move_to_entity(&mut self, target: &LivingEntity, speed: f64, mob: &MobEntity) -> bool;
    fn move_to_path(&mut self, path: Option<Path>, speed: f64, entity: &LivingEntity) -> bool;
    fn create_path(
        &mut self,
        mob: &MobEntity,
        destination: Vector3<f64>,
        reach_range: i32,
    ) -> Option<Path>;
    fn recompute_path(&mut self, mob: &MobEntity);
    fn set_avoid_sun(&mut self, avoid_sun: bool);
    fn set_can_walk_over_fences(&mut self, can_walk: bool);
    fn set_can_open_doors(&mut self, can_open: bool);
    fn set_can_pass_doors(&mut self, can_pass: bool);
    fn set_can_float(&mut self, can_float: bool);
    fn can_float(&self) -> bool;
    fn can_navigate_ground(&self) -> bool;
    fn set_required_path_length(&mut self, length: f32);
    fn set_max_visited_nodes_multiplier(&mut self, multiplier: f32);
    fn reset_max_visited_nodes_multiplier(&mut self);
    fn get_target_pos(&self) -> Option<BlockPos>;
    fn can_path_to_targets_below_surface(&self) -> bool;
    fn set_can_path_to_targets_below_surface(&mut self, can_path: bool);

    /// Whether the mob can settle at `pos`, which random targets are checked
    /// against: vanilla's `PathNavigation.isStableDestination`.
    fn is_stable_destination(
        &self,
        world: &World,
        pos: &BlockPos,
        _entity: &dyn EntityBase,
    ) -> bool {
        world.get_block_state(&pos.down()).is_solid_render()
    }
}

pub struct PathNavigation {
    pub current_goal: Option<NavigatorGoal>,
    pub evaluator: EvaluatorKind,
    pub path: Option<Path>,
    pub speed_modifier: f64,
    pub tick_count: u32,
    pub last_stuck_check: u32,
    pub last_stuck_check_pos: Vector3<f64>,
    pub timeout_cached_node: Vector3<i32>,
    pub timeout_timer: u64,
    pub last_timeout_check: u64,
    pub timeout_limit: f64,
    pub max_distance_to_waypoint: f32,
    pub has_delayed_recomputation: bool,
    pub time_last_recompute: u64,
    pub target_pos: Option<BlockPos>,
    pub reach_range: i32,
    pub max_visited_nodes_multiplier: f32,
    pub is_stuck: bool,
    pub required_path_length: f32,
    pub path_type_overrides: FxHashMap<PathType, f32>,
    pub mob_width: f32,
    pub mob_height: f32,
    pub can_float: bool,
    pub can_walk_over_fences: bool,
    pub can_open_doors: bool,
    pub can_pass_doors: bool,
    pub avoid_sun: bool,
    pub can_path_to_targets_below_surface: bool,
    pub open_set: BinaryHeap,
    pub neighbors_buf: Vec<Node>,
    pub is_idle: AtomicBool,
    next_ground_y: Option<f64>,
    stands_on_lava: bool,
    pending_reach_range: i32,
}

impl Default for PathNavigation {
    fn default() -> Self {
        Self::new(EvaluatorKind::Walk(WalkNodeEvaluator::default()))
    }
}

impl PathNavigation {
    #[must_use]
    pub fn new(evaluator: EvaluatorKind) -> Self {
        Self {
            current_goal: None,
            evaluator,
            path: None,
            speed_modifier: 1.0,
            tick_count: 0,
            last_stuck_check: 0,
            last_stuck_check_pos: Vector3::new(0.0, 0.0, 0.0),
            timeout_cached_node: Vector3::new(0, 0, 0),
            timeout_timer: 0,
            last_timeout_check: 0,
            timeout_limit: 0.0,
            max_distance_to_waypoint: 0.5,
            has_delayed_recomputation: false,
            time_last_recompute: 0,
            target_pos: None,
            reach_range: 1,
            max_visited_nodes_multiplier: 1.0,
            is_stuck: false,
            required_path_length: 16.0,
            path_type_overrides: FxHashMap::default(),
            mob_width: 0.6,
            mob_height: 1.95,
            can_float: false,
            can_walk_over_fences: false,
            can_open_doors: false,
            can_pass_doors: true,
            avoid_sun: false,
            can_path_to_targets_below_surface: false,
            open_set: BinaryHeap::new(),
            neighbors_buf: Vec::new(),
            is_idle: AtomicBool::new(true),
            next_ground_y: None,
            stands_on_lava: false,
            pending_reach_range: 1,
        }
    }

    // PathNavigation.tick hands the next node to MoveControl for species steering.
    fn next_move_target(&self) -> Option<(Vector3<f64>, f64)> {
        let pos = self.path.as_ref()?.get_next_node_pos()?;
        let center = f64::from((self.mob_width + 1.0) as i32) * 0.5;
        Some((
            Vector3::new(
                f64::from(pos.0.x) + center,
                self.next_ground_y.unwrap_or(f64::from(pos.0.y)),
                f64::from(pos.0.z) + center,
            ),
            self.speed_modifier,
        ))
    }

    pub fn set_progress(&mut self, goal: NavigatorGoal) {
        self.is_idle.store(false, Ordering::Relaxed);
        self.speed_modifier = goal.speed;
        self.last_stuck_check = self.tick_count;
        self.last_stuck_check_pos = goal.current_progress;
        if self.path.as_ref().is_some_and(|path| {
            !path.is_done() && path.get_target() == BlockPos::floored_v(goal.destination)
        }) {
            return;
        }
        // PathNavigation.moveTo coordinates queues createPath with reach range 1.
        self.pending_reach_range = 1;
        self.current_goal = Some(goal);
        self.path = None;
        self.next_ground_y = None;
        self.reset_stuck_timeout();
    }

    pub const fn set_speed(&mut self, speed: f64) {
        self.speed_modifier = speed;
        if let Some(goal) = &mut self.current_goal {
            goal.speed = speed;
        }
    }

    pub fn stop(&mut self) {
        self.is_idle.store(true, Ordering::Relaxed);
        self.current_goal = None;
        self.path = None;
        self.next_ground_y = None;
    }

    pub fn set_pathfinding_malus(&mut self, path_type: PathType, malus: f32) {
        self.path_type_overrides.insert(path_type, malus);
    }

    #[must_use]
    pub fn get_pathfinding_malus(&self, path_type: PathType) -> f32 {
        self.path_type_overrides
            .get(&path_type)
            .copied()
            .unwrap_or_else(|| path_type.get_malus())
    }

    pub const fn set_mob_dimensions(&mut self, width: f32, height: f32) {
        self.mob_width = width;
        self.mob_height = height;
    }

    pub fn can_reach_within(
        &mut self,
        mob: &MobEntity,
        destination: Vector3<f64>,
        distance: f32,
    ) -> bool {
        self.compute_path(mob, destination, 1)
            .is_some_and(|path| path.can_reach() || path.get_dist_to_target() <= distance)
    }

    fn mob_max_follow_range(&self, entity: &LivingEntity) -> f32 {
        let follow_range = entity.get_attribute_value(&Attributes::FOLLOW_RANGE) as f32;
        follow_range.max(self.required_path_length)
    }

    #[allow(clippy::too_many_lines)]
    pub fn compute_path(
        &mut self,
        mob: &MobEntity,
        destination: Vector3<f64>,
        reach_range: i32,
    ) -> Option<Path> {
        let entity = &mob.living_entity;
        if !self.can_update_path(entity)
            || entity.entity.pos.load().y < f64::from(entity.entity.world.load().min_y)
        {
            return None;
        }
        self.prepare_evaluator(mob);
        let start_node = self.evaluator.get_start();
        let Some(mut start_node) = start_node else {
            self.evaluator.done();
            return None;
        };
        let mut target = self.evaluator.get_target(BlockPos::floored_v(destination));

        start_node.g = 0.0;
        let start_dist = start_node.distance(&target);
        target.update_best(start_dist, &start_node);
        start_node.h = start_dist;
        start_node.f = start_node.h;
        start_node.walked_dist = 0.0;
        start_node.came_from = None;

        let start_pos = start_node.pos.0;
        self.open_set.reserve_for_search();
        self.open_set.clear();
        self.open_set.insert(start_node);

        let mut iterations = 0usize;
        let mut reached = false;
        // Read once: it's an attribute lookup, and the loop below runs up to 2048 times.
        let follow_range = self.mob_max_follow_range(entity);
        let max_iters = ((follow_range * 16.0 * self.max_visited_nodes_multiplier) as usize)
            .clamp(100, 2048)
            .max(MAX_ITERS);

        while !self.open_set.is_empty() {
            iterations += 1;
            if iterations >= max_iters {
                break;
            }

            let Some(current) = self.open_set.pop() else {
                break;
            };
            self.evaluator.close_node(current);

            if current.distance_manhattan(&target) <= reach_range as f32 {
                target.reached = true;
                reached = true;
                target.update_best(0.0, &current);
                break;
            }

            let dx = (current.pos.0.x - start_pos.x) as f32;
            let dy = (current.pos.0.y - start_pos.y) as f32;
            let dz = (current.pos.0.z - start_pos.z) as f32;
            let euclidean = (dx * dx + dy * dy + dz * dz).sqrt();
            if euclidean >= follow_range {
                continue;
            }

            self.neighbors_buf.clear();
            self.evaluator
                .get_neighbors(&current, &mut self.neighbors_buf);

            for mut neighbor in self.neighbors_buf.drain(..) {
                let step_cost = current.distance(&neighbor);
                neighbor.walked_dist = current.walked_dist + step_cost;
                let tentative_g = current.g + step_cost + neighbor.cost_malus;

                let existing_g = self.open_set.get_node(&neighbor).map(|existing| existing.g);
                let in_heap = existing_g.is_some();
                if neighbor.walked_dist < follow_range
                    && existing_g.is_none_or(|existing_g| tentative_g < existing_g)
                {
                    neighbor.came_from = Some(current.pos.0);
                    neighbor.g = tentative_g;
                    let dist_to_target = neighbor.distance(&target);
                    target.update_best(dist_to_target, &neighbor);
                    neighbor.h = dist_to_target * TARGET_DISTANCE_MULTIPLIER;
                    neighbor.f = neighbor.g + neighbor.h;

                    if in_heap {
                        self.open_set.update_node(&neighbor, neighbor);
                    } else {
                        self.open_set.insert(neighbor);
                    }
                }
            }
        }

        self.evaluator.done();

        let path = target.best_node.map(|best_node| {
            // The latest copy of the best node: closed, or still open. Every node before it in the
            // chain was expanded, so it is closed and cached.
            let best_pos = best_node.pos.0;
            let latest = self
                .evaluator
                .closed_node(&best_pos)
                .or_else(|| self.open_set.get_node(&best_node).copied())
                .unwrap_or(best_node);
            let mut path_nodes = vec![best_node];
            let mut visited: FxHashSet<Vector3<i32>> = FxHashSet::default();
            visited.insert(best_pos);
            let mut came_from = latest.came_from;
            while let Some(prev_pos) = came_from {
                if !visited.insert(prev_pos) {
                    break;
                }
                let Some(prev_node) = self.evaluator.closed_node(&prev_pos) else {
                    break;
                };
                path_nodes.push(prev_node);
                came_from = prev_node.came_from;
            }
            path_nodes.reverse();
            Path::new(path_nodes, target.node.pos, reached)
        });
        self.open_set.clear();
        if path.is_some() {
            self.target_pos = Some(BlockPos::floored_v(destination));
            self.reach_range = reach_range;
            self.reset_stuck_timeout();
        }
        path
    }

    pub fn find_surface_position(world: &World, mut pos: BlockPos) -> BlockPos {
        let block_state = world.get_block_state(&pos);
        if block_state.is_air() {
            let mut column_pos = pos;
            while column_pos.0.y >= world.min_y && world.get_block_state(&column_pos).is_air() {
                column_pos.0.y -= 1;
            }
            if column_pos.0.y >= world.min_y {
                return BlockPos::new(column_pos.0.x, column_pos.0.y + 1, column_pos.0.z);
            }
            column_pos.0.y = pos.0.y + 1;
            while column_pos.0.y <= world.get_top_y() && world.get_block_state(&column_pos).is_air()
            {
                column_pos.0.y += 1;
            }
            pos = column_pos;
        }
        if !world.get_block_state(&pos).is_solid() {
            return pos;
        }
        let mut column_pos = pos;
        while column_pos.0.y <= world.get_top_y() && world.get_block_state(&column_pos).is_solid() {
            column_pos.0.y += 1;
        }
        column_pos
    }

    pub fn get_surface_y(&self, entity: &LivingEntity) -> f64 {
        let pos = entity.entity.pos.load();
        if entity.entity.touching_water.load(Ordering::Relaxed) && self.can_float {
            let mut surface = pos.y.floor() as i32;
            let world = entity.entity.world.load();
            let mut steps = 0;
            loop {
                let bp = BlockPos::new(pos.x.floor() as i32, surface, pos.z.floor() as i32);
                let fluid = world.get_fluid(&bp);
                if pumpkin_data::tag::Taggable::has_tag(
                    fluid,
                    &pumpkin_data::tag::Fluid::MINECRAFT_ENTITY_FLOATABLE,
                ) {
                    surface += 1;
                    steps += 1;
                    if steps > 16 {
                        return pos.y.floor();
                    }
                } else {
                    break;
                }
            }
            f64::from(surface)
        } else {
            (pos.y + 0.5).floor()
        }
    }

    pub const fn reset_stuck_timeout(&mut self) {
        self.timeout_cached_node = Vector3::new(0, 0, 0);
        self.timeout_timer = 0;
        self.timeout_limit = 0.0;
        self.is_stuck = false;
    }

    pub fn do_stuck_detection(&mut self, mob_pos: Vector3<f64>, mob: &MobEntity) {
        let entity = &mob.living_entity;
        let world_age = entity.entity.world.load().get_world_age() as u64;
        if self.tick_count.saturating_sub(self.last_stuck_check) > 100 {
            let speed = mob.movement_speed.load();
            let effective_speed = if speed >= 1.0 { speed } else { speed * speed };
            let threshold_distance = effective_speed * 100.0 * 0.25;
            let dx = mob_pos.x - self.last_stuck_check_pos.x;
            let dy = mob_pos.y - self.last_stuck_check_pos.y;
            let dz = mob_pos.z - self.last_stuck_check_pos.z;
            let dist_sq = dx * dx + dy * dy + dz * dz;

            if dist_sq < f64::from(threshold_distance * threshold_distance) {
                self.is_stuck = true;
                self.stop();
            } else {
                self.is_stuck = false;
            }
            self.last_stuck_check = self.tick_count;
            self.last_stuck_check_pos = mob_pos;
        }

        if let Some(path) = &self.path
            && !path.is_done()
            && let Some(pos) = path.get_next_node_pos()
        {
            if pos.0 == self.timeout_cached_node {
                self.timeout_timer = self
                    .timeout_timer
                    .saturating_add(world_age.saturating_sub(self.last_timeout_check));
            } else {
                self.timeout_cached_node = pos.0;
                let node_center = Vector3::new(
                    f64::from(pos.0.x) + 0.5,
                    f64::from(pos.0.y),
                    f64::from(pos.0.z) + 0.5,
                );
                let dist_to_node = (mob_pos - node_center).length();
                let speed = f64::from(mob.movement_speed.load());
                self.timeout_limit = if speed > 0.0 {
                    dist_to_node / speed * 20.0
                } else {
                    0.0
                };
            }

            if self.timeout_limit > 0.0 && self.timeout_timer as f64 > self.timeout_limit * 3.0 {
                self.reset_stuck_timeout();
                self.stop();
            }
            self.last_timeout_check = world_age;
        }
    }

    pub fn trim_path(&mut self, entity: &LivingEntity) {
        if self.avoid_sun {
            let pos = entity.entity.pos.load();
            let bp = BlockPos::new(
                pos.x.floor() as i32,
                (pos.y + 0.5).floor() as i32,
                pos.z.floor() as i32,
            );
            let world = entity.entity.world.load();
            if world.get_sky_light_level(&bp) < 15
                && let Some(path) = &mut self.path
            {
                let mut cut_index = None;
                for i in 0..path.get_node_count() {
                    if let Some(node) = path.get_node(i) {
                        let node_bp = BlockPos::new(node.pos.0.x, node.pos.0.y, node.pos.0.z);
                        if world.get_sky_light_level(&node_bp) >= 15 {
                            cut_index = Some(i);
                            break;
                        }
                    }
                }
                if let Some(idx) = cut_index {
                    path.truncate_nodes(idx);
                }
            }
        }
    }

    pub fn tick_ground(&mut self, mob: &MobEntity, caller: &dyn EntityBase) {
        self.tick_navigation(mob, caller);
    }
}

pub struct GroundPathNavigation {
    pub inner: PathNavigation,
}

impl Default for GroundPathNavigation {
    fn default() -> Self {
        Self::new()
    }
}

impl GroundPathNavigation {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: PathNavigation::new(EvaluatorKind::Walk(WalkNodeEvaluator::default())),
        }
    }
}

impl PathNavigationTrait for GroundPathNavigation {
    fn is_stable_destination(
        &self,
        world: &World,
        pos: &BlockPos,
        _entity: &dyn EntityBase,
    ) -> bool {
        // PathNavigation.isStableDestination, with StriderPathNavigation's lava exception.
        self.inner.stands_on_lava && world.get_block(pos) == &pumpkin_data::Block::LAVA
            || world.get_block_state(&pos.down()).is_solid_render()
    }

    fn path_type_at(&mut self, entity: &LivingEntity, pos: BlockPos) -> PathType {
        self.inner.path_type_at(entity, pos)
    }

    fn set_progress(&mut self, goal: NavigatorGoal) {
        self.inner.set_progress(goal);
    }

    fn set_speed(&mut self, speed: f64) {
        self.inner.set_speed(speed);
    }

    fn stop(&mut self) {
        self.inner.stop();
    }

    fn is_idle(&self) -> bool {
        self.inner.is_idle.load(Ordering::Relaxed)
    }

    fn is_done(&self) -> bool {
        self.inner.path.as_ref().is_none_or(Path::is_done)
    }

    fn is_in_progress(&self) -> bool {
        !self.is_done()
    }

    fn is_stuck(&self) -> bool {
        self.inner.is_stuck
    }

    fn get_path(&self) -> Option<&Path> {
        self.inner.path.as_ref()
    }

    fn get_path_mut(&mut self) -> Option<&mut Path> {
        self.inner.path.as_mut()
    }

    fn set_pathfinding_malus(&mut self, path_type: PathType, malus: f32) {
        self.inner.set_pathfinding_malus(path_type, malus);
    }

    fn get_pathfinding_malus(&self, path_type: PathType) -> f32 {
        self.inner.get_pathfinding_malus(path_type)
    }

    fn set_mob_dimensions(&mut self, width: f32, height: f32) {
        self.inner.set_mob_dimensions(width, height);
    }

    fn can_reach_within(
        &mut self,
        mob: &MobEntity,
        destination: Vector3<f64>,
        distance: f32,
    ) -> bool {
        self.inner.can_reach_within(mob, destination, distance)
    }

    fn tick(&mut self, mob: &MobEntity, caller: &dyn EntityBase) {
        self.inner.tick_ground(mob, caller);
    }

    fn next_move_target(&self) -> Option<(Vector3<f64>, f64)> {
        self.inner.next_move_target()
    }

    fn move_to_coords(
        &mut self,
        x: f64,
        y: f64,
        z: f64,
        speed: f64,
        entity: &LivingEntity,
    ) -> bool {
        let pos = entity.entity.pos.load();
        self.set_progress(NavigatorGoal::new(pos, Vector3::new(x, y, z), speed));
        true
    }

    fn move_to_pos(&mut self, pos: BlockPos, speed: f64, entity: &LivingEntity) -> bool {
        let p = entity.entity.pos.load();
        let target = Vector3::new(
            f64::from(pos.0.x) + 0.5,
            f64::from(pos.0.y) + 0.5,
            f64::from(pos.0.z) + 0.5,
        );
        self.set_progress(NavigatorGoal::new(p, target, speed));
        true
    }

    fn move_to_entity(&mut self, target: &LivingEntity, speed: f64, mob: &MobEntity) -> bool {
        let entity = &mob.living_entity;
        let p = entity.entity.pos.load();
        let target_pos = target.entity.pos.load();
        self.set_progress(NavigatorGoal::new(p, target_pos, speed));
        true
    }

    fn move_to_path(&mut self, path: Option<Path>, speed: f64, entity: &LivingEntity) -> bool {
        // PathNavigation.moveTo(Path, double) supersedes queued coordinate requests.
        self.inner.clear_pending_goal();
        if let Some(new_path) = path {
            self.inner.path = Some(new_path);
            if self.is_done() {
                return false;
            }
            self.inner.trim_path(entity);
            if self
                .inner
                .path
                .as_ref()
                .map_or(0, path::Path::get_node_count)
                == 0
            {
                return false;
            }
            self.inner.speed_modifier = speed;
            let mob_pos = Vector3::new(
                entity.entity.pos.load().x,
                self.inner.get_surface_y(entity),
                entity.entity.pos.load().z,
            );
            self.inner.last_stuck_check = self.inner.tick_count;
            self.inner.last_stuck_check_pos = mob_pos;
            self.inner.reset_stuck_timeout();
            self.inner.is_idle.store(false, Ordering::Relaxed);
            true
        } else {
            self.inner.path = None;
            self.inner.is_idle.store(true, Ordering::Relaxed);
            false
        }
    }

    fn create_path(
        &mut self,
        mob: &MobEntity,
        destination: Vector3<f64>,
        reach_range: i32,
    ) -> Option<Path> {
        let entity = &mob.living_entity;
        let mut dest_pos = BlockPos::floored_v(destination);
        if !self.inner.can_path_to_targets_below_surface {
            let world = entity.entity.world.load();
            dest_pos = PathNavigation::find_surface_position(&world, dest_pos);
        }
        let dest_v = Vector3::new(
            f64::from(dest_pos.0.x) + 0.5,
            f64::from(dest_pos.0.y),
            f64::from(dest_pos.0.z) + 0.5,
        );
        self.inner.compute_path(mob, dest_v, reach_range)
    }

    fn recompute_path(&mut self, mob: &MobEntity) {
        self.inner.recompute(mob);
    }

    fn set_avoid_sun(&mut self, avoid_sun: bool) {
        self.inner.avoid_sun = avoid_sun;
    }

    fn set_can_walk_over_fences(&mut self, can_walk: bool) {
        self.inner.can_walk_over_fences = can_walk;
    }

    fn set_can_open_doors(&mut self, can_open: bool) {
        self.inner.can_open_doors = can_open;
    }

    fn set_can_pass_doors(&mut self, can_pass: bool) {
        self.inner.can_pass_doors = can_pass;
    }

    fn set_can_float(&mut self, can_float: bool) {
        self.inner.can_float = can_float;
    }

    fn can_float(&self) -> bool {
        self.inner.can_float
    }

    fn can_navigate_ground(&self) -> bool {
        true
    }

    fn set_required_path_length(&mut self, length: f32) {
        self.inner.required_path_length = length;
    }

    fn set_max_visited_nodes_multiplier(&mut self, multiplier: f32) {
        self.inner.max_visited_nodes_multiplier = multiplier;
    }

    fn reset_max_visited_nodes_multiplier(&mut self) {
        self.inner.max_visited_nodes_multiplier = 1.0;
    }

    fn get_target_pos(&self) -> Option<BlockPos> {
        self.inner.target_pos
    }

    fn can_path_to_targets_below_surface(&self) -> bool {
        self.inner.can_path_to_targets_below_surface
    }

    fn set_can_path_to_targets_below_surface(&mut self, can_path: bool) {
        self.inner.can_path_to_targets_below_surface = can_path;
    }
}

pub struct FlyingPathNavigation {
    pub inner: PathNavigation,
    bee_destinations: bool,
}

impl Default for FlyingPathNavigation {
    fn default() -> Self {
        Self::new()
    }
}

impl FlyingPathNavigation {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: PathNavigation::new(EvaluatorKind::Fly(FlyNodeEvaluator::default())),
            bee_destinations: false,
        }
    }
}

impl PathNavigationTrait for FlyingPathNavigation {
    fn path_type_at(&mut self, entity: &LivingEntity, pos: BlockPos) -> PathType {
        self.inner.path_type_at(entity, pos)
    }

    fn is_stable_destination(
        &self,
        world: &World,
        pos: &BlockPos,
        entity: &dyn EntityBase,
    ) -> bool {
        // Bee.createNavigation's anonymous FlyingPathNavigation override.
        if self.bee_destinations {
            !world.get_block_state(&pos.down()).is_air()
        } else {
            navigation_geometry::entity_can_stand_on(world, pos, entity)
        }
    }

    fn set_progress(&mut self, goal: NavigatorGoal) {
        self.inner.set_progress(goal);
    }

    fn set_speed(&mut self, speed: f64) {
        self.inner.set_speed(speed);
    }

    fn stop(&mut self) {
        self.inner.stop();
    }

    fn is_idle(&self) -> bool {
        self.inner.is_idle.load(Ordering::Relaxed)
    }

    fn is_done(&self) -> bool {
        self.inner.path.as_ref().is_none_or(Path::is_done)
    }

    fn is_in_progress(&self) -> bool {
        !self.is_done()
    }

    fn is_stuck(&self) -> bool {
        self.inner.is_stuck
    }

    fn get_path(&self) -> Option<&Path> {
        self.inner.path.as_ref()
    }

    fn get_path_mut(&mut self) -> Option<&mut Path> {
        self.inner.path.as_mut()
    }

    fn set_pathfinding_malus(&mut self, path_type: PathType, malus: f32) {
        self.inner.set_pathfinding_malus(path_type, malus);
    }

    fn get_pathfinding_malus(&self, path_type: PathType) -> f32 {
        self.inner.get_pathfinding_malus(path_type)
    }

    fn set_mob_dimensions(&mut self, width: f32, height: f32) {
        self.inner.set_mob_dimensions(width, height);
    }

    fn can_reach_within(
        &mut self,
        mob: &MobEntity,
        destination: Vector3<f64>,
        distance: f32,
    ) -> bool {
        self.inner.can_reach_within(mob, destination, distance)
    }

    fn tick(&mut self, mob: &MobEntity, caller: &dyn EntityBase) {
        self.inner.tick_navigation(mob, caller);
    }

    fn next_move_target(&self) -> Option<(Vector3<f64>, f64)> {
        self.inner.next_move_target()
    }

    fn move_to_coords(
        &mut self,
        x: f64,
        y: f64,
        z: f64,
        speed: f64,
        entity: &LivingEntity,
    ) -> bool {
        let pos = entity.entity.pos.load();
        self.set_progress(NavigatorGoal::new(pos, Vector3::new(x, y, z), speed));
        true
    }

    fn move_to_pos(&mut self, pos: BlockPos, speed: f64, entity: &LivingEntity) -> bool {
        let p = entity.entity.pos.load();
        let target = Vector3::new(
            f64::from(pos.0.x) + 0.5,
            f64::from(pos.0.y) + 0.5,
            f64::from(pos.0.z) + 0.5,
        );
        self.set_progress(NavigatorGoal::new(p, target, speed));
        true
    }

    fn move_to_entity(&mut self, target: &LivingEntity, speed: f64, mob: &MobEntity) -> bool {
        let entity = &mob.living_entity;
        let p = entity.entity.pos.load();
        let target_pos = target.entity.pos.load();
        self.set_progress(NavigatorGoal::new(p, target_pos, speed));
        true
    }

    fn move_to_path(&mut self, path: Option<Path>, speed: f64, entity: &LivingEntity) -> bool {
        // PathNavigation.moveTo(Path, double) supersedes queued coordinate requests.
        self.inner.clear_pending_goal();
        if let Some(new_path) = path {
            self.inner.path = Some(new_path);
            if self.is_done() {
                return false;
            }
            self.inner.speed_modifier = speed;
            let mob_pos = entity.entity.pos.load();
            self.inner.last_stuck_check = self.inner.tick_count;
            self.inner.last_stuck_check_pos = mob_pos;
            self.inner.reset_stuck_timeout();
            self.inner.is_idle.store(false, Ordering::Relaxed);
            true
        } else {
            self.inner.path = None;
            self.inner.is_idle.store(true, Ordering::Relaxed);
            false
        }
    }

    fn create_path(
        &mut self,
        mob: &MobEntity,
        destination: Vector3<f64>,
        reach_range: i32,
    ) -> Option<Path> {
        self.inner.compute_path(mob, destination, reach_range)
    }

    fn recompute_path(&mut self, mob: &MobEntity) {
        self.inner.recompute(mob);
    }

    fn set_avoid_sun(&mut self, avoid_sun: bool) {
        self.inner.avoid_sun = avoid_sun;
    }

    fn set_can_walk_over_fences(&mut self, can_walk: bool) {
        self.inner.can_walk_over_fences = can_walk;
    }

    fn set_can_open_doors(&mut self, can_open: bool) {
        self.inner.can_open_doors = can_open;
    }

    fn set_can_pass_doors(&mut self, can_pass: bool) {
        self.inner.can_pass_doors = can_pass;
    }

    fn set_can_float(&mut self, can_float: bool) {
        self.inner.can_float = can_float;
    }

    fn can_float(&self) -> bool {
        self.inner.can_float
    }

    fn can_navigate_ground(&self) -> bool {
        false
    }

    fn set_required_path_length(&mut self, length: f32) {
        self.inner.required_path_length = length;
    }

    fn set_max_visited_nodes_multiplier(&mut self, multiplier: f32) {
        self.inner.max_visited_nodes_multiplier = multiplier;
    }

    fn reset_max_visited_nodes_multiplier(&mut self) {
        self.inner.max_visited_nodes_multiplier = 1.0;
    }

    fn get_target_pos(&self) -> Option<BlockPos> {
        self.inner.target_pos
    }

    fn can_path_to_targets_below_surface(&self) -> bool {
        self.inner.can_path_to_targets_below_surface
    }

    fn set_can_path_to_targets_below_surface(&mut self, can_path: bool) {
        self.inner.can_path_to_targets_below_surface = can_path;
    }
}

pub struct WaterBoundPathNavigation {
    pub inner: PathNavigation,
    pub allow_breaching: bool,
}

impl Default for WaterBoundPathNavigation {
    fn default() -> Self {
        Self::new(false)
    }
}

impl WaterBoundPathNavigation {
    #[must_use]
    pub fn new(allow_breaching: bool) -> Self {
        let mut inner =
            PathNavigation::new(EvaluatorKind::Swim(SwimNodeEvaluator::new(allow_breaching)));
        inner.can_pass_doors = false;
        Self {
            inner,
            allow_breaching,
        }
    }
}

impl PathNavigationTrait for WaterBoundPathNavigation {
    fn path_type_at(&mut self, entity: &LivingEntity, pos: BlockPos) -> PathType {
        self.inner.path_type_at(entity, pos)
    }

    fn set_progress(&mut self, goal: NavigatorGoal) {
        self.inner.set_progress(goal);
    }

    fn set_speed(&mut self, speed: f64) {
        self.inner.set_speed(speed);
    }

    fn stop(&mut self) {
        self.inner.stop();
    }

    fn is_idle(&self) -> bool {
        self.inner.is_idle.load(Ordering::Relaxed)
    }

    fn is_done(&self) -> bool {
        self.inner.path.as_ref().is_none_or(Path::is_done)
    }

    fn is_in_progress(&self) -> bool {
        !self.is_done()
    }

    fn is_stuck(&self) -> bool {
        self.inner.is_stuck
    }

    fn get_path(&self) -> Option<&Path> {
        self.inner.path.as_ref()
    }

    fn get_path_mut(&mut self) -> Option<&mut Path> {
        self.inner.path.as_mut()
    }

    fn set_pathfinding_malus(&mut self, path_type: PathType, malus: f32) {
        self.inner.set_pathfinding_malus(path_type, malus);
    }

    fn get_pathfinding_malus(&self, path_type: PathType) -> f32 {
        self.inner.get_pathfinding_malus(path_type)
    }

    fn set_mob_dimensions(&mut self, width: f32, height: f32) {
        self.inner.set_mob_dimensions(width, height);
    }

    fn can_reach_within(
        &mut self,
        mob: &MobEntity,
        destination: Vector3<f64>,
        distance: f32,
    ) -> bool {
        self.inner.can_reach_within(mob, destination, distance)
    }

    fn tick(&mut self, mob: &MobEntity, caller: &dyn EntityBase) {
        self.inner.tick_navigation(mob, caller);
    }

    fn next_move_target(&self) -> Option<(Vector3<f64>, f64)> {
        let target = self
            .inner
            .path
            .as_ref()?
            .get_next_entity_pos(self.inner.mob_width)?;
        Some((target, self.inner.speed_modifier))
    }

    fn move_to_coords(
        &mut self,
        x: f64,
        y: f64,
        z: f64,
        speed: f64,
        entity: &LivingEntity,
    ) -> bool {
        let pos = entity.entity.pos.load();
        self.set_progress(NavigatorGoal::new(pos, Vector3::new(x, y, z), speed));
        true
    }

    fn move_to_pos(&mut self, pos: BlockPos, speed: f64, entity: &LivingEntity) -> bool {
        let p = entity.entity.pos.load();
        let target = Vector3::new(
            f64::from(pos.0.x) + 0.5,
            f64::from(pos.0.y),
            f64::from(pos.0.z) + 0.5,
        );
        self.set_progress(NavigatorGoal::new(p, target, speed));
        true
    }

    fn move_to_entity(&mut self, target: &LivingEntity, speed: f64, mob: &MobEntity) -> bool {
        let entity = &mob.living_entity;
        let p = entity.entity.pos.load();
        let target_pos = target.entity.pos.load();
        self.set_progress(NavigatorGoal::new(p, target_pos, speed));
        true
    }

    fn move_to_path(&mut self, path: Option<Path>, speed: f64, entity: &LivingEntity) -> bool {
        // PathNavigation.moveTo(Path, double) supersedes queued coordinate requests.
        self.inner.clear_pending_goal();
        if let Some(new_path) = path {
            self.inner.path = Some(new_path);
            if self.is_done() {
                return false;
            }
            self.inner.speed_modifier = speed;
            let mob_pos = Vector3::new(
                entity.entity.pos.load().x,
                entity.entity.pos.load().y + f64::from(self.inner.mob_height) * 0.5,
                entity.entity.pos.load().z,
            );
            self.inner.last_stuck_check = self.inner.tick_count;
            self.inner.last_stuck_check_pos = mob_pos;
            self.inner.reset_stuck_timeout();
            self.inner.is_idle.store(false, Ordering::Relaxed);
            true
        } else {
            self.inner.path = None;
            self.inner.is_idle.store(true, Ordering::Relaxed);
            false
        }
    }

    fn create_path(
        &mut self,
        mob: &MobEntity,
        destination: Vector3<f64>,
        reach_range: i32,
    ) -> Option<Path> {
        self.inner.compute_path(mob, destination, reach_range)
    }

    fn recompute_path(&mut self, mob: &MobEntity) {
        self.inner.recompute(mob);
    }

    fn set_avoid_sun(&mut self, avoid_sun: bool) {
        self.inner.avoid_sun = avoid_sun;
    }

    fn set_can_walk_over_fences(&mut self, can_walk: bool) {
        self.inner.can_walk_over_fences = can_walk;
    }

    fn set_can_open_doors(&mut self, can_open: bool) {
        self.inner.can_open_doors = can_open;
    }

    fn set_can_pass_doors(&mut self, can_pass: bool) {
        self.inner.can_pass_doors = can_pass;
    }

    fn set_can_float(&mut self, _can_float: bool) {}

    fn can_float(&self) -> bool {
        // Aquatic navigation ignores setCanFloat; NodeEvaluator.canFloat stays false.
        false
    }

    fn can_navigate_ground(&self) -> bool {
        false
    }

    fn set_required_path_length(&mut self, length: f32) {
        self.inner.required_path_length = length;
    }

    fn set_max_visited_nodes_multiplier(&mut self, multiplier: f32) {
        self.inner.max_visited_nodes_multiplier = multiplier;
    }

    fn reset_max_visited_nodes_multiplier(&mut self) {
        self.inner.max_visited_nodes_multiplier = 1.0;
    }

    fn get_target_pos(&self) -> Option<BlockPos> {
        self.inner.target_pos
    }

    fn can_path_to_targets_below_surface(&self) -> bool {
        self.inner.can_path_to_targets_below_surface
    }

    fn set_can_path_to_targets_below_surface(&mut self, can_path: bool) {
        self.inner.can_path_to_targets_below_surface = can_path;
    }

    /// Vanilla WaterBoundPathNavigation.isStableDestination.
    fn is_stable_destination(
        &self,
        world: &World,
        pos: &BlockPos,
        _entity: &dyn EntityBase,
    ) -> bool {
        !world.get_block_state(pos).is_solid_render()
    }
}

pub struct WallClimberNavigation {
    pub inner: GroundPathNavigation,
    pub path_to_position: Option<BlockPos>,
    following_path_this_tick: bool,
}

impl Default for WallClimberNavigation {
    fn default() -> Self {
        Self::new()
    }
}

impl WallClimberNavigation {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: GroundPathNavigation::new(),
            path_to_position: None,
            following_path_this_tick: false,
        }
    }
}

impl PathNavigationTrait for WallClimberNavigation {
    fn path_type_at(&mut self, entity: &LivingEntity, pos: BlockPos) -> PathType {
        self.inner.path_type_at(entity, pos)
    }

    fn set_progress(&mut self, goal: NavigatorGoal) {
        self.path_to_position = Some(BlockPos::floored_v(goal.destination));
        self.inner.set_progress(goal);
    }

    fn set_speed(&mut self, speed: f64) {
        self.inner.set_speed(speed);
    }

    fn stop(&mut self) {
        // WallClimberNavigation inherits stop; the fallback destination survives.
        self.inner.stop();
    }

    fn is_idle(&self) -> bool {
        self.inner.is_idle()
    }

    fn is_done(&self) -> bool {
        self.inner.is_done()
    }

    fn is_in_progress(&self) -> bool {
        !self.is_done()
    }

    fn is_stuck(&self) -> bool {
        self.inner.is_stuck()
    }

    fn get_path(&self) -> Option<&Path> {
        self.inner.get_path()
    }

    fn get_path_mut(&mut self) -> Option<&mut Path> {
        self.inner.get_path_mut()
    }

    fn set_pathfinding_malus(&mut self, path_type: PathType, malus: f32) {
        self.inner.set_pathfinding_malus(path_type, malus);
    }

    fn get_pathfinding_malus(&self, path_type: PathType) -> f32 {
        self.inner.get_pathfinding_malus(path_type)
    }

    fn set_mob_dimensions(&mut self, width: f32, height: f32) {
        self.inner.set_mob_dimensions(width, height);
    }

    fn can_reach_within(
        &mut self,
        mob: &MobEntity,
        destination: Vector3<f64>,
        distance: f32,
    ) -> bool {
        self.inner.can_reach_within(mob, destination, distance)
    }

    fn tick(&mut self, mob: &MobEntity, caller: &dyn EntityBase) {
        // WallClimberNavigation.moveTo searches synchronously in Java. Flush our queued request first.
        self.inner.inner.process_pending_goal(mob);
        // WallClimberNavigation.tick selects superclass or fallback from the state at entry.
        self.following_path_this_tick = !self.inner.is_done();
        if self.following_path_this_tick {
            self.inner.tick(mob, caller);
        } else if let Some(target) = self.path_to_position
            && navigation_following::wall_target_reached(
                mob.living_entity.entity.pos.load(),
                target,
                self.inner.inner.mob_width,
            )
        {
            self.path_to_position = None;
        }
    }

    fn next_move_target(&self) -> Option<(Vector3<f64>, f64)> {
        if !self.following_path_this_tick
            && self.inner.is_done()
            && self.inner.inner.current_goal.is_none()
        {
            self.path_to_position.map(|pos| {
                (
                    Vector3::new(f64::from(pos.0.x), f64::from(pos.0.y), f64::from(pos.0.z)),
                    self.inner.inner.speed_modifier,
                )
            })
        } else {
            self.inner.next_move_target()
        }
    }

    fn move_to_coords(
        &mut self,
        x: f64,
        y: f64,
        z: f64,
        speed: f64,
        entity: &LivingEntity,
    ) -> bool {
        self.path_to_position = Some(BlockPos::new(
            x.floor() as i32,
            y.floor() as i32,
            z.floor() as i32,
        ));
        self.inner.move_to_coords(x, y, z, speed, entity)
    }

    fn move_to_pos(&mut self, pos: BlockPos, speed: f64, entity: &LivingEntity) -> bool {
        self.path_to_position = Some(pos);
        self.inner.move_to_pos(pos, speed, entity)
    }

    fn move_to_entity(&mut self, target: &LivingEntity, speed: f64, mob: &MobEntity) -> bool {
        let entity = &mob.living_entity;
        self.path_to_position = Some(target.entity.block_pos.load());
        let path = self.create_path(mob, target.entity.pos.load(), 0);
        if path.is_some() {
            self.move_to_path(path, speed, entity)
        } else {
            self.inner.inner.speed_modifier = speed;
            true
        }
    }

    fn move_to_path(&mut self, path: Option<Path>, speed: f64, entity: &LivingEntity) -> bool {
        self.inner.move_to_path(path, speed, entity)
    }

    fn create_path(
        &mut self,
        mob: &MobEntity,
        destination: Vector3<f64>,
        reach_range: i32,
    ) -> Option<Path> {
        self.path_to_position = Some(BlockPos::floored_v(destination));
        self.inner.create_path(mob, destination, reach_range)
    }

    fn recompute_path(&mut self, mob: &MobEntity) {
        self.inner.recompute_path(mob);
    }

    fn set_avoid_sun(&mut self, avoid_sun: bool) {
        self.inner.set_avoid_sun(avoid_sun);
    }

    fn set_can_walk_over_fences(&mut self, can_walk: bool) {
        self.inner.set_can_walk_over_fences(can_walk);
    }

    fn set_can_open_doors(&mut self, can_open: bool) {
        self.inner.set_can_open_doors(can_open);
    }

    fn set_can_pass_doors(&mut self, can_pass: bool) {
        self.inner.set_can_pass_doors(can_pass);
    }

    fn set_can_float(&mut self, can_float: bool) {
        self.inner.set_can_float(can_float);
    }

    fn can_float(&self) -> bool {
        self.inner.can_float()
    }

    fn can_navigate_ground(&self) -> bool {
        true
    }

    fn set_required_path_length(&mut self, length: f32) {
        self.inner.set_required_path_length(length);
    }

    fn set_max_visited_nodes_multiplier(&mut self, multiplier: f32) {
        self.inner.set_max_visited_nodes_multiplier(multiplier);
    }

    fn reset_max_visited_nodes_multiplier(&mut self) {
        self.inner.reset_max_visited_nodes_multiplier();
    }

    fn get_target_pos(&self) -> Option<BlockPos> {
        self.inner.get_target_pos()
    }

    fn can_path_to_targets_below_surface(&self) -> bool {
        self.inner.can_path_to_targets_below_surface()
    }

    fn set_can_path_to_targets_below_surface(&mut self, can_path: bool) {
        self.inner.set_can_path_to_targets_below_surface(can_path);
    }
}

pub struct AmphibiousPathNavigation {
    pub inner: PathNavigation,
    travelling: Option<std::sync::Arc<AtomicBool>>,
}

impl Default for AmphibiousPathNavigation {
    fn default() -> Self {
        Self::new(false)
    }
}

impl AmphibiousPathNavigation {
    #[must_use]
    pub fn new(prefers_shallow_swimming: bool) -> Self {
        Self {
            travelling: None,
            inner: PathNavigation::new(EvaluatorKind::Amphibious(AmphibiousNodeEvaluator::new(
                prefers_shallow_swimming,
            ))),
        }
    }
}

impl PathNavigationTrait for AmphibiousPathNavigation {
    fn path_type_at(&mut self, entity: &LivingEntity, pos: BlockPos) -> PathType {
        self.inner.path_type_at(entity, pos)
    }

    fn is_stable_destination(
        &self,
        world: &World,
        pos: &BlockPos,
        _entity: &dyn EntityBase,
    ) -> bool {
        // TurtlePathNavigation.isStableDestination.
        if self
            .travelling
            .as_ref()
            .is_some_and(|v| v.load(Ordering::Relaxed))
        {
            world.get_block(pos) == &pumpkin_data::Block::WATER
        } else {
            !world.get_block_state(&pos.down()).is_air()
        }
    }

    fn set_progress(&mut self, goal: NavigatorGoal) {
        self.inner.set_progress(goal);
    }

    fn set_speed(&mut self, speed: f64) {
        self.inner.set_speed(speed);
    }

    fn stop(&mut self) {
        self.inner.stop();
    }

    fn is_idle(&self) -> bool {
        self.inner.is_idle.load(Ordering::Relaxed)
    }

    fn is_done(&self) -> bool {
        self.inner.path.as_ref().is_none_or(Path::is_done)
    }

    fn is_in_progress(&self) -> bool {
        !self.is_done()
    }

    fn is_stuck(&self) -> bool {
        self.inner.is_stuck
    }

    fn get_path(&self) -> Option<&Path> {
        self.inner.path.as_ref()
    }

    fn get_path_mut(&mut self) -> Option<&mut Path> {
        self.inner.path.as_mut()
    }

    fn set_pathfinding_malus(&mut self, path_type: PathType, malus: f32) {
        self.inner.set_pathfinding_malus(path_type, malus);
    }

    fn get_pathfinding_malus(&self, path_type: PathType) -> f32 {
        self.inner.get_pathfinding_malus(path_type)
    }

    fn set_mob_dimensions(&mut self, width: f32, height: f32) {
        self.inner.set_mob_dimensions(width, height);
    }

    fn can_reach_within(
        &mut self,
        mob: &MobEntity,
        destination: Vector3<f64>,
        distance: f32,
    ) -> bool {
        self.inner.can_reach_within(mob, destination, distance)
    }

    fn tick(&mut self, mob: &MobEntity, caller: &dyn EntityBase) {
        self.inner.tick_navigation(mob, caller);
    }

    fn next_move_target(&self) -> Option<(Vector3<f64>, f64)> {
        self.inner.next_move_target()
    }

    fn move_to_coords(
        &mut self,
        x: f64,
        y: f64,
        z: f64,
        speed: f64,
        entity: &LivingEntity,
    ) -> bool {
        let pos = entity.entity.pos.load();
        self.set_progress(NavigatorGoal::new(pos, Vector3::new(x, y, z), speed));
        true
    }

    fn move_to_pos(&mut self, pos: BlockPos, speed: f64, entity: &LivingEntity) -> bool {
        let p = entity.entity.pos.load();
        let target = Vector3::new(
            f64::from(pos.0.x) + 0.5,
            f64::from(pos.0.y) + 0.5,
            f64::from(pos.0.z) + 0.5,
        );
        self.set_progress(NavigatorGoal::new(p, target, speed));
        true
    }

    fn move_to_entity(&mut self, target: &LivingEntity, speed: f64, mob: &MobEntity) -> bool {
        let entity = &mob.living_entity;
        let p = entity.entity.pos.load();
        let target_pos = target.entity.pos.load();
        self.set_progress(NavigatorGoal::new(p, target_pos, speed));
        true
    }

    fn move_to_path(&mut self, path: Option<Path>, speed: f64, entity: &LivingEntity) -> bool {
        // PathNavigation.moveTo(Path, double) supersedes queued coordinate requests.
        self.inner.clear_pending_goal();
        if let Some(new_path) = path {
            self.inner.path = Some(new_path);
            if self.is_done() {
                return false;
            }
            self.inner.speed_modifier = speed;
            let mob_pos = Vector3::new(
                entity.entity.pos.load().x,
                entity.entity.pos.load().y + f64::from(self.inner.mob_height) * 0.5,
                entity.entity.pos.load().z,
            );
            self.inner.last_stuck_check = self.inner.tick_count;
            self.inner.last_stuck_check_pos = mob_pos;
            self.inner.reset_stuck_timeout();
            self.inner.is_idle.store(false, Ordering::Relaxed);
            true
        } else {
            self.inner.path = None;
            self.inner.is_idle.store(true, Ordering::Relaxed);
            false
        }
    }

    fn create_path(
        &mut self,
        mob: &MobEntity,
        destination: Vector3<f64>,
        reach_range: i32,
    ) -> Option<Path> {
        self.inner.compute_path(mob, destination, reach_range)
    }

    fn recompute_path(&mut self, mob: &MobEntity) {
        self.inner.recompute(mob);
    }

    fn set_avoid_sun(&mut self, avoid_sun: bool) {
        self.inner.avoid_sun = avoid_sun;
    }

    fn set_can_walk_over_fences(&mut self, can_walk: bool) {
        self.inner.can_walk_over_fences = can_walk;
    }

    fn set_can_open_doors(&mut self, can_open: bool) {
        self.inner.can_open_doors = can_open;
    }

    fn set_can_pass_doors(&mut self, can_pass: bool) {
        self.inner.can_pass_doors = can_pass;
    }

    fn set_can_float(&mut self, _can_float: bool) {}

    fn can_float(&self) -> bool {
        // Aquatic navigation ignores setCanFloat; NodeEvaluator.canFloat stays false.
        false
    }

    fn can_navigate_ground(&self) -> bool {
        true
    }

    fn set_required_path_length(&mut self, length: f32) {
        self.inner.required_path_length = length;
    }

    fn set_max_visited_nodes_multiplier(&mut self, multiplier: f32) {
        self.inner.max_visited_nodes_multiplier = multiplier;
    }

    fn reset_max_visited_nodes_multiplier(&mut self) {
        self.inner.max_visited_nodes_multiplier = 1.0;
    }

    fn get_target_pos(&self) -> Option<BlockPos> {
        self.inner.target_pos
    }

    fn can_path_to_targets_below_surface(&self) -> bool {
        self.inner.can_path_to_targets_below_surface
    }

    fn set_can_path_to_targets_below_surface(&mut self, can_path: bool) {
        self.inner.can_path_to_targets_below_surface = can_path;
    }
}

pub struct Navigator {
    inner: Box<dyn PathNavigationTrait>,
}

impl Default for Navigator {
    fn default() -> Self {
        Self::ground()
    }
}

impl Navigator {
    /// Applies constructor pathfinding maluses inherited by this species in vanilla.
    pub fn configure_species_maluses(&mut self, entity_type: &pumpkin_data::entity::EntityType) {
        passive_malus::configure(self, entity_type.resource_name);
        mob_malus::configure(self, entity_type.resource_name);
    }

    #[must_use]
    pub fn new<N: PathNavigationTrait + 'static>(nav: N) -> Self {
        Self {
            inner: Box::new(nav),
        }
    }

    #[must_use]
    pub fn ground() -> Self {
        Self::new(GroundPathNavigation::new())
    }

    /// `StriderPathNavigation` accepts lava as a stable destination and starts at its surface.
    #[must_use]
    pub fn strider() -> Self {
        let mut navigation = GroundPathNavigation::new();
        navigation.inner.stands_on_lava = true;
        Self::new(navigation)
    }

    #[must_use]
    pub fn flying() -> Self {
        Self::new(FlyingPathNavigation::new())
    }

    #[must_use]
    pub fn water_bound(allow_breaching: bool) -> Self {
        Self::new(WaterBoundPathNavigation::new(allow_breaching))
    }

    #[must_use]
    pub fn wall_climber() -> Self {
        Self::new(WallClimberNavigation::new())
    }

    /// Turtle navigation restricts travel destinations to water while its travel goal runs.
    pub fn turtle(travelling: std::sync::Arc<AtomicBool>) -> Self {
        let mut navigation = AmphibiousPathNavigation::new(false);
        navigation.travelling = Some(travelling);
        Self::new(navigation)
    }

    /// Bee navigation accepts any non-air support below its destination.
    #[must_use]
    pub fn bee() -> Self {
        let mut navigation = FlyingPathNavigation::new();
        navigation.bee_destinations = true;
        Self::new(navigation)
    }

    /// `FrogPathNavigation` uses a shallow-water evaluator with frog jump destination tags.
    #[must_use]
    pub fn frog() -> Self {
        let mut navigation = AmphibiousPathNavigation::new(true);
        if let EvaluatorKind::Amphibious(evaluator) = &mut navigation.inner.evaluator {
            evaluator.walk.is_frog = true;
        }
        Self::new(navigation)
    }

    #[must_use]
    pub fn amphibious(prefers_shallow_swimming: bool) -> Self {
        Self::new(AmphibiousPathNavigation::new(prefers_shallow_swimming))
    }
}

impl Deref for Navigator {
    type Target = dyn PathNavigationTrait;

    fn deref(&self) -> &Self::Target {
        &*self.inner
    }
}

impl DerefMut for Navigator {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut *self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn water_navigation_hands_the_next_waypoint_to_move_control() {
        let mut navigation = WaterBoundPathNavigation::new(false);
        navigation.set_mob_dimensions(0.5, 0.3);
        navigation.set_speed(1.6);
        navigation.inner.path = Some(Path::new(
            vec![
                Node::new(BlockPos::new(4, 60, -2)),
                Node::new(BlockPos::new(4, 59, -2)),
            ],
            BlockPos::new(4, 59, -2),
            true,
        ));
        assert_eq!(
            navigation.next_move_target(),
            Some((Vector3::new(4.5, 60.0, -1.5), 1.6))
        );
        if let Some(path) = navigation.get_path_mut() {
            path.advance();
        }
        assert_eq!(
            navigation.next_move_target(),
            Some((Vector3::new(4.5, 59.0, -1.5), 1.6))
        );
        navigation.stop();
        assert_eq!(navigation.next_move_target(), None);
    }
}

#[cfg(test)]
mod movement_wiring_tests {
    use super::*;
    #[test]
    fn every_path_navigation_passes_its_waypoint_to_the_move_control() {
        let path = || {
            Path::new(
                vec![Node::new(BlockPos::new(2, 4, 6))],
                BlockPos::new(2, 4, 6),
                true,
            )
        };
        let mut ground = GroundPathNavigation::new();
        ground.inner.path = Some(path());
        let mut flying = FlyingPathNavigation::new();
        flying.inner.path = Some(path());
        let mut amphibious = AmphibiousPathNavigation::new(false);
        amphibious.inner.path = Some(path());
        let mut climber = WallClimberNavigation::new();
        climber.inner.inner.path = Some(path());
        for nav in [
            &ground as &dyn PathNavigationTrait,
            &flying,
            &amphibious,
            &climber,
        ] {
            assert_eq!(
                nav.next_move_target().map(|(p, _)| p),
                Some(Vector3::new(2.5, 4.0, 6.5))
            );
        }
    }
}
