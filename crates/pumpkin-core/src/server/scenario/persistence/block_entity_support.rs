use crate::block::entities::{
    BlockEntity, chest::ChestBlockEntity, chiseled_bookshelf::ChiseledBookshelfBlockEntity,
};
use pumpkin_inventory::Inventory;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::position::BlockPos;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};

pub(super) const BLOCK_ENTITY_BARRIER_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(30);

pub(super) fn wait_for_std_barrier<T>(receiver: &mpsc::Receiver<T>, name: &str) -> T {
    receiver
        .recv_timeout(BLOCK_ENTITY_BARRIER_TIMEOUT)
        .unwrap_or_else(|error| {
            panic!("{name} barrier did not complete within 30 seconds: {error}")
        })
}

pub(super) fn wait_for_crossbeam_barrier<T>(
    receiver: &crossbeam::channel::Receiver<T>,
    name: &str,
) -> T {
    receiver
        .recv_timeout(BLOCK_ENTITY_BARRIER_TIMEOUT)
        .unwrap_or_else(|error| {
            panic!("{name} barrier did not complete within 30 seconds: {error}")
        })
}

pub(super) const CHEST: BlockPos = BlockPos::new(8, 64, 8);
pub(super) const BOOKSHELF: BlockPos = BlockPos::new(8, 64, 8);

pub(super) struct PausedChestCapture {
    pub(super) chest: Arc<ChestBlockEntity>,
    pub(super) captured: mpsc::Sender<()>,
    pub(super) resume: Mutex<Option<mpsc::Receiver<()>>>,
    pub(super) pause: AtomicBool,
}

impl BlockEntity for PausedChestCapture {
    fn container_lock(&self) -> Option<pumpkin_data::data_component_impl::LockImpl> {
        self.chest.container_lock()
    }

    fn write_nbt(&self, nbt: &mut NbtCompound) {
        self.chest.write_nbt(nbt);
        if self.pause.swap(false, Ordering::AcqRel) {
            let resume = self.resume.lock().unwrap().take();
            if let Some(resume) = resume {
                self.captured.send(()).unwrap();
                wait_for_std_barrier(&resume, "chest capture resume");
            }
        }
    }

    fn from_nbt(nbt: &NbtCompound, position: BlockPos) -> Self {
        Self {
            chest: Arc::new(ChestBlockEntity::from_nbt(nbt, position)),
            captured: mpsc::channel().0,
            resume: Mutex::new(None),
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
        self.chest.chunk_data_nbt()
    }

    fn get_inventory(self: Arc<Self>) -> Option<Arc<dyn Inventory>> {
        Some(self.chest.clone())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

pub(super) struct PausedBookshelfCapture {
    pub(super) bookshelf: Arc<ChiseledBookshelfBlockEntity>,
    pub(super) captured: crossbeam::channel::Sender<()>,
    pub(super) resume: crossbeam::channel::Receiver<()>,
    pub(super) pause: AtomicBool,
}

impl BlockEntity for PausedBookshelfCapture {
    fn write_nbt(&self, nbt: &mut NbtCompound) {
        self.bookshelf.write_nbt(nbt);
        if self.pause.swap(false, Ordering::Relaxed) {
            self.captured.send(()).unwrap();
            wait_for_crossbeam_barrier(&self.resume, "bookshelf capture resume");
        }
    }

    fn from_nbt(nbt: &NbtCompound, position: BlockPos) -> Self {
        let (captured, _capture_receiver) = crossbeam::channel::bounded(1);
        let (_resume_sender, resume) = crossbeam::channel::bounded(1);
        Self {
            bookshelf: Arc::new(ChiseledBookshelfBlockEntity::from_nbt(nbt, position)),
            captured,
            resume,
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

pub(super) struct GatedBookshelfCapture {
    pub(super) bookshelf: Arc<ChiseledBookshelfBlockEntity>,
    pub(super) capture_id: usize,
    pub(super) captured: crossbeam::channel::Sender<usize>,
    pub(super) resume: crossbeam::channel::Receiver<()>,
    pub(super) pause: AtomicBool,
}

impl BlockEntity for GatedBookshelfCapture {
    fn write_nbt(&self, nbt: &mut NbtCompound) {
        self.bookshelf.write_nbt(nbt);
        if self.pause.swap(false, Ordering::AcqRel) {
            self.captured.send(self.capture_id).unwrap();
            wait_for_crossbeam_barrier(&self.resume, "gated bookshelf capture resume");
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
