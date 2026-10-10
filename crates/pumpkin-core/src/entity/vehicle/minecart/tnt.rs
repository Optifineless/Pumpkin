use crate::entity::EntityBase;
use crate::world::ExplosionDamageSource;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI32, Ordering};

use crossbeam::atomic::AtomicCell;
use pumpkin_data::entity::EntityStatus;
use pumpkin_data::particle::Particle;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::bedrock::server::actor_event::ActorEventID;
use pumpkin_util::math::vector3::Vector3;
use rand::RngExt;

use crate::entity::Entity;

pub(super) struct TntMinecart {
    fuse: AtomicI32,
    ignition_source: Mutex<Option<ExplosionDamageSource>>,
    explosion_power: AtomicCell<f32>,
    explosion_speed_factor: AtomicCell<f32>,
}

impl TntMinecart {
    pub(super) const fn new() -> Self {
        Self {
            fuse: AtomicI32::new(-1),
            ignition_source: Mutex::new(None),
            explosion_power: AtomicCell::new(4.0),
            explosion_speed_factor: AtomicCell::new(1.0),
        }
    }

    pub(super) fn prime(&self, entity: &Entity, fuse: i32, cause: Option<&dyn EntityBase>) {
        if self.fuse.load(Ordering::Relaxed) >= 0
            || !entity
                .world
                .load()
                .level_info
                .load()
                .game_rules
                .tnt_explodes
        {
            return;
        }

        // MinecartTNT.primeFuse captures a custom explosion source once.
        if let Some(cause) = cause {
            let mut ignition = self
                .ignition_source
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if ignition.is_none() {
                *ignition = Some(ExplosionDamageSource {
                    cause: crate::entity::projectile::ownership::resolve_owner(
                        &entity.world.load(),
                        cause.get_entity().entity_uuid,
                    ),
                });
            }
        }
        self.fuse.store(fuse, Ordering::Relaxed);
        let world = entity.world.load();
        world.send_entity_status(
            entity,
            EntityStatus::TntPrime,
            Some(ActorEventID::PrimeTNTCart),
        );
        world.play_sound(
            Sound::EntityTntPrimed,
            SoundCategory::Blocks,
            &entity.pos.load(),
        );
    }

    pub(super) fn tick(&self, entity: &Entity) -> bool {
        let fuse = self.fuse.load(Ordering::Relaxed);
        if fuse > 0 {
            self.fuse.store(fuse - 1, Ordering::Relaxed);
            let mut smoke_pos = entity.pos.load();
            smoke_pos.y += 0.5;
            entity.world.load().spawn_particle(
                smoke_pos,
                Vector3::new(0.0, 0.0, 0.0),
                0.0,
                1,
                Particle::Smoke,
            );
        } else if fuse == 0 {
            let velocity = entity.velocity.load();
            self.explode(
                entity,
                velocity.x.mul_add(velocity.x, velocity.z * velocity.z),
                None,
            );
            return true;
        }
        false
    }

    pub(super) fn explode(
        &self,
        entity: &Entity,
        horizontal_speed_squared: f64,
        source: Option<ExplosionDamageSource>,
    ) {
        let world = entity.world.load();
        if !world.level_info.load().game_rules.tnt_explodes {
            if self.fuse.load(Ordering::Relaxed) > -1 {
                entity.remove();
            }
            return;
        }

        let power = Self::explosion_strength(
            self.explosion_power.load(),
            self.explosion_speed_factor.load(),
            horizontal_speed_squared,
            rand::rng().random_range(0.0..1.0),
        );
        let pos = entity.pos.load();
        let primed = self.fuse.load(Ordering::Relaxed) > -1;
        // MinecartTNT.explode retains the direct minecart while applying its primed rail calculator.
        let mut explosion = crate::world::Explosion::new(
            power,
            pos,
            world.get_block_interaction(crate::world::ExplosionInteraction::Tnt),
        )
        .with_source(world.get_entity_by_id(entity.entity_id));
        let ignition = source.or_else(|| {
            self.ignition_source
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        });
        if let Some(source) = ignition {
            explosion = explosion.with_cause(source.cause);
        }
        if primed {
            explosion = explosion.preserving_rails();
        }
        entity.remove();
        world.run_explosion(&explosion);
    }

    pub(super) fn set_fuse(&self, fuse: i32) {
        self.fuse.store(fuse, Ordering::Relaxed);
    }

    pub(super) fn write_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put_int("fuse", self.fuse.load(Ordering::Relaxed));
        let power = self.explosion_power.load();
        if power != 4.0 {
            nbt.put_float("explosion_power", power);
        }
        let speed_factor = self.explosion_speed_factor.load();
        if speed_factor != 1.0 {
            nbt.put_float("explosion_speed_factor", speed_factor);
        }
    }

    pub(super) fn read_nbt(&self, nbt: &NbtCompound) {
        self.fuse
            .store(nbt.get_int("fuse").unwrap_or(-1), Ordering::Relaxed);
        self.explosion_power.store(
            nbt.get_float("explosion_power")
                .unwrap_or(4.0)
                .clamp(0.0, 128.0),
        );
        self.explosion_speed_factor.store(
            nbt.get_float("explosion_speed_factor")
                .unwrap_or(1.0)
                .clamp(0.0, 128.0),
        );
    }

    fn explosion_strength(
        base: f32,
        speed_factor: f32,
        horizontal_speed_squared: f64,
        random: f32,
    ) -> f32 {
        let speed = horizontal_speed_squared.sqrt().min(5.0) as f32;
        base + speed_factor * random * 1.5 * speed
    }
}

#[cfg(test)]
mod tests {
    use super::TntMinecart;

