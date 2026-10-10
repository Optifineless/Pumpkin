//! Host results are guest trap boundaries, so stale setters must complete successfully.
use std::sync::{Arc, Weak, atomic::Ordering};

use arc_swap::ArcSwap;
use pumpkin_core::{entity::EntityBase, world::World};
use pumpkin_data::{dimension::Dimension, entity::EntityType};
use pumpkin_nbt::tag::NbtTag;
use pumpkin_util::{math::vector3::Vector3, world_seed::Seed};
use pumpkin_wasm_host_common::state::PluginHostState;
use wasmtime::component::Resource;

use crate::pumpkin::plugin::world::{
    Entity, HostEntity, HostLivingEntity, HostMob, LivingEntity, Mob,
};

#[tokio::test]
async fn removed_entity_setters_complete_without_trapping_or_writing() {
    let dir = tempfile::tempdir().unwrap();
    let level = pumpkin_world::level::Level::from_root_folder(
        &pumpkin_config::world::LevelConfig::default(),
        dir.path().into(),
        0,
        Dimension::OVERWORLD,
    );
    let world = Arc::new(World::load(
        level,
        Arc::new(ArcSwap::from_pointee(
            pumpkin_world::world_info::LevelData::default(Seed(0)),
        )),
        Dimension::OVERWORLD,
        pumpkin_core::block::registry::default_registry(),
        Weak::new(),
    ));
    let pig: Arc<dyn EntityBase> = pumpkin_core::entity::r#type::from_type(
        &EntityType::PIG,
        Vector3::new(1.5, 64.0, 1.5),
        &world,
        uuid::Uuid::new_v4(),
    );
    let health = pig.get_living_entity().unwrap().health.load();
    let velocity = pig.get_entity().velocity.load();
    let no_ai = pig.get_mob().unwrap().get_mob_entity().is_no_ai();
    pig.get_entity()
        .removal_reason
        .store(Some(pumpkin_core::entity::RemovalReason::UnloadedToChunk));
    pig.get_entity().removed.store(true, Ordering::Release);
    let mut host = PluginHostState::new();
    let entity: Resource<Entity> = host.add(pig.clone()).unwrap();
    let living: Resource<LivingEntity> = host.add(pig.clone()).unwrap();
    let mob: Resource<Mob> = host.add(pig.clone()).unwrap();
    assert!(HostEntity::set_velocity(&mut host, entity, (1.0, 2.0, 3.0)).is_ok());
    assert!(HostLivingEntity::set_health(&mut host, living, health / 2.0).is_ok());
    assert!(HostMob::set_ai_disabled(&mut host, mob, !no_ai).is_ok());
    assert_eq!(pig.get_entity().velocity.load(), velocity);
    assert_eq!(pig.get_living_entity().unwrap().health.load(), health);
    assert_eq!(pig.get_mob().unwrap().get_mob_entity().is_no_ai(), no_ai);
    world.level.shutdown().await.unwrap();
}

#[tokio::test]
async fn owner_review_replaced_chunk_setter_succeeds_without_writing() {
    use crate::pumpkin::plugin::world::{Chunk, HostChunk};
    let dir = tempfile::tempdir().unwrap();
    let level = pumpkin_world::level::Level::from_root_folder(
        &pumpkin_config::world::LevelConfig::default(),
        dir.path().into(),
        0,
        Dimension::OVERWORLD,
    );
    let world = Arc::new(World::load(
        level,
        Arc::new(ArcSwap::from_pointee(
            pumpkin_world::world_info::LevelData::default(Seed(0)),
        )),
        Dimension::OVERWORLD,
        pumpkin_core::block::registry::default_registry(),
        Weak::new(),
    ));
    let stale = pumpkin_world::chunk::ChunkData::empty_sync(0, 0);
    let canonical = pumpkin_world::chunk::ChunkData::empty_sync(0, 0);
    let pos = pumpkin_util::math::vector2::Vector2::new(0, 0);
    world.level.loaded_chunks.insert(pos, stale.clone());
    world
        .level
        .set_retained_chunk_custom_data(&stale, "test", "key", NbtTag::Int(7))
        .unwrap();
    world.level.loaded_chunks.insert(pos, canonical.clone());
    world
        .level
        .set_retained_chunk_custom_data(&canonical, "test", "key", NbtTag::Int(8))
        .unwrap();
    let mut host = PluginHostState::new();
    let handle: Resource<Chunk> = host.add((world.clone(), Arc::downgrade(&stale))).unwrap();
    let rep = handle.rep();
    assert!(
        HostChunk::set_block_state(
            &mut host,
            handle,
            crate::pumpkin::plugin::world::BlockPos { x: 1, y: 64, z: 1 },
            pumpkin_data::Block::STONE.default_state.id.as_u16()
        )
        .is_ok()
    );
    assert_eq!(
        stale.section.get_block_absolute_y(1, 64, 1),
        Some(pumpkin_data::Block::AIR.default_state.id)
    );
    assert_eq!(
        canonical.section.get_block_absolute_y(1, 64, 1),
        Some(pumpkin_data::Block::AIR.default_state.id)
    );
    assert_stale_custom_setters(&mut host, rep, stale, &canonical);
    world.level.shutdown().await.unwrap();
}

fn assert_stale_custom_setters(
    host: &mut PluginHostState,
    rep: u32,
    stale: Arc<pumpkin_world::chunk::ChunkData>,
    canonical: &pumpkin_world::chunk::ChunkData,
) {
    use crate::pumpkin::plugin::world::HostChunk;
    let set = HostChunk::set_custom_data(
        host,
        Resource::new_borrow(rep),
        "test".into(),
        "key".into(),
        crate::common::to_wit_nbt_tree(NbtTag::Int(9)),
    );
    let remove =
        HostChunk::remove_custom_data(host, Resource::new_borrow(rep), "test".into(), "key".into());
    assert!(
        set.is_ok() && remove.is_ok(),
        "stale custom setters: set={set:?}, remove={remove:?}"
    );
    assert_eq!(stale.get_custom_data("test", "key"), Some(NbtTag::Int(7)));
    assert_eq!(
        canonical.get_custom_data("test", "key"),
        Some(NbtTag::Int(8))
    );
    assert!(
        HostChunk::set_custom_data(
            host,
            Resource::new_borrow(rep),
            "test".into(),
            "key".into(),
            crate::common::WitNbtTree {
                root: 0,
                tags: Vec::new()
            },
        )
        .is_err()
    );
    drop(stale);
    assert!(
        HostChunk::set_custom_data(
            host,
            Resource::new_borrow(rep),
            "test".into(),
            "key".into(),
            crate::common::to_wit_nbt_tree(NbtTag::Int(9)),
        )
        .is_err()
    );
    assert!(
        HostChunk::remove_custom_data(
            host,
            Resource::new_borrow(rep),
            "test".into(),
            "key".into(),
        )
        .is_err()
    );
}
