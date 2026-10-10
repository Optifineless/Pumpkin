use crate::{
    entity::{Entity, EntityBase, projectile::ThrownItemEntity},
    server::Server,
    world::{Explosion, World},
};
use pumpkin_data::{
    data_component_impl::FireworksImpl, entity::EntityStatus, item::Item, item_stack::ItemStack,
};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::{
    bedrock::server::actor_event::ActorEventID,
    codec::{item_stack_seralizer::ItemStackSerializer, optional_int::OptionalInt},
};
use pumpkin_util::math::vector3::Vector3;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicI32, Ordering},
};

pub struct FireworkRocketEntity {
    entity: ThrownItemEntity,
    life: AtomicI32,
    life_time: AtomicI32,
    item: Mutex<ItemStack>,
    attached: Mutex<Option<i32>>,
    shot_at_angle: AtomicBool,
}

impl FireworkRocketEntity {
    pub fn new(entity: Entity) -> Self {
        Self::with_item(
            entity,
            ItemStack::new(1, &Item::FIREWORK_ROCKET),
            None,
            false,
        )
    }

    /// `FireworkRocketEntity` constructors retain the actual stack and distinguish attachment from ownership.
    pub fn with_item(
        entity: Entity,
        mut item: ItemStack,
        owner: Option<&Entity>,
        attached: bool,
    ) -> Self {
        let flight = item
            .get_data_component::<FireworksImpl>()
            .map_or(0, |fireworks| fireworks.flight_duration);
        let lifetime = 10 * (1 + flight) + rand::random_range(0..6) + rand::random_range(0..7);
        let triangle = || (rand::random::<f64>() - rand::random::<f64>()) * 0.002_297;
        entity.set_velocity(Vector3::new(triangle(), 0.05, triangle()));
        item.item_count = 1;
        Self {
            entity: ThrownItemEntity {
                entity,
                projectile: super::ownership::ProjectileState::new(
                    owner.map(|owner| owner.entity_uuid),
                ),
                has_hit: AtomicBool::new(false),
                gravity: 0.0,
            },
            life: AtomicI32::new(0),
            life_time: AtomicI32::new(lifetime),
            item: Mutex::new(item),
            attached: Mutex::new(if attached {
                owner.map(|owner| owner.entity_id)
            } else {
                None
            }),
            shot_at_angle: AtomicBool::new(false),
        }
    }

    /// `CrossbowItem.createProjectile` / getShootingPower: rockets use speed 1.6 and fly at an angle.
    pub fn shoot(
        world: &std::sync::Arc<World>,
        shooter: &Entity,
        item: &ItemStack,
        pitch: f32,
        yaw: f32,
        uncertainty: f32,
    ) {
        let mut position = shooter.pos.load();
        position.y += shooter.get_eye_height() - 0.15;
        let entity = Entity::new(
            world.clone(),
            position,
            &pumpkin_data::entity::EntityType::FIREWORK_ROCKET,
        );
        let rocket = Self::with_item(entity, item.clone(), Some(shooter), false);
        rocket.set_shot_at_angle(true);
        rocket
            .entity
            .set_velocity_from(pitch, yaw, 0.0, 1.6, uncertainty);
        world.spawn_entity(std::sync::Arc::new(rocket));
    }

    /// Player.getProjectile / `CrossbowItem.getSupportedHeldProjectiles` allow rockets only in either hand.
    pub(crate) fn crossbow_ammunition(player: &crate::entity::player::Player) -> Option<usize> {
        use pumpkin_data::tag::Taggable;
        use pumpkin_inventory::player::player_inventory::PlayerInventory;
        let inventory = player.inventory();
        for slot in [
            PlayerInventory::OFF_HAND_SLOT,
            inventory.get_selected_slot() as usize,
        ] {
            let stack = inventory.get_slot(slot);
            if !stack.is_empty()
                && (stack.item == &Item::FIREWORK_ROCKET
                    || stack
                        .item
                        .has_tag(&pumpkin_data::tag::Item::MINECRAFT_ARROWS))
            {
                return Some(slot);
            }
        }
        player.find_arrow()
    }

    pub fn set_shot_at_angle(&self, value: bool) {
        self.shot_at_angle.store(value, Ordering::Relaxed);
        self.get_entity().set_synced_data(
            pumpkin_data::tracked_data::firework_rocket::SHOT_AT_ANGLE,
            value,
        );
    }

    pub fn explode_and_remove(&self, world: &World) {
        let entity = self.get_entity();
        if entity.is_removed() {
            return;
        }
        if let Some(server) = world.server.upgrade() {
            let mut event =
                crate::plugin::api::events::entity::firework_explode::FireworkExplodeEvent {
                    entity_id: entity.entity_id,
                    cancelled: false,
                };
            server.plugin_manager.fire_blocking(&server, &mut event);
            if event.cancelled {
                return;
            }
        }
        world.send_entity_status(
            entity,
            EntityStatus::FireworksExplode,
            Some(ActorEventID::FireworksExplode),
        );
        world.emit_game_event(
            pumpkin_data::game_event::GameEvent::Explode.name(),
            entity.pos.load(),
        );
        self.deal_explosion_damage(world);
        entity.remove();
    }

