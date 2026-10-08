use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use crate::{
    entity::{Entity, EntityBase, living::LivingEntity, player::Player},
    server::Server,
};
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_protocol::IdOr;
use pumpkin_protocol::java::client::play::CSoundEffect;
use pumpkin_util::math::boundingbox::BoundingBox;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

use super::arrow::ArrowPickup;
use super::{ProjectileHit, calculate_ray_intersection};

pub struct TridentEntity {
    pub entity: Entity,
    pub projectile: super::ownership::ProjectileState,
    pub item_stack: Arc<Mutex<ItemStack>>,
    pub pickup: crossbeam::atomic::AtomicCell<ArrowPickup>,
    pub in_ground: AtomicBool,
    pub in_ground_time: AtomicU32,
    pub life: AtomicU32,
    pub shake_time: AtomicU8,
    pub has_hit: AtomicBool,
    pub(super) dealt_damage: AtomicBool,
    pub last_block_pos: Arc<std::sync::RwLock<Option<BlockPos>>>,
}

impl TridentEntity {
    pub(super) const BASE_DAMAGE: f64 = 8.0;
    // ThrownTrident.getWaterInertia.
    const WATER_INERTIA: f64 = 0.99f32 as f64;
    const GRAVITY: f64 = 0.05;
    const DESPAWN_TIME: u32 = 1200;

    pub fn new(entity: Entity, owner_id: Option<i32>) -> Self {
        Self {
            projectile: super::ownership::ProjectileState::from_id(&entity, owner_id),
            entity,
            item_stack: Arc::new(Mutex::new(ItemStack::new(1, &Item::TRIDENT))),
            pickup: crossbeam::atomic::AtomicCell::new(ArrowPickup::Disallowed),
            in_ground: AtomicBool::new(false),
            in_ground_time: AtomicU32::new(0),
            life: AtomicU32::new(0),
            shake_time: AtomicU8::new(0),
            has_hit: AtomicBool::new(false),
            dealt_damage: AtomicBool::new(false),
            last_block_pos: Arc::new(std::sync::RwLock::new(None)),
        }
    }

    pub fn new_shot(
        entity: Entity,
        shooter: &Entity,
        item_stack: ItemStack,
        pickup: ArrowPickup,
    ) -> Self {
        let mut owner_pos = shooter.pos.load();
        owner_pos.y = owner_pos.y + f64::from(shooter.entity_dimension.load().eye_height) - 0.1;
        entity.pos.store(owner_pos);
        entity.set_velocity(Vector3::new(0.0, 0.1, 0.0));

        Self {
            entity,
            projectile: super::ownership::ProjectileState::new(Some(shooter.entity_uuid)),
            item_stack: Arc::new(Mutex::new(item_stack)),
            pickup: crossbeam::atomic::AtomicCell::new(pickup),
            in_ground: AtomicBool::new(false),
            in_ground_time: AtomicU32::new(0),
            life: AtomicU32::new(0),
            shake_time: AtomicU8::new(0),
            has_hit: AtomicBool::new(false),
            dealt_damage: AtomicBool::new(false),
            last_block_pos: Arc::new(std::sync::RwLock::new(None)),
        }
    }

    /// Applies projectile-spawned enchantment effects matching vanilla `Projectile::applyOnProjectileSpawned`.
    pub fn apply_on_projectile_spawned(&self, pickup_item_stack: &ItemStack) {
        super::apply_on_projectile_spawned(self.get_entity(), pickup_item_stack, None, None);
    }

    pub fn set_velocity_from_rotation(
        &self,
        pitch: f32,
        yaw: f32,
        roll: f32,
        speed: f32,
        divergence: f32,
    ) {
        let yaw_rad = yaw.to_radians();
        let pitch_rad = pitch.to_radians();
        let roll_rad = (pitch + roll).to_radians();

        let x = -yaw_rad.sin() * pitch_rad.cos();
        let y = -roll_rad.sin();
        let z = yaw_rad.cos() * pitch_rad.cos();

        self.set_velocity(
            f64::from(x),
            f64::from(y),
            f64::from(z),
            f64::from(speed),
            f64::from(divergence),
        );
    }

