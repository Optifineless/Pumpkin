use super::Player;
use crate::entity::EntityBase;
use pumpkin_data::{
    Block,
    block_properties::WhiteBedLikeProperties,
    entity::EntityPose,
    statistic as statistics,
    tag::{self, Taggable},
};
use pumpkin_protocol::java::client::play::{Animation, CEntityAnimation};
use pumpkin_util::math::position::BlockPos;

impl Player {
    // LivingEntity.tick/checkBedExists and Player.tick validate sleep before advancing its timer.
    /// Wakes an invalid sleeper before advancing or counting its sleep timer.
    pub(crate) fn tick_sleep_lifecycle(&self) {
        if !self.is_sleeping() {
            return;
        }
        let world = self.world();
        let valid = self.sleeping_bed_pos.load().is_some_and(|head| {
            crate::block::blocks::abstract_bed::sleep_position(&world, head).is_some()
                && crate::block::blocks::abstract_bed::bed_rule(&world, world.get_block(&head))
                    .can_sleep(world.is_dark_outside())
        });
        if !valid {
            self.wake_up();
        }
    }

    pub(super) fn stop_sleeping(&self) {
        let world = self.world();
        let Some(bed_pos) = self.sleeping_bed_pos.load() else {
            self.living_entity.entity.set_pose(EntityPose::Standing);
            self.sleeping_since.store(None);
            return;
        };

        if let Some(server) = world.server.upgrade()
            && let Some(player_arc) = world.get_player_by_uuid(self.gameprofile.id)
        {
            let mut event =
                crate::plugin::api::events::player::player_bed::PlayerBedLeaveEvent::new(
                    player_arc, bed_pos,
                );
            server.plugin_manager.fire_blocking(&server, &mut event);
        }

        let (bed, bed_state) = world.get_block_and_state_id(&bed_pos);
        let is_bed = bed == &Block::STRAW_BED || bed.has_tag(&tag::Block::MINECRAFT_BEDS);
        if is_bed {
            let facing = WhiteBedLikeProperties::from_state_id(bed_state).facing;
            crate::block::blocks::bed::BedBlock::set_occupied(
                false, &world, bed, &bed_pos, bed_state,
            );
            let rule = crate::block::blocks::abstract_bed::bed_rule(&world, bed);
            // LivingEntity.stopSleeping calls onStopSleeping before finding the standing position.
            if rule.destroy_on_leave && bed == &Block::STRAW_BED {
                crate::block::blocks::straw_bed::StrawBedBlock::destroy_after_use(&world, bed_pos);
            }
            let position = crate::block::blocks::abstract_bed::stand_up_position_with_facing(
                &world,
                self.get_entity(),
                bed_pos,
                facing,
            );
            let direction = bed_pos.to_f64().add_raw(0.5, 0.0, 0.5) - position;
            let yaw = ((direction.z.atan2(direction.x).to_degrees() - 90.0 + 180.0)
                .rem_euclid(360.0)
                - 180.0) as f32;
            self.get_entity().set_rotation(yaw, 0.0);
            self.get_entity().set_pos(position);
        }
        self.get_entity().set_pose(EntityPose::Standing);
        // ServerPlayer.stopSleepInBed corrects the owner's client prediction after standing up.
        let _ = self.request_teleport(
            self.position(),
            self.get_entity().yaw.load(),
            self.get_entity().pitch.load(),
        );
        self.living_entity.entity.set_synced_data(
            pumpkin_data::tracked_data::player::SLEEPING_POS_ID,
            None::<BlockPos>,
        );

        self.set_stat(
            statistics::StatisticCategory::Custom,
            statistics::CustomStatistic::TimeSinceRest as i32,
            0,
        );

        let chunk_pos = self.living_entity.entity.chunk_pos.load();
        world.broadcast_to_chunk(
            chunk_pos,
            &CEntityAnimation::new(self.entity_id().into(), Animation::LeaveBed),
        );

        self.sleeping_since.store(None);
        self.sleeping_bed_pos.store(None);
    }
}
