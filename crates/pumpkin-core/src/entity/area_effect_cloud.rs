mod application;
pub(crate) mod bottle;
mod storage;
#[cfg(test)]
mod tests;

use crate::{
    entity::{
        Entity, EntityBase,
        projectile::{ownership::ProjectileState, potion_effects::EffectEntry},
    },
    server::Server,
};
use pumpkin_data::{item::Item, item_stack::ItemStack};
use pumpkin_util::math::{boundingbox::BoundingBox, vector3::Vector3};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, atomic::Ordering},
};
use uuid::Uuid;

const TIME_BETWEEN_APPLICATIONS: i32 = 5;
const MINIMAL_RADIUS: f32 = 0.5;

pub struct AreaEffectCloudEntity {
    pub entity: Entity,
    state: Mutex<CloudState>,
    owner: ProjectileState,
    pub item_stack: Mutex<ItemStack>,
    effects: Mutex<Vec<EffectEntry>>,
    dragon: Mutex<bool>,
    sitting_dragon: std::sync::atomic::AtomicBool,
}

struct CloudState {
    radius: f32,
    duration: i32,
    age: i32,
    wait_time: i32,
    reapplication_delay: i32,
    radius_per_tick: f32,
    radius_on_use: f32,
    duration_on_use: i32,
    victims: HashMap<Uuid, i32>,
}

impl CloudState {
    // AreaEffectCloud.serverTick: wait time is outside duration, shrink precedes the five-tick application gate.
    fn tick(&mut self) -> bool {
        self.age += 1;
        if self.duration != -1 && self.age - self.wait_time >= self.duration {
            return false;
        }
        if self.age >= self.wait_time && self.radius_per_tick != 0.0 {
            self.radius += self.radius_per_tick;
            if self.radius < MINIMAL_RADIUS {
                return false;
            }
            self.radius = self.radius.clamp(0.0, 32.0);
        }
        true
    }

    fn on_use(&mut self) -> bool {
        if self.radius_on_use != 0.0 {
            self.radius += self.radius_on_use;
            if self.radius < MINIMAL_RADIUS {
                return false;
            }
            self.radius = self.radius.clamp(0.0, 32.0);
        }
        if self.duration_on_use != 0 && self.duration != -1 {
            self.duration += self.duration_on_use;
            if self.duration <= 0 {
                return false;
            }
        }
        true
    }
}