    fn explosion_count(&self) -> usize {
        self.item
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_data_component::<FireworksImpl>()
            .map_or(0, |fireworks| fireworks.explosions.len())
    }

    // FireworkRocketEntity.dealExplosionDamage: two visibility rays, spherical falloff, direct rocket/cause owner.
    fn deal_explosion_damage(&self, world: &World) {
        let count = self.explosion_count();
        if count == 0 {
            return;
        }
        let damage = explosion_damage(count, 0.0);
        let entity = self.get_entity();
        let owner = self.entity.projectile.owner(entity);
        let attached = *self
            .attached
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(target) = attached.and_then(|id| world.get_entity_by_id(id)) {
            super::damage::hurt_entity(
                target.as_ref(),
                damage,
                pumpkin_data::damage::DamageType::FIREWORKS,
                self,
                owner.as_deref(),
            );
        }
        let pos = entity.pos.load();
        for target in world.get_all_at_box(&entity.bounding_box.load().expand_all(5.0)) {
            let other = target.get_entity();
            if Some(other.entity_id) == attached || target.get_living_entity().is_none() {
                continue;
            }
            let distance = other.pos.load().squared_distance_to_vec(&pos).sqrt();
            if distance > 5.0 {
                continue;
            }
            let visible = [0.0, 0.5].into_iter().any(|fraction| {
                let mut to = other.pos.load();
                to.y += (other.bounding_box.load().max.y - to.y) * fraction;
                Explosion::ray_clear(world, pos, to)
            });
            if visible {
                super::damage::hurt_entity(
                    target.as_ref(),
                    explosion_damage(count, distance),
                    pumpkin_data::damage::DamageType::FIREWORKS,
                    self,
                    owner.as_deref(),
                );
            }
        }
    }

    fn tick_attachment(&self, world: &World, id: i32) {
        let Some(target) = world.get_entity_by_id(id) else {
            return;
        };
        let shooter = target.get_entity();
        if shooter.is_fall_flying() {
            let cancelled = if let Some(player) = world.get_player_by_id(id)
                && let Some(server) = world.server.upgrade()
            {
                let mut event = crate::plugin::api::events::player::player_elytra_boost::PlayerElytraBoostEvent {
                    player, firework_id: self.get_entity().entity_id, cancelled: false,
                };
                server.plugin_manager.fire_blocking(&server, &mut event);
                event.cancelled
            } else {
                false
            };
            if !cancelled {
                let direction = shooter.rotation().to_f64();
                let velocity = shooter.velocity.load();
                // FireworkRocketEntity.tick uses setDeltaMovement; the attached client predicts the same boost.
                shooter
                    .velocity
                    .store(velocity + (direction * 0.1 + (direction * 1.5 - velocity) * 0.5));
            }
        }
        self.get_entity().set_pos(shooter.pos.load());
        self.get_entity().set_velocity(shooter.velocity.load());
    }
    // FireworkRocketEntity.tick preserves attempted motion after collision, so plain rockets can slide along blocks.
    fn tick_flight(&self, caller: &dyn EntityBase) {
        let entity = self.get_entity();
        let movement = entity.velocity.load();
        // Safety divergence: reject before ray queries and never restore the rejected impulse.
        if !entity.no_physics.load(Ordering::Relaxed) && !entity.accept_movement(movement) {
            entity.tick_block_collisions(caller);
            return;
        }
        let hit = self.entity.find_hit(caller, entity.pos.load(), movement);
        entity.move_entity(caller, movement);
        entity.tick_block_collisions(caller);
        entity.velocity.store(movement);
        if !entity.no_physics.load(Ordering::Relaxed)
            && entity.is_alive()
            && let Some(hit) = hit
        {
            self.entity.hit_target(caller, hit);
            entity.velocity_dirty.store(true, Ordering::Relaxed);
        }
    }
}

// FireworkRocketEntity.dealExplosionDamage; the first explosion also contributes two points.
fn explosion_damage(count: usize, distance: f64) -> f32 {
    if count == 0 {
        0.0
    } else {
        (5.0 + count as f32 * 2.0) * ((5.0 - distance) / 5.0).sqrt() as f32
    }
}

