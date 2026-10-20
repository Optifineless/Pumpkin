use std::{
    cell::Cell,
    sync::{Arc, atomic::Ordering::Relaxed},
};

use pumpkin_data::{
    Block, BlockStateId,
    block_properties::{BedPart, WhiteBedLikeProperties},
    entity::EntityPose,
    tag::{self, Taggable},
    tracked_data,
};
use pumpkin_protocol::java::client::play::{Animation, CEntityAnimation};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};

use super::{Player, bed_respawn, stand_up, statistics};
use crate::{
    block::{blocks::straw_bed::StrawBedBlock, entities::BlockEntity},
    entity::EntityBase,
    plugin::api::events::player::player_bed::PlayerBedLeaveEvent,
    world::World,
};

struct WakeContext {
    operation: u64,
    world: Arc<World>,
    bed_pos: Option<BlockPos>,
    life: u64,
    registered: bool,
    movement: WakeMovement,
}

struct WakeMovement {
    position: Vector3<f64>,
    yaw: u32,
    pitch: u32,
    teleport: i32,
}

impl WakeMovement {
    fn capture(player: &Player) -> Self {
        let entity = player.get_entity();
        Self {
            position: player.position(),
            yaw: entity.yaw.load().to_bits(),
            pitch: entity.pitch.load().to_bits(),
            teleport: player.teleport_id_count.load(Relaxed),
        }
    }

    fn is_current(&self, player: &Player) -> bool {
        let entity = player.get_entity();
        player.position() == self.position
            && entity.yaw.load().to_bits() == self.yaw
            && entity.pitch.load().to_bits() == self.pitch
            && player.teleport_id_count.load(Relaxed) == self.teleport
    }
}

impl WakeContext {
    fn capture(player: &Player, world: &Arc<World>, operation: u64) -> Self {
        Self {
            operation,
            world: world.clone(),
            bed_pos: player.sleeping_bed_pos.load(),
            life: player.living_entity.damage_lifecycle(),
            registered: world.get_player_by_uuid(player.gameprofile.id).is_some(),
            movement: WakeMovement::capture(player),
        }
    }

    fn is_current(&self, player: &Player) -> bool {
        self.is_current_at(player, self.bed_pos)
    }

    fn is_current_at(&self, player: &Player, bed_pos: Option<BlockPos>) -> bool {
        player.sleep_operation.load(Relaxed) == self.operation
            && player.sleeping_bed_pos.load() == bed_pos
            && player.living_entity.damage_lifecycle() == self.life
            && Arc::ptr_eq(&player.world(), &self.world)
            && self
                .world
                .get_player_by_uuid(player.gameprofile.id)
                .map_or(!self.registered, |current| {
                    std::ptr::eq(Arc::as_ptr(&current), player)
                })
    }
}

struct BedHalf {
    position: BlockPos,
    state: BlockStateId,
    entity: Option<Arc<dyn BlockEntity>>,
}

impl BedHalf {
    fn capture(world: &World, position: BlockPos) -> Option<Self> {
        Some(Self {
            position,
            state: world.get_block_state_id_if_loaded(&position)?,
            entity: world.get_block_entity(&position),
        })
    }

    fn capture_registered(world: &World, position: BlockPos) -> Option<Self> {
        Some(Self {
            position,
            state: world.get_block_state_id_if_loaded(&position)?,
            entity: world
                .block_entities
                .get(&position.chunk_position())
                .and_then(|entities| entities.get(&position).cloned()),
        })
    }

    fn unoccupied(&self) -> Self {
        let mut properties = WhiteBedLikeProperties::from_state_id(self.state);
        properties.occupied = false;
        Self {
            position: self.position,
            state: properties.to_state_id(Block::from_state_id(self.state)),
            entity: self.entity.clone(),
        }
    }

