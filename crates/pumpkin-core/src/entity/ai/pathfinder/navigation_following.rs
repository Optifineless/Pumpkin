use super::{EvaluatorKind, MobEntity, Path, PathNavigation, PathType};
use crate::entity::EntityBase;
use crate::entity::living::LivingEntity;
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use std::sync::atomic::Ordering;

// PathNavigation's two independent timers: recomputation debounce and stuck detection.
const MAX_TIME_RECOMPUTE: u64 = 20;

#[derive(Clone, Copy)]
pub(super) enum NavigationKind {
    Ground,
    Flying { can_float: bool },
    Water { allow_breaching: bool },
    Amphibious,
}

impl NavigationKind {
    // Ground/Flying/WaterBound/AmphibiousPathNavigation.canUpdatePath.
    pub(super) const fn can_update(self, ground: bool, liquid: bool, passenger: bool) -> bool {
        match self {
            Self::Ground => ground || liquid || passenger,
            Self::Flying { can_float } => can_float && liquid || !passenger,
            Self::Water { allow_breaching } => allow_breaching || liquid,
            Self::Amphibious => true,
        }
    }
}

impl PathNavigation {
    const fn kind(&self) -> NavigationKind {
        match &self.evaluator {
            EvaluatorKind::Walk(_) => NavigationKind::Ground,
            EvaluatorKind::Fly(_) => NavigationKind::Flying {
                can_float: self.can_float,
            },
            EvaluatorKind::Swim(e) => NavigationKind::Water {
                allow_breaching: e.allow_breaching,
            },
            EvaluatorKind::Amphibious(_) => NavigationKind::Amphibious,
        }
    }

    pub(super) fn can_update_path(&self, living: &LivingEntity) -> bool {
        let entity = &living.entity;
        self.kind().can_update(
            entity.on_ground.load(Ordering::Relaxed),
            entity.touching_water.load(Ordering::Relaxed)
                || entity.touching_lava.load(Ordering::Relaxed),
            entity.has_vehicle(),
        )
    }

    fn temp_mob_pos(&self, entity: &LivingEntity) -> Vector3<f64> {
        let mut pos = entity.entity.pos.load();
        match self.kind() {
            NavigationKind::Ground => pos.y = self.get_surface_y(entity),
            NavigationKind::Flying { .. } => {}
            NavigationKind::Water { .. } | NavigationKind::Amphibious => {
                pos.y += f64::from(entity.entity.height()) * 0.5;
            }
        }
        pos
    }

    fn find_path_to(
        &mut self,
        mob: &MobEntity,
        destination: Vector3<f64>,
        reach_range: i32,
    ) -> Option<Path> {
        let entity = &mob.living_entity;
        if !self.can_update_path(entity) {
            return None;
        }
        let destination = if matches!(self.kind(), NavigationKind::Ground)
            && !self.can_path_to_targets_below_surface
        {
            let pos = Self::find_surface_position(
                &entity.entity.world.load(),
                BlockPos::floored_v(destination),
            );
            pos.0.to_f64()
        } else {
            destination
        };
        self.compute_path(mob, destination, reach_range)
    }

    // PathNavigation.recomputePath: no periodic recompute; defer requests until >20 game ticks.
    pub(super) fn recompute(&mut self, mob: &MobEntity) {
        let entity = &mob.living_entity;
        let time = entity.entity.world.load().get_world_age() as u64;
        if time.saturating_sub(self.time_last_recompute) <= MAX_TIME_RECOMPUTE
            || !self.can_update_path(entity)
        {
            self.has_delayed_recomputation = true;
        } else if let Some(target) = self.target_pos {
            self.path = self.find_path_to(mob, target.0.to_f64(), self.reach_range);
            self.time_last_recompute = time;
            self.has_delayed_recomputation = false;
        }
    }

    // PathNavigation.tick and FlyingPathNavigation.tick; queued requests are Pumpkin's moveTo adapter.
    pub(super) fn tick_navigation(&mut self, mob: &MobEntity, caller: &dyn EntityBase) {
        let entity = &mob.living_entity;
        self.tick_count += 1;
        self.next_ground_y = None;
        self.set_mob_dimensions(entity.entity.width(), entity.entity.height());
        if self.has_delayed_recomputation {
            self.recompute(mob);
        }
        self.process_pending_goal(mob);
        if self.path.as_ref().is_none_or(Path::is_done) {
            self.is_idle
                .store(self.current_goal.is_none(), Ordering::Relaxed);
            return;
        }
        let mob_pos = self.temp_mob_pos(entity);
        if self.can_update_path(entity) {
            self.follow_the_path(mob, caller, mob_pos);
        } else if let Some(path) = &mut self.path
            && let Some(target) = path.get_next_entity_pos(self.mob_width)
        {
            let at_node = if matches!(self.evaluator, EvaluatorKind::Fly(_)) {
                BlockPos::floored_v(entity.entity.pos.load()) == BlockPos::floored_v(target)
            } else {
                mob_pos.y > target.y
                    && !entity.entity.on_ground.load(Ordering::Relaxed)
                    && mob_pos.x.floor() == target.x.floor()
                    && mob_pos.z.floor() == target.z.floor()
            };
            if at_node {
                path.advance();
            }
        }
        if matches!(self.kind(), NavigationKind::Ground)
            && let Some((target, _)) = self.next_move_target()
        {
            self.next_ground_y = Some(Self::get_ground_y(&entity.entity.world.load(), target));
        }
        self.is_idle.store(
            self.path.as_ref().is_none_or(Path::is_done),
            Ordering::Relaxed,
        );
    }

