use std::sync::Arc;

use pumpkin_data::{attributes::Attributes, sound::Sound};

use crate::entity::{
    Entity, EntityBase,
    custom_sound::CustomSound,
    mob::{Mob, MobEntity, slime::SlimeEntity},
};

pub struct MagmaCubeEntity {
    pub slime: Arc<SlimeEntity>,
}

impl MagmaCubeEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let slime = SlimeEntity::new(entity);
        Arc::new(Self { slime })
    }

    /// `MagmaCube.setSize` inherits cube health/speed, then updates armor and attack damage.
    pub fn set_size(&self, size: i32, update_health: bool) {
        self.slime.set_size(size, update_health);
        self.get_mob_entity()
            .living_entity
            .set_attribute_base(&Attributes::ARMOR, f64::from(size.wrapping_mul(3)));
    }
    // MagmaCube.getAttackDamage adds two after reading the size-dependent attribute.
    // Keep the existing contact-attack side effects while leaving saved attributes vanilla-shaped.
    fn attack_with_cube_damage(&self, target: &dyn EntityBase) {
        use std::sync::atomic::Ordering::Relaxed;
        let living = &self.get_mob_entity().living_entity;
        if living.dead.load(Relaxed) {
            return;
        }
        let damage = living.get_attribute_value(&Attributes::ATTACK_DAMAGE) as f32 + 2.0;
        if target.damage_with_context(
            target,
            damage,
            pumpkin_data::damage::DamageType::MOB_ATTACK,
            None,
            Some(self),
            Some(self),
        ) {
            living.set_last_hurt_mob(target);
        }
    }
}

impl CustomSound for MagmaCubeEntity {
    fn death_sound(&self) -> Option<Sound> {
        let size = self.slime.get_size();
        Some(if size == 1 {
            Sound::EntityMagmaCubeDeathSmall
        } else {
            Sound::EntityMagmaCubeDeath
        })
    }
    fn hurt_sound(&self) -> Option<Sound> {
        let size = self.slime.get_size();
        Some(if size == 1 {
            Sound::EntityMagmaCubeHurtSmall
        } else {
            Sound::EntityMagmaCubeHurt
        })
    }
}

impl Mob for MagmaCubeEntity {
    fn mob_pre_load_nbt(&self, nbt: &pumpkin_nbt::compound::NbtCompound) {
        self.set_size(super::cube_spawn::read_size(nbt), false);
    }
    fn mob_read_nbt(&self, nbt: &pumpkin_nbt::compound::NbtCompound) {
        self.slime.mob_read_nbt(nbt);
    }
    fn mob_write_nbt(&self, nbt: &mut pumpkin_nbt::compound::NbtCompound) {
        self.slime.mob_write_nbt(nbt);
    }
    fn get_base_experience_reward(&self) -> u32 {
        // Slime / MagmaCube.setSize set xpReward to the actual size.
        super::equipped_mob_experience(
            &self.get_mob_entity().living_entity,
            self.slime.get_size() as u32,
        )
    }
    fn finalize_spawn_with_context(
        &self,
        _entity: &Arc<dyn EntityBase>,
        _view: &crate::world::spawn_view::SpawnView<'_>,
        difficulty: &super::equipment::RegionalDifficulty,
        _reason: super::spawn::SpawnReason,
        group: Option<super::spawn::SpawnGroupData>,
    ) -> Option<super::spawn::SpawnGroupData> {
        let mut random = pumpkin_util::random::RandomGenerator::Xoroshiro(
            pumpkin_util::random::xoroshiro128::Xoroshiro::from_seed(
                pumpkin_util::random::get_seed(),
            ),
        );
        Some(super::cube_spawn::finalize_spawn(
            self,
            difficulty,
            group,
            &mut random,
            |size, health| {
                self.set_size(size, health);
            },
        ))
    }

    // AbstractCubeMob.getMaxHeadXRot.
    fn get_max_look_pitch_change(&self) -> f32 {
        0.0
    }

    fn get_mob_entity(&self) -> &MobEntity {
        self.slime.get_mob_entity()
    }

    fn mob_tick(&self, caller: &dyn EntityBase) {
        self.slime.mob_tick(caller);
    }

    fn post_tick(&self) {
        self.slime.post_tick();
    }

    fn mob_player_collision(&self, player: &Arc<crate::entity::player::Player>) {
        self.attack_with_cube_damage(&**player);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::spawn_test_support::Fixture;
    use pumpkin_data::entity::EntityType;
    use pumpkin_util::math::vector3::Vector3;
    #[tokio::test]
    async fn size_attribute_keeps_magma_contact_damage_bonus() {
        let fixture = Fixture::new();
        let cube = MagmaCubeEntity::new(Entity::new(
            fixture.world.clone(),
            Vector3::new(8.0, 64.0, 8.0),
            &EntityType::MAGMA_CUBE,
        ));
        cube.set_size(4, true);
        let target = crate::entity::r#type::from_type(
            &EntityType::COW,
            Vector3::new(8.0, 64.0, 8.0),
            &fixture.world,
            uuid::Uuid::new_v4(),
        );
        let living = target.get_living_entity().unwrap();
        cube.attack_with_cube_damage(target.as_ref());
        // LivingEntity records the submitted hit before its server-only animation/application tail.
        assert_eq!(living.last_damage_taken.load(), 6.0);
        assert_eq!(
            cube.get_mob_entity()
                .living_entity
                .get_attribute_value(&Attributes::ATTACK_DAMAGE),
            4.0
        );
        fixture.finish().await;
    }
}