    fn is_current(&self, world: &World) -> bool {
        // Registry identity checks must not promote lazy data or call a native block entity.
        let current = world
            .block_entities
            .get(&self.position.chunk_position())
            .and_then(|entities| entities.get(&self.position).cloned());
        world.get_block_state_id_if_loaded(&self.position) == Some(self.state)
            && match (&self.entity, current) {
                (Some(expected), Some(current)) => Arc::ptr_eq(expected, &current),
                (None, None) => true,
                _ => false,
            }
    }
}

pub struct BedSleepAdmission {
    context: WakeContext,
    head: BedHalf,
    foot: BedHalf,
    height: f64,
}

impl BedSleepAdmission {
    fn is_current(&self, player: &Player) -> bool {
        let occupied = WhiteBedLikeProperties::from_state_id(self.head.state).occupied
            || WhiteBedLikeProperties::from_state_id(self.foot.state).occupied;
        let owns_sleep = self.context.bed_pos == Some(self.head.position)
            && player.sleeping_since.load().is_some();
        self.context.is_current(player)
            && (!occupied || owns_sleep)
            && self
                .context
                .bed_pos
                .is_none_or(|position| position == self.head.position)
            && player.living_entity.is_alive()
            && !player.get_entity().is_removed()
            && self.head.is_current(&self.context.world)
            && self.foot.is_current(&self.context.world)
    }

    const fn halves(&self) -> [(BlockPos, BlockStateId); 2] {
        [
            (self.head.position, self.head.state),
            (self.foot.position, self.foot.state),
        ]
    }
}

impl Player {
    pub(super) fn saved_sleep_counter(&self) -> i16 {
        self.sleeping_since
            .load()
            .map_or_else(|| self.persisted_sleep_counter.load(Relaxed), i16::from)
    }

    pub(super) fn invalidate_sleep_operation(&self) {
        // Even revisions admit a transition; odd revisions reserve one without holding a lock.
        let _ = self
            .sleep_operation
            .fetch_update(Relaxed, Relaxed, |old| Some(old.wrapping_add(2) & !1));
    }

    pub(crate) fn bed_sleep_admission(
        &self,
        world: &Arc<World>,
        head_pos: BlockPos,
        expected: BlockStateId,
    ) -> Option<BedSleepAdmission> {
        let operation = self.sleep_operation.load(Relaxed);
        if operation & 1 != 0 || !Arc::ptr_eq(&self.world(), world) {
            return None;
        }
        let head = BedHalf::capture(world, head_pos)?;
        if head.state != expected {
            return None;
        }
        let (foot_pos, _) = bed_partner(world, head_pos, expected)?;
        let foot = BedHalf::capture(world, foot_pos)?;
        let laying_on = if Block::from_state_id(expected) == &Block::STRAW_BED
            && WhiteBedLikeProperties::from_state_id(expected).part == BedPart::Head
        {
            &foot
        } else {
            &head
        };
        // AbstractBedBlock.getSleepHeight; StrawBedBlock selects the foot's shape.
        let mut height = f64::NEG_INFINITY;
        world.for_each_collision_shape(
            laying_on.state.to_state(),
            &laying_on.position,
            None,
            &mut |shape| {
                height = height.max(shape.max.y);
            },
        );
        let admission = BedSleepAdmission {
            context: WakeContext::capture(self, world, operation),
            head,
            foot,
            height,
        };
        (height.is_finite() && admission.is_current(self)).then_some(admission)
    }

    pub(crate) fn bed_sleep_admission_is_current(&self, admission: &BedSleepAdmission) -> bool {
        admission.is_current(self)
    }

