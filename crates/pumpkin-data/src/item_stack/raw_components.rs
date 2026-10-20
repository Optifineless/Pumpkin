use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};

use crate::{
    data_component::DataComponent,
    data_component_impl::{
        BeesImpl, BlockEntityDataImpl, BucketEntityDataImpl, ChargedProjectilesImpl,
        DataComponentImpl, EntityDataImpl,
    },
    item_stack::ItemStack,
};

// Bounded, lock-free diagnostics: saturated tables suppress further names, never items.
static WARNED_NAMES: [AtomicU64; 1024] = [const { AtomicU64::new(0) }; 1024];
static WARNINGS_FULL: AtomicBool = AtomicBool::new(false);

pub(crate) fn warn_preserved_component(name: &str) {
    if WARNINGS_FULL.load(Ordering::Relaxed) {
        return;
    }
    let mut hasher = DefaultHasher::new();
    name.strip_prefix('!').unwrap_or(name).hash(&mut hasher);
    let fingerprint = hasher.finish().max(1);
    for offset in 0..WARNED_NAMES.len() {
        let index = (fingerprint as usize).wrapping_add(offset) % WARNED_NAMES.len();
        match WARNED_NAMES[index].compare_exchange(
            0,
            fingerprint,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => {
                tracing::warn!(
                    component = name,
                    "Preserving undecodable item component as raw NBT"
                );
                return;
            }
            Err(previous) if previous == fingerprint => return,
            Err(_) => {}
        }
    }
    WARNINGS_FULL.store(true, Ordering::Relaxed);
}

pub(crate) fn raw_components_equal(
    left: &[(Box<str>, NbtTag)],
    right: &[(Box<str>, NbtTag)],
) -> bool {
    left.len() == right.len()
        && left.iter().all(|(name, value)| {
            right.iter().any(|(other_name, other_value)| {
                name == other_name && tags_equal(value, other_value)
            })
        })
}

pub(crate) fn embedded_raw_components_equal(
    id: DataComponent,
    left: &dyn DataComponentImpl,
    right: &dyn DataComponentImpl,
) -> bool {
    // Generic entity payloads keep nested items as NBT, outside typed container equality.
    if !matches!(
        id,
        DataComponent::BlockEntityData
            | DataComponent::EntityData
            | DataComponent::BucketEntityData
            | DataComponent::Bees
            | DataComponent::ChargedProjectiles
    ) {
        return true;
    }
    let (left, right) = (left.as_any(), right.as_any());
    let pair = match id {
        DataComponent::BlockEntityData => left
            .downcast_ref::<BlockEntityDataImpl>()
            .zip(right.downcast_ref::<BlockEntityDataImpl>())
            .map(|(a, b)| (&a.nbt, &b.nbt)),
        DataComponent::EntityData => left
            .downcast_ref::<EntityDataImpl>()
            .zip(right.downcast_ref::<EntityDataImpl>())
            .and_then(|(a, b)| a.nbt.as_ref().zip(b.nbt.as_ref())),
        DataComponent::BucketEntityData => left
            .downcast_ref::<BucketEntityDataImpl>()
            .zip(right.downcast_ref::<BucketEntityDataImpl>())
            .and_then(|(a, b)| a.nbt.as_ref().zip(b.nbt.as_ref())),
        DataComponent::Bees => {
            return left
                .downcast_ref::<BeesImpl>()
                .zip(right.downcast_ref::<BeesImpl>())
                .is_none_or(|(a, b)| {
                    a.bees
                        .iter()
                        .zip(&b.bees)
                        .all(|(a, b)| embedded_compounds_equal(&a.entity_data, &b.entity_data))
                });
        }
        DataComponent::ChargedProjectiles => {
            return left
                .downcast_ref::<ChargedProjectilesImpl>()
                .zip(right.downcast_ref::<ChargedProjectilesImpl>())
                .is_none_or(|(a, b)| {
                    a.projectiles
                        .iter()
                        .zip(&b.projectiles)
                        .all(|(a, b)| embedded_compounds_equal(a, b))
                });
        }
        _ => return true,
    };
    pair.is_none_or(|(left, right)| embedded_compounds_equal(left, right))
}

fn embedded_compounds_equal(left: &NbtCompound, right: &NbtCompound) -> bool {
    // Decoded equality already checked shapes and ordinary scalar values.
    if left.has("components")
        && let (Some(left), Some(right)) = (
            ItemStack::read_item_stack(left),
            ItemStack::read_item_stack(right),
        )
        && !raw_components_equal(&left.raw_components, &right.raw_components)
    {
        return false;
    }
    left.child_tags.iter().all(|(name, left)| {
        right
            .get(name)
            .is_none_or(|right| embedded_tags_equal(left, right))
    })
}

fn embedded_tags_equal(left: &NbtTag, right: &NbtTag) -> bool {
    match (left, right) {
        (NbtTag::Compound(left), NbtTag::Compound(right)) => embedded_compounds_equal(left, right),
        (NbtTag::List(left), NbtTag::List(right)) => left
            .iter()
            .zip(right)
            .all(|(a, b)| embedded_tags_equal(a, b)),
        _ => true,
    }
}

fn tags_equal(left: &NbtTag, right: &NbtTag) -> bool {
    match (left, right) {
        (NbtTag::Float(left), NbtTag::Float(right)) => left.to_bits() == right.to_bits(),
        (NbtTag::Double(left), NbtTag::Double(right)) => left.to_bits() == right.to_bits(),
        (NbtTag::List(left), NbtTag::List(right)) => {
            left.len() == right.len() && left.iter().zip(right).all(|(a, b)| tags_equal(a, b))
        }
        (NbtTag::Compound(left), NbtTag::Compound(right)) => {
            left.child_tags.len() == right.child_tags.len()
                && left.child_tags.iter().all(|(name, value)| {
                    right
                        .get(name)
                        .is_some_and(|other| tags_equal(value, other))
                })
        }
        _ => left == right,
    }
}
