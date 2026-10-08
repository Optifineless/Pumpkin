use super::{deflection, trident::TridentEntity};
use crate::entity::{EntityBase, projectile_deflection::ProjectileDeflectionType};
use pumpkin_data::{
    damage::DamageType,
    entity::EntityType,
    item_stack::ItemStack,
    sound::{Sound, SoundCategory},
    tag::{self, Taggable},
};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::vector3::Vector3;
use std::sync::atomic::Ordering;

impl TridentEntity {
    // ThrownTrident.onHitEntity marks even rejected damage and uses anisotropic reverse deflection.
    pub(super) fn hit_entity(&self, target: &dyn EntityBase, hit_pos: Vector3<f64>) {
        let owner = self.projectile_owner();
        self.dealt_damage.store(true, Ordering::Relaxed);
        let item = self
            .item_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let mut damage = Self::BASE_DAMAGE;
        // The generated Impaling effect supplies the amount; its requirement is the data's sensitive_to_impaling tag.
        if target
            .get_entity()
            .entity_type
            .has_tag(&tag::EntityType::MINECRAFT_SENSITIVE_TO_IMPALING)
        {
            let level = item.get_enchantment_level(&pumpkin_data::Enchantment::IMPALING);
            if level > 0 {
                pumpkin_data::Enchantment::IMPALING.modify_damage(level, &mut damage);
            }
        }
        let hurt = super::damage::hurt_entity(
            target,
            damage as f32,
            DamageType::TRIDENT,
            self,
            owner.as_deref().or(Some(self)),
        );
        if hurt {
            super::damage::post_attack_with_item(
                target,
                DamageType::TRIDENT,
                self,
                owner.as_deref(),
                Some(item),
            );
        }
        // EnderMan.projectileReceivesSideEffectsOnHit accepts effects only after an accepted hit.
        if hurt || target.get_entity().entity_type != &EntityType::ENDERMAN {
            self.entity.world.load().play_sound_fine(
                Sound::ItemTridentHit,
                SoundCategory::Neutral,
                &hit_pos,
                1.0,
                1.0,
            );
            let owner_uuid = self.projectile.owner_uuid();
            deflection::deflect(
                self,
                ProjectileDeflectionType::Simple,
                Some(target),
                owner.as_deref(),
                false,
                Vector3::new(0.02, 0.2, 0.02),
            );
            self.projectile.set_owner_uuid(owner_uuid);
        }
        self.has_hit.store(false, Ordering::Relaxed);
    }

    // AbstractArrow and ThrownTrident's additional save data, in addition to Projectile's owner UUID.
    pub(super) fn write_trident(&self, nbt: &mut NbtCompound) {
        nbt.put_bool("DealtDamage", self.dealt_damage.load(Ordering::Relaxed));
        nbt.put_bool("inGround", self.in_ground.load(Ordering::Relaxed));
        nbt.put_byte("pickup", self.pickup.load().to_byte() as i8);
        nbt.put_short("life", self.life.load(Ordering::Relaxed) as i16);
        let mut item = NbtCompound::new();
        self.item_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .write_item_stack(&mut item);
        nbt.put_compound("item", item);
    }

    pub(super) fn read_trident(&self, nbt: &NbtCompound) {
        self.pickup.store(super::arrow::ArrowPickup::from_byte(
            nbt.get_byte("pickup").unwrap_or(0) as u8,
        ));
        self.dealt_damage.store(
            nbt.get_bool("DealtDamage").unwrap_or(false),
            Ordering::Relaxed,
        );
        self.in_ground
            .store(nbt.get_bool("inGround").unwrap_or(false), Ordering::Relaxed);
        self.life.store(
            nbt.get_short("life").unwrap_or(0).max(0) as u32,
            Ordering::Relaxed,
        );
        if let Some(item) = nbt
            .get_compound("item")
            .and_then(ItemStack::read_item_stack)
        {
            *self
                .item_stack
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = item;
        }
    }
}
