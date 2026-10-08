use std::sync::Arc;

use pumpkin_data::sound::Sound;

use crate::entity::{
    Entity, EntityBase,
    mob::{Mob, MobEntity},
    passive::fish,
};

// Salmon.Variant.DEFAULT is MEDIUM.
const DEFAULT: i32 = 1;

/// Represents a Salmon, a passive fish of rivers and cold oceans.
///
/// Wiki: <https://minecraft.wiki/w/Salmon>
pub struct SalmonEntity {
    pub mob_entity: MobEntity,
    // Salmon.DATA_TYPE, medium is Variant.DEFAULT.
    size: std::sync::atomic::AtomicI32,
}

impl SalmonEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        fish::init(&mob_entity);
        Arc::new(Self {
            mob_entity,
            size: std::sync::atomic::AtomicI32::new(DEFAULT),
        })
    }

    // Salmon.applyImplicitComponents / setVariant.
    pub(crate) fn apply_bucket_components(&self, stack: &pumpkin_data::item_stack::ItemStack) {
        if let Some(size) =
            stack.get_data_component::<pumpkin_data::data_component_impl::SalmonSizeImpl>()
        {
            self.set_size(size.variant_id().unwrap_or(DEFAULT));
        }
    }

    fn set_size(&self, value: i32) {
        self.size.store(value, std::sync::atomic::Ordering::Relaxed);
        let entity = self.get_entity();
        entity.set_synced_data(
            pumpkin_data::tracked_data::salmon::DATA_TYPE,
            pumpkin_protocol::VarInt(value),
        );
        // Salmon.Variant boundingBoxScale and getDefaultDimensions.
        let scale = match value {
            0 => 0.5,
            2 => 1.5,
            _ => 1.0,
        };
        let mut dimensions = crate::entity::Entity::type_dimensions(entity.entity_type);
        dimensions.width *= scale;
        dimensions.height *= scale;
        dimensions.eye_height *= scale;
        entity.entity_dimension.store(dimensions);
        let pos = entity.pos.load();
        entity
            .bounding_box
            .store(pumpkin_util::math::boundingbox::BoundingBox::new_from_pos(
                pos.x,
                pos.y,
                pos.z,
                &dimensions,
            ));
    }
}

impl Mob for SalmonEntity {
    // Salmon.addAdditionalSaveData / readAdditionalSaveData.
    fn mob_write_nbt(&self, nbt: &mut pumpkin_nbt::compound::NbtCompound) {
        if let Some(size) = pumpkin_data::data_component_impl::SalmonSizeImpl::from_variant_id(
            self.size.load(std::sync::atomic::Ordering::Relaxed),
        ) {
            nbt.put_string("type", size.value.into_owned());
        }
    }

    fn mob_read_nbt(&self, nbt: &pumpkin_nbt::compound::NbtCompound) {
        let size = pumpkin_data::data_component_impl::SalmonSizeImpl {
            value: nbt.get_string("type").unwrap_or("medium").to_owned().into(),
        };
        self.set_size(size.variant_id().unwrap_or(DEFAULT));
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn mob_tick(&self, _caller: &dyn EntityBase) {
        fish::flop(self, Sound::EntitySalmonFlop);
    }

    fn custom_travel(&self, caller: &dyn EntityBase) -> bool {
        fish::travel(self, caller)
    }

    fn mob_is_pushed_by_fluids(&self) -> bool {
        false
    }
}
