//! Observes the existing Navigator interface while forwarding every operation to it.
use super::*;
use crate::entity::{
    ai::pathfinder::{Navigator, NavigatorGoal, PathNavigationTrait, node::PathType, path::Path},
    living::LivingEntity,
    mob::MobEntity,
};
use crossbeam::atomic::AtomicCell;

struct NavigationWitness {
    inner: Navigator,
    dimensions: Arc<AtomicCell<(f32, f32)>>,
    dimensions_action: Option<Box<dyn FnOnce() + Send + Sync>>,
    stop_action: Option<Box<dyn FnOnce() + Send + Sync>>,
}
impl PathNavigationTrait for NavigationWitness {
    fn set_progress(&mut self, goal: NavigatorGoal) {
        self.inner.set_progress(goal);
    }
    fn set_speed(&mut self, speed: f64) {
        self.inner.set_speed(speed);
    }
    fn stop(&mut self) {
        self.inner.stop();
        if let Some(action) = self.stop_action.take() {
            action();
        }
    }
    fn is_idle(&self) -> bool {
        self.inner.is_idle()
    }
    fn is_done(&self) -> bool {
        self.inner.is_done()
    }
    fn is_in_progress(&self) -> bool {
        self.inner.is_in_progress()
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
        self.dimensions.store((width, height));
        self.inner.set_mob_dimensions(width, height);
        if let Some(action) = self.dimensions_action.take() {
            action();
        }
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
        self.inner.tick(mob, caller);
    }
    fn path_type_at(&mut self, entity: &LivingEntity, pos: BlockPos) -> PathType {
        self.inner.path_type_at(entity, pos)
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
        self.inner.move_to_coords(x, y, z, speed, entity)
    }
    fn move_to_pos(&mut self, pos: BlockPos, speed: f64, entity: &LivingEntity) -> bool {
        self.inner.move_to_pos(pos, speed, entity)
    }
    fn move_to_entity(&mut self, target: &LivingEntity, speed: f64, mob: &MobEntity) -> bool {
        self.inner.move_to_entity(target, speed, mob)
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
        self.inner.can_navigate_ground()
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
    fn is_stable_destination(
        &self,
        world: &World,
        pos: &BlockPos,
        entity: &dyn EntityBase,
    ) -> bool {
        self.inner.is_stable_destination(world, pos, entity)
    }
}

pub(super) fn watch(villager: &VillagerEntity) -> Arc<AtomicCell<(f32, f32)>> {
    watch_actions(villager, None, None)
}

pub(super) fn watch_actions(
    villager: &VillagerEntity,
    dimensions_action: Option<Box<dyn FnOnce() + Send + Sync>>,
    stop_action: Option<Box<dyn FnOnce() + Send + Sync>>,
) -> Arc<AtomicCell<(f32, f32)>> {
    let dimensions = Arc::new(AtomicCell::new((
        villager.get_entity().width(),
        villager.get_entity().height(),
    )));
    let placeholder = Navigator::ground();
    let mut navigation = villager.mob_entity.navigator.lock().unwrap();
    let inner = std::mem::replace(&mut *navigation, placeholder);
    *navigation = Navigator::new(NavigationWitness {
        inner,
        dimensions: dimensions.clone(),
        dimensions_action,
        stop_action,
    });
    dimensions
}
