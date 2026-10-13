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
use pumpkin_util::math::vector3::Vector3;

impl Player {
    pub fn sleep(&self, bed_head_pos: BlockPos) {
        // TODO: Stop riding

        let world = self.world();
        let Some(position) =
            crate::block::blocks::abstract_bed::sleep_position(&world, bed_head_pos)
        else {
            return;
        };
        let (block, state) = world.get_block_and_state_id(&bed_head_pos);
        crate::block::blocks::bed::BedBlock::set_occupied(
            true,
            &world,
            block,
            &bed_head_pos,
            state,
        );
        // LivingEntity.startSleeping uses the bed species' sleep height before syncing its pose.
        self.get_entity().set_pose(EntityPose::Sleeping);
        self.get_entity().set_pos(position);
        self.get_entity().set_synced_data(
            pumpkin_data::tracked_data::player::SLEEPING_POS_ID,
            Some(bed_head_pos),
        );
        self.get_entity().set_velocity(Vector3::default());

        self.sleeping_since.store(Some(0));
        self.sleeping_bed_pos.store(Some(bed_head_pos));
        self.set_stat(
            statistics::StatisticCategory::Custom,
            statistics::CustomStatistic::TimeSinceRest as i32,
            0,
        );
        // ServerPlayer.startSleeping teleports the owner after LivingEntity.startSleeping.
        let _ = self.request_teleport(
            position,
            self.get_entity().yaw.load(),
            self.get_entity().pitch.load(),
        );
    }

    /// Starts bed sleep using the vanilla server admission and side-effect order.
    pub(crate) fn start_sleep_in_bed(
        self: &std::sync::Arc<Self>,
        world: &std::sync::Arc<crate::world::World>,
        block: &Block,
        bed: (BlockPos, BlockPos, pumpkin_data::BlockStateId),
        rule: pumpkin_data::dimension::BedRule,
    ) -> crate::block::registry::BlockActionResult {
        use pumpkin_data::translation;

        let (bed_head_pos, bed_foot_pos, _) = bed;
        // ServerPlayer.startSleepInBed checks reach and obstruction before setting spawn.
        let is_dark = world.is_dark_outside();
        let can_sleep = rule.can_sleep(is_dark);
        let can_set_spawn = rule.can_set_spawn(is_dark);
        if !self.living_entity.is_alive() || self.is_sleeping() {
            return crate::block::registry::BlockActionResult::SuccessServer;
        }
        if let Some(message) = crate::block::blocks::abstract_bed::sleep_admission(
            world,
            self,
            bed_head_pos,
            bed_foot_pos,
        ) {
            self.send_system_message_raw(&message, true);
            return crate::block::registry::BlockActionResult::SuccessServer;
        }

        if can_set_spawn
            && self.set_respawn_point(
                world.dimension.clone(),
                bed_head_pos,
                self.get_entity().yaw.load(),
                self.get_entity().pitch.load(),
                false,
            )
        {
            self.send_system_message(&pumpkin_macros::translate_cross!(
                translation::java::BLOCK_MINECRAFT_SET_SPAWN,
                translation::bedrock::TILE_BED_RESPAWNSET
            ));
        }

        if !can_sleep {
            if let Some(message) = sleep_refusal_message(rule) {
                self.send_system_message_raw(&message, true);
            }
            return crate::block::registry::BlockActionResult::SuccessServer;
        }

        if crate::block::blocks::abstract_bed::monsters_prevent_sleep(world, self, bed_head_pos) {
            self.send_system_message_raw(
                &pumpkin_macros::translate_cross!(
                    translation::java::BLOCK_MINECRAFT_BED_NOT_SAFE,
                    translation::bedrock::TILE_BED_NOTSAFE
                ),
                true,
            );
            return crate::block::registry::BlockActionResult::SuccessServer;
        }

        if let Some(server) = world.server.upgrade() {
            let mut event =
                crate::plugin::api::events::player::player_bed::PlayerBedEnterEvent::new(
                    self.clone(),
                    bed_head_pos,
                );
            server.plugin_manager.fire_blocking(&server, &mut event);
            if event.cancelled {
                return crate::block::registry::BlockActionResult::SuccessServer;
            }
        }

        self.sleep(bed_head_pos);
        if !self.is_sleeping() {
            return crate::block::registry::BlockActionResult::SuccessServer;
        }
        if can_set_spawn {
            self.trigger_advancement(
                crate::entity::player::advancement::trigger::AdvancementTrigger::SleptInBed,
            );
        }
        let statistic = if block == &Block::STRAW_BED {
            statistics::CustomStatistic::SleepInStrawBed
        } else {
            statistics::CustomStatistic::SleepInBed
        };
        self.increment_stat(statistics::StatisticCategory::Custom, statistic as i32, 1);

        crate::block::registry::BlockActionResult::SuccessServer
    }

    // Pumpkin's Player::tick validates sleep before advancing the vanilla SleepTimer.
    /// Wakes an invalid sleeper before advancing or counting its sleep timer.
    pub(crate) fn tick_sleep_lifecycle(&self) {
        if !self.is_sleeping() {
            return;
        }
        let world = self.world();
        let valid = self.living_entity.is_alive()
            && self.gamemode.load() != pumpkin_util::gamemode::GameMode::Spectator
            && self.sleeping_bed_pos.load().is_some_and(|head| {
                crate::block::blocks::abstract_bed::sleep_position(&world, head).is_some()
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
        let position = if is_bed {
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
            position
        } else {
            // LivingEntity.stopSleeping uses its above-bed fallback if the saved bed is gone.
            crate::block::blocks::abstract_bed::stand_up_position(
                &world,
                self.get_entity(),
                bed_pos,
            )
        };
        self.get_entity().set_pos(position);
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

fn sleep_refusal_message(
    rule: pumpkin_data::dimension::BedRule,
) -> Option<pumpkin_util::text::TextComponent> {
    use pumpkin_data::translation;

    // ServerPlayer.startSleepInBed returns BedRule.asProblem's optional component.
    let mut message = rule
        .error_message
        .and_then(|json| serde_json::from_str::<pumpkin_util::text::TextComponent>(json).ok())?;
    // BedRule.CAN_SLEEP_WHEN_DARK's Java key needs its Bedrock counterpart.
    if let pumpkin_util::text::TextContent::Translate {
        translate,
        bedrock_translate,
        ..
    } = &mut *message.0.content
        && translate.as_ref() == translation::java::BLOCK_MINECRAFT_BED_NO_SLEEP
    {
        *bedrock_translate = Some(translation::bedrock::TILE_BED_NOSLEEP.into());
    }
    Some(message)
}
