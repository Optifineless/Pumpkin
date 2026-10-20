use std::sync::atomic::Ordering::Relaxed;

use pumpkin_data::{entity::EntityPose, tracked_data};
use pumpkin_protocol::{
    VarInt,
    bedrock::client::{
        move_player::CMovePlayer,
        set_actor_data::{MetadataValue, SyncedActorDataList, entity_data_key},
    },
    codec::var_ulong::VarULong,
    java::client::play::CEntityPositionSync,
};
use pumpkin_util::math::{boundingbox::BoundingBox, vector3::Vector3};

use super::Player;
use crate::{entity::EntityBase, world::World};

impl Player {
    pub(super) fn force_retired_standing(&self) {
        let entity = self.get_entity();
        let dimensions = crate::entity::Entity::get_entity_dimensions(EntityPose::Standing);
        let position = entity.pos.load();
        entity.pose.store(EntityPose::Standing);
        entity.entity_dimension.store(dimensions);
        entity.bounding_box.store(BoundingBox::new_from_pos(
            position.x,
            position.y,
            position.z,
            &dimensions,
        ));
        entity.set_synced_data(
            tracked_data::entity::DATA_POSE,
            VarInt(EntityPose::Standing as i32),
        );
        let mut bedrock = SyncedActorDataList::new();
        bedrock.set(
            entity_data_key::POSE_INDEX,
            MetadataValue::Int(EntityPose::Standing as i32),
        );
        bedrock.set(
            entity_data_key::WIDTH,
            MetadataValue::Float(dimensions.width),
        );
        bedrock.set(
            entity_data_key::HEIGHT,
            MetadataValue::Float(dimensions.height),
        );
        entity.send_bedrock_actor_data(&bedrock);
    }

    pub(super) fn correct_sleep_position(&self, world: &World) {
        let entity = self.get_entity();
        let position = entity.pos.load();
        let yaw = entity.yaw.load();
        let pitch = entity.pitch.load();
        // ServerPlayer.startSleeping/stopSleepInBed correct the owner without another teleport event.
        self.request_accepted_teleport(position, yaw, pitch);
        entity.last_sent_pos.store(position);
        entity
            .last_sent_yaw
            .store(pumpkin_util::math::pack_degrees(yaw), Relaxed);
        entity
            .last_sent_head_yaw
            .store(pumpkin_util::math::pack_degrees(yaw), Relaxed);
        entity
            .last_sent_pitch
            .store(pumpkin_util::math::pack_degrees(pitch), Relaxed);
        world.broadcast_to_chunk_except_editioned(
            entity.chunk_pos.load(),
            &[self.gameprofile.id],
            &CEntityPositionSync::new(
                self.entity_id().into(),
                position,
                entity.velocity.load(),
                yaw,
                pitch,
                entity.on_ground.load(Relaxed),
            ),
            &CMovePlayer::new(
                VarULong(self.entity_id() as u64),
                Vector3::new(
                    position.x as f32,
                    position.y as f32 + entity.entity_type.eye_height,
                    position.z as f32,
                ),
                pitch,
                yaw,
                yaw,
                CMovePlayer::MODE_TELEPORT,
                entity.on_ground.load(Relaxed),
                VarULong(0),
                0,
                0,
                VarULong(self.tick_counter.load(Relaxed).max(0) as u64),
            ),
        );
    }
}