impl AreaEffectCloudEntity {
    #[allow(clippy::new_ret_no_self)]
    pub fn new(entity: Entity) -> Arc<dyn EntityBase> {
        Self::create(
            entity,
            ItemStack::new(1, &Item::GLASS_BOTTLE),
            Vec::new(),
            -1,
            3.0,
            20,
            20,
            0.0,
            0,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create(
        entity: Entity,
        item_stack: ItemStack,
        effects: Vec<EffectEntry>,
        duration: i32,
        radius: f32,
        reapplication_delay: i32,
        wait_time: i32,
        radius_on_use: f32,
        duration_on_use: i32,
    ) -> Arc<Self> {
        entity.no_physics.store(true, Ordering::Relaxed);
        let cloud = Arc::new(Self {
            entity,
            owner: ProjectileState::default(),
            item_stack: Mutex::new(item_stack),
            effects: Mutex::new(effects),
            dragon: Mutex::new(false),
            sitting_dragon: std::sync::atomic::AtomicBool::new(false),
            state: Mutex::new(CloudState {
                radius,
                duration,
                age: 0,
                wait_time,
                reapplication_delay,
                radius_per_tick: 0.0,
                radius_on_use,
                duration_on_use,
                victims: HashMap::new(),
            }),
        });
        cloud.set_radius(radius);
        cloud
    }

    pub fn set_owner(&self, owner: Option<&dyn EntityBase>) {
        self.owner.set_owner(
            owner
                .filter(|owner| owner.get_living_entity().is_some())
                .map(EntityBase::get_entity),
        );
    }
    pub fn owner(&self) -> Option<Arc<dyn EntityBase>> {
        self.owner
            .owner(&self.entity)
            .filter(|owner| owner.get_living_entity().is_some())
    }
    pub fn radius(&self) -> f32 {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .radius
    }
    pub fn set_radius_per_tick(&self, delta: f32) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .radius_per_tick = delta;
    }
    pub fn set_radius(&self, radius: f32) {
        let radius = radius.clamp(0.0, 32.0);
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .radius = radius;
        self.sync_radius(radius);
    }
    fn sync_radius(&self, radius: f32) {
        // AreaEffectCloud.getDimensions: a horizontal disc with a half-block height.
        let pos = self.entity.pos.load();
        let r = f64::from(radius);
        self.entity.bounding_box.store(BoundingBox::new(
            Vector3::new(pos.x - r, pos.y, pos.z - r),
            Vector3::new(pos.x + r, pos.y + 0.5, pos.z + r),
        ));
        self.entity.set_synced_data(
            pumpkin_data::tracked_data::area_effect_cloud::RADIUS,
            radius,
        );
    }

    /// `DragonSittingFlamingPhase` and `DragonFireball` cloud parameters; growth is independent of use.
    pub fn dragon_cloud(entity: Entity, owner: &dyn EntityBase, fireball: bool) -> Arc<Self> {
        let cloud = Self::create(
            entity,
            ItemStack::new(1, &Item::DRAGON_BREATH),
            vec![(
                &pumpkin_data::effect::StatusEffect::INSTANT_DAMAGE,
                1,
                u8::from(fireball),
                false,
                true,
                true,
            )],
            if fireball { 600 } else { 200 },
            if fireball { 3.0 } else { 5.0 },
            20,
            20,
            0.0,
            0,
        );
        cloud.set_owner(Some(owner));
        cloud
            .item_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .set_data_component(pumpkin_data::data_component_impl::PotionDurationScaleImpl {
                scale: 0.25,
            });
        cloud.sitting_dragon.store(!fireball, Ordering::Relaxed);
        *cloud
            .dragon
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = true;
        if fireball {
            cloud.set_radius_per_tick((7.0 - 3.0) / 600.0);
        }
        cloud
    }

    /// `DragonSittingFlamingPhase.end` removes only its sitting-breath cloud.
    pub(crate) fn remove_sitting_clouds(owner: &Entity) {
        for entity in owner.world.load().entities.load().iter() {
            if let Some(cloud) = entity.cast_any().downcast_ref::<Self>()
                && cloud.sitting_dragon.load(Ordering::Relaxed)
                && cloud.owner.owned_by(owner)
            {
                cloud.entity.remove();
            }
        }
    }
}

impl EntityBase for AreaEffectCloudEntity {
    fn get_owner_id(&self) -> Option<i32> {
        self.owner().map(|owner| owner.get_entity().entity_id)
    }
    fn init_data_tracker(&self) {
        self.sync_cloud_data();
    }
    fn tick(&self, _caller: &dyn EntityBase, _server: &Server) {
        let (alive, waiting, radius, apply) = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let alive = state.tick();
            (
                alive,
                state.age < state.wait_time,
                state.radius,
                state.age % TIME_BETWEEN_APPLICATIONS == 0,
            )
        };
        if !alive {
            self.entity.remove();
            return;
        }
        self.entity.set_synced_data(
            pumpkin_data::tracked_data::area_effect_cloud::WAITING,
            waiting,
        );
        if waiting {
            return;
        }
        self.sync_radius(radius);
        if apply {
            self.apply_to_entities();
        }
    }
    fn write_custom_nbt(&self, nbt: &mut pumpkin_nbt::compound::NbtCompound) {
        self.write_cloud(nbt);
    }
    fn read_custom_nbt(&self, nbt: &pumpkin_nbt::compound::NbtCompound) {
        self.read_cloud(nbt);
    }
    fn get_entity(&self) -> &Entity {
        &self.entity
    }
    fn get_living_entity(&self) -> Option<&crate::entity::living::LivingEntity> {
        None
    }
    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }
}