    pub(crate) fn start_sleeping_in_bed(
        &self,
        mut admission: BedSleepAdmission,
        revalidate: impl Fn() -> bool,
    ) -> bool {
        if !revalidate()
            || !admission.is_current(self)
            || self
                .sleep_operation
                .compare_exchange(
                    admission.context.operation,
                    admission.context.operation.wrapping_add(1),
                    Relaxed,
                    Relaxed,
                )
                .is_err()
        {
            return false;
        }
        admission.context.operation = admission.context.operation.wrapping_add(1);
        let entity = self.get_entity();
        // LivingEntity.startSleeping stops riding before changing pose or occupying the bed.
        if let Some(vehicle) = entity.get_vehicle() {
            vehicle.get_entity().remove_passenger(self.entity_id());
        }
        if !revalidate() || !admission.is_current(self) || entity.has_vehicle() {
            self.release_sleep_operation(admission.context.operation);
            return false;
        }
        let did_commit = Cell::new(false);
        let commit = || {
            let committed = self.commit_sleep_admission(&admission, &revalidate);
            did_commit.set(committed);
            committed
        };
        if entity.pose.load() == EntityPose::Sleeping {
            commit();
        } else {
            // Commit the bed and remembered position only after the pose callback admits sleep.
            entity.set_pose_if(EntityPose::Sleeping, commit);
        }
        let committed = did_commit.get()
            && admission
                .context
                .is_current_at(self, Some(admission.head.position))
            && self.sleeping_since.load().is_some()
            && entity.pose.load() == EntityPose::Sleeping;
        if committed {
            entity.set_synced_data(
                tracked_data::player::SLEEPING_POS_ID,
                Some(admission.head.position),
            );
            entity.set_velocity(Vector3::default());
            self.set_stat(
                statistics::StatisticCategory::Custom,
                statistics::CustomStatistic::TimeSinceRest as i32,
                0,
            );
            self.correct_sleep_position(&admission.context.world);
        }
        self.release_sleep_operation(admission.context.operation);
        committed
    }

    fn commit_sleep_admission(
        &self,
        admission: &BedSleepAdmission,
        revalidate: &impl Fn() -> bool,
    ) -> bool {
        if !revalidate()
            || !admission.is_current(self)
            || !admission
                .context
                .world
                .set_sleep_bed_occupied_if(&admission.halves(), true, || admission.is_current(self))
        {
            return false;
        }
        self.get_entity()
            .set_pos(
                admission
                    .head
                    .position
                    .to_f64()
                    .add_raw(0.0, admission.height + 0.125, 0.0),
            );
        self.sleeping_since.store(Some(0));
        self.sleeping_bed_pos.store(Some(admission.head.position));
        self.persisted_sleep_counter.store(0, Relaxed);
        true
    }

    pub(super) fn sleep_at_bed(&self, bed_head_pos: BlockPos) {
        if self.get_entity().is_removed()
            || !self.living_entity.is_alive()
            || self
                .sleeping_bed_pos
                .load()
                .is_some_and(|position| position != bed_head_pos)
        {
            return;
        }
        // The public native API can supersede an in-flight wake from its callback.
        self.invalidate_sleep_operation();
        let world = self.world();
        if let Some(state) = world.get_block_state_id_if_loaded(&bed_head_pos)
            && let Some(admission) = self.bed_sleep_admission(&world, bed_head_pos, state)
        {
            self.start_sleeping_in_bed(admission, || true);
        }
    }

    pub(super) fn tick_sleeping(&self) {
        // LivingEntity.tick / checkBedExists: validate the remembered bed before advancing sleep.
        let revision = self.sleep_operation.load(Relaxed);
        if let Some(position) = self.sleeping_bed_pos.load() {
            let bed = self.world().get_block(&position);
            if !is_bed(bed) {
                self.stop_sleeping_at_revision(true, revision, false, true);
            }
        }
        if let Some(timer) = self.sleeping_since.load()
            && timer < 101
        {
            self.sleeping_since.store(Some(timer + 1));
        }
    }

    pub(super) fn stop_sleeping(&self, removed_bed: bool) {
        self.stop_sleeping_at_revision(
            removed_bed,
            self.sleep_operation.load(Relaxed),
            false,
            true,
        );
    }

    pub(super) fn stop_sleeping_on_departure(&self) {
        // Close entry admission before leave callbacks can try to sleep again.
        self.mark_departing();
        if self.sleeping_bed_pos.load().is_none() && self.sleeping_since.load().is_none() {
            return;
        }
        let notify_events = self.sleep_operation.load(Relaxed) & 1 == 0;
        if !notify_events {
            // Retirement supersedes an in-flight callback without recursively dispatching it.
            self.invalidate_sleep_operation();
        }
        self.stop_sleeping_at_revision(
            false,
            self.sleep_operation.load(Relaxed),
            true,
            notify_events,
        );
    }

