use super::*;
use crate::plugin::block::block_drop_item::BlockDropItemEvent;
use crossbeam::channel::{Receiver, Sender, bounded};
use std::time::Duration;

const BOUND: Duration = Duration::from_secs(5);

struct PauseFirstDrop {
    entered: Sender<()>,
    resume: Receiver<()>,
    calls: Arc<AtomicUsize>,
}

impl EventHandler<BlockDropItemEvent> for PauseFirstDrop {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        _event: &'a mut BlockDropItemEvent,
    ) -> BoxFuture<'a, ()> {
        if self.calls.fetch_add(1, Relaxed) == 0 {
            self.entered.send(()).unwrap();
            self.resume.recv_timeout(BOUND).unwrap();
        }
        Box::pin(async {})
    }
}

fn hit_on_worker(arrow: Arc<dyn EntityBase>, position: BlockPos) -> Receiver<()> {
    let runtime = tokio::runtime::Handle::current();
    let (finished, wait_for_finish) = bounded(1);
    drop(std::thread::spawn(move || {
        let _runtime = runtime.enter();
        arrow.on_hit(impact(position));
        finished.send(()).unwrap();
    }));
    wait_for_finish
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_projectile_overlapping_hits_drop_once() {
    let fixture = Fixture::new();
    let position = crafted_content_pot(&fixture);
    let first = launch(&fixture, position, ItemStack::new(1, &Item::ARROW));
    let second = launch(&fixture, position, ItemStack::new(1, &Item::ARROW));
    let (entered, wait_for_entry) = bounded(1);
    let (resume, wait_for_resume) = bounded(1);
    let calls = Arc::new(AtomicUsize::new(0));
    fixture
        .server
        .plugin_manager
        .register::<BlockDropItemEvent, _>(
            Arc::new(PauseFirstDrop {
                entered,
                resume: wait_for_resume,
                calls: calls.clone(),
            }),
            EventPriority::Normal,
            true,
        );
    let first_finished = hit_on_worker(first, position);
    wait_for_entry.recv_timeout(BOUND).unwrap();
    // Deliver the already-detected second impact while the first drop event is still running.
    let second_finished = hit_on_worker(second, position);
    second_finished.recv_timeout(BOUND).unwrap();
    resume.send(()).unwrap();
    first_finished.recv_timeout(BOUND).unwrap();
    assert_shattered(&fixture, position);
    assert_eq!(calls.load(Relaxed), 1);
    fixture.shutdown().await;
}

struct ReenterDrop {
    arrow: Arc<dyn EntityBase>,
    calls: Arc<AtomicUsize>,
}

impl EventHandler<BlockDropItemEvent> for ReenterDrop {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut BlockDropItemEvent,
    ) -> BoxFuture<'a, ()> {
        if self.calls.fetch_add(1, Relaxed) == 0 {
            self.arrow.on_hit(impact(event.block_pos));
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_projectile_drop_callback_reentry_drops_once() {
    let fixture = Fixture::new();
    let position = crafted_content_pot(&fixture);
    let first = launch(&fixture, position, ItemStack::new(1, &Item::ARROW));
    let second = launch(&fixture, position, ItemStack::new(1, &Item::ARROW));
    let calls = Arc::new(AtomicUsize::new(0));
    fixture
        .server
        .plugin_manager
        .register::<BlockDropItemEvent, _>(
            Arc::new(ReenterDrop {
                arrow: second,
                calls: calls.clone(),
            }),
            EventPriority::Normal,
            true,
        );
    hit_on_worker(first, position).recv_timeout(BOUND).unwrap();
    assert_shattered(&fixture, position);
    assert_eq!(calls.load(Relaxed), 1);
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_projectile_already_cracked_pot_still_drops_once() {
    let fixture = Fixture::new();
    let position = crafted_content_pot(&fixture);
    let mut properties =
        DecoratedPotLikeProperties::from_state_id(fixture.world.get_block_state_id(&position));
    properties.cracked = true;
    fixture.world.set_block_state(
        &position,
        properties.to_state_id(&Block::DECORATED_POT),
        BlockFlags::UPDATE_KNOWN_SHAPE | BlockFlags::NOTIFY_LISTENERS,
    );
    shoot(&fixture, position, ItemStack::new(1, &Item::ARROW));
    assert_shattered(&fixture, position);
    fixture.shutdown().await;
}
