//! Real server, Java player and loaded chunk for the PR 116 animal review.
use crate::{
    entity::{EntityBase, r#type::from_type},
    net::java::combat_test_support::TestPlayer,
    server::{Server, combat_test_support},
    world::{World, spawn_test_support},
};
use pumpkin_data::{Block, biome::Biome, entity::EntityType};
use pumpkin_protocol::{codec::var_int::VarInt, java::server::play::SInteract};
use pumpkin_util::math::vector3::Vector3;
use std::sync::{Arc, atomic::Ordering::Relaxed};

pub struct Fixture {
    pub server: Arc<Server>,
    pub world: Arc<World>,
    pub player: TestPlayer,
    _directory: tempfile::TempDir,
}

impl Fixture {
    pub fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        let world = combat_test_support::world(&server, directory.path());
        spawn_test_support::publish(
            &world,
            spawn_test_support::proto(&Biome::PLAINS, &Block::STONE),
        );
        let player = TestPlayer::new(&world);
        player
            .player
            .get_entity()
            .set_pos(Vector3::new(8.5, 64.0, 8.5));
        Self {
            server,
            world,
            player,
            _directory: directory,
        }
    }

    pub fn spawn(&self, kind: &'static EntityType, age: i32) -> Arc<dyn EntityBase> {
        let entity = from_type(
            kind,
            Vector3::new(9.5, 64.0, 8.5),
            &self.world,
            uuid::Uuid::new_v4(),
        );
        entity.get_entity().has_no_gravity.store(true, Relaxed);
        if let Some(mob) = entity.get_mob() {
            mob.get_mob_entity()
                .persistence_required
                .store(true, Relaxed);
            mob.get_mob_entity().set_no_ai(true);
            if let Some(ageable) = mob.as_ageable() {
                ageable.set_age(age);
            } else {
                entity.get_entity().set_age(age);
            }
            if let Some(tamable) = mob.as_tamable() {
                tamable.set_tame(true);
                tamable.set_owner(Some(self.player.player.gameprofile.id));
            }
        }
        assert!(self.world.spawn_entity(entity.clone()));
        entity
    }

    pub fn interact(&self, target: &dyn EntityBase) {
        self.player.client().handle_interact(
            &self.player.player,
            &SInteract {
                entity_id: VarInt(target.get_entity().entity_id),
                r#type: VarInt(2),
                target_position: Some(Vector3::new(0.0, 0.5, 0.0)),
                hand: Some(VarInt(0)),
                sneaking: false,
            },
            &self.server,
        );
    }

    pub fn tick(&self) {
        self.world.tick(&self.server);
    }

    pub async fn finish(self) {
        self.world.level.shutdown().await.unwrap();
    }
}
