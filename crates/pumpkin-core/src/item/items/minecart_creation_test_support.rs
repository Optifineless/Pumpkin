use super::*;
use pumpkin_protocol::{
    ClientPacket, RawPacket,
    codec::item_stack_seralizer::ItemStackSerializer,
    java::client::play::{CSetPlayerInventory, CSpawnEntity},
    packet::MultiVersionJavaPacket,
    ser::NetworkReadExt,
};

pub(super) struct Fixture {
    pub context: DeathTestWorld,
    pub world: Arc<crate::world::World>,
    pub actor: TestPlayer,
}

impl Fixture {
    pub async fn new(experimental: bool) -> Self {
        let context =
            tokio::time::timeout(std::time::Duration::from_secs(60), DeathTestWorld::new())
                .await
                .unwrap();
        let world = context.world();
        if experimental {
            context.server.level_info.rcu(|info| {
                let mut info = (**info).clone();
                info.data_packs
                    .enabled
                    .push("minecart_improvements".to_owned());
                info
            });
        }
        let actor = TestPlayer::new(&world);
        actor
            .player
            .permission_lvl
            .store(pumpkin_util::permission::PermissionLvl::Four);
        Self {
            context,
            world,
            actor,
        }
    }

    pub fn rail(&self, position: BlockPos, shape: RailShape, block: &Block) {
        let chunk = position.chunk_position();
        if !self.world.level.loaded_chunks.contains_key(&chunk) {
            combat_test_support::publish_empty_chunk(&self.world, chunk);
        }
        let state = if block == &Block::RAIL {
            let mut props = RailLikeProperties::default(block);
            props.shape = shape;
            props.to_state_id(block)
        } else {
            let mut props = PoweredRailLikeProperties::default(block);
            props.shape =
                pumpkin_data::block_properties::RailShapeStraight::from_value(shape.to_value());
            props.to_state_id(block)
        };
        self.world.set_block_state(
            &position,
            state,
            BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
        );
        self.actor
            .player
            .chunk_sender
            .lock()
            .unwrap()
            .mark_sent_out_of_band(chunk);
    }

    pub fn equip(&mut self, position: BlockPos, item: &'static Item, hand: Hand) {
        self.actor.player.get_entity().set_pos(Vector3::new(
            position.to_f64().x,
            f64::from(position.0.y) + 1.0,
            position.to_f64().z,
        ));
        self.actor
            .player
            .inventory()
            .set_stack_in_hand(hand, ItemStack::new(2, item));
        self.actor.take_packets();
    }

    pub fn use_on(&self, position: BlockPos, hand: Hand) {
        let packet = SUseItemOn {
            hand: VarInt(i32::from(hand == Hand::Left)),
            position,
            face: VarInt(BlockDirection::Up as i32),
            cursor_pos: Vector3::new(0.5, 1.0, 0.5),
            inside_block: false,
            is_against_world_border: false,
            sequence: VarInt(1),
        };
        let mut payload = Vec::new();
        packet
            .write_packet_data(&mut payload, &CURRENT_MC_VERSION)
            .unwrap();
        self.actor
            .client()
            .handle_play_packet(
                &self.actor.player,
                &self.context.server,
                &RawPacket {
                    id: SUseItemOn::to_id(CURRENT_MC_VERSION),
                    payload: payload.into(),
                },
            )
            .unwrap();
    }

    pub fn carts(&self) -> Vec<Arc<dyn EntityBase>> {
        self.world
            .entities
            .load()
            .iter()
            .filter(|e| e.cast_any().is::<MinecartEntity>())
            .cloned()
            .collect()
    }

    pub fn cart(&self, position: Vector3<f64>, kind: &'static EntityType) -> Arc<dyn EntityBase> {
        let cart: Arc<dyn EntityBase> = Arc::new(MinecartEntity::new(Entity::new(
            self.world.clone(),
            position,
            kind,
        )));
        self.world.add_entity_silent(cart.clone());
        cart
    }

    pub fn packets(&mut self) -> (Vec<CSpawnEntity>, Vec<(i32, ItemStack)>) {
        let mut spawns = Vec::new();
        let mut hands = Vec::new();
        for bytes in self.actor.take_packets() {
            let mut read = bytes.as_ref();
            let id = read.get_var_int().unwrap().0;
            if id == CSpawnEntity::to_id(CURRENT_MC_VERSION) {
                spawns.push(CSpawnEntity::read_packet_data(read, &CURRENT_MC_VERSION).unwrap());
            } else if id == CSetPlayerInventory::to_id(CURRENT_MC_VERSION) {
                let slot = read.get_var_int().unwrap().0;
                let item =
                    ItemStackSerializer::read_with_version(&mut read, &CURRENT_MC_VERSION).unwrap();
                hands.push((slot, item.0.into_owned()));
            }
        }
        (spawns, hands)
    }

    pub fn hand_count(&self, hand: Hand) -> u8 {
        self.actor
            .player
            .inventory()
            .get_stack_in_hand(hand)
            .item_count
    }

    pub async fn close(self) {
        tokio::time::timeout(
            std::time::Duration::from_secs(20),
            self.context.server.shutdown(),
        )
        .await
        .unwrap();
    }
}

pub(super) fn assert_geometry(cart: &Arc<dyn EntityBase>, expected: (f64, f64, f64, f32, f32)) {
    let entity = cart.get_entity();
    let actual = entity.pos.load();
    assert!(
        (actual.x - expected.0).abs() < 1.0e-9
            && (actual.y - expected.1).abs() < 1.0e-9
            && (actual.z - expected.2).abs() < 1.0e-9,
        "aligned creation coordinate: {actual:?}, expected {expected:?}"
    );
    assert_eq!(
        (entity.yaw.load(), entity.pitch.load()),
        (expected.3, expected.4)
    );
}
