use super::*;
use bytes::Bytes;
use pumpkin_data::{packet::clientbound::play, tracked_data::entity as tracked};
use pumpkin_inventory::screen_handler::InventoryPlayer;
use pumpkin_protocol::{
    ClientPacket, RawPacket, ServerPacket,
    codec::item_stack_seralizer::{ItemStackSerializer, OptionalItemStackHash},
    java::{
        client::play::{CSetEntityMetadata, CSetEquipment, Metadata},
        server::play::{Action, SClickSlot, SPlayerCommand, SSetPlayerGround, SlotActionType},
    },
    packet::MultiVersionJavaPacket,
};

pub struct Fixture {
    pub _directory: tempfile::TempDir,
    pub server: Arc<Server>,
    pub world: Arc<crate::world::World>,
    pub owner: TestPlayer,
    pub observer: TestPlayer,
}

impl Fixture {
    pub fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        let world = combat_test_support::world(&server, directory.path());
        combat_test_support::publish_empty_chunk(&world, Vector2::new(0, 0));
        server.worlds.store(Arc::new(vec![world.clone()]));
        let owner = TestPlayer::new(&world);
        let observer = TestPlayer::new(&world);
        world.players.store(Arc::new(vec![
            owner.player.clone(),
            observer.player.clone(),
        ]));
        owner
            .player
            .screen_handler_sync_handler
            .store_player(owner.player.clone());
        owner
            .player
            .get_entity()
            .set_pos(Vector3::new(4.0, 80.0, 4.0));
        observer
            .player
            .get_entity()
            .set_pos(Vector3::new(12.0, 80.0, 12.0));
        observer
            .player
            .chunk_sender
            .lock()
            .unwrap()
            .mark_sent_out_of_band(Vector2::new(0, 0));
        world
            .entity_tracker
            .get_tracked_entity(owner.player.entity_id())
            .unwrap()
            .update_player(&observer.player, &world);
        let mut fixture = Self {
            _directory: directory,
            server,
            world,
            owner,
            observer,
        };
        fixture.equip(38, ItemStack::new(1, &Item::ELYTRA), &EquipmentSlot::CHEST);
        fixture
            .owner
            .player
            .inventory
            .set_slot(0, ItemStack::new(1, &Item::IRON_PICKAXE));
        fixture.tick(1);
        fixture.clear_packets();
        fixture
    }

    pub fn equip(&self, index: usize, stack: ItemStack, slot: &EquipmentSlot) {
        self.owner.player.inventory.set_slot(index, stack);
        self.owner
            .player
            .enqueue_equipment_change(slot, &self.owner.player.inventory.get_slot(index));
    }

    pub fn tick(&self, count: usize) {
        for _ in 0..count {
            self.owner.player.tick(&self.server);
            self.world.entity_tracker.update_all(&self.world);
        }
    }

    pub fn start(&self) {
        dispatch(
            &self.owner.player,
            &self.server,
            &SSetPlayerGround { on_ground: false },
        );
        dispatch(
            &self.owner.player,
            &self.server,
            &SPlayerCommand {
                entity_id: self.owner.player.entity_id().into(),
                action: Action::StartFlyingElytra,
                jump_boost: VarInt(0),
            },
        );
        assert!(
            self.owner.player.get_entity().is_fall_flying(),
            "real StartFlyingElytra was not admitted"
        );
    }

    pub fn clear_packets(&mut self) {
        self.owner.take_packets();
        self.observer.take_packets();
    }

    pub fn assert_damage_published(&mut self, index: usize, slot: &EquipmentSlot, damage: i32) {
        assert_eq!(
            self.owner.player.inventory.get_slot(index).get_damage(),
            damage,
            "authoritative equipped wear"
        );
        let equipment: Vec<_> = self
            .observer
            .take_packets()
            .into_iter()
            .filter_map(|packet| {
                let mut data = packet.as_ref();
                (VarInt::decode(&mut data).unwrap().0 == play::SET_EQUIPMENT.0)
                    .then(|| CSetEquipment::read(&mut data, &CURRENT_MC_VERSION).unwrap())
            })
            .filter(|packet| packet.entity_id.0 == self.owner.player.entity_id())
            .flat_map(|packet| packet.equipment)
            .filter(|(id, _)| *id == slot.discriminant())
            .collect();
        assert_eq!(
            equipment.last().unwrap().1.0.get_damage(),
            damage,
            "observer equipment wear"
        );
        let owner = self.owner.take_packets();
        assert!(
            owner.iter().any(|packet| {
                let mut data = packet.as_ref();
                if VarInt::decode(&mut data).unwrap().0 != play::SET_PLAYER_INVENTORY.0
                    || VarInt::decode(&mut data).unwrap().0 != index as i32
                {
                    return false;
                }
                ItemStackSerializer::read_with_version(&mut data, &CURRENT_MC_VERSION)
                    .unwrap()
                    .0
                    .get_damage()
                    == damage
            }),
            "owner inventory wear was not synchronized"
        );
    }

    pub fn assert_stopped(&mut self) {
        assert!(
            !self.owner.player.get_entity().is_fall_flying(),
            "invalid flight survived the normal tick"
        );
        assert!(self.owner.player.get_entity().pose.load() == EntityPose::Standing);
        let id = self.owner.player.entity_id();
        for packets in [self.owner.take_packets(), self.observer.take_packets()] {
            let metadata: Vec<_> = packets
                .into_iter()
                .filter_map(|packet| {
                    let mut data = packet.as_ref();
                    (VarInt::decode(&mut data).unwrap().0 == play::SET_ENTITY_DATA.0)
                        .then(|| CSetEntityMetadata::read(&mut data, &CURRENT_MC_VERSION).unwrap())
                })
                .filter(|packet| packet.entity_id.0 == id)
                .collect();
            let mut flags = Vec::new();
            Metadata::new(tracked::DATA_SHARED_FLAGS_ID, 0i8)
                .write(&mut flags, &CURRENT_MC_VERSION)
                .unwrap();
            let mut pose = Vec::new();
            Metadata::new(tracked::DATA_POSE, VarInt(EntityPose::Standing as i32))
                .write(&mut pose, &CURRENT_MC_VERSION)
                .unwrap();
            for expected in [flags, pose] {
                assert!(
                    metadata.iter().any(|packet| packet
                        .metadata
                        .windows(expected.len())
                        .any(|window| window == expected)),
                    "owner/observer missing stop flag or standing pose"
                );
            }
        }
    }

    pub async fn close(self) {
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            self.world.level.shutdown(),
        )
        .await
        .unwrap()
        .unwrap();
    }
}

