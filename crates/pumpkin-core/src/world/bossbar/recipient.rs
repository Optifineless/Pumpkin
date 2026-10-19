use crate::{entity::player::Player, world::World};
use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, Ordering},
};

// ServerBossEvent owns ServerPlayer recipients, not reusable account UUIDs.
// Each allocation is one subscription; weak handles cannot keep a closed world alive.
pub struct BossbarRecipient {
    player: Weak<Player>,
    published: AtomicBool,
}

impl BossbarRecipient {
    pub fn new(player: &Arc<Player>) -> Arc<Self> {
        Arc::new(Self {
            player: Arc::downgrade(player),
            published: AtomicBool::new(false),
        })
    }

    pub fn matches(&self, player: &Arc<Player>) -> bool {
        self.player.ptr_eq(&Arc::downgrade(player))
    }

    pub fn player(&self) -> Option<Arc<Player>> {
        self.player
            .upgrade()
            .filter(|player| !player.client.closed())
    }

    pub fn publish(&self) -> bool {
        !self.published.swap(true, Ordering::AcqRel)
    }

    pub fn is_published(&self) -> bool {
        self.published.load(Ordering::Acquire)
    }

    pub fn retire(&self) -> bool {
        self.published.swap(false, Ordering::AcqRel)
    }

    pub fn is_current(player: &Arc<Player>, world: &Arc<World>) -> bool {
        !player.client.closed()
            && Arc::ptr_eq(&player.world(), world)
            && world.players.load().iter().any(|p| Arc::ptr_eq(p, player))
    }
}
