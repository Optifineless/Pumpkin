use std::sync::atomic::AtomicBool;

use crate::entity::projectile::ProjectileHit;
use crate::{
    entity::{
        Entity, EntityBase, EntityType, mob::endermite::EndermiteEntity,
        projectile::ThrownItemEntity,
    },
    server::Server,
};
use pumpkin_data::damage::DamageType;
use pumpkin_data::entity::{EntityPose, EntityStatus};
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_protocol::bedrock::server::actor_event::ActorEventID;
use pumpkin_util::math::vector3::Vector3;

const GRAVITY: f64 = 0.03;
const ENDERMITE_SPAWN_CHANCE: f32 = 0.05;

pub struct EnderPearlEntity {
    pub thrown: ThrownItemEntity,
}

impl EnderPearlEntity {
    pub fn new(entity: Entity) -> Self {
        entity.set_velocity(Vector3::new(0.0, 0.1, 0.0));

        let thrown = ThrownItemEntity {
            entity,
            owner_id: None,
            collides_with_projectiles: false,
            has_hit: AtomicBool::new(false),
            gravity: GRAVITY,
        };

        Self { thrown }
    }

    pub fn new_shot(entity: Entity, shooter: &Entity) -> Self {
        let thrown = ThrownItemEntity::new(entity, shooter, GRAVITY);
        thrown.entity.set_velocity(Vector3::new(0.0, 0.1, 0.0));
        Self { thrown }
    }
}

impl EntityBase for EnderPearlEntity {
    fn get_owner_id(&self) -> Option<i32> {
        self.thrown.owner_id
    }

    fn tick(&self, caller: &dyn EntityBase, _server: &Server) {
        self.thrown.process_tick(caller);
    }

    fn get_entity(&self) -> &Entity {
        self.thrown.get_entity()
    }

    fn get_living_entity(&self) -> Option<&crate::entity::living::LivingEntity> {
        None
    }
    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }

    fn on_hit(&self, hit: ProjectileHit) {
        let entity = self.get_entity();
        let world = entity.world.load();

        let attacker = self
            .thrown
            .owner_id
            .and_then(|id| world.get_entity_by_id(id));

        let owner_id = self.thrown.owner_id;
        let teleport_pos = entity.last_pos.load();

        // ThrownEnderpearl.onHitEntity also delivers ownerless zero-damage hits.
        if let ProjectileHit::Entity {
            entity: hit_entity, ..
        } = &hit
        {
            super::damage::hurt_entity(
                hit_entity.as_ref(),
                0.0,
                DamageType::THROWN,
                self,
                attacker.as_deref(),
            );
        }

        if let Some(owner_id) = owner_id
            && let Some(owner) = world.get_entity_by_id(owner_id)
            && owner.get_entity().is_alive()
            && owner.get_living_entity().is_none_or(|living| {
                living.health.load() > 0.0 && owner.get_entity().pose.load() != EntityPose::Sleeping
            })
        {
            let should_spawn_endermite = rand::random::<f32>() < ENDERMITE_SPAWN_CHANCE;
            if world.should_spawn_monsters() && should_spawn_endermite {
                let entity = Entity::new(
                    world.clone(),
                    owner.get_entity().pos.load(),
                    &EntityType::ENDERMITE,
                );
                let endermite = EndermiteEntity::new(entity);
                world.spawn_entity(endermite);
            }

            // In vanilla, teleport handles everything including sound
            owner.teleport(
                teleport_pos,
                Some(owner.get_entity().yaw.load()),
                Some(owner.get_entity().pitch.load()),
                world.clone(),
            );

            // Play teleport sound at new position
            world.play_sound(
                Sound::EntityPlayerTeleport,
                SoundCategory::Players,
                &teleport_pos,
            );

            // Deal 5 damage to owner
            owner.damage(
                owner.as_ref(),
                5.0,
                pumpkin_data::damage::DamageType::ENDER_PEARL,
            );
        }

        world.send_entity_status(entity, EntityStatus::Death, Some(ActorEventID::Death));
    }
}
