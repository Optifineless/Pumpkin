use super::{BlockPos, MobEntity, Path, PathFinder, PathNavigation};

impl PathNavigation {
    pub(super) fn compute_path_to_targets(
        &mut self,
        mob: &MobEntity,
        targets: &[BlockPos],
        reach_range: i32,
    ) -> Option<Path> {
        // PathNavigation.createPath(Set<BlockPos>, int) / AcquirePoi.findPathToPois.
        let living = &mob.living_entity;
        if targets.is_empty() || !self.can_update_path(living) {
            return None;
        }
        self.prepare_evaluator(mob);
        let max_range = self.mob_max_follow_range(living);
        // PathNavigation.updatePathfinderMaxVisitedNodes scales the search budget with range.
        let mut finder = PathFinder::new((max_range * 16.0).floor() as usize);
        let path = finder.find_path(
            &mut self.evaluator,
            targets,
            max_range,
            reach_range,
            self.max_visited_nodes_multiplier,
        );
        self.evaluator.done();
        if let Some(path) = &path {
            self.target_pos = Some(path.get_target());
            self.reach_range = reach_range;
            self.reset_stuck_timeout();
        }
        path
    }
}
