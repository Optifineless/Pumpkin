#![expect(clippy::unwrap_used, reason = "Bounded real Java interaction fixtures")]

use std::sync::atomic::Ordering::Relaxed;

use bytes::Bytes;
use pumpkin_data::{
    entity::EntityType, item::Item, item_stack::ItemStack, meta_data_type::MetaDataType,
    packet::clientbound::play, tracked_data,
};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::{VarInt, ser::NetworkReadExt};
use pumpkin_util::Hand;

use super::{b1020d_g04_test_support::Fixture, cat::CatEntity, cow::CowEntity};
use crate::entity::{EntityBase, ageable::AgeableMob, mob::Mob};

fn baby_values(packets: &[Bytes], entity_id: i32) -> Vec<bool> {
    packets
        .iter()
        .filter_map(|packet| {
            let mut reader = packet.as_ref();
            if VarInt::decode(&mut reader).unwrap().0 != play::SET_ENTITY_DATA.0
                || reader.get_var_int().unwrap().0 != entity_id
            {
                return None;
            }
            baby_value(reader)
        })
        .collect()
}

fn baby_value(mut reader: &[u8]) -> Option<bool> {
    let tracked = tracked_data::ageable_mob::DATA_BABY_ID;
    let mut baby = None;
    loop {
        let index = reader.get_u8().unwrap();
        if index == u8::MAX {
            assert!(reader.is_empty(), "metadata terminator must end the packet");
            return baby;
        }
        let serializer = MetaDataType {
            id: reader.get_var_int().unwrap().0,
        };
        if index == u8::from(tracked.id) {
            assert_eq!(serializer, MetaDataType::BOOLEAN);
        }
        if serializer == MetaDataType::BOOLEAN {
            let value = reader.get_bool().unwrap();
            if index == u8::from(tracked.id) {
                assert!(baby.replace(value).is_none(), "duplicate baby metadata");
            }
        } else if serializer == MetaDataType::BYTE {
            reader.get_u8().unwrap();
        } else if serializer == MetaDataType::FLOAT {
            reader.get_f32_be().unwrap();
        } else {
            assert!(
                [
                    MetaDataType::INT,
                    MetaDataType::POSE,
                    MetaDataType::COW_VARIANT,
                    MetaDataType::COW_SOUND_VARIANT,
                ]
                .contains(&serializer),
                "unexpected cow metadata serializer: {serializer:?}"
            );
            reader.get_var_int().unwrap();
        }
    }
}

fn assert_baby_metadata(fixture: &mut Fixture, cow: &CowEntity, baby: bool) {
    fixture.drain_packets();
    cow.get_entity().refresh_synced_data();
    for observer in &mut fixture.observers {
        assert_eq!(
            baby_values(&observer.take_packets(), cow.get_entity().entity_id),
            [baby],
            "real observer metadata must describe the cow's baby state"
        );
    }
}