pub fn dispatch<P: ClientPacket + MultiVersionJavaPacket>(
    player: &Arc<Player>,
    server: &Arc<Server>,
    packet: &P,
) {
    let mut data = Vec::new();
    packet
        .write_packet_data(&mut data, &CURRENT_MC_VERSION)
        .unwrap();
    let crate::net::ClientPlatform::Java(client) = player.client.as_ref() else {
        panic!("Java gliding fixture required")
    };
    client
        .handle_play_packet(
            player,
            server,
            &RawPacket {
                id: P::to_id(CURRENT_MC_VERSION),
                payload: Bytes::from(data),
            },
        )
        .unwrap();
}

pub fn remove_chest(player: &Arc<Player>, server: &Arc<Server>, mode: SlotActionType) {
    let revision = player
        .current_screen_handler
        .lock()
        .unwrap()
        .lock()
        .unwrap()
        .get_behaviour()
        .revision
        .load(Relaxed);
    dispatch(
        player,
        server,
        &SClickSlot {
            sync_id: VarInt(0),
            revision: VarInt(revision as i32),
            slot: 6,
            button: SClickSlot::BUTTON_LEFT,
            mode,
            length_of_array: VarInt(0),
            array_of_changed_slots: Vec::new(),
            carried_item: OptionalItemStackHash(None),
        },
    );
    assert!(
        player.inventory.get_slot(38).is_empty(),
        "real armor click did not remove elytra"
    );
}

pub struct DamageCallback<F>(pub F);
impl<F: Fn(&mut PlayerItemDamageEvent) + Send + Sync> EventHandler<PlayerItemDamageEvent>
    for DamageCallback<F>
{
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerItemDamageEvent,
    ) -> BoxFuture<'a, ()> {
        (self.0)(event);
        Box::pin(async {})
    }
}

pub struct GlideCallback<F>(pub F);
impl<F: Fn(&mut EntityToggleGlideEvent) + Send + Sync> EventHandler<EntityToggleGlideEvent>
    for GlideCallback<F>
{
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut EntityToggleGlideEvent,
    ) -> BoxFuture<'a, ()> {
        (self.0)(event);
        Box::pin(async {})
    }
}

pub struct GlideEvents {
    pub player: Arc<Player>,
    pub events: Mutex<Vec<(i32, i32)>>,
}

pub struct GameCallback<F>(pub F);
impl<F: Fn(&mut GenericGameEvent) + Send + Sync> EventHandler<GenericGameEvent>
    for GameCallback<F>
{
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut GenericGameEvent,
    ) -> BoxFuture<'a, ()> {
        (self.0)(event);
        Box::pin(async {})
    }
}
impl EventHandler<GenericGameEvent> for GlideEvents {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut GenericGameEvent,
    ) -> BoxFuture<'a, ()> {
        if event.event_key == "elytra_glide" {
            self.events.lock().unwrap().push((
                self.player.tick_counter.load(Relaxed),
                self.player.inventory.get_slot(38).get_damage(),
            ));
        }
        Box::pin(async {})
    }
}
