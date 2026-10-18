use std::sync::{Arc, Weak};

use pumpkin_util::{
    math::position::BlockPos,
    sync::{LockClass, Mutex, SnapshotBoundary as SB, assert_not_in_snapshot},
};

use crate::world::World;

pub(super) enum BlockEntityMutationContextLock {}
impl LockClass for BlockEntityMutationContextLock {
    const NAME: &'static str = "block entity setter context";
    const LEVEL: u8 = 10;
}

/// Supplies BlockEntity.setLevel context so native setters share final-capture admission.
#[derive(Clone, Default)]
pub(super) struct BlockEntityMutationContext {
    binding: Arc<Mutex<Option<Binding>, BlockEntityMutationContextLock>>,
}

#[derive(Clone)]
struct Binding {
    world: Weak<World>,
    position: BlockPos,
}

impl BlockEntityMutationContext {
    pub(super) fn bind_world(&self, world: &Arc<World>, position: BlockPos) {
        *self
            .binding
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Binding {
            world: Arc::downgrade(world),
            position,
        });
    }

    pub(super) fn mutate(&self, mutation: impl FnOnce()) {
        assert_not_in_snapshot(SB::Mutation, "BlockEntityMutationContext::mutate");
        // Copy context before world admission: neither owner guard reaches the mutation.
        let binding = self
            .binding
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let Some(Binding { world, position }) = binding else {
            // Detached constructors and NBT decoding have no live world to retire against.
            mutation();
            return;
        };
        let Some(world) = world.upgrade() else {
            tracing::warn!("Block entity mutation rejected after its world closed at {position:?}");
            return;
        };
        let Some(_mutation) = world.block_entity_mutation_permit(position) else {
            tracing::warn!(
                "Block entity mutation rejected while chunk is unloading at {position:?}"
            );
            return;
        };
        mutation();
    }
}
