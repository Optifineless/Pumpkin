use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crossbeam::atomic::AtomicCell;

use crate::entity::player::Player;
use crate::entity::{Entity, EntityBase, living::LivingEntity};
use crate::server::Server;

use pumpkin_data::damage::DamageType;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::{
    data_component::DataComponent,
    data_component_impl::{CustomNameImpl, DataComponentImpl},
    entity::EntityType,
    item::Item,
};
use pumpkin_inventory::Inventory;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::java::client::play::Metadata;

use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::text::TextComponent;

use super::minecart::container::{self, MinecartInventory};
use crate::entity::vehicle::vehicle::VehicleEntity;

pub struct BoatEntity {
    pub vehicle: VehicleEntity,
    ticks_underwater: AtomicCell<f32>,
    left_paddle_moving: AtomicBool,
    right_paddle_moving: AtomicBool,
    inventory: Option<Arc<MinecartInventory>>,
}

impl BoatEntity {
    /// Identifies `AbstractChestBoat` types by generated registry identity, without item lookups.
    pub(crate) fn has_chest_inventory(entity_type: &EntityType) -> bool {
        const CHEST_BOATS: &[EntityType] = &[
            EntityType::ACACIA_CHEST_BOAT,
            EntityType::BAMBOO_CHEST_RAFT,
            EntityType::BIRCH_CHEST_BOAT,
            EntityType::CHERRY_CHEST_BOAT,
            EntityType::DARK_OAK_CHEST_BOAT,
            EntityType::JUNGLE_CHEST_BOAT,
            EntityType::MANGROVE_CHEST_BOAT,
            EntityType::OAK_CHEST_BOAT,
            EntityType::PALE_OAK_CHEST_BOAT,
            EntityType::POPLAR_CHEST_BOAT,
            EntityType::SPRUCE_CHEST_BOAT,
        ];
        CHEST_BOATS.contains(entity_type)
    }

    pub fn new(entity: Entity) -> Self {
        // AbstractChestBoat.CONTAINER_SIZE. Boat and item registry names agree for every variant.
        const CONTAINER_SIZE: usize = 27;
        let inventory = Self::has_chest_inventory(entity.entity_type)
            .then(|| Arc::new(MinecartInventory::new(CONTAINER_SIZE)));
        Self {
            vehicle: VehicleEntity::new(entity),
            ticks_underwater: AtomicCell::new(0.0),
            left_paddle_moving: AtomicBool::new(false),
            right_paddle_moving: AtomicBool::new(false),
            inventory,
        }
    }

    pub fn set_paddles(&self, left: bool, right: bool) {
        self.left_paddle_moving.store(left, Ordering::Relaxed);
        self.right_paddle_moving.store(right, Ordering::Relaxed);

        self.vehicle.entity.send_meta_data(
            &[
                Metadata::new(pumpkin_data::tracked_data::boat::ID_PADDLE_LEFT, left),
                Metadata::new(pumpkin_data::tracked_data::boat::ID_PADDLE_RIGHT, right),
            ],
            None,
        );
    }

    fn send_wobble_metadata(&self) {
        self.vehicle.send_wobble_metadata();
    }

    fn destroy(&self) {
        // VehicleEntity.destroy(Item) / AbstractChestBoat.destroy: boats don't use entity loot tables.
        let entity = &self.vehicle.entity;
        let world = entity.world.load();
        if !world.level_info.load().game_rules.entity_drops {
            return;
        }
        if let Some(item) = Item::from_registry_key(entity.entity_type.resource_name) {
            let mut stack = ItemStack::new(1, item);
            if let Some(name) = entity.custom_name.load().as_ref().clone() {
                stack.patch.push((
                    DataComponent::CustomName,
                    Some(CustomNameImpl { name }.to_dyn()),
                ));
            }
            entity.spawn_at_location(stack);
        }
    }

    fn drop_contents(&self) {
        // AbstractChestBoat.remove drops contents even on a creative discard or with entity drops off.
        let entity = &self.vehicle.entity;
        let world = entity.world.load();
        if let Some(inventory) = &self.inventory
            && inventory.claim_drops()
        {
            let params =
                crate::world::loot::build_container_loot_context(&world, entity.pos.load(), None);
            inventory.unpack_loot(&params);
            let inventory: Arc<dyn Inventory> = inventory.clone();
            world.scatter_container_inventory(entity.pos.load(), &inventory);
        }
    }

