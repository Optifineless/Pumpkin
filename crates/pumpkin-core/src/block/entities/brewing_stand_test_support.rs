#![expect(clippy::unwrap_used, reason = "Bounded brewing regression fixtures")]

use std::{
    sync::{Arc, mpsc},
    thread,
    time::Duration,
};

use pumpkin_data::{
    Block, BlockDirection, data_component_impl::PotionContentsImpl, item::Item,
    item_stack::ItemStack, potion::Potion,
};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::{
    ServerPacket, java::client::play::CSetContainerProperty, ser::NetworkReadExt,
};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use pumpkin_world::{chunk::ChunkData, world::BlockFlags};

use super::{BlockEntity, brewing_stand::BrewingStandBlockEntity};
use crate::{
    block::{BlockHitResult, NormalUseArgs},
    entity::{EntityBase, death_test_world::DeathTestWorld},
    net::java::combat_test_support::TestPlayer,
    plugin::{BoxFuture, EventHandler, EventPriority, Payload},
    server::Server,
    world::World,
};

pub(super) const POSITION: BlockPos = BlockPos::new(4, 64, 4);

pub(super) struct BrewingFixture {
    pub host: DeathTestWorld,
    pub world: Arc<World>,
    pub stand: Arc<BrewingStandBlockEntity>,
}

impl BrewingFixture {
    pub async fn new() -> Self {
        let host = tokio::time::timeout(Duration::from_secs(30), DeathTestWorld::new())
            .await
            .unwrap();
        let world = host.world();
        world
            .level
            .loaded_chunks
            .insert(POSITION.chunk_position(), ChunkData::empty_sync(0, 0));
        world.set_block_state(
            &POSITION,
            Block::BREWING_STAND.default_state.id,
            BlockFlags::FORCE_STATE,
        );
        let stand = Arc::new(BrewingStandBlockEntity::new(POSITION));
        world.add_block_entity(stand.clone());
        Self { host, world, stand }
    }

    pub fn ingredients(&self, potion: ItemStack, reagent: ItemStack) {
        self.stand.set_stack(0, potion);
        self.stand.set_stack(3, reagent);
        self.stand
            .set_stack(4, ItemStack::new(2, &Item::BLAZE_POWDER));
    }

    pub fn ticks(&self, count: usize) {
        assert!(count <= 401, "brewing tests must have bounded tick counts");
        for _ in 0..count {
            self.stand.tick(&self.world);
        }
    }

    pub fn open_menu(&self) -> TestPlayer {
        let player = TestPlayer::new(&self.world);
        player
            .player
            .screen_handler_sync_handler
            .store_player(player.player.clone());
        player
            .player
            .get_entity()
            .set_pos(POSITION.to_centered_f64());
        self.world
            .block_registry
            .get_pumpkin_block(Block::BREWING_STAND.id)
            .unwrap()
            .normal_use(NormalUseArgs {
                server: &self.host.server,
                world: &self.world,
                block: &Block::BREWING_STAND,
                position: &POSITION,
                player: &player.player,
                hit: &BlockHitResult {
                    face: &BlockDirection::Up,
                    cursor_pos: &Vector3::new(0.5, 0.5, 0.5),
                },
            });
        player
    }

    pub fn dropped_items(&self) -> Vec<ItemStack> {
        self.world
            .entities
            .load()
            .iter()
            .filter_map(|entity| entity.get_item_entity())
            .map(|item| item.get_item_stack().lock().unwrap().clone())
            .collect()
    }

    pub async fn shutdown(self) {
        tokio::time::timeout(Duration::from_secs(30), self.host.server.shutdown())
            .await
            .unwrap();
    }
}

pub(super) fn potion(item: &'static Item, potion: &'static Potion) -> ItemStack {
    let mut stack = ItemStack::new(1, item);
    stack.set_data_component(PotionContentsImpl {
        potion_id: Some(i32::from(potion.id)),
        custom_color: Some(0x123456),
        custom_effects: Vec::new(),
        custom_name: Some("test potion".into()),
    });
    stack
}

pub(super) fn assert_inventory_unlocked(stand: &BrewingStandBlockEntity) {
    assert!(
        stand.items.try_write().is_ok(),
        "callback must leave inventory storage"
    );
    let owner = stand.slot_transaction(0);
    let (send, receive) = mpsc::sync_channel(1);
    let worker = thread::spawn(move || {
        let available = owner.try_lock().is_some();
        let _ = send.send(available);
    });
    assert!(
        receive.recv_timeout(Duration::from_secs(3)).unwrap(),
        "callback must leave its transaction owner"
    );
    worker.join().unwrap();
}

pub(super) fn property_packets(player: &mut TestPlayer) -> Vec<(i16, i16)> {
    let mut properties = Vec::new();
    for bytes in player.take_packets() {
        let mut data = bytes.as_ref();
        if data.get_var_int().unwrap().0
            == pumpkin_data::packet::clientbound::play::CONTAINER_SET_DATA.0
        {
            let packet =
                CSetContainerProperty::read(&mut data, &pumpkin_data::packet::CURRENT_MC_VERSION)
                    .unwrap();
            assert!(packet.window_id.0 > 0);
            assert!(data.is_empty());
            properties.push((packet.property, packet.value));
        }
    }
    properties
}

struct Handler<F>(F);
impl<E: Payload, F: Fn(&mut E) + Send + Sync> EventHandler<E> for Handler<F> {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut E,
    ) -> BoxFuture<'a, ()> {
        (self.0)(event);
        Box::pin(async {})
    }
}

pub(super) fn register<E: Payload + Send + Sync + 'static>(
    server: &Server,
    callback: impl Fn(&mut E) + Send + Sync + 'static,
) {
    server.plugin_manager.register::<E, _>(
        Arc::new(Handler(callback)),
        EventPriority::Normal,
        true,
    );
}
