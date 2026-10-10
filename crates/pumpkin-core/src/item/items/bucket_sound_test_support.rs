use crate::{
    block::{OnScheduledTickArgs, entities::dispenser::DispenserBlockEntity},
    entity::EntityBase,
    net::java::{
        combat_test_support::TestPlayer,
        sound_test_support::{SoundPacket, decode_sounds},
    },
    plugin::{
        BoxFuture, EventHandler,
        api::events::player::player_bucket::{PlayerBucketEmptyEvent, PlayerBucketFillEvent},
    },
    server::{Server, combat_test_support},
    world::World,
};
use pumpkin_data::{
    Block,
    block_properties::{DispenserLikeProperties, Facing},
    data_component_impl::BucketEntityDataImpl,
    dimension::Dimension,
    item::Item,
    item_stack::ItemStack,
    sound::{Sound, SoundCategory},
    statistic::StatisticCategory,
};
use pumpkin_inventory::Inventory;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::{VarInt, java::server::play::SUseItem};
use pumpkin_util::{
    Hand,
    math::{position::BlockPos, vector2::Vector2, vector3::Vector3},
};
use pumpkin_world::{level::Level, world::BlockFlags};
use std::{
    ops::RangeInclusive,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering::Relaxed},
    },
};

pub(super) const DESTINATION: BlockPos = BlockPos::new(8, 65, 10);
pub(super) const ACTOR_SOUND_POSITION: Vector3<i32> = Vector3::new(68, 512, 68);
pub(super) const BLOCK_SOUND_POSITION: Vector3<i32> = Vector3::new(68, 524, 84);

pub(super) struct Fixture {
    pub server: Arc<Server>,
    pub world: Arc<World>,
    pub actor: TestPlayer,
    pub observer: TestPlayer,
    _directory: tempfile::TempDir,
}

impl Fixture {
    pub fn new(dimension: Dimension) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        let world = Arc::new(World::load(
            Level::from_root_folder(
                &pumpkin_config::world::LevelConfig::default(),
                directory.path().to_path_buf(),
                0,
                dimension.clone(),
            ),
            server.level_info.clone(),
            dimension,
            server.block_registry.clone(),
            Arc::downgrade(&server),
        ));
        combat_test_support::publish_empty_chunk(&world, Vector2::new(0, 0));
        let actor = TestPlayer::new(&world);
        let observer = TestPlayer::new(&world);
        // TestPlayer::new replaces this list; both clients must remain broadcast recipients.
        world.players.store(Arc::new(vec![
            actor.player.clone(),
            observer.player.clone(),
        ]));
        actor
            .player
            .get_entity()
            .set_pos(Vector3::new(8.5, 64.0, 8.5));
        observer
            .player
            .get_entity()
            .set_pos(Vector3::new(10.5, 64.0, 8.5));
        Self {
            server,
            world,
            actor,
            observer,
            _directory: directory,
        }
    }

    pub fn prepare(&mut self, hand: Hand, item: &'static Item, fluid: &'static Block) {
        for (position, block) in [
            (DESTINATION, fluid),
            (BlockPos::new(8, 65, 11), &Block::STONE),
        ] {
            self.world
                .set_block_state(&position, block.default_state.id, BlockFlags::FORCE_STATE);
        }
        let inventory = self.actor.player.inventory();
        inventory.set_stack_in_hand(other_hand(hand), ItemStack::new(1, &Item::DIAMOND_SWORD));
        let mut bucket = ItemStack::new(1, item);
        if item == &Item::COD_BUCKET {
            let mut tag = NbtCompound::new();
            tag.put_bool("Silent", true);
            bucket.set_data_component(BucketEntityDataImpl { nbt: Some(tag) });
        }
        inventory.set_stack_in_hand(hand, bucket);
        self.clear_packets();
    }

    pub fn clear_packets(&mut self) {
        self.actor.take_packets();
        self.observer.take_packets();
    }

    pub fn use_item(&self, hand: Hand, yaw: f32) {
        self.actor.client().handle_use_item(
            &self.actor.player,
            &SUseItem {
                hand: VarInt(i32::from(hand != Hand::Right)),
                sequence: VarInt(1),
                yaw,
                pitch: 0.0,
            },
            &self.server,
        );
    }

    pub fn used_stat(&self, item: &'static Item) -> i32 {
        self.actor
            .player
            .get_stat(StatisticCategory::Used, i32::from(item.id))
    }

    pub fn assert_hand(&self, hand: Hand, item: &'static Item) {
        let inventory = self.actor.player.inventory();
        assert!(
            inventory
                .get_stack_in_hand(hand)
                .are_equal(&ItemStack::new(1, item))
        );
        assert!(
            inventory
                .get_stack_in_hand(other_hand(hand))
                .are_equal(&ItemStack::new(1, &Item::DIAMOND_SWORD))
        );
    }

    pub fn take_delivery(&mut self, context: String) -> Delivery {
        Delivery {
            context,
            actor: decode_sounds(&self.actor.take_packets()),
            observer: decode_sounds(&self.observer.take_packets()),
        }
    }

    pub fn dispense(&mut self, item: &'static Item) -> ItemStack {
        let position = BlockPos::new(8, 65, 11);
        self.world.set_block_state(
            &DESTINATION,
            Block::AIR.default_state.id,
            BlockFlags::FORCE_STATE,
        );
        let mut properties = DispenserLikeProperties::default(&Block::DISPENSER);
        properties.facing = Facing::North;
        self.world.set_block_state(
            &position,
            properties.to_state_id(&Block::DISPENSER),
            BlockFlags::FORCE_STATE,
        );
        let dispenser = Arc::new(DispenserBlockEntity::new(position));
        dispenser.set_stack(0, ItemStack::new(1, item));
        self.world.add_block_entity(dispenser.clone());
        self.clear_packets();
        self.server
            .block_registry
            .get_pumpkin_block(Block::DISPENSER.id)
            .unwrap()
            .on_scheduled_tick(OnScheduledTickArgs {
                world: &self.world,
                block: &Block::DISPENSER,
                position: &position,
            });
        dispenser.get_stack(0)
    }

    pub async fn finish(self) {
        self.world.level.shutdown().await.unwrap();
    }
}

