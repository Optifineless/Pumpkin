use super::*;
use crate::entity::{
    Entity,
    living::{LivingEntity, test_support::armor_test_world},
    tnt::TNTEntity,
};
use pumpkin_data::{damage::DamageType, entity::EntityType};
use pumpkin_nbt::compound::NbtCompound;
use std::sync::Mutex;

type DamageContext = (u8, Option<i32>, Option<i32>);

struct Victim {
    entity: Entity,
    context: Mutex<Option<DamageContext>>,
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
        kind: DamageType,
        _position: Option<Vector3<f64>>,
        direct: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) -> bool {
        *self.context.lock().unwrap() = Some((
            kind.id,
            direct.map(|e| e.get_entity().entity_id),
            cause.map(|e| e.get_entity().entity_id),
        ));
        true
    }
}

#[tokio::test]
async fn primed_tnt_reload_and_explosion_retain_the_igniter_and_direct_source() {
    let dir = tempfile::tempdir().unwrap();
    let world = armor_test_world(dir.path());
    let owner = Arc::new(LivingEntity::new(Entity::new(
        world.clone(),
        Vector3::default(),
        &EntityType::COW,
    )));
    world.entities.store(Arc::new(vec![owner.clone()]));
    let tnt = TNTEntity::new(
        Entity::new(world.clone(), Vector3::default(), &EntityType::TNT),
        4.0,
        80,
    );
    tnt.set_owner(Some(owner.as_ref()));
    let mut nbt = NbtCompound::new();
    EntityBase::write_nbt(&tnt, &mut nbt);
    assert_eq!(nbt.get_uuid("owner"), Some(owner.entity.entity_uuid));
    let restored = Arc::new(TNTEntity::new(
        Entity::new(world.clone(), Vector3::default(), &EntityType::TNT),
        4.0,
        80,
    ));
    EntityBase::read_nbt_non_mut(restored.as_ref(), &nbt);
    let explosion = Explosion::new(4.0, Vector3::default(), BlockInteraction::Destroy)
        .with_source(Some(restored.clone()));
    restored.get_entity().remove();
    let target = Victim {
        entity: Entity::new(world, Vector3::default(), &EntityType::COW),
        context: Mutex::new(None),
    };
    explosion.hurt_from_explosion(&target, 22.0);
    assert_eq!(
        *target.context.lock().unwrap(),
        Some((
            DamageType::PLAYER_EXPLOSION.id,
            Some(restored.get_entity().entity_id),
            Some(owner.entity.entity_id)
        ))
    );
    let chained_owner = explosion.indirect_source().unwrap();
    assert_eq!(
        chained_owner.get_entity().entity_uuid,
        owner.entity.entity_uuid
    );
}

#[tokio::test]
async fn crystal_attacker_gets_damage_credit_without_owning_chained_tnt() {
    use crate::entity::decoration::end_crystal::EndCrystalEntity;
    use pumpkin_data::Block;
    use pumpkin_util::math::{position::BlockPos, vector2::Vector2};
    let dir = tempfile::tempdir().unwrap();
    let server = crate::server::combat_test_support::server(dir.path());
    let world = crate::server::combat_test_support::world(&server, dir.path());
    let player = crate::net::java::combat_test_support::TestPlayer::new(&world).player;
    player.get_entity().set_pos(Vector3::new(100.0, 64.0, 0.0));
    let crystal = Arc::new(EndCrystalEntity::new(Entity::new(
        world.clone(),
        Vector3::new(8.5, 64.5, 8.5),
        &EntityType::END_CRYSTAL,
    )));
    let target = Arc::new(Victim {
        entity: Entity::new(
            world.clone(),
            Vector3::new(10.5, 64.0, 8.5),
            &EntityType::COW,
        ),
        context: Mutex::default(),
    });
    world
        .entities
        .store(Arc::new(vec![crystal.clone(), target.clone()]));
    let ore_pos = BlockPos::new(9, 64, 8);
    let chunk = pumpkin_world::chunk::ChunkData::empty_sync(0, 0);
    chunk.set_block_absolute_y(8, 64, 8, Block::TNT.default_state.id);
    chunk.set_block_absolute_y(9, 64, 8, Block::DIAMOND_ORE.default_state.id);
    world.level.loaded_chunks.insert(Vector2::new(0, 0), chunk);
    assert!(crystal.damage_with_context(
        crystal.as_ref(),
        1.0,
        DamageType::PLAYER_ATTACK,
        None,
        Some(player.as_ref()),
        Some(player.as_ref()),
    ));
    assert!(crystal.get_entity().is_removed());
    assert_eq!(
        *target.context.lock().unwrap(),
        Some((
            DamageType::PLAYER_EXPLOSION.id,
            Some(crystal.get_entity().entity_id),
            Some(player.get_entity().entity_id)
        ))
    );
    let entities = world.entities.load();
    let tnt = entities
        .iter()
        .find_map(|entity| entity.cast_any().downcast_ref::<TNTEntity>())
        .unwrap();
    assert!(tnt.owner().is_none());
    assert!(world.get_block(&ore_pos).is_air());
    assert!(
        entities
            .iter()
            .all(|entity| entity.get_entity().entity_type != &EntityType::EXPERIENCE_ORB)
    );
}

#[tokio::test]
async fn verification_explosion_redirects_fireball_and_wind_charge_to_custom_cause() {
    use crate::entity::projectile::{
        ThrownItemEntity, fireball::FireballEntity, ownership::ProjectileState,
        wind_charge::WindChargeEntity,
    };
    let dir = tempfile::tempdir().unwrap();
    let world = armor_test_world(dir.path());
    let igniter = Arc::new(LivingEntity::new(Entity::new(
        world.clone(),
        Vector3::default(),
        &EntityType::COW,
    )));
    let attacker = Arc::new(LivingEntity::new(Entity::new(
        world.clone(),
        Vector3::default(),
        &EntityType::COW,
    )));
    let fireball = Arc::new(FireballEntity::new(Entity::new(
        world.clone(),
        Vector3::new(1.0, 64.0, 0.0),
        &EntityType::FIREBALL,
    )));
    let charge = Arc::new(WindChargeEntity::new_normal(ThrownItemEntity {
        entity: Entity::new(
            world.clone(),
            Vector3::new(1.0, 64.0, 0.0),
            &EntityType::WIND_CHARGE,
        ),
        projectile: ProjectileState::default(),
        has_hit: std::sync::atomic::AtomicBool::new(false),
        gravity: 0.0,
    }));
    world.entities.store(Arc::new(vec![
        igniter.clone(),
        attacker.clone(),
        fireball.clone(),
        charge.clone(),
    ]));
    for cause in [Some(attacker as Arc<dyn EntityBase>), None] {
        let explosion = Explosion::new(4.0, Vector3::new(0.0, 64.0, 0.0), BlockInteraction::Keep)
            .with_source(Some(igniter.clone()))
            .with_cause(cause.clone());
        explosion.damage_entities(&world);
        for projectile in [fireball.as_ref() as &dyn EntityBase, charge.as_ref()] {
            assert_eq!(
                projectile
                    .projectile_owner()
                    .map(|owner| owner.get_entity().entity_uuid),
                cause.as_ref().map(|owner| owner.get_entity().entity_uuid)
            );
        }
        assert_eq!(
            explosion
                .indirect_source()
                .unwrap()
                .get_entity()
                .entity_uuid,
            igniter.entity.entity_uuid
        );
    }
}
