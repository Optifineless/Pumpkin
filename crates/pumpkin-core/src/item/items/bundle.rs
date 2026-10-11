use crate::entity::player::Player;
use crate::item::{ItemBehaviour, ItemMetadata};
use pumpkin_data::{
    data_component_impl::BundleContentsImpl,
    item::Item,
    item_stack::ItemStack,
    sound::{Sound, SoundCategory},
    statistic::StatisticCategory,
    tag,
};
use pumpkin_util::Hand;

pub struct BundleItem;

// BundleItem.getUseDuration and onUseTick hardcode 200 ticks and a ten-tick initial delay.
const TICKS_MAX_THROW_DURATION: i32 = 200;
const TICKS_AFTER_FIRST_THROW: i32 = 10;
const TICKS_BETWEEN_THROWS: i32 = 2;
const fn should_drop(remaining: i32) -> bool {
    remaining == TICKS_MAX_THROW_DURATION
        || (remaining < TICKS_MAX_THROW_DURATION - TICKS_AFTER_FIRST_THROW
            && remaining % TICKS_BETWEEN_THROWS == 0)
}

/// Plays bundle click and pouring sounds with vanilla volume and pitch.
pub(crate) fn play_bundle_sound(player: &Player, sound: Sound) {
    // BundleItem.playInsertSound / playRemoveOneSound / playInsertFailSound / playDropContentsSound.
    const SOUND_BASE: f32 = 0.8;
    const SOUND_PITCH_RANGE: f32 = 0.4;
    let (volume, pitch) = if sound == Sound::ItemBundleInsertFail {
        (1.0, 1.0)
    } else {
        (
            SOUND_BASE,
            SOUND_BASE + rand::random::<f32>() * SOUND_PITCH_RANGE,
        )
    };
    let position = if sound == Sound::ItemBundleDropContents {
        player.position().to_block_pos().to_centered_f64()
    } else {
        player.position()
    };
    player
        .world()
        .play_sound_fine(sound, SoundCategory::Players, &position, volume, pitch);
}

impl ItemMetadata for BundleItem {
    fn ids() -> Box<[u16]> {
        tag::Item::MINECRAFT_BUNDLES.1.into()
    }
}

impl ItemBehaviour for BundleItem {
    fn normal_use(&self, item: &Item, player: &Player) {
        self.normal_use_with_hand(item, player, 0.0, 0.0, Hand::Right);
    }

    fn normal_use_with_hand(
        &self,
        _item: &Item,
        player: &Player,
        _yaw: f32,
        _pitch: f32,
        hand: Hand,
    ) {
        // BundleItem.use starts using the requested hand, including an empty bundle.
        let stack = player.inventory.get_stack_in_hand(hand);
        player
            .living_entity
            .set_active_hand(hand, stack, TICKS_MAX_THROW_DURATION);
    }

    fn on_use_tick(&self, stack: &ItemStack, player: &Player, remaining: i32) {
        if !should_drop(remaining) {
            return;
        }
        let hand = *player
            .living_entity
            .active_hand
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(hand) = hand else {
            return;
        };
        let mut bundle = player.inventory.get_stack_in_hand(hand);
        if bundle.uid != stack.uid || bundle.item != stack.item {
            return;
        }
        // BundleItem.removeOneItemFromBundle persists contents before spawning the dropped item.
        let Some(extracted) = bundle
            .get_data_component_mut::<BundleContentsImpl>()
            .and_then(BundleContentsImpl::try_extract)
        else {
            return;
        };
        play_bundle_sound(player, Sound::ItemBundleRemoveOne);
        player.inventory.set_stack_in_hand(hand, bundle.clone());
        let slot = match hand {
            Hand::Right => player.inventory.get_selected_slot() as usize,
            Hand::Left => {
                pumpkin_inventory::player::player_inventory::PlayerInventory::OFF_HAND_SLOT
            }
        };
        player.sync_hand_slot(slot, bundle);
        player.drop_item(extracted);
        play_bundle_sound(player, Sound::ItemBundleDropContents);
        player.increment_stat(StatisticCategory::Used, i32::from(stack.item.id), 1);
    }

