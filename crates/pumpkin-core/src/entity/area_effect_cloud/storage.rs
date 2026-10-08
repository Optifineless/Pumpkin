use super::AreaEffectCloudEntity;
use crate::entity::projectile::potion_effects;
use pumpkin_data::{
    data_component::DataComponent,
    data_component_impl::{
        DataComponentImpl, PotionContentsImpl, PotionDurationScaleImpl, StatusEffectInstance,
    },
    item::Item,
    item_stack::ItemStack,
    particle::Particle,
};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_protocol::{codec::var_int::VarInt, java::client::play::MetadataSerializer};

#[derive(Clone)]
struct CloudParticle {
    particle: Particle,
    data: [u8; 4],
}
impl MetadataSerializer for CloudParticle {
    fn write_metadata(
        &self,
        writer: &mut impl std::io::Write,
        _version: &pumpkin_util::version::JavaMinecraftVersion,
    ) -> Result<(), pumpkin_protocol::ser::WritingError> {
        use pumpkin_protocol::ser::NetworkWriteExt;
        writer.write_var_int(&VarInt(self.particle as i32))?;
        writer.write_slice(&self.data)
    }
}

impl AreaEffectCloudEntity {
    pub(super) fn sync_cloud_data(&self) {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (radius, waiting) = (state.radius, state.age < state.wait_time);
        drop(state);
        self.sync_radius(radius);
        let dragon = *self
            .dragon
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let effects = self
            .effects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let item = self
            .item_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let particle = if dragon {
            CloudParticle {
                particle: Particle::DragonBreath,
                data: 1.0f32.to_be_bytes(),
            }
        } else {
            CloudParticle {
                particle: Particle::EntityEffect,
                data: potion_effects::potion_color(&item, &effects).to_be_bytes(),
            }
        };
        self.entity.set_synced_data(
            pumpkin_data::tracked_data::area_effect_cloud::PARTICLE,
            particle,
        );
        self.entity.set_synced_data(
            pumpkin_data::tracked_data::area_effect_cloud::WAITING,
            waiting,
        );
    }

    // AreaEffectCloud.addAdditionalSaveData / readAdditionalSaveData; victims intentionally are not saved.
    pub(super) fn write_cloud(&self, nbt: &mut NbtCompound) {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        nbt.put_int("Age", state.age);
        nbt.put_int("Duration", state.duration);
        nbt.put_int("WaitTime", state.wait_time);
        nbt.put_int("ReapplicationDelay", state.reapplication_delay);
        nbt.put_int("DurationOnUse", state.duration_on_use);
        nbt.put_float("Radius", state.radius);
        nbt.put_float("RadiusOnUse", state.radius_on_use);
        nbt.put_float("RadiusPerTick", state.radius_per_tick);
        drop(state);
        if let Some(owner) = self.owner.owner_uuid() {
            nbt.put_uuid("Owner", owner);
        }
        let stack = self
            .item_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let original = stack.get_data_component::<PotionContentsImpl>();
        let effects = self
            .effects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let contents = PotionContentsImpl {
            potion_id: None,
            custom_color: original.and_then(|p| p.custom_color),
            custom_name: original.and_then(|p| p.custom_name.clone()),
            custom_effects: effects
                .iter()
                .map(
                    |(effect, duration, amplifier, ambient, show_particles, show_icon)| {
                        StatusEffectInstance {
                            effect_id: effect.minecraft_name.into(),
                            duration: *duration,
                            amplifier: i32::from(*amplifier),
                            ambient: *ambient,
                            show_particles: *show_particles,
                            show_icon: *show_icon,
                        }
                    },
                )
                .collect(),
        };
        nbt.put("potion_contents", contents.write_data());
        nbt.put_float(
            "potion_duration_scale",
            stack
                .get_data_component::<PotionDurationScaleImpl>()
                .map_or(1.0, |scale| scale.scale),
        );
        if *self
            .dragon
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
        {
            let mut particle = NbtCompound::new();
            particle.put_string("type", "minecraft:dragon_breath".to_string());
            particle.put_float("power", 1.0);
            nbt.put_compound("custom_particle", particle);
        }
    }

    pub(super) fn read_cloud(&self, nbt: &NbtCompound) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.age = nbt.get_int("Age").unwrap_or(0);
        state.duration = nbt.get_int("Duration").unwrap_or(-1);
        state.wait_time = nbt.get_int("WaitTime").unwrap_or(20);
        state.reapplication_delay = nbt.get_int("ReapplicationDelay").unwrap_or(20);
        state.duration_on_use = nbt.get_int("DurationOnUse").unwrap_or(0);
        state.radius = nbt.get_float("Radius").unwrap_or(3.0).clamp(0.0, 32.0);
        state.radius_on_use = nbt.get_float("RadiusOnUse").unwrap_or(0.0);
        state.radius_per_tick = nbt.get_float("RadiusPerTick").unwrap_or(0.0);
        state.victims.clear();
        drop(state);
        self.owner.read_nbt(nbt);
        let mut stack = ItemStack::new(1, &Item::POTION);
        if let Some(contents) = nbt
            .get_compound("potion_contents")
            .and_then(|c| PotionContentsImpl::read_data(&NbtTag::Compound(c.clone())))
        {
            stack
                .patch
                .push((DataComponent::PotionContents, Some(contents.to_dyn())));
        }
        stack.patch.push((
            DataComponent::PotionDurationScale,
            Some(
                PotionDurationScaleImpl {
                    scale: nbt.get_float("potion_duration_scale").unwrap_or(1.0),
                }
                .to_dyn(),
            ),
        ));
        *self
            .effects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            crate::item::potion::PotionContents::read_potion_effects(&stack);
        *self
            .item_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = stack;
        *self
            .dragon
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = nbt
            .get_compound("custom_particle")
            .and_then(|particle| particle.get_string("type"))
            .is_some_and(|kind| kind == "minecraft:dragon_breath");
        self.sync_cloud_data();
    }
}