    #[test]
    fn tnt_minecart_explosion_bonus_is_speed_capped() {
        assert_eq!(TntMinecart::explosion_strength(4.0, 1.0, 100.0, 1.0), 11.5);
        assert_eq!(TntMinecart::explosion_strength(4.0, 1.0, 100.0, 0.0), 4.0);
    }
}

#[cfg(test)]
mod review_tests {
    use super::*;
    use crate::{
        entity::{
            living::LivingEntity,
            vehicle::minecart::{MinecartEntity, MinecartKind},
        },
        server::combat_test_support::{server, world},
    };
    use pumpkin_data::{damage::DamageType, entity::EntityType};
    use pumpkin_util::math::vector2::Vector2;
    use std::sync::Arc;

    struct Victim {
        entity: Entity,
        cause: Mutex<Option<i32>>,
    }
    impl EntityBase for Victim {
        fn get_entity(&self) -> &Entity {
            &self.entity
        }
        fn get_living_entity(&self) -> Option<&LivingEntity> {
            None
        }
        fn cast_any(&self) -> &dyn std::any::Any {
            self
        }
        fn damage_with_context(
            &self,
            _caller: &dyn EntityBase,
            _amount: f32,
            _kind: DamageType,
            _pos: Option<Vector3<f64>>,
            _direct: Option<&dyn EntityBase>,
            cause: Option<&dyn EntityBase>,
        ) -> bool {
            *self.cause.lock().unwrap() = cause.map(|owner| owner.get_entity().entity_id);
            true
        }
    }
    fn triggered_minecart_credit(burning_arrow: bool) {
        let dir = tempfile::tempdir().unwrap();
        let server = server(dir.path());
        let world = world(&server, dir.path());
        world.level.loaded_chunks.insert(
            Vector2::new(0, 0),
            pumpkin_world::chunk::ChunkData::empty_sync(0, 0),
        );
        let player = crate::net::java::combat_test_support::TestPlayer::new(&world).player;
        player.get_entity().set_pos(Vector3::new(100.0, 64.0, 0.0));
        let cart = Arc::new(MinecartEntity::new(Entity::new(
            world.clone(),
            Vector3::new(8.5, 64.0, 8.5),
            &EntityType::TNT_MINECART,
        )));
        let victim = Arc::new(Victim {
            entity: Entity::new(
                world.clone(),
                Vector3::new(9.5, 64.0, 8.5),
                &EntityType::COW,
            ),
            cause: Mutex::default(),
        });
        world
            .entities
            .store(Arc::new(vec![cart.clone(), victim.clone()]));
        let arrow = Entity::new(world, Vector3::default(), &EntityType::ARROW);
        if burning_arrow {
            arrow.set_on_fire_for(10.0);
        }
        cart.damage_with_context(
            cart.as_ref(),
            5.0,
            if burning_arrow {
                DamageType::ARROW
            } else {
                DamageType::IN_FIRE
            },
            None,
            burning_arrow.then_some(&arrow as &dyn EntityBase),
            Some(player.as_ref()),
        );
        if !burning_arrow {
            let MinecartKind::Tnt(tnt) = &cart.kind else {
                panic!("TNT cart required");
            };
            for _ in 0..41 {
                if tnt.tick(cart.get_entity()) {
                    break;
                }
            }
        }
        assert!(cart.get_entity().is_removed());
        assert_eq!(
            *victim.cause.lock().unwrap(),
            Some(player.get_entity().entity_id)
        );
    }
    #[tokio::test]
    async fn burning_arrow_minecart_explosion_retains_player_credit() {
        triggered_minecart_credit(true);
        crate::server::fixture_lifecycle::finish().await;
    }
    #[tokio::test]
    async fn primed_minecart_fuse_retains_triggering_damage_credit() {
        triggered_minecart_credit(false);
        crate::server::fixture_lifecycle::finish().await;
    }

    #[tokio::test]
    async fn verification_ownerless_burning_arrow_overrides_minecart_ignition_credit() {
        let dir = tempfile::tempdir().unwrap();
        let server = server(dir.path());
        let world = world(&server, dir.path());
        world.level.loaded_chunks.insert(
            Vector2::new(0, 0),
            pumpkin_world::chunk::ChunkData::empty_sync(0, 0),
        );
        let player = crate::net::java::combat_test_support::TestPlayer::new(&world).player;
        player.get_entity().set_pos(Vector3::new(100.0, 64.0, 0.0));
        let cart = Arc::new(MinecartEntity::new(Entity::new(
            world.clone(),
            Vector3::new(8.5, 64.0, 8.5),
            &EntityType::TNT_MINECART,
        )));
        let victim = Arc::new(Victim {
            entity: Entity::new(
                world.clone(),
                Vector3::new(9.5, 64.0, 8.5),
                &EntityType::COW,
            ),
            // A sentinel proves the victim was actually hit with a null cause.
            cause: Mutex::new(Some(-1)),
        });
        world
            .entities
            .store(Arc::new(vec![cart.clone(), victim.clone()]));
        let MinecartKind::Tnt(tnt) = &cart.kind else {
            panic!("TNT cart required");
        };
        tnt.prime(cart.get_entity(), 80, Some(player.as_ref()));
        let arrow = Entity::new(world, Vector3::default(), &EntityType::ARROW);
        arrow.set_on_fire_for(10.0);
        assert!(cart.damage_with_context(
            cart.as_ref(),
            1.0,
            DamageType::ARROW,
            None,
            Some(&arrow),
            None
        ));
        assert!(cart.get_entity().is_removed());
        assert_eq!(*victim.cause.lock().unwrap(), None);
        crate::server::fixture_lifecycle::finish().await;
    }
}
