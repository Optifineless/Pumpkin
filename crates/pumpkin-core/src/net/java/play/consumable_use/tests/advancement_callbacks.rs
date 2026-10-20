use super::*;
use crate::plugin::api::events::player::player_advancement_done::PlayerAdvancementDoneEvent;
use pumpkin_data::{Advancement, data_component_impl::JukeboxPlayableImpl};
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};

pub(super) fn jukebox_scenario(hand: Hand) -> Scenario {
    let mut scenario = Scenario::new(&Item::CARROT, hand, &Block::JUKEBOX);
    let mut chunk = spawn_test_support::proto(&Biome::MEADOW, &Block::STONE);
    chunk.set_block_state(8, 64, 8, Block::JUKEBOX.default_state);
    spawn_test_support::publish(&scenario.world, chunk);
    let playable = ItemStack::new(1, &Item::MUSIC_DISC_CAT)
        .get_data_component::<JukeboxPlayableImpl>()
        .unwrap()
        .clone();
    scenario.original.set_data_component(playable);
    scenario
        .fixture
        .player
        .inventory()
        .set_stack(scenario.source, scenario.original.clone());
    scenario.fixture.player.get_entity().set_sneaking(true);
    scenario
}

#[derive(Clone, Copy)]
enum AdvancementEdit {
    Replace,
    Select,
    StartNew,
}
struct AdvancementCallback {
    edit: AdvancementEdit,
    hand: Hand,
    source: usize,
    calls: AtomicUsize,
}
impl EventHandler<PlayerAdvancementDoneEvent> for AdvancementCallback {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerAdvancementDoneEvent,
    ) -> BoxFuture<'a, ()> {
        if event.advancement_id
            != Advancement::ADVENTURE_PLAY_JUKEBOX_IN_MEADOWS
                .id
                .to_string()
        {
            return Box::pin(async {});
        }
        self.calls.fetch_add(1, SeqCst);
        let inventory = event.player.inventory();
        match self.edit {
            AdvancementEdit::Replace => {
                let mut replacement = inventory.get_stack(self.source);
                replacement.uid = ItemStack::new(2, &Item::CARROT).uid;
                inventory.set_stack(self.source, replacement);
            }
            AdvancementEdit::Select => {
                inventory.set_stack(1, ItemStack::new(2, &Item::CARROT));
                inventory.set_selected_slot(1);
            }
            AdvancementEdit::StartNew => {
                event.player.living_entity.clear_active_hand();
                let bow = ItemStack::new(1, &Item::BOW);
                inventory.set_stack_in_hand(self.hand, bow.clone());
                event
                    .player
                    .living_entity
                    .set_active_hand(self.hand, bow, 72_000);
            }
        }
        Box::pin(async {})
    }
}
async fn advancement_callback(edit: AdvancementEdit) {
    for hand in [Hand::Right, Hand::Left] {
        let scenario = jukebox_scenario(hand);
        let callback = Arc::new(AdvancementCallback {
            edit,
            hand,
            source: scenario.source,
            calls: AtomicUsize::new(0),
        });
        scenario
            .server
            .plugin_manager
            .register::<PlayerAdvancementDoneEvent, _>(
                callback.clone(),
                EventPriority::Normal,
                true,
            );
        scenario.use_block();
        assert_eq!(
            callback.calls.load(SeqCst),
            1,
            "real block-use criterion did not complete"
        );
        let continues = matches!(edit, AdvancementEdit::Select) && hand == Hand::Left;
        let newer = matches!(edit, AdvancementEdit::StartNew);
        assert_eq!(scenario.active(), (continues || newer).then_some(hand));
        scenario.tick(64);
        let live = scenario
            .fixture
            .player
            .inventory()
            .get_stack(scenario.source);
        if newer {
            assert_eq!(live.item, &Item::BOW);
            assert_eq!(
                scenario.active(),
                Some(hand),
                "cleanup stopped callback's newer use"
            );
        } else if matches!(edit, AdvancementEdit::Replace) {
            assert_ne!(live.uid, scenario.original.uid);
            assert!(
                live.are_equal(&scenario.original),
                "callback replacement was consumed"
            );
        } else {
            scenario.assert_stock_and_food(
                if continues { 1 } else { 2 },
                if continues { 13 } else { 10 },
            );
            assert_eq!(
                scenario.fixture.player.inventory().get_stack(1).item_count,
                2
            );
        }
        assert_eq!(
            scenario.fixture.player.hunger_manager.level.load(),
            if continues { 13 } else { 10 }
        );
        scenario.finish().await;
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g18_advancement_callback_replacement_is_preserved() {
    advancement_callback(AdvancementEdit::Replace).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g18_advancement_callback_selection_is_preserved() {
    advancement_callback(AdvancementEdit::Select).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g18_advancement_callback_newer_use_is_preserved() {
    advancement_callback(AdvancementEdit::StartNew).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g18_advancement_replacement_during_existing_same_stack_use_is_preserved() {
    let scenario = jukebox_scenario(Hand::Right);
    let callback = Arc::new(AdvancementCallback {
        edit: AdvancementEdit::Replace,
        hand: scenario.hand,
        source: scenario.source,
        calls: AtomicUsize::new(0),
    });
    scenario
        .server
        .plugin_manager
        .register::<PlayerAdvancementDoneEvent, _>(callback.clone(), EventPriority::Normal, true);
    scenario.fixture.client().handle_use_item(
        &scenario.fixture.player,
        &SUseItem {
            hand: VarInt(0),
            sequence: VarInt(16),
            yaw: 0.0,
            pitch: -90.0,
        },
        &scenario.server,
    );
    assert_eq!(scenario.fixture.client().packet_sequence.load(Relaxed), 16);
    assert_eq!(scenario.active(), Some(Hand::Right));
    scenario.tick(5);
    scenario.use_block();
    assert_eq!(callback.calls.load(SeqCst), 1);
    assert_eq!(scenario.active(), None);
    scenario.tick(64);
    let current = scenario
        .fixture
        .player
        .inventory()
        .get_stack(scenario.source);
    assert_ne!(current.uid, scenario.original.uid);
    assert!(current.are_equal(&scenario.original));
    assert_eq!(scenario.fixture.player.hunger_manager.level.load(), 10);
    scenario.finish().await;
}
