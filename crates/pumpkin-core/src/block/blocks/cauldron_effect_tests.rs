//! Review regressions use the Java packet handler and observe its synchronous effects.
use std::sync::{Arc, Mutex};

use super::support::{
    Fixture, POS, dyed_armor, layered, named_shulker, other_hand, patterned_banner,
};
use crate::{
    entity::player::Player,
    item::items::glass_bottle::water_bottle,
    plugin::{BoxFuture, EventHandler, EventPriority, world::generic_game::GenericGameEvent},
    server::Server,
    world::World,
};
use pumpkin_data::{
    Block, BlockStateId,
    data_component::DataComponent,
    data_component_impl::BannerPatternsImpl,
    game_event::GameEvent,
    item::Item,
    item_stack::ItemStack,
    sound::{Sound, SoundCategory},
    statistic::{CustomStatistic, StatisticCategory},
};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::{
    codec::{item_stack_seralizer::ItemStackSerializer, var_int::VarInt},
    java::client::play::CSetPlayerInventory,
};
use pumpkin_util::{GameMode, Hand};

struct HandSnapshot {
    event: String,
    state: BlockStateId,
    right: ItemStack,
    left: ItemStack,
}

impl HandSnapshot {
    fn hand(&self, hand: Hand) -> &ItemStack {
        if hand == Hand::Right {
            &self.right
        } else {
            &self.left
        }
    }
}

struct HandObserver {
    player: Arc<Player>,
    world: Arc<World>,
    seen: Mutex<Vec<HandSnapshot>>,
    replacement: Mutex<Option<(Hand, ItemStack)>>,
}

impl HandObserver {
    fn register(fixture: &Fixture) -> Arc<Self> {
        let observer = Arc::new(Self {
            player: fixture.client.player.clone(),
            world: fixture.world.clone(),
            seen: Mutex::new(Vec::new()),
            replacement: Mutex::new(None),
        });
        fixture
            .server
            .plugin_manager
            .register::<GenericGameEvent, _>(observer.clone(), EventPriority::Normal, true);
        observer
    }
}

impl EventHandler<GenericGameEvent> for HandObserver {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut GenericGameEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let inventory = self.player.inventory();
            self.seen.lock().unwrap().push(HandSnapshot {
                event: event.event_key.clone(),
                state: self.world.get_block_state_id(&POS),
                right: inventory.held_item(),
                left: inventory.off_hand_item(),
            });
            let replacement = self.replacement.lock().unwrap().take();
            if let Some((hand, stack)) = replacement {
                inventory.set_stack_in_hand(hand, stack);
            }
        })
    }
}

type ExchangeCase = (BlockStateId, ItemStack, ItemStack, GameEvent, BlockStateId);