fn saved_age(target: &dyn EntityBase) -> NbtCompound {
    let mut saved = NbtCompound::new();
    target.write_custom_nbt(&mut saved);
    saved
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g04_baby_cow_food_uses_ageable_contract() {
    let mut fixture = Fixture::new();
    for hand in [Hand::Right, Hand::Left] {
        for at in [false, true] {
            let target = fixture.mob(&EntityType::COW);
            let cow = target.cast_any().downcast_ref::<CowEntity>().unwrap();
            cow.set_baby(true);
            fixture.set_hand(hand, ItemStack::new(3, &Item::WHEAT));
            assert_baby_metadata(&mut fixture, cow, true);

            fixture.interact(target.as_ref(), hand, at);

            assert_eq!(
                fixture.hand(hand).item_count,
                2,
                "one wheat pays for growth"
            );
            assert_eq!(cow.get_age(), -21600, "literal newborn first-feed age");
            assert_eq!(
                cow.get_ageable_data().forced_age.load(Relaxed),
                2400,
                "feeding must record forced growth through AgeableMob"
            );
            let saved = saved_age(target.as_ref());
            assert_eq!(saved.get_int("Age"), Some(-21600));
            assert_eq!(saved.get_int("ForcedAge"), Some(2400));
            let restored = fixture.mob(&EntityType::COW);
            restored.read_custom_nbt(&saved);
            let restored_cow = restored.cast_any().downcast_ref::<CowEntity>().unwrap();
            assert_eq!(restored_cow.get_age(), -21600);
            assert_eq!(
                restored_cow.get_ageable_data().forced_age.load(Relaxed),
                2400
            );
            assert_baby_metadata(&mut fixture, cow, true);
        }
    }
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g04_baby_cow_food_whole_second_edges() {
    let fixture = Fixture::new();
    for hand in [Hand::Right, Hand::Left] {
        for at in [false, true] {
            for (initial, seconds, expected, forced) in [
                (-1, 0, -1, 0),
                (-19, 0, -19, 0),
                (-20, 0, -20, 0),
                (-199, 0, -199, 0),
                (-200, 1, -180, 20),
            ] {
                let target = fixture.mob(&EntityType::COW);
                let cow = target.cast_any().downcast_ref::<CowEntity>().unwrap();
                cow.set_age(initial);
                fixture.set_hand(hand, ItemStack::new(3, &Item::WHEAT));
                assert_eq!(
                    CowEntity::get_speed_up_seconds_when_feeding(-initial),
                    seconds
                );

                fixture.interact(target.as_ref(), hand, at);

                assert_eq!(
                    fixture.hand(hand).item_count,
                    2,
                    "even rounded-zero feeding pays once"
                );
                assert_eq!(
                    cow.get_age(),
                    expected,
                    "whole-second feeding at age {initial}"
                );
                assert_eq!(cow.get_ageable_data().forced_age.load(Relaxed), forced);
                let saved = saved_age(target.as_ref());
                assert_eq!(saved.get_int("Age"), Some(expected));
                assert_eq!(saved.get_int("ForcedAge"), Some(forced));
            }
        }
    }
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g04_baby_cow_age_lock_refuses_growth_payment() {
    let fixture = Fixture::new();
    for hand in [Hand::Right, Hand::Left] {
        for at in [false, true] {
            let target = fixture.mob(&EntityType::COW);
            let cow = target.cast_any().downcast_ref::<CowEntity>().unwrap();
            cow.set_baby(true);
            cow.set_age_locked(true);
            let wheat = ItemStack::new(3, &Item::WHEAT);
            fixture.set_hand(hand, wheat.clone());

            fixture.interact(target.as_ref(), hand, at);

            assert!(
                fixture.hand(hand).are_equal(&wheat),
                "locked growth cannot charge food"
            );
            assert_eq!(cow.get_age(), -24000);
            assert_eq!(cow.get_ageable_data().forced_age.load(Relaxed), 0);
            let saved = saved_age(target.as_ref());
            assert_eq!(saved.get_int("Age"), Some(-24000));
            assert_eq!(saved.get_int("ForcedAge"), Some(0));
            assert_eq!(saved.get_bool("AgeLocked"), Some(true));
        }
    }
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g04_baby_food_payment_and_setter_metadata_control() {
    let mut fixture = Fixture::new();
    for hand in [Hand::Right, Hand::Left] {
        for at in [false, true] {
            let target = fixture.mob(&EntityType::COW);
            let cow = target.cast_any().downcast_ref::<CowEntity>().unwrap();
            cow.set_baby(true);
            fixture.set_hand(hand, ItemStack::new(3, &Item::WHEAT));
            assert_baby_metadata(&mut fixture, cow, true);

            fixture.interact(target.as_ref(), hand, at);

            assert_eq!(fixture.hand(hand).item_count, 2, "payment control");
            assert_baby_metadata(&mut fixture, cow, true);
            cow.set_age(0);
            assert_baby_metadata(&mut fixture, cow, false);
            assert_eq!(
                fixture.hand(hand).item_count,
                2,
                "setter does not charge food"
            );
        }
    }
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g04_baby_cat_retains_legacy_growth_control() {
    let fixture = Fixture::new();
    for hand in [Hand::Right, Hand::Left] {
        for at in [false, true] {
            let target = fixture.mob(&EntityType::CAT);
            let cat = target.cast_any().downcast_ref::<CatEntity>().unwrap();
            assert!(
                cat.as_ageable().is_none(),
                "Cat age foundation belongs to issue #370"
            );
            cat.set_tame(true, Some(fixture.player.gameprofile.id));
            cat.set_sitting(false);
            let living = &cat.mob_entity.living_entity;
            living.set_health(living.get_max_health());
            cat.get_entity().set_age(-200);
            fixture.set_hand(hand, ItemStack::new(3, &Item::COD));

            fixture.interact(target.as_ref(), hand, at);

            assert_eq!(fixture.hand(hand).item_count, 2);
            assert_eq!(
                cat.get_entity().age.load(Relaxed),
                -180,
                "legacy growth remains unchanged"
            );
            assert_eq!(living.health.load(), living.get_max_health());
            assert!(!cat.is_sitting());
        }
    }
    fixture.finish().await;
}