    pub fn set_velocity(&self, x: f64, y: f64, z: f64, power: f64, uncertainty: f64) {
        fn next_triangular(mode: f64, deviation: f64) -> f64 {
            deviation.mul_add(rand::random::<f64>() - rand::random::<f64>(), mode)
        }

        let velocity = Vector3::new(x, y, z)
            .normalize()
            .add_raw(
                next_triangular(0.0, 0.017_227_5 * uncertainty),
                next_triangular(0.0, 0.017_227_5 * uncertainty),
                next_triangular(0.0, 0.017_227_5 * uncertainty),
            )
            .multiply(power, power, power);

        self.entity.velocity.store(velocity);
        self.entity.velocity_dirty.store(true, Ordering::Relaxed);
        let len = velocity.horizontal_length();
        self.entity.set_rotation(
            velocity.x.atan2(velocity.z) as f32 * 57.295_776,
            velocity.y.atan2(len) as f32 * 57.295_776,
        );
    }

    fn step_move_and_hit(
        &self,
        caller: &dyn EntityBase,
        start_pos: Vector3<f64>,
        new_pos: Vector3<f64>,
        movement: Vector3<f64>,
    ) {
        let entity = &self.entity;
        let world = entity.world.load();
        // Check for collisions using raycasting
        let search_box = BoundingBox::new(
            Vector3::new(
                start_pos.x.min(new_pos.x),
                start_pos.y.min(new_pos.y),
                start_pos.z.min(new_pos.z),
            ),
            Vector3::new(
                start_pos.x.max(new_pos.x),
                start_pos.y.max(new_pos.y),
                start_pos.z.max(new_pos.z),
            ),
        )
        .expand(0.3, 0.3, 0.3);

        let mut hit = super::collision::first_block_hit(caller, start_pos, movement);
        let movement = hit
            .as_ref()
            .map_or(movement, |hit| hit.hit_pos() - start_pos);
        let mut closest_t = 1.0;

        // Entity collisions
        let owner = self.projectile_owner();
        let candidates = world.get_all_at_box(&search_box);
        for cand in candidates.into_iter().filter(super::can_hit_entity) {
            if self.dealt_damage.load(Ordering::Relaxed) {
                break;
            }
            if self.should_skip_collision(entity, &cand, owner.as_ref())
                || !super::arrow::can_hit_player(entity, owner.as_deref(), &cand)
            {
                continue;
            }

            let ebb = cand
                .get_entity()
                .bounding_box
                .load()
                .expand_all(super::collision::compute_margin(entity));
            if let Some(t) = calculate_ray_intersection(&start_pos, &movement, &ebb)
                && t < closest_t
            {
                closest_t = t;
                let hit_pos = start_pos.add(&movement.multiply(t, t, t));
                hit = Some(ProjectileHit::Entity {
                    entity: cand.clone(),
                    hit_pos,
                    normal: movement.normalize().multiply(-1.0, -1.0, -1.0),
                });
            }
        }

        // AbstractArrow.stepMoveAndHit applies swept effects at contact before deflection.
        entity.set_pos(hit.as_ref().map_or(new_pos, ProjectileHit::hit_pos));
        super::block_effects::apply(caller, start_pos, entity.pos.load());
        if !entity.is_alive() {
            return;
        }
        if let Some(h) = hit {
            if super::deflection::hit_target_or_deflect_self(caller, &h) {
                return;
            }
            if !self.has_hit.swap(true, Ordering::SeqCst) {
                super::collision::on_hit(caller, h);
                entity.velocity_dirty.store(true, Ordering::Relaxed);
            }
        }
    }

    fn should_skip_collision(
        &self,
        self_ent: &Entity,
        other: &Arc<dyn EntityBase>,
        owner: Option<&Arc<dyn EntityBase>>,
    ) -> bool {
        let other_ent = other.get_entity();

        // Don't collide with self
        if other_ent.entity_id == self_ent.entity_id {
            return true;
        }

        if !self.projectile.can_hit_with_owner(self_ent, other, owner) {
            return true;
        }

        false
    }
}

impl EntityBase for TridentEntity {
    fn write_custom_nbt(&self, nbt: &mut pumpkin_nbt::compound::NbtCompound) {
        self.write_trident(nbt);
    }
    fn read_custom_nbt(&self, nbt: &pumpkin_nbt::compound::NbtCompound) {
        self.read_trident(nbt);
    }
    fn projectile_state(&self) -> Option<&super::ownership::ProjectileState> {
        Some(&self.projectile)
    }