const fn other_hand(hand: Hand) -> Hand {
    if matches!(hand, Hand::Right) {
        Hand::Left
    } else {
        Hand::Right
    }
}

pub(super) struct ExpectedSound {
    sound: Sound,
    category: SoundCategory,
    position: Vector3<i32>,
    volume: f32,
    pitch: RangeInclusive<f32>,
}

impl ExpectedSound {
    pub fn regular(sound: Sound, category: SoundCategory, position: Vector3<i32>) -> Self {
        Self {
            sound,
            category,
            position,
            volume: 1.0,
            pitch: 1.0..=1.0,
        }
    }

    pub fn evaporation() -> Self {
        Self {
            sound: Sound::BlockFireExtinguish,
            category: SoundCategory::Blocks,
            position: BLOCK_SOUND_POSITION,
            volume: 0.5,
            // BucketItem.emptyContents: 2.6 + (random - random) * 0.8, with f32 tolerance.
            pitch: (1.8 - 1.0e-6)..=(3.4 + 1.0e-6),
        }
    }

    fn assert_matches(&self, packets: &[SoundPacket], context: &str) {
        assert_eq!(
            packets.len(),
            1,
            "{context}: expected one sound, got {packets:?}"
        );
        let packet = &packets[0];
        assert_eq!(packet.sound_id, self.sound as u16, "{context}");
        assert_eq!(packet.category, self.category as i32, "{context}");
        assert_eq!(packet.position, self.position, "{context}");
        assert_eq!(packet.volume, self.volume, "{context}");
        assert!(
            self.pitch.contains(&packet.pitch),
            "{context}: pitch {}",
            packet.pitch
        );
    }
}

pub(super) struct Delivery {
    context: String,
    actor: Vec<SoundPacket>,
    observer: Vec<SoundPacket>,
}

impl Delivery {
    pub fn assert_predicted(&self, expected: &ExpectedSound) {
        assert!(
            self.actor.is_empty(),
            "{}: actor received predicted sound again: {:?}",
            self.context,
            self.actor
        );
        expected.assert_matches(&self.observer, &self.context);
    }

    pub fn assert_broadcast(&self, expected: &ExpectedSound) {
        assert_eq!(
            self.actor, self.observer,
            "{}: actorless broadcast differs",
            self.context
        );
        expected.assert_matches(&self.actor, &self.context);
    }

    pub fn assert_silent(&self) {
        assert!(self.actor.is_empty(), "{}: {:?}", self.context, self.actor);
        assert!(
            self.observer.is_empty(),
            "{}: {:?}",
            self.context,
            self.observer
        );
    }
}

#[derive(Default)]
pub(super) struct CancelBuckets {
    pub fills: AtomicUsize,
    pub empties: AtomicUsize,
}

impl EventHandler<PlayerBucketFillEvent> for CancelBuckets {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerBucketFillEvent,
    ) -> BoxFuture<'a, ()> {
        self.fills.fetch_add(1, Relaxed);
        event.cancelled = true;
        Box::pin(async {})
    }
}

impl EventHandler<PlayerBucketEmptyEvent> for CancelBuckets {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerBucketEmptyEvent,
    ) -> BoxFuture<'a, ()> {
        self.empties.fetch_add(1, Relaxed);
        event.cancelled = true;
        Box::pin(async {})
    }
}
