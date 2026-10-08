//! `ZombieNautilus` spawn variant and persistence on the existing `AbstractNautilus` implementation.
use super::nautilus::NautilusEntity;
use crate::entity::{
    Entity, EntityBase,
    mob::{Mob, MobEntity},
};
use pumpkin_data::{tracked_data, zombie_nautilus_variant::ZombieNautilusVariant};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::codec::var_int::VarInt;
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering::Relaxed},
};

pub struct ZombieNautilusEntity {
    nautilus: Arc<NautilusEntity>,
    variant: AtomicU8,
}
impl ZombieNautilusEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let nautilus = NautilusEntity::new(entity);
        nautilus
            .get_mob_entity()
            .brain
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .register_memory(crate::entity::ai::brain::memory::types::ATTACK_TARGET_COOLDOWN.id());
        Arc::new(Self {
            nautilus,
            variant: AtomicU8::new(ZombieNautilusVariant::default().id()),
        })
    }
}
impl Mob for ZombieNautilusEntity {
    // ZombieNautilus inherits AbstractNautilus.isPushedByFluid and travelInWater.
    fn mob_is_pushed_by_fluids(&self) -> bool {
        self.nautilus.mob_is_pushed_by_fluids()
    }

    fn custom_travel(&self, caller: &dyn EntityBase) -> bool {
        self.nautilus.custom_travel(caller)
    }

    fn make_brain(
        &self,
        packed: &crate::entity::ai::brain::memory::PackedMemories,
    ) -> crate::entity::ai::brain::Brain {
        let mut brain = crate::entity::ai::brain::Brain::default();
        brain.register_memory(crate::entity::ai::brain::memory::types::ATTACK_TARGET_COOLDOWN.id());
        brain.load_packed(packed);
        brain
    }
    fn get_mob_entity(&self) -> &MobEntity {
        self.nautilus.get_mob_entity()
    }
    fn finalize_spawn_with_context(
        &self,
        _entity: &Arc<dyn EntityBase>,
        _view: &crate::world::spawn_view::SpawnView<'_>,
        _difficulty: &crate::entity::mob::equipment::RegionalDifficulty,
        _reason: crate::entity::mob::spawn::SpawnReason,
        group: Option<crate::entity::mob::spawn::SpawnGroupData>,
    ) -> Option<crate::entity::mob::spawn::SpawnGroupData> {
        use crate::entity::mob::spawn::{AgeableGroupData, SpawnGroupData};
        // AbstractNautilus.finalizeSpawn -> NautilusAi.initMemories (Java:45), then AgeableMob.
        self.get_mob_entity()
            .brain
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .set(
                crate::entity::ai::brain::memory::types::ATTACK_TARGET_COOLDOWN,
                rand::random_range(2400..=3600),
            );
        let mut data = match group {
            Some(SpawnGroupData::Ageable(data)) => data,
            _ => AgeableGroupData::new(true, 0.05),
        };
        // ZombieNautilus.canBeABaby is false; AgeableMob still advances the shared group.
        data.next_is_baby(rand::random());
        self.get_mob_entity().finalize_spawn_base();
        Some(SpawnGroupData::Ageable(data))
    }
    fn requires_custom_persistence(&self) -> bool {
        self.nautilus.requires_custom_persistence()
    }
    fn mob_set_variant_name(&self, name: &str) {
        if let Some(variant) = ZombieNautilusVariant::from_name(name) {
            self.variant.store(variant.id(), Relaxed);
            self.get_entity().set_synced_data(
                tracked_data::zombie_nautilus::DATA_VARIANT_ID,
                VarInt(i32::from(variant.id())),
            );
        }
    }
    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        self.nautilus.mob_write_nbt(nbt);
        let variant =
            ZombieNautilusVariant::from_id(self.variant.load(Relaxed)).unwrap_or_default();
        nbt.put_string("variant", format!("minecraft:{}", variant.to_name()));
    }
    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.nautilus.mob_read_nbt(nbt);
        if let Some(name) = nbt.get_string("variant") {
            self.mob_set_variant_name(name);
        }
    }
    fn mob_init_data_tracker(&self) {
        self.nautilus.mob_init_data_tracker();
        self.get_entity().set_synced_data(
            tracked_data::zombie_nautilus::DATA_VARIANT_ID,
            VarInt(i32::from(self.variant.load(Relaxed))),
        );
    }
    fn mob_tick(&self, caller: &dyn EntityBase) {
        self.nautilus.mob_tick(caller);
    }
}