    fn tick(&self, caller: &dyn EntityBase, server: &Server) {
        let entity = self.get_entity();
        if self.in_ground_time.load(Ordering::Relaxed) > 4 {
            self.dealt_damage.store(true, Ordering::Relaxed);
        }
        // Handle shake time
        let shake = self.shake_time.load(Ordering::Relaxed);
        if shake > 0 {
            self.shake_time.store(shake - 1, Ordering::Relaxed);
        }

        if self.in_ground.load(Ordering::Relaxed) {
            let _in_ground_time = self.in_ground_time.fetch_add(1, Ordering::Relaxed);
            let life = self.life.fetch_add(1, Ordering::Relaxed);

            // Despawn after enough time
            if life >= Self::DESPAWN_TIME {
                entity.remove();
            }
            // AbstractArrow.tick's grounded branch applies block effects without super.tick.
            entity.tick_block_collisions(caller);
            return;
        }

        // ThrownTrident.tick delegates its flight to AbstractArrow.tick.
        self.projectile.check_left_owner(entity);
        let velocity = super::arrow::tick_flight(
            entity.velocity.load(),
            entity.touching_water.load(Ordering::Relaxed) || entity.is_in_water(),
            Self::WATER_INERTIA,
            if entity.has_no_gravity() {
                0.0
            } else {
                Self::GRAVITY
            },
            |movement, velocity| {
                entity.velocity.store(velocity);
                super::arrow::update_flight_rotation(entity, movement, true);
                let start_pos = entity.pos.load();
                self.step_move_and_hit(caller, start_pos, start_pos.add(&movement), movement);
                (
                    entity.velocity.load(),
                    self.in_ground.load(Ordering::Relaxed),
                )
            },
        );
        entity.velocity.store(velocity);
        self.projectile.tick(entity);
        EntityBase::tick(entity, caller, server);
    }

    fn get_entity(&self) -> &Entity {
        &self.entity
    }

    fn get_living_entity(&self) -> Option<&LivingEntity> {
        None
    }
    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }

    fn on_hit(&self, hit: ProjectileHit) {
        let entity = self.get_entity();
        let world = entity.world.load();

        match hit {
            ProjectileHit::Block { pos, hit_pos, .. } => {
                self.in_ground.store(true, Ordering::Relaxed);
                self.shake_time.store(7, Ordering::Relaxed);
                *self
                    .last_block_pos
                    .write()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(pos);

                let block = world.get_block(&pos);
                let state = world.get_block_state(&pos);
                if let Some(server) = world.server.upgrade() {
                    world
                        .block_registry
                        .on_projectile_hit(block, &world, self, &pos, state, &hit_pos, &server);
                }

                // Stop the trident
                entity.velocity.store(Vector3::new(0.0, 0.0, 0.0));
                entity.set_pos(hit_pos);

                // Play sound
                let sound_packet = CSoundEffect::new(
                    IdOr::Id(Sound::ItemTridentHitGround as u16),
                    SoundCategory::Neutral,
                    &hit_pos,
                    1.0,
                    1.0,
                    0,
                );
                let chunk_pos = entity.chunk_pos.load();
                world.broadcast_to_chunk(chunk_pos, &sound_packet);
            }
            ProjectileHit::Entity {
                entity: target,
                hit_pos,
                ..
            } => {
                self.hit_entity(target.as_ref(), hit_pos);
            }
        }
    }

    fn on_player_collision(&self, player: &Arc<Player>) {
        // Can only pick up when on the ground
        // ThrownTrident.playerTouch / AbstractArrow.playerTouch.
        if (!self.in_ground.load(Ordering::Relaxed)
            && !self.entity.no_physics.load(Ordering::Relaxed))
            || self.shake_time.load(Ordering::Relaxed) > 0
            || self.projectile_owner().is_some_and(|owner| {
                owner.get_entity().entity_uuid != player.get_entity().entity_uuid
            })
        {
            return;
        }

        if player.living_entity.health.load() <= 0.0 {
            return;
        }

        match self.pickup.load() {
            ArrowPickup::Disallowed => return,
            ArrowPickup::CreativeOnly if !player.is_creative() => return,
            _ => {}
        }

        if let Some(server) = player.world().server.upgrade() {
            let mut event = crate::plugin::api::events::player::player_pickup_arrow::PlayerPickupArrowEvent::new(player.clone(), self.entity.entity_id);
            server.plugin_manager.fire_blocking(&server, &mut event);
            if event.cancelled {
                return;
            }
        }

        let mut stack = self
            .item_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if player.is_creative() || player.inventory.insert_stack_anywhere(&mut stack) {
            player.increment_stat(
                pumpkin_data::statistic::StatisticCategory::PickedUp,
                stack.item.id as i32,
                1,
            );
            player.living_entity.pickup(&self.entity, 1);
            self.get_entity().remove();
        }
    }
}