    // AbstractChestBoat.getMaxPassengers / AbstractBoat.getMaxPassengers.
    const fn max_passengers(&self) -> usize {
        if self.inventory.is_some() { 1 } else { 2 }
    }

    fn can_add_passenger(&self) -> bool {
        !self.vehicle.entity.is_submerged_in_water()
            && self
                .vehicle
                .entity
                .passengers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len()
                < self.max_passengers()
    }

    fn try_mount(&self, player: &Arc<Player>) -> bool {
        if player.get_entity().is_sneaking() || self.ticks_underwater.load() >= 60.0 {
            return false;
        }
        if !self.can_add_passenger() || player.get_entity().has_vehicle() {
            return false;
        }
        let world = self.vehicle.entity.world.load();
        let Some(vehicle) = world.get_entity_by_id(self.vehicle.entity.entity_id) else {
            return false;
        };
        let Some(passenger) = world.get_player_by_id(player.entity_id()) else {
            return false;
        };
        self.vehicle
            .entity
            .add_passenger(vehicle, passenger as Arc<dyn EntityBase>);
        true
    }
}

impl EntityBase for BoatEntity {
    fn get_entity(&self) -> &Entity {
        &self.vehicle.entity
    }

    fn get_living_entity(&self) -> Option<&LivingEntity> {
        None
    }

    fn tick(&self, caller: &dyn EntityBase, server: &Server) {
        self.vehicle.tick();
        // AbstractBoat.tick -> Entity.tick applies fluid contact (Entity.lavaHurt).
        self.vehicle.entity.tick(caller, server);
        self.vehicle.entity.tick_block_collisions(caller);

        let underwater = self.ticks_underwater.load();
        if self.vehicle.entity.touching_water.load(Ordering::Relaxed) {
            self.ticks_underwater.store((underwater + 1.0).min(60.0));
        } else if underwater > 0.0 {
            self.ticks_underwater.store((underwater - 1.0).max(0.0));
        }
    }

    fn init_data_tracker(&self) {
        self.send_wobble_metadata();
    }

    fn can_hit(&self) -> bool {
        self.vehicle.entity.is_alive()
    }

    fn is_collidable(&self, _entity: Option<Box<dyn EntityBase>>) -> bool {
        true
    }

    fn damage_with_context(
        &self,
        _caller: &dyn EntityBase,
        amount: f32,
        _damage_type: DamageType,
        _position: Option<Vector3<f64>>,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) -> bool {
        self.vehicle
            .damage_with_destruction(amount, source, cause, |creative| {
                // VehicleEntity.destroy drops its item only for the owned destructive removal.
                if self.vehicle.entity.world.load().remove_entity(self) && !creative {
                    self.destroy();
                }
            })
    }

    fn on_removed(&self, reason: crate::entity::RemovalReason) {
        // AbstractChestBoat.remove scatters only for destructive removal, including Entity.kill.
        if reason.should_destroy() {
            self.vehicle
                .destruction_claimed
                .store(true, Ordering::Release);
            self.drop_contents();
        }
    }

    fn interact(&self, player: &Arc<Player>, _item_stack: &mut ItemStack) -> bool {
        if self.try_mount(player) {
            return true;
        }
        // AbstractChestBoat.interact keeps Pass when mounting is still possible.
        if self.can_add_passenger() && !player.get_entity().is_sneaking() {
            return false;
        }
        // AbstractChestBoat.interact opens the container when AbstractBoat.interact passes.
        self.inventory.as_ref().is_some_and(|inventory| {
            let entity = &self.vehicle.entity;
            container::open(
                entity,
                entity.custom_name.load().as_ref().clone(),
                player,
                inventory,
                TextComponent::translate(
                    format!("entity.minecraft.{}", entity.entity_type.resource_name),
                    [],
                ),
                false,
            )
        })
    }

    fn write_custom_nbt(&self, nbt: &mut NbtCompound) {
        if let Some(inventory) = &self.inventory {
            inventory.write_nbt(nbt);
        }
    }

    fn read_custom_nbt(&self, nbt: &NbtCompound) {
        if let Some(inventory) = &self.inventory {
            inventory.read_nbt(nbt);
        }
    }

    fn set_paddle_state(&self, left: bool, right: bool) {
        self.set_paddles(left, right);
    }
    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }

    fn is_pushable(&self) -> bool {
        true
    }
}