fn exchange_cases() -> Vec<ExchangeCase> {
    let empty = Block::CAULDRON.default_state.id;
    let water = layered(&Block::WATER_CAULDRON, "3");
    let colored = named_shulker();
    let mut cleaned_shulker = colored.copy_with_count(1);
    cleaned_shulker.item = &Item::SHULKER_BOX;
    let banner = patterned_banner().copy_with_count(1);
    let mut cleaned_banner = banner.clone();
    cleaned_banner
        .get_data_component_mut::<BannerPatternsImpl>()
        .unwrap()
        .layers
        .pop();
    let armor = dyed_armor();
    let mut cleaned_armor = armor.clone();
    cleaned_armor.remove_data_component(DataComponent::DyedColor);
    vec![
        (
            empty,
            ItemStack::new(1, &Item::WATER_BUCKET),
            ItemStack::new(1, &Item::BUCKET),
            GameEvent::FluidPlace,
            water,
        ),
        (
            water,
            ItemStack::new(1, &Item::BUCKET),
            ItemStack::new(1, &Item::WATER_BUCKET),
            GameEvent::FluidPickup,
            empty,
        ),
        (
            empty,
            water_bottle(),
            ItemStack::new(1, &Item::GLASS_BOTTLE),
            GameEvent::FluidPlace,
            layered(&Block::WATER_CAULDRON, "1"),
        ),
        (
            water,
            ItemStack::new(1, &Item::GLASS_BOTTLE),
            water_bottle(),
            GameEvent::BlockChange,
            layered(&Block::WATER_CAULDRON, "2"),
        ),
        (
            water,
            colored,
            cleaned_shulker,
            GameEvent::BlockChange,
            layered(&Block::WATER_CAULDRON, "2"),
        ),
        (
            water,
            banner,
            cleaned_banner,
            GameEvent::BlockChange,
            layered(&Block::WATER_CAULDRON, "2"),
        ),
        (
            water,
            armor,
            cleaned_armor,
            GameEvent::BlockChange,
            layered(&Block::WATER_CAULDRON, "2"),
        ),
    ]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review116_cauldron_hand_exchange_precedes_game_events_and_syncs() {
    let mut fixture = Fixture::new();
    let observer = HandObserver::register(&fixture);
    for hand in [Hand::Right, Hand::Left] {
        for (state, input, result, event, new_state) in exchange_cases() {
            fixture.reset(state, hand, input);
            observer.seen.lock().unwrap().clear();
            let other = fixture
                .client
                .player
                .inventory()
                .get_stack_in_hand(other_hand(hand));
            fixture.use_top(hand);
            let seen = std::mem::take(&mut *observer.seen.lock().unwrap());
            assert!(!seen.is_empty(), "the cauldron emitted its game event");
            assert_eq!(seen[0].event, event.name());
            assert_eq!(seen[0].state, new_state);
            assert_eq!(
                seen[0].hand(hand).get_item().id,
                result.get_item().id,
                "the real used hand must be exchanged before the game-event callback"
            );
            assert!(seen[0].hand(hand).are_equal(&result));
            assert!(seen[0].hand(other_hand(hand)).are_equal(&other));
            assert!(
                fixture
                    .client
                    .player
                    .inventory()
                    .get_stack_in_hand(hand)
                    .are_equal(&result)
            );
            // The deferred write-back skips a direct writer, so that writer must sync the slot.
            let slot = if hand == Hand::Right { 0 } else { 40 };
            let expected = fixture
                .client
                .client()
                .serialize_packet(&CSetPlayerInventory::new(
                    VarInt(slot),
                    &ItemStackSerializer::from(result),
                ))
                .unwrap();
            assert!(fixture.client.take_packets().contains(&expected));
        }
    }
    fixture.world.level.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review116_cauldron_callbacks_keep_replaced_or_deleted_hand() {
    let mut fixture = Fixture::new();
    let observer = HandObserver::register(&fixture);
    for hand in [Hand::Right, Hand::Left] {
        for replacement in [ItemStack::EMPTY.clone(), ItemStack::new(1, &Item::DIAMOND)] {
            fixture.reset(
                Block::CAULDRON.default_state.id,
                hand,
                ItemStack::new(1, &Item::WATER_BUCKET),
            );
            observer.seen.lock().unwrap().clear();
            *observer.replacement.lock().unwrap() = Some((hand, replacement.clone()));
            fixture.use_top(hand);
            // This control already passes: later write-back must preserve a callback's writer.
            assert!(
                fixture
                    .client
                    .player
                    .inventory()
                    .get_stack_in_hand(hand)
                    .are_equal(&replacement)
            );
            let seen = std::mem::take(&mut *observer.seen.lock().unwrap());
            assert_eq!(seen.len(), 1);
            assert_eq!(seen[0].event, GameEvent::FluidPlace.name());
            assert_eq!(seen[0].hand(hand).get_item().id, Item::BUCKET.id);
        }
    }
    fixture.world.level.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review116_cauldron_creative_banner_overflow_is_discarded() {
    let mut fixture = Fixture::new();
    let original = patterned_banner();
    let mut cleaned = original.copy_with_count(1);
    cleaned
        .get_data_component_mut::<BannerPatternsImpl>()
        .unwrap()
        .layers
        .pop();
    // Survival is the positive drop control before the Creative refusal to drop.
    for mode in [GameMode::Survival, GameMode::Creative] {
        fixture.client.player.gamemode.store(mode);
        fixture.reset(
            layered(&Block::WATER_CAULDRON, "1"),
            Hand::Left,
            original.clone(),
        );
        let inventory = fixture.client.player.inventory();
        for slot in 0..36 {
            inventory.set_stack(slot, ItemStack::new(64, &Item::STONE));
        }
        let before_drops = fixture.drops().len();
        fixture.use_top(Hand::Left);
        let expected_count = if mode == GameMode::Creative { 2 } else { 1 };
        assert!(
            inventory
                .off_hand_item()
                .are_equal(&original.copy_with_count(expected_count))
        );
        assert_eq!(
            fixture.world.get_block_state_id(&POS),
            Block::CAULDRON.default_state.id
        );
        assert_eq!(
            fixture.client.player.stats.lock().unwrap().get(
                StatisticCategory::Custom,
                CustomStatistic::CleanBanner as i32,
            ),
            1
        );
        let drops = fixture.drops();
        if mode == GameMode::Creative {
            assert_eq!(drops.len(), before_drops, "Creative overflow is discarded");
        } else {
            assert_eq!(drops.len(), before_drops + 1);
            assert!(drops.last().unwrap().are_equal(&cleaned));
        }
    }
    fixture.world.level.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review116_cauldron_potion_stats_follow_the_consumed_input_alias() {
    let mut fixture = Fixture::new();
    for state in [
        Block::CAULDRON.default_state.id,
        layered(&Block::WATER_CAULDRON, "1"),
    ] {
        for mode in [GameMode::Survival, GameMode::Creative] {
            for count in [1, 2] {
                fixture.client.player.gamemode.store(mode);
                fixture.reset(state, Hand::Left, water_bottle().copy_with_count(count));
                fixture.use_top(Hand::Left);
                let used_air = state != Block::CAULDRON.default_state.id
                    && mode == GameMode::Survival
                    && count == 1;
                let stats = fixture.client.player.stats.lock().unwrap();
                assert_eq!(
                    stats.get(StatisticCategory::Used, i32::from(Item::AIR.id)),
                    i32::from(used_air)
                );
                assert_eq!(
                    stats.get(StatisticCategory::Used, i32::from(Item::POTION.id)),
                    i32::from(!used_air)
                );
                assert_eq!(
                    stats.get(StatisticCategory::Used, i32::from(Item::GLASS_BOTTLE.id)),
                    0
                );
                assert_eq!(
                    stats.get(
                        StatisticCategory::Custom,
                        CustomStatistic::UseCauldron as i32
                    ),
                    1
                );
            }
        }
    }
    fixture.world.level.shutdown().await.unwrap();
}

#[derive(Debug, PartialEq)]
struct HeardSound {
    id: i32,
    category: i32,
    position: [i32; 3],
    volume: f32,
    pitch: f32,
}

fn four_bytes(input: &mut &[u8]) -> [u8; 4] {
    let (bytes, rest) = input.split_at(4);
    *input = rest;
    bytes.try_into().unwrap()
}

fn heard_sounds(fixture: &mut Fixture) -> Vec<HeardSound> {
    fixture
        .client
        .take_packets()
        .into_iter()
        .filter_map(|packet| {
            let mut input: &[u8] = packet.as_ref();
            if VarInt::decode(&mut input).unwrap().0
                != pumpkin_data::packet::clientbound::play::SOUND.0
            {
                return None;
            }
            Some(HeardSound {
                id: VarInt::decode(&mut input).unwrap().0,
                category: VarInt::decode(&mut input).unwrap().0,
                position: [
                    i32::from_be_bytes(four_bytes(&mut input)),
                    i32::from_be_bytes(four_bytes(&mut input)),
                    i32::from_be_bytes(four_bytes(&mut input)),
                ],
                volume: f32::from_be_bytes(four_bytes(&mut input)),
                pitch: f32::from_be_bytes(four_bytes(&mut input)),
            })
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review116_cauldron_sounds_use_block_center() {
    let mut fixture = Fixture::new();
    let empty = Block::CAULDRON.default_state.id;
    for (state, input, sound) in [
        (
            empty,
            ItemStack::new(1, &Item::WATER_BUCKET),
            Sound::ItemBucketEmpty,
        ),
        (
            empty,
            ItemStack::new(1, &Item::LAVA_BUCKET),
            Sound::ItemBucketEmptyLava,
        ),
        (
            empty,
            ItemStack::new(1, &Item::POWDER_SNOW_BUCKET),
            Sound::ItemBucketEmptyPowderSnow,
        ),
        (
            layered(&Block::WATER_CAULDRON, "3"),
            ItemStack::new(1, &Item::BUCKET),
            Sound::ItemBucketFill,
        ),
        (
            Block::LAVA_CAULDRON.default_state.id,
            ItemStack::new(1, &Item::BUCKET),
            Sound::ItemBucketFillLava,
        ),
        (
            layered(&Block::POWDER_SNOW_CAULDRON, "3"),
            ItemStack::new(1, &Item::BUCKET),
            Sound::ItemBucketFillPowderSnow,
        ),
        (empty, water_bottle(), Sound::ItemBottleEmpty),
        (
            layered(&Block::WATER_CAULDRON, "2"),
            ItemStack::new(1, &Item::GLASS_BOTTLE),
            Sound::ItemBottleFill,
        ),
    ] {
        fixture.reset(state, Hand::Right, input);
        fixture.use_top(Hand::Right);
        // Level.playSound(null, BlockPos, ...) broadcasts from the cell center, including the actor.
        assert_eq!(
            heard_sounds(&mut fixture),
            vec![HeardSound {
                id: i32::from(sound as u16) + 1,
                category: SoundCategory::Blocks as i32,
                position: [POS.0.x * 8 + 4, POS.0.y * 8 + 4, POS.0.z * 8 + 4],
                volume: 1.0,
                pitch: 1.0,
            }]
        );
    }
    fixture.world.level.shutdown().await.unwrap();
}