    fn stop_sleeping_at_revision(
        &self,
        removed_bed: bool,
        revision: u64,
        departure: bool,
        notify_events: bool,
    ) {
        if self.sleeping_bed_pos.load().is_none() && self.sleeping_since.load().is_none() {
            return;
        }
        if revision & 1 != 0
            || self
                .sleep_operation
                .compare_exchange(revision, revision.wrapping_add(1), Relaxed, Relaxed)
                .is_err()
        {
            return;
        }
        let context = WakeContext::capture(self, &self.world(), revision.wrapping_add(1));
        self.finish_stop_sleeping(&context, removed_bed, departure, notify_events);
        self.release_sleep_operation(context.operation);
    }

    fn release_sleep_operation(&self, operation: u64) {
        let _ = self.sleep_operation.compare_exchange(
            operation,
            operation.wrapping_add(1),
            Relaxed,
            Relaxed,
        );
    }

    fn finish_stop_sleeping(
        &self,
        context: &WakeContext,
        removed_bed: bool,
        departure: bool,
        notify_events: bool,
    ) {
        if !context.is_current(self) {
            return;
        }
        let capture = |pos| {
            if notify_events {
                BedHalf::capture(&context.world, pos)
            } else {
                BedHalf::capture_registered(&context.world, pos)
            }
        };
        let bed = context.bed_pos.and_then(capture);
        let partner = bed
            .as_ref()
            .and_then(|half| bed_partner(&context.world, half.position, half.state))
            .and_then(|(pos, _)| capture(pos));
        if !context.is_current(self) {
            return;
        }
        if notify_events
            && let Some(bed_pos) = context.bed_pos
            && let Some(server) = context.world.server.upgrade()
            && let Some(player) = context.world.get_player_by_uuid(self.gameprofile.id)
        {
            let mut event = PlayerBedLeaveEvent::new(player, bed_pos);
            server.plugin_manager.fire_blocking(&server, &mut event);
        }
        if !context.is_current(self) {
            return;
        }
        let entity = self.get_entity();
        let commit = || {
            self.commit_stop_sleeping(
                context,
                removed_bed,
                departure,
                bed.as_ref(),
                partner.as_ref(),
            )
        };
        if notify_events {
            if entity.pose.load() == EntityPose::Standing {
                commit();
            } else {
                entity.set_pose_if(EntityPose::Standing, commit);
            }
        }
        // Retirement cannot leave a bed reserved by a player who is no longer registered.
        if departure && context.is_current(self) {
            commit();
            if context.is_current_at(self, None) {
                self.force_retired_standing();
            }
        }
        if context.is_current_at(self, None) && self.sleeping_since.load().is_none() {
            entity.set_synced_data(tracked_data::player::SLEEPING_POS_ID, None::<BlockPos>);
            if !removed_bed {
                self.finish_wake_position(context, bed.as_ref(), departure);
            }
            if !context.is_current_at(self, None) {
                return;
            }
            if context.bed_pos.is_some() {
                context.world.broadcast_to_chunk(
                    entity.chunk_pos.load(),
                    &CEntityAnimation::new(self.entity_id().into(), Animation::LeaveBed),
                );
            }
            if !departure {
                self.correct_sleep_position(&context.world);
            }
        }
    }

    fn commit_stop_sleeping(
        &self,
        context: &WakeContext,
        removed_bed: bool,
        departure: bool,
        bed: Option<&BedHalf>,
        partner: Option<&BedHalf>,
    ) -> bool {
        if !context.is_current(self) {
            return false;
        }
        if !removed_bed
            && let Some(bed) = bed.filter(|bed| {
                is_bed(Block::from_state_id(bed.state)) && bed.is_current(&context.world)
            })
        {
            if !self.release_sleep_bed(context, bed, partner) {
                return false;
            }
            if !context.is_current(self) {
                return false;
            }
        }
        self.persisted_sleep_counter
            .store(if removed_bed || departure { 0 } else { 100 }, Relaxed);
        self.sleeping_since.store(None);
        self.sleeping_bed_pos.store(None);
        true
    }