    fn get_use_duration(&self) -> i32 {
        TICKS_MAX_THROW_DURATION
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pouring_starts_immediately_then_waits_ten_ticks() {
        let drops: Vec<_> = (184..=200).rev().filter(|r| should_drop(*r)).collect();
        assert_eq!(drops, [200, 188, 186, 184]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pouring_writes_back_the_active_hand_contents_once() {
        use crate::{net::java::combat_test_support::TestPlayer, server::combat_test_support};
        let dir = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(dir.path());
        let world = combat_test_support::world(&server, dir.path());
        let fixture = TestPlayer::new(&world);
        let player = &fixture.player;
        let mut bundle = ItemStack::new(1, &Item::BUNDLE);
        bundle.set_data_component(BundleContentsImpl {
            items: vec![
                ItemStack::new(3, &Item::STONE),
                ItemStack::new(2, &Item::DIAMOND),
            ],
            selected_item: -1,
        });
        player
            .inventory
            .set_stack_in_hand(Hand::Left, bundle.clone());
        BundleItem.normal_use_with_hand(bundle.item, player, 0.0, 0.0, Hand::Left);
        BundleItem.on_use_tick(&bundle, player, 200);
        assert_eq!(
            player
                .inventory
                .off_hand_item()
                .get_data_component::<BundleContentsImpl>()
                .unwrap()
                .items
                .len(),
            1
        );
        BundleItem.on_use_tick(&bundle, player, 188);
        BundleItem.on_use_tick(&bundle, player, 186);
        assert!(
            player
                .inventory
                .off_hand_item()
                .get_data_component::<BundleContentsImpl>()
                .unwrap()
                .items
                .is_empty()
        );
        assert!(world.level.shutdown().await.is_ok());
        crate::server::fixture_lifecycle::finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn packet_use_counts_only_successful_bundle_drops_and_swap_stops_pouring() {
        use crate::{net::java::combat_test_support::TestPlayer, server::combat_test_support};
        use pumpkin_protocol::{codec::var_int::VarInt, java::server::play::SUseItem};

        let dir = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(dir.path());
        let world = combat_test_support::world(&server, dir.path());
        for hand in [Hand::Right, Hand::Left] {
            let fixture = TestPlayer::new(&world);
            let player = &fixture.player;
            let other_hand = if hand == Hand::Right {
                Hand::Left
            } else {
                Hand::Right
            };
            let sword = ItemStack::new(1, &Item::DIAMOND_SWORD);
            player
                .inventory
                .set_stack_in_hand(other_hand, sword.clone());
            let mut bundle = ItemStack::new(1, &Item::BUNDLE);
            bundle.set_data_component(BundleContentsImpl {
                items: vec![
                    ItemStack::new(3, &Item::STONE),
                    ItemStack::new(2, &Item::DIAMOND),
                    ItemStack::new(1, &Item::GOLD_INGOT),
                ],
                selected_item: -1,
            });
            player.inventory.set_stack_in_hand(hand, bundle.clone());
            let packet = SUseItem {
                hand: VarInt(i32::from(hand == Hand::Left)),
                sequence: VarInt(1),
                yaw: 0.0,
                pitch: 0.0,
            };
            fixture.client().handle_use_item(player, &packet, &server);
            let used = || player.get_stat(StatisticCategory::Used, i32::from(Item::BUNDLE.id));
            assert_eq!(used(), 0);
            assert_eq!(
                *player.living_entity.active_hand.lock().unwrap(),
                Some(hand)
            );
            server.item_registry.on_use_tick(&bundle, player, 200);
            assert_eq!(used(), 1);
            server.item_registry.on_use_tick(&bundle, player, 198);
            assert_eq!(used(), 1);
            server.item_registry.on_use_tick(&bundle, player, 188);
            assert_eq!(used(), 2);
            let remaining = player.inventory.get_stack_in_hand(hand);
            assert_eq!(remaining.item_count, 1);
            assert_eq!(
                remaining
                    .get_data_component::<BundleContentsImpl>()
                    .unwrap()
                    .items
                    .len(),
                1
            );
            assert!(
                player
                    .inventory
                    .get_stack_in_hand(other_hand)
                    .are_equal(&sword)
            );
            player.swap_item();
            assert!(player.living_entity.active_hand.lock().unwrap().is_none());
            server.item_registry.on_use_tick(&bundle, player, 186);
            assert_eq!(used(), 2);
            assert!(
                player
                    .inventory
                    .get_stack_in_hand(other_hand)
                    .are_equal(&remaining)
            );
            assert!(player.inventory.get_stack_in_hand(hand).are_equal(&sword));

            let empty_bundle = ItemStack::new(1, &Item::BUNDLE);
            player
                .inventory
                .set_stack_in_hand(hand, empty_bundle.clone());
            fixture.client().handle_use_item(player, &packet, &server);
            server.item_registry.on_use_tick(&empty_bundle, player, 200);
            assert_eq!(used(), 2);
        }
        assert!(world.level.shutdown().await.is_ok());
        crate::server::fixture_lifecycle::finish().await;
    }
}
