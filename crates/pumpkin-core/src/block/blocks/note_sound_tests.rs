#[path = "note_sound_test_support.rs"]
mod support;

use crate::{
    block::entities::skull::SkullBlockEntity,
    plugin::{
        BoxFuture, EventHandler, EventPriority, api::events::block::note_play::NotePlayEvent,
    },
    server::Server,
};
use pumpkin_data::{
    Block, data_component_impl::NoteBlockSoundImpl, item::Item, item_stack::ItemStack, sound::Sound,
};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use support::*;

struct CancelNote(AtomicBool);
impl EventHandler<NotePlayEvent> for CancelNote {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut NotePlayEvent,
    ) -> BoxFuture<'a, ()> {
        event.cancelled = self.0.load(Ordering::Relaxed);
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g08_note_instrument_controls() {
    let mut fixture = Fixture::new();
    // Existing instruments remain pitched/untuned as appropriate through actual events.
    for (support, sound) in [
        (&Block::DIAMOND_BLOCK, Sound::BlockNoteBlockHarp),
        (&Block::GLOWSTONE, Sound::BlockNoteBlockPling),
    ] {
        fixture.prepare(support);
        assert_sound(
            &fixture.attack(),
            &EventSound::Registered(sound as u16),
            2.0,
        );
    }
    for (item, sound) in [
        (&Item::ZOMBIE_HEAD, Sound::BlockNoteBlockImitateZombie),
        (&Item::SKELETON_SKULL, Sound::BlockNoteBlockImitateSkeleton),
        (&Item::CREEPER_HEAD, Sound::BlockNoteBlockImitateCreeper),
        (&Item::DRAGON_HEAD, Sound::BlockNoteBlockImitateEnderDragon),
        (
            &Item::WITHER_SKELETON_SKULL,
            Sound::BlockNoteBlockImitateWitherSkeleton,
        ),
        (&Item::PIGLIN_HEAD, Sound::BlockNoteBlockImitatePiglin),
    ] {
        fixture.prepare(&Block::STONE);
        fixture.place_head(item, None);
        assert_sound(
            &fixture.attack(),
            &EventSound::Registered(sound as u16),
            1.0,
        );
    }
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g08_custom_head_sound_and_silent_default() {
    let mut fixture = Fixture::new();
    fixture.prepare(&Block::STONE);
    fixture.place_head(&Item::PLAYER_HEAD, Some(CUSTOM_SOUND));
    assert_sound(
        &fixture.attack(),
        &EventSound::Named(CUSTOM_SOUND.to_owned()),
        1.0,
    );
    assert_sound(
        &fixture.power(),
        &EventSound::Named(CUSTOM_SOUND.to_owned()),
        1.0,
    );

    let cancel = Arc::new(CancelNote(AtomicBool::new(true)));
    fixture.server.plugin_manager.register::<NotePlayEvent, _>(
        cancel.clone(),
        EventPriority::Normal,
        true,
    );
    let cancelled = fixture.attack();
    assert!(
        sounds(&cancelled).is_empty(),
        "cancelled note must remain silent"
    );
    assert_eq!(block_events(&cancelled), 0);
    cancel.0.store(false, Ordering::Relaxed);

    fixture.prepare(&Block::STONE);
    fixture.place_head(&Item::PLAYER_HEAD, None);
    let silent = fixture.attack();
    assert!(
        sounds(&silent).is_empty(),
        "unconfigured custom head must not emit a button click"
    );
    assert_eq!(
        block_events(&silent),
        0,
        "missing custom sound must fail the synced event"
    );
    fixture.finish().await;
}

fn head_drop(fixture: &Fixture) -> ItemStack {
    // Ordinary block destruction runs vanilla's generated copy_components loot function.
    assert!(
        fixture
            .world
            .break_block(
                &NOTE.up(),
                Some(&fixture.player.player),
                BlockFlags::NOTIFY_ALL
            )
            .is_some()
    );
    let drops: Vec<_> = fixture
        .world
        .entities
        .load()
        .iter()
        .filter_map(|entity| {
            entity
                .get_item_entity()
                .map(|item| item.get_item_stack().lock().unwrap().clone())
        })
        .filter(|stack| stack.item == &Item::PLAYER_HEAD)
        .collect();
    assert_eq!(
        drops.len(),
        1,
        "configured skull must produce one ordinary head drop"
    );
    let dropped = drops.into_iter().next().unwrap();
    assert_eq!(
        dropped
            .get_data_component::<NoteBlockSoundImpl>()
            .expect("ordinary configured-head drop must retain its sound component")
            .sound,
        CUSTOM_SOUND
    );
    dropped
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g08_skull_sound_survives_real_reload() {
    let mut fixture = Fixture::new();
    fixture.prepare(&Block::STONE);
    fixture.place_head(&Item::PLAYER_HEAD, Some(CUSTOM_SOUND));
    let placed = fixture.world.get_block_entity(&NOTE.up()).unwrap();
    let skull = placed.as_any().downcast_ref::<SkullBlockEntity>().unwrap();
    let mut profile = NbtCompound::new();
    profile.put_string("name", "PreservedOwner".to_owned());
    let name = r#"{"text":"Preserved head"}"#;
    *skull.profile.lock().unwrap() = Some(profile.clone());
    *skull.custom_name.lock().unwrap() = Some(name.to_owned());

    // These produce literal invalid/absent optional fields in the ordinary disk save.
    let invalid_pos = BlockPos::new(10, 65, 8);
    let absent_pos = BlockPos::new(12, 65, 8);
    for position in [invalid_pos, absent_pos] {
        fixture.put(position, &Block::PLAYER_HEAD);
    }
    let invalid = fixture.world.get_block_entity(&invalid_pos).unwrap();
    *invalid
        .as_any()
        .downcast_ref::<SkullBlockEntity>()
        .unwrap()
        .note_block_sound
        .lock()
        .unwrap() = Some("INVALID sound:id".to_owned());
    fixture.reload().await;

    let reloaded = fixture.world.get_block_entity(&NOTE.up()).unwrap();
    assert!(
        !Arc::ptr_eq(&placed, &reloaded),
        "reload must create a fresh skull from disk"
    );
    let skull = reloaded
        .as_any()
        .downcast_ref::<SkullBlockEntity>()
        .unwrap();
    assert_eq!(*skull.profile.lock().unwrap(), Some(profile.clone()));
    assert_eq!(skull.custom_name.lock().unwrap().as_deref(), Some(name));
    assert_sound(
        &fixture.attack(),
        &EventSound::Named(CUSTOM_SOUND.to_owned()),
        1.0,
    );
    for position in [invalid_pos, absent_pos] {
        let loaded = fixture.world.get_block_entity(&position).unwrap();
        assert!(
            loaded
                .as_any()
                .downcast_ref::<SkullBlockEntity>()
                .unwrap()
                .note_block_sound
                .lock()
                .unwrap()
                .is_none(),
            "invalid and old-save absent sounds must load as absent"
        );
    }

    let dropped = head_drop(&fixture);
    // Collection must not alter unrelated block entity state.
    assert_eq!(*skull.profile.lock().unwrap(), Some(profile));
    assert_eq!(skull.custom_name.lock().unwrap().as_deref(), Some(name));
    fixture.use_stack(dropped);
    assert_sound(
        &fixture.attack(),
        &EventSound::Named(CUSTOM_SOUND.to_owned()),
        1.0,
    );

    // A fresh unconfigured replacement must not inherit the previous skull's sound.
    fixture.put(NOTE.up(), &Block::AIR);
    fixture.use_stack(ItemStack::new(1, &Item::PLAYER_HEAD));
    assert!(sounds(&fixture.attack()).is_empty());
    fixture.finish().await;
}
