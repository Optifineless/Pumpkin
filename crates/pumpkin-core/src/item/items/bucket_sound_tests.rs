use super::sound_test_support::{
    ACTOR_SOUND_POSITION, BLOCK_SOUND_POSITION, CancelBuckets, DESTINATION, ExpectedSound, Fixture,
};
use crate::plugin::{
    EventPriority,
    api::events::player::player_bucket::{PlayerBucketEmptyEvent, PlayerBucketFillEvent},
};
use pumpkin_data::{
    Block,
    dimension::Dimension,
    entity::EntityType,
    item::Item,
    sound::{Sound, SoundCategory},
};
use pumpkin_util::Hand;
use std::sync::{Arc, atomic::Ordering::Relaxed};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue99_bucket_pickup_excludes_used_hand_actor_at_player_position() {
    let mut fixture = Fixture::new(Dimension::OVERWORLD);
    let mut deliveries = Vec::new();
    for hand in [Hand::Right, Hand::Left] {
        for (fluid, result, sound) in [
            (&Block::WATER, &Item::WATER_BUCKET, Sound::ItemBucketFill),
            (&Block::LAVA, &Item::LAVA_BUCKET, Sound::ItemBucketFillLava),
        ] {
            fixture.prepare(hand, &Item::BUCKET, fluid);
            let used_before = fixture.used_stat(&Item::BUCKET);
            fixture.use_item(hand, 0.0);
            fixture.assert_hand(hand, result);
            assert_eq!(fixture.world.get_block(&DESTINATION), &Block::AIR);
            assert_eq!(fixture.used_stat(&Item::BUCKET), used_before + 1);
            deliveries.push((
                fixture.take_delivery(format!("pickup {hand:?} {}", result.registry_key)),
                sound,
            ));
        }
    }
    fixture.finish().await;
    // BucketItem.use -> Player.playSound uses the player's category and exact coordinates.
    for (delivery, sound) in deliveries {
        delivery.assert_predicted(&ExpectedSound::regular(
            sound,
            SoundCategory::Players,
            ACTOR_SOUND_POSITION,
        ));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue99_bucket_empty_excludes_actor_and_preserves_bucket_variants() {
    let mut fixture = Fixture::new(Dimension::OVERWORLD);
    let mut deliveries = Vec::new();
    for hand in [Hand::Right, Hand::Left] {
        for (item, block, sound, category) in [
            (
                &Item::WATER_BUCKET,
                &Block::WATER,
                Sound::ItemBucketEmpty,
                SoundCategory::Blocks,
            ),
            (
                &Item::LAVA_BUCKET,
                &Block::LAVA,
                Sound::ItemBucketEmptyLava,
                SoundCategory::Blocks,
            ),
            (
                &Item::POWDER_SNOW_BUCKET,
                &Block::POWDER_SNOW,
                Sound::ItemBucketEmptyPowderSnow,
                SoundCategory::Blocks,
            ),
            (
                &Item::COD_BUCKET,
                &Block::WATER,
                Sound::ItemBucketEmptyFish,
                SoundCategory::Neutral,
            ),
        ] {
            fixture.prepare(hand, item, &Block::AIR);
            let used_before = fixture.used_stat(item);
            let entities_before = fixture.world.entities.load().len();
            fixture.use_item(hand, 0.0);
            fixture.assert_hand(hand, &Item::BUCKET);
            assert_eq!(fixture.world.get_block(&DESTINATION), block);
            assert_eq!(fixture.used_stat(item), used_before + 1);
            if item == &Item::COD_BUCKET {
                let entities = fixture.world.entities.load();
                assert_eq!(entities.len(), entities_before + 1);
                assert!(entities.iter().all(|entity| {
                    entity.get_entity().entity_type == &EntityType::COD
                        && entity.get_entity().is_silent()
                }));
            }
            deliveries.push((
                fixture.take_delivery(format!("empty {hand:?} {}", item.registry_key)),
                sound,
                category,
            ));
        }
    }
    fixture.finish().await;
    // BucketItem/SolidBucketItem/MobBucketItem use the destination block center.
    for (delivery, sound, category) in deliveries {
        delivery.assert_predicted(&ExpectedSound::regular(
            sound,
            category,
            BLOCK_SOUND_POSITION,
        ));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue99_bucket_evaporation_excludes_actor_without_placing_water() {
    let mut fixture = Fixture::new(Dimension::THE_NETHER);
    let mut deliveries = Vec::new();
    for hand in [Hand::Right, Hand::Left] {
        fixture.prepare(hand, &Item::WATER_BUCKET, &Block::AIR);
        let used_before = fixture.used_stat(&Item::WATER_BUCKET);
        fixture.use_item(hand, 0.0);
        fixture.assert_hand(hand, &Item::BUCKET);
        assert_eq!(fixture.world.get_block(&DESTINATION), &Block::AIR);
        assert_eq!(fixture.used_stat(&Item::WATER_BUCKET), used_before + 1);
        deliveries.push(fixture.take_delivery(format!("evaporation {hand:?}")));
    }
    fixture.finish().await;
    for delivery in deliveries {
        delivery.assert_predicted(&ExpectedSound::evaporation());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue99_bucket_cancelled_and_missed_uses_are_silent() {
    let mut fixture = Fixture::new(Dimension::OVERWORLD);
    let cancellation = Arc::new(CancelBuckets::default());
    fixture
        .server
        .plugin_manager
        .register::<PlayerBucketFillEvent, _>(cancellation.clone(), EventPriority::Normal, true);
    fixture
        .server
        .plugin_manager
        .register::<PlayerBucketEmptyEvent, _>(cancellation.clone(), EventPriority::Normal, true);
    let mut deliveries = Vec::new();
    for hand in [Hand::Right, Hand::Left] {
        for (item, target) in [
            (&Item::BUCKET, &Block::WATER),
            (&Item::WATER_BUCKET, &Block::AIR),
        ] {
            fixture.prepare(hand, item, target);
            let used_before = fixture.used_stat(item);
            fixture.use_item(hand, 0.0);
            fixture.assert_hand(hand, item);
            assert_eq!(fixture.world.get_block(&DESTINATION), target);
            assert_eq!(fixture.used_stat(item), used_before);
            deliveries
                .push(fixture.take_delivery(format!("cancelled {hand:?} {}", item.registry_key)));
            // The westward ray remains in the loaded chunk and misses the prepared target.
            fixture.use_item(hand, 90.0);
            fixture.assert_hand(hand, item);
            assert_eq!(fixture.world.get_block(&DESTINATION), target);
            assert_eq!(fixture.used_stat(item), used_before);
            deliveries
                .push(fixture.take_delivery(format!("missed {hand:?} {}", item.registry_key)));
        }
    }
    assert_eq!(cancellation.fills.load(Relaxed), 2);
    assert_eq!(cancellation.empties.load(Relaxed), 2);
    fixture.finish().await;
    for delivery in deliveries {
        delivery.assert_silent();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue99_bucket_dispenser_keeps_actorless_empty_and_evaporation_broadcasts() {
    let mut fixture = Fixture::new(Dimension::OVERWORLD);
    let mut deliveries = Vec::new();
    for (item, block, sound) in [
        (&Item::WATER_BUCKET, &Block::WATER, Sound::ItemBucketEmpty),
        (&Item::LAVA_BUCKET, &Block::LAVA, Sound::ItemBucketEmptyLava),
    ] {
        let remainder = fixture.dispense(item);
        assert_eq!(remainder.item, &Item::BUCKET);
        assert_eq!(remainder.item_count, 1);
        assert_eq!(fixture.world.get_block(&DESTINATION), block);
        deliveries.push((
            fixture.take_delivery(format!("dispenser {}", item.registry_key)),
            sound,
        ));
    }
    fixture.finish().await;
    let mut nether = Fixture::new(Dimension::THE_NETHER);
    let remainder = nether.dispense(&Item::WATER_BUCKET);
    assert_eq!(remainder.item, &Item::BUCKET);
    assert_eq!(remainder.item_count, 1);
    assert_eq!(nether.world.get_block(&DESTINATION), &Block::AIR);
    let evaporation = nether.take_delivery("dispenser evaporation".into());
    nether.finish().await;
    for (delivery, sound) in deliveries {
        delivery.assert_broadcast(&ExpectedSound::regular(
            sound,
            SoundCategory::Blocks,
            BLOCK_SOUND_POSITION,
        ));
    }
    evaporation.assert_broadcast(&ExpectedSound::evaporation());
}