impl EntityBase for FireworkRocketEntity {
    fn projectile_state(&self) -> Option<&super::ownership::ProjectileState> {
        Some(&self.entity.projectile)
    }
    fn init_data_tracker(&self) {
        let entity = self.get_entity();
        entity.set_synced_data(
            pumpkin_data::tracked_data::firework_rocket::ID_FIREWORKS_ITEM,
            ItemStackSerializer::from(
                self.item
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone(),
            ),
        );
        entity.set_synced_data(
            pumpkin_data::tracked_data::firework_rocket::ATTACHED_TO_TARGET,
            OptionalInt(
                *self
                    .attached
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            ),
        );
        entity.set_synced_data(
            pumpkin_data::tracked_data::firework_rocket::SHOT_AT_ANGLE,
            self.shot_at_angle.load(Ordering::Relaxed),
        );
    }
    fn tick(&self, caller: &dyn EntityBase, server: &Server) {
        let entity = self.get_entity();
        self.entity.projectile.tick(entity);
        // FireworkRocketEntity.tick calls Projectile.tick -> Entity.tick before flight/attachment.
        EntityBase::tick(entity, caller, server);
        let world = entity.world.load();
        let attached = *self
            .attached
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(id) = attached {
            self.tick_attachment(&world, id);
            if !entity.no_physics.load(Ordering::Relaxed)
                && let Some(hit) =
                    self.entity
                        .find_hit(caller, entity.pos.load(), entity.velocity.load())
            {
                self.entity.hit_target(caller, hit);
            }
        } else {
            if !self.shot_at_angle.load(Ordering::Relaxed) {
                let mut velocity = entity.velocity.load();
                let acceleration = if entity.horizontal_collision.load(Ordering::Relaxed) {
                    1.0
                } else {
                    1.15
                };
                velocity.x *= acceleration;
                velocity.z *= acceleration;
                velocity.y += 0.04;
                entity.velocity.store(velocity);
            }
            self.tick_flight(caller);
        }
        if self.life.load(Ordering::Relaxed) == 0 && !entity.silent.load(Ordering::Relaxed) {
            world.play_sound_fine(
                pumpkin_data::sound::Sound::EntityFireworkRocketLaunch,
                pumpkin_data::sound::SoundCategory::Ambient,
                &entity.pos.load(),
                3.0,
                1.0,
            );
        }
        if self.life.fetch_add(1, Ordering::Relaxed) + 1 > self.life_time.load(Ordering::Relaxed) {
            self.explode_and_remove(&world);
        }
    }
    fn write_custom_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put_int("Life", self.life.load(Ordering::Relaxed));
        nbt.put_int("LifeTime", self.life_time.load(Ordering::Relaxed));
        nbt.put_bool("ShotAtAngle", self.shot_at_angle.load(Ordering::Relaxed));
        let mut item = NbtCompound::new();
        self.item
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .write_item_stack(&mut item);
        nbt.put_compound("FireworksItem", item);
    }
    fn read_custom_nbt(&self, nbt: &NbtCompound) {
        self.life
            .store(nbt.get_int("Life").unwrap_or(0), Ordering::Relaxed);
        self.life_time
            .store(nbt.get_int("LifeTime").unwrap_or(0), Ordering::Relaxed);
        self.shot_at_angle.store(
            nbt.get_bool("ShotAtAngle").unwrap_or(false),
            Ordering::Relaxed,
        );
        if let Some(item) = nbt
            .get_compound("FireworksItem")
            .and_then(ItemStack::read_item_stack)
        {
            *self
                .item
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = item;
        }
    }
    fn get_entity(&self) -> &Entity {
        &self.entity.entity
    }
    fn get_living_entity(&self) -> Option<&crate::entity::living::LivingEntity> {
        None
    }
    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }
    fn on_hit(&self, hit: super::ProjectileHit) {
        if matches!(hit, super::ProjectileHit::Entity { .. }) || self.explosion_count() != 0 {
            self.explode_and_remove(&self.get_entity().world.load());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn firework_damage_requires_explosions_and_falls_off_with_square_root() {
        assert_eq!(explosion_damage(0, 0.0), 0.0);
        assert_eq!(explosion_damage(1, 0.0), 7.0);
        assert_eq!(explosion_damage(3, 0.0), 11.0);
        assert_eq!(explosion_damage(1, 3.75), 3.5);
        assert_eq!(explosion_damage(1, 5.0), 0.0);
    }
    #[tokio::test]
    async fn plain_rocket_collides_without_exploding_or_losing_its_attempted_motion() {
        use crate::entity::living::test_support::armor_test_world;
        use pumpkin_util::math::vector2::Vector2;
        let dir = tempfile::tempdir().unwrap();
        let world = armor_test_world(dir.path());
        let chunk = pumpkin_world::chunk::ChunkData::empty_sync(0, 0);
        chunk.set_block_absolute_y(2, 65, 8, pumpkin_data::Block::STONE.default_state.id);
        world.level.loaded_chunks.insert(Vector2::new(0, 0), chunk);
        let rocket = FireworkRocketEntity::new(Entity::new(
            world,
            Vector3::new(1.5, 65.5, 8.5),
            &pumpkin_data::entity::EntityType::FIREWORK_ROCKET,
        ));
        let motion = Vector3::new(1.0, 0.0, 0.0);
        rocket.get_entity().velocity.store(motion);
        rocket.tick_flight(&rocket);
        assert!(!rocket.get_entity().is_removed());
        assert!(rocket.get_entity().pos.load().x < 2.0);
        assert_eq!(rocket.get_entity().velocity.load(), motion);
    }
}