    // Java moveTo installs its path before tick; Pumpkin queues coordinate requests.
    pub(super) fn process_pending_goal(&mut self, mob: &MobEntity) {
        let entity = &mob.living_entity;
        if let Some(goal) = self.current_goal.take() {
            self.path = self.find_path_to(mob, goal.destination, self.pending_reach_range);
            self.trim_path(entity);
            self.last_stuck_check = self.tick_count;
            self.last_stuck_check_pos = self.temp_mob_pos(entity);
        }
    }

    // PathNavigation.followThePath uses actual feet for reach checks, temporary position for corners.
    fn follow_the_path(&mut self, mob: &MobEntity, caller: &dyn EntityBase, mob_pos: Vector3<f64>) {
        self.max_distance_to_waypoint = max_distance_to_waypoint(self.mob_width);
        let vertical = if matches!(self.kind(), NavigationKind::Water { .. }) {
            0.5
        } else {
            1.0
        };
        let kind = self.kind();
        self.follow_the_path_at(
            mob.living_entity.entity.pos.load(),
            mob_pos,
            vertical,
            |target| Self::can_move_directly(kind, caller, mob_pos, target),
        );
        self.do_stuck_detection(mob_pos, mob);
    }

    fn follow_the_path_at(
        &mut self,
        entity_pos: Vector3<f64>,
        mob_pos: Vector3<f64>,
        vertical: f64,
        can_move_directly: impl FnOnce(Vector3<f64>) -> bool,
    ) {
        let advance = self.path.as_ref().is_some_and(|path| {
            path.get_next_node().is_some_and(|node| {
                waypoint_reached(entity_pos, node.pos, self.mob_width, vertical)
                    || self.can_cut_corner(node.path_type)
                        && should_target_next_node_in_direction(mob_pos, path, || {
                            path.get_next_entity_pos(self.mob_width)
                                .is_some_and(can_move_directly)
                        })
            })
        });
        if advance && let Some(path) = &mut self.path {
            path.advance();
        }
    }

    fn can_cut_corner(&self, path_type: PathType) -> bool {
        // FrogPathNavigation additionally refuses WATER_BORDER.
        !(matches!(
            path_type,
            PathType::DangerFire | PathType::DangerOther | PathType::WalkableDoor
        ) || path_type == PathType::WaterBorder
            && matches!(&self.evaluator, EvaluatorKind::Amphibious(e) if e.walk.is_frog))
    }

    fn can_move_directly(
        kind: NavigationKind,
        caller: &dyn EntityBase,
        start: Vector3<f64>,
        stop: Vector3<f64>,
    ) -> bool {
        let entity = caller.get_entity();
        let blocked_by_fluids = match kind {
            NavigationKind::Ground => return false,
            NavigationKind::Flying { .. } => true,
            NavigationKind::Water { .. } => false,
            NavigationKind::Amphibious => {
                if !entity.touching_water.load(Ordering::Relaxed)
                    && !entity.touching_lava.load(Ordering::Relaxed)
                {
                    return false;
                }
                false
            }
        };
        super::navigation_geometry::is_clear_for_movement_between(
            caller,
            start,
            stop,
            entity.height(),
            blocked_by_fluids,
        )
    }
}

fn max_distance_to_waypoint(width: f32) -> f32 {
    if width > 0.75 {
        width / 2.0
    } else {
        0.75 - width / 2.0
    }
}

pub(super) fn waypoint_reached(
    pos: Vector3<f64>,
    node: BlockPos,
    width: f32,
    vertical: f64,
) -> bool {
    let distance = f64::from(max_distance_to_waypoint(width));
    (pos.x - (f64::from(node.0.x) + 0.5)).abs() < distance
        && (pos.z - (f64::from(node.0.z) + 0.5)).abs() < distance
        && (pos.y - f64::from(node.0.y)).abs() < vertical
}

fn should_target_next_node_in_direction(
    mob_pos: Vector3<f64>,
    path: &Path,
    can_move_directly: impl FnOnce() -> bool,
) -> bool {
    let Some(current) = path.get_next_node_pos() else {
        return false;
    };
    let Some(next) = path.get_node_pos(path.get_next_node_index() + 1) else {
        return false;
    };
    let to_current = current.0.to_f64() + Vector3::new(0.5, 0.0, 0.5) - mob_pos;
    if to_current.length_squared() >= 4.0 {
        return false;
    }
    if can_move_directly() {
        return true;
    }
    let to_next = next.0.to_f64() + Vector3::new(0.5, 0.0, 0.5) - mob_pos;
    if to_next.length_squared() >= to_current.length_squared() && to_current.length_squared() >= 0.5
    {
        return false;
    }
    // Normalization cannot change the sign of this dot product (Vec3.normalize returns zero near zero).
    to_current.length() >= 1e-5 && to_next.length() >= 1e-5 && to_current.dot(&to_next) < 0.0
}

// WallClimberNavigation.tick uses full block centers, including Y, for its reach test.
pub(super) fn wall_target_reached(pos: Vector3<f64>, target: BlockPos, width: f32) -> bool {
    let center = target.0.to_f64() + Vector3::new(0.5, 0.5, 0.5);
    let radius_sq = f64::from(width) * f64::from(width);
    (center - pos).length_squared() < radius_sq
        || pos.y > f64::from(target.0.y)
            && (Vector3::new(center.x, pos.y.floor() + 0.5, center.z) - pos).length_squared()
                < radius_sq
}

#[cfg(test)]
#[path = "navigation_tests.rs"]
mod tests;
