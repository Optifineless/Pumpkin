use std::{sync::Arc, time::Duration};

use pumpkin_data::{
    Block, attributes::Attributes, biome::Biome, entity::EntityType, item::Item,
    item_stack::ItemStack,
};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::{VarInt, java::server::play::SUseItemOn};
use pumpkin_util::{
    Hand, PermissionLvl,
    math::{position::BlockPos, vector3::Vector3},
};

use crate::{
    entity::EntityBase,
    net::java::combat_test_support::TestPlayer,
    server::{Server, combat_test_support},
    world::{World, spawn_test_support},
};

pub const POS: Vector3<f64> = Vector3::new(8.5, 64.0, 8.5);
pub const INTERNAL: [f64; 3] = [22.0, 0.224_999_997_019_767_75, 0.550_000_005_960_464_5];
pub const ATTRS: [Attributes; 3] = [
    Attributes::MAX_HEALTH,
    Attributes::MOVEMENT_SPEED,
    Attributes::JUMP_STRENGTH,
];

pub struct Fixture {
    pub server: Arc<Server>,
    pub world: Arc<World>,
    pub actor: TestPlayer,
    pub directory: tempfile::TempDir,
}

impl Fixture {
    pub fn new(enabled: bool) -> Self {
        Self::from_directory(enabled, tempfile::tempdir().unwrap())
    }

    fn from_directory(enabled: bool, directory: tempfile::TempDir) -> Self {
        let mut server = combat_test_support::server(directory.path());
        Arc::get_mut(&mut server)
            .unwrap()
            .advanced_config
            .horse_spawn_stats
            .enabled = enabled;
        let world = combat_test_support::world(&server, directory.path());
        server.worlds.store(Arc::new(vec![world.clone()]));
        spawn_test_support::publish(
            &world,
            spawn_test_support::proto(&Biome::PLAINS, &Block::GRASS_BLOCK),
        );
        let mut actor = TestPlayer::new(&world);
        actor.player.permission_lvl.store(PermissionLvl::Four);
        actor
            .player
            .get_entity()
            .set_pos(Vector3::new(8.5, 64.0, 6.5));
        actor
            .player
            .chunk_sender
            .lock()
            .unwrap()
            .mark_sent_out_of_band(pumpkin_util::math::vector2::Vector2::new(0, 0));
        actor.take_packets();
        Self {
            server,
            world,
            actor,
            directory,
        }
    }

    pub fn summon(&self, arguments: &str) -> Arc<dyn EntityBase> {
        let before = self.world.entities.load().len();
        self.server
            .command_dispatcher
            .load()
            .execute_input(
                &format!("summon {arguments}"),
                &self.actor.player.get_command_source(&self.server),
            )
            .unwrap();
        let entities = self.world.entities.load_full();
        assert_eq!(entities.len(), before + 1);
        entities.last().unwrap().clone()
    }

    pub fn egg(&self, stack: ItemStack) -> Arc<dyn EntityBase> {
        let before = self.world.entities.load().len();
        self.actor
            .player
            .inventory()
            .set_stack_in_hand(Hand::Right, stack);
        self.actor
            .client()
            .handle_use_item_on(
                &self.actor.player,
                &SUseItemOn {
                    hand: VarInt(0),
                    position: BlockPos::new(8, 63, 8),
                    face: VarInt(1),
                    cursor_pos: Vector3::new(0.5, 1.0, 0.5),
                    inside_block: false,
                    is_against_world_border: false,
                    sequence: VarInt(1),
                },
                &self.server,
            )
            .unwrap();
        let entities = self.world.entities.load_full();
        assert_eq!(entities.len(), before + 1);
        assert_eq!(
            self.actor
                .player
                .inventory()
                .get_stack_in_hand(Hand::Right)
                .item_count,
            0
        );
        entities.last().unwrap().clone()
    }

    pub async fn restart(self, enabled: bool) -> Self {
        let Self {
            server,
            world,
            actor,
            directory,
        } = self;
        actor.client().close();
        tokio::time::timeout(Duration::from_secs(30), world.shutdown())
            .await
            .unwrap();
        drop(actor);
        drop(world);
        drop(server);
        Self::from_directory(enabled, directory)
    }

    pub async fn finish(self) {
        self.actor.client().close();
        tokio::time::timeout(Duration::from_secs(30), self.world.shutdown())
            .await
            .unwrap();
    }
}

pub fn horse_egg() -> ItemStack {
    ItemStack::new(1, &Item::HORSE_SPAWN_EGG)
}

pub fn save(entity: &dyn EntityBase) -> NbtCompound {
    let mut nbt = NbtCompound::new();
    entity.write_nbt(&mut nbt);
    nbt
}

