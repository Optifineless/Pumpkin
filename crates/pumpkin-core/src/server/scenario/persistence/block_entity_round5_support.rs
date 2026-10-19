use super::block_entity_support::wait_for_crossbeam_barrier;
use crate::block::entities::{
    BlockEntity, chest::ChestBlockEntity, chiseled_bookshelf::ChiseledBookshelfBlockEntity,
    sign::SignBlockEntity,
};
use pumpkin_inventory::Inventory;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::position::BlockPos;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc,
};

pub(super) async fn wait_for_crossbeam_capture<T: Send + 'static>(
    receiver: &crossbeam::channel::Receiver<T>,
    name: &'static str,
) -> T {
    let receiver = receiver.clone();
    tokio::task::spawn_blocking(move || wait_for_crossbeam_barrier(&receiver, name))
        .await
        .unwrap()
}

pub(super) struct PausedRegistrationChest {
    pub(super) chest: Arc<ChestBlockEntity>,
    pub(super) captured: mpsc::Sender<()>,
    pub(super) resume: crossbeam::channel::Receiver<()>,
    pub(super) pause: AtomicBool,
}

impl BlockEntity for PausedRegistrationChest {
    fn container_lock(&self) -> Option<pumpkin_data::data_component_impl::LockImpl> {
        self.chest.container_lock()
    }

    fn write_nbt(&self, nbt: &mut NbtCompound) {
        self.chest.write_nbt(nbt);
    }

    fn from_nbt(nbt: &NbtCompound, position: BlockPos) -> Self {
        Self {
            chest: Arc::new(ChestBlockEntity::from_nbt(nbt, position)),
            captured: mpsc::channel().0,
            resume: crossbeam::channel::bounded(1).1,
            pause: AtomicBool::new(false),
        }
    }

    fn resource_location(&self) -> &'static str {
        ChestBlockEntity::ID
    }

    fn get_position(&self) -> BlockPos {
        self.chest.get_position()
    }

    fn chunk_data_nbt(&self) -> Option<NbtCompound> {
        let nbt = self.chest.chunk_data_nbt();
        if self.pause.swap(false, Ordering::AcqRel) {
            self.captured.send(()).unwrap();
            wait_for_crossbeam_barrier(&self.resume, "replacement registration resume");
        }
        nbt
    }

    fn get_inventory(self: Arc<Self>) -> Option<Arc<dyn Inventory>> {
        Some(self.chest.clone())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

pub(super) struct GatedSignCapture {
    pub(super) sign: Arc<SignBlockEntity>,
    pub(super) capture_id: usize,
    pub(super) captured: crossbeam::channel::Sender<usize>,
    pub(super) resume: crossbeam::channel::Receiver<()>,
    pub(super) pause: AtomicBool,
}

impl BlockEntity for GatedSignCapture {
    fn write_nbt(&self, nbt: &mut NbtCompound) {
        self.sign.write_nbt(nbt);
        if self.pause.swap(false, Ordering::AcqRel) {
            self.captured.send(self.capture_id).unwrap();
            wait_for_crossbeam_barrier(&self.resume, "sign capture resume");
        }
    }

    fn from_nbt(nbt: &NbtCompound, position: BlockPos) -> Self {
        Self {
            sign: Arc::new(SignBlockEntity::from_nbt(nbt, position)),
            capture_id: 0,
            captured: crossbeam::channel::bounded(1).0,
            resume: crossbeam::channel::bounded(1).1,
            pause: AtomicBool::new(false),
        }
    }

    fn resource_location(&self) -> &'static str {
        SignBlockEntity::ID
    }

    fn get_position(&self) -> BlockPos {
        self.sign.get_position()
    }

    fn chunk_data_nbt(&self) -> Option<NbtCompound> {
        self.sign.chunk_data_nbt()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self.sign.as_ref()
    }
}

pub(super) struct GatedBookshelfCapture {
    pub(super) bookshelf: Arc<ChiseledBookshelfBlockEntity>,
    pub(super) capture_id: usize,
    pub(super) captured: crossbeam::channel::Sender<usize>,
    pub(super) resume: crossbeam::channel::Receiver<()>,
    pub(super) pause: AtomicBool,
}

impl GatedBookshelfCapture {
    pub(super) fn new(
        bookshelf: Arc<ChiseledBookshelfBlockEntity>,
        capture_id: usize,
        captured: crossbeam::channel::Sender<usize>,
        resume: crossbeam::channel::Receiver<()>,
    ) -> Self {
        Self {
            bookshelf,
            capture_id,
            captured,
            resume,
            pause: AtomicBool::new(false),
        }
    }
}

impl BlockEntity for GatedBookshelfCapture {
    fn write_nbt(&self, nbt: &mut NbtCompound) {
        self.bookshelf.write_nbt(nbt);
        if self.pause.swap(false, Ordering::AcqRel) {
            self.captured.send(self.capture_id).unwrap();
            wait_for_crossbeam_barrier(&self.resume, "save replacement capture resume");
        }
    }

    fn from_nbt(nbt: &NbtCompound, position: BlockPos) -> Self {
        Self {
            bookshelf: Arc::new(ChiseledBookshelfBlockEntity::from_nbt(nbt, position)),
            capture_id: 0,
            captured: crossbeam::channel::bounded(1).0,
            resume: crossbeam::channel::bounded(1).1,
            pause: AtomicBool::new(false),
        }
    }

    fn resource_location(&self) -> &'static str {
        ChiseledBookshelfBlockEntity::ID
    }

    fn get_position(&self) -> BlockPos {
        self.bookshelf.get_position()
    }

    fn get_inventory(self: Arc<Self>) -> Option<Arc<dyn Inventory>> {
        Some(self.bookshelf.clone())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

pub(super) struct PredecessorGateChest {
    pub(super) chest: Arc<ChestBlockEntity>,
    pub(super) calls: AtomicUsize,
    pub(super) enabled: AtomicBool,
    pub(super) observed: crossbeam::channel::Sender<usize>,
    pub(super) resume: [crossbeam::channel::Receiver<()>; 2],
}

impl BlockEntity for PredecessorGateChest {
    fn write_nbt(&self, nbt: &mut NbtCompound) {
        self.chest.write_nbt(nbt);
    }

    fn from_nbt(nbt: &NbtCompound, position: BlockPos) -> Self {
        let (observed, _receiver) = crossbeam::channel::bounded(2);
        let (_sender_a, receiver_a) = crossbeam::channel::bounded(1);
        let (_sender_b, receiver_b) = crossbeam::channel::bounded(1);
        Self {
            chest: Arc::new(ChestBlockEntity::from_nbt(nbt, position)),
            calls: AtomicUsize::new(0),
            enabled: AtomicBool::new(false),
            observed,
            resume: [receiver_a, receiver_b],
        }
    }

    fn container_lock(&self) -> Option<pumpkin_data::data_component_impl::LockImpl> {
        self.chest.container_lock()
    }

    fn resource_location(&self) -> &'static str {
        ChestBlockEntity::ID
    }

    fn get_position(&self) -> BlockPos {
        self.chest.get_position()
    }

    fn chunk_data_nbt(&self) -> Option<NbtCompound> {
        self.chest.chunk_data_nbt()
    }

    fn get_inventory(self: Arc<Self>) -> Option<Arc<dyn Inventory>> {
        let call = self.calls.fetch_add(1, Ordering::AcqRel);
        if self.enabled.load(Ordering::Acquire) && call < self.resume.len() {
            self.observed.send(call).unwrap();
            wait_for_crossbeam_barrier(&self.resume[call], "replacement predecessor read");
        }
        Some(self.chest.clone())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