    fn release_sleep_bed(
        &self,
        context: &WakeContext,
        bed: &BedHalf,
        partner: Option<&BedHalf>,
    ) -> bool {
        partner
            .filter(|half| half.is_current(&context.world))
            .map_or_else(
                || {
                    // LivingEntity.stopSleeping also releases a surviving remembered half.
                    context.world.set_sleep_bed_half_occupied_if(
                        (bed.position, bed.state),
                        false,
                        || context.is_current(self) && bed.is_current(&context.world),
                    )
                },
                |partner| {
                    context.world.set_sleep_bed_occupied_if(
                        &[(bed.position, bed.state), (partner.position, partner.state)],
                        false,
                        || {
                            context.is_current(self)
                                && bed.is_current(&context.world)
                                && partner.is_current(&context.world)
                        },
                    )
                },
            )
    }

    fn finish_wake_position(&self, context: &WakeContext, bed: Option<&BedHalf>, departure: bool) {
        let Some(bed) = bed
            .filter(|bed| is_bed(Block::from_state_id(bed.state)))
            .map(BedHalf::unoccupied)
        else {
            return;
        };
        if !bed.is_current(&context.world) {
            return;
        }
        if Block::from_state_id(bed.state) == &Block::STRAW_BED
            && context.world.dimension.straw_bed_rule.destroy_on_leave
        {
            // Break callbacks see the completed wake, with no pose or sleeping membership left.
            StrawBedBlock::destroy_after_use(&context.world, bed.position);
        }
        if departure
            || !context.is_current_at(self, None)
            || !context.movement.is_current(self)
            || (!bed.is_current(&context.world)
                && context.world.get_block_state_id_if_loaded(&bed.position)
                    != Some(BlockStateId::AIR))
        {
            return;
        }
        let facing = WhiteBedLikeProperties::from_state_id(bed.state).facing;
        let Some(wake_cell) = BedHalf::capture_registered(&context.world, bed.position) else {
            return;
        };
        let position = bed_respawn::find_bed_spawn_position(
            &context.world,
            bed.position,
            facing,
            self.get_entity().yaw.load(),
        )
        .unwrap_or_else(|| bed.position.up().to_f64().add_raw(0.0, 0.1, 0.0));
        if !context.is_current_at(self, None)
            || !context.movement.is_current(self)
            || self.get_entity().is_removed()
            || !wake_cell.is_current(&context.world)
        {
            return;
        }
        self.get_entity().set_pos(position);
        self.get_entity()
            .set_rotation(stand_up::look_at_yaw(position, bed.position), 0.0);
    }
}

fn is_bed(block: &Block) -> bool {
    block == &Block::STRAW_BED || block.has_tag(&tag::Block::MINECRAFT_BEDS)
}

fn bed_partner(
    world: &World,
    position: BlockPos,
    state: BlockStateId,
) -> Option<(BlockPos, BlockStateId)> {
    let bed = Block::from_state_id(state);
    if !is_bed(bed) {
        return None;
    }
    let properties = WhiteBedLikeProperties::from_state_id(state);
    let offset = if properties.part == BedPart::Head {
        properties.facing.opposite()
    } else {
        properties.facing
    };
    let partner = position.offset(offset.to_offset());
    let expected = world.get_block_state_id_if_loaded(&partner)?;
    if Block::from_state_id(expected) != bed {
        return None;
    }
    let other = WhiteBedLikeProperties::from_state_id(expected);
    (other.part != properties.part && other.facing == properties.facing)
        .then_some((partner, expected))
}

#[path = "sleeping/position_sync.rs"]
mod position_sync;

#[cfg(test)]
mod tests;