pub fn bases(entity: &dyn EntityBase) -> [f64; 3] {
    let living = entity.get_living_entity().unwrap();
    ATTRS.map(|attribute| living.get_attribute_base(&attribute))
}

pub fn assert_stats(entity: &dyn EntityBase, expected: [f64; 3], hp: f32) {
    let nbt = save(entity);
    for (actual, expected) in bases(entity).into_iter().zip(expected) {
        assert!(
            (actual - expected).abs() < 1e-14,
            "base {actual:?} != {expected:?}; current HP={:?}, saved Health={:?}, saved attributes={:?}",
            entity.get_living_entity().unwrap().health.load(),
            nbt.get_float("Health"),
            nbt.get_list("attributes")
        );
    }
    assert_eq!(entity.get_living_entity().unwrap().health.load(), hp);
    assert_eq!(nbt.get_float("Health"), Some(hp));
    for (attribute, expected) in ATTRS.iter().zip(expected) {
        let packed = nbt
            .get_list("attributes")
            .unwrap()
            .iter()
            .filter_map(|tag| tag.extract_compound())
            .find(|packed| packed.get_string("id") == Some(attribute.name))
            .unwrap();
        assert!((packed.get_double("base").unwrap() - expected).abs() < 1e-14);
    }
}

pub fn default_bases(ty: &'static EntityType) -> [f64; 3] {
    ATTRS.map(|attribute| {
        ty.attributes
            .iter()
            .find(|(attr, _)| attr.id == attribute.id)
            .map_or(attribute.default_value, |(_, base)| *base)
    })
}

pub fn assert_saved(left: &NbtCompound, right: &NbtCompound) {
    assert_eq!(left.get_uuid("UUID"), right.get_uuid("UUID"));
    assert_eq!(left.get_float("Health"), right.get_float("Health"));
    let sorted = |nbt: &NbtCompound| {
        let mut list = nbt.get_list("attributes").unwrap().to_vec();
        list.sort_by_key(|tag| {
            tag.extract_compound()
                .unwrap()
                .get_string("id")
                .unwrap()
                .to_owned()
        });
        list
    };
    assert_eq!(sorted(left), sorted(right));
}

pub fn assert_permanent_modifiers(entity: &dyn EntityBase) {
    use crate::entity::attributes::{Modifier, ModifierOperation};
    let living = entity.get_living_entity().unwrap();
    for attribute in ATTRS {
        let packed = living.attributes.read().unwrap()[&attribute.id].pack();
        assert_eq!(
            packed,
            vec![Modifier {
                id: "test:permanent".to_owned(),
                amount: 0.125,
                operation: ModifierOperation::Add,
                permanent: true,
            }]
        );
    }
}

pub fn assert_observer(
    packets: &[bytes::Bytes],
    horse: &dyn EntityBase,
    expected: [f64; 3],
    hp: f32,
) -> Vec<pumpkin_protocol::java::client::play::Property> {
    use pumpkin_data::packet::CURRENT_MC_VERSION;
    use pumpkin_protocol::{
        ServerPacket, VarInt,
        java::client::play::{CSetEntityMetadata, CUpdateAttributes, Metadata},
        packet::MultiVersionJavaPacket,
    };
    let mut properties = Vec::new();
    let mut health_received = false;
    let mut health = Vec::new();
    Metadata::new(
        pumpkin_data::tracked_data::living_entity::DATA_HEALTH_ID,
        hp,
    )
    .write(&mut health, &CURRENT_MC_VERSION)
    .unwrap();
    for packet in packets {
        let mut bytes = packet.as_ref();
        let id = VarInt::decode(&mut bytes).unwrap().0;
        if id == CUpdateAttributes::to_id(CURRENT_MC_VERSION) {
            let update = CUpdateAttributes::read(&mut bytes, &CURRENT_MC_VERSION).unwrap();
            if update.entity_id.0 == horse.get_entity().entity_id {
                properties.extend(update.properties);
            }
        }
        if id == CSetEntityMetadata::to_id(CURRENT_MC_VERSION) {
            let update = CSetEntityMetadata::read(&mut bytes, &CURRENT_MC_VERSION).unwrap();
            health_received |= update.entity_id.0 == horse.get_entity().entity_id
                && update
                    .metadata
                    .windows(health.len())
                    .any(|part| part == health);
        }
    }
    assert!(health_received, "observer did not receive legal current HP");
    for (attribute, expected) in ATTRS.iter().zip(expected) {
        assert!(
            properties
                .iter()
                .any(|property| property.id.0 == i32::from(attribute.id)
                    && (property.value - expected).abs() < 1e-14),
            "observer missed {}",
            attribute.name
        );
    }
    properties
}
