use super::{BeaconBlockEntity, BeaconMenuMetadata};
use pumpkin_data::{
    data_component_impl::{CustomNameImpl, DataComponentImpl, LockImpl},
    item_stack::ItemStack,
};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_util::text::TextComponent;
use std::sync::{Mutex, MutexGuard};

type LoadedName = (Option<String>, Option<(String, CustomNameImpl)>);

pub(super) fn read_custom_name(nbt: &NbtCompound) -> LoadedName {
    let Some(tag) = nbt.get("CustomName").or_else(|| nbt.get("custom_name")) else {
        return (None, None);
    };
    let Some(name) = CustomNameImpl::read_data(tag) else {
        return (None, None);
    };
    let legacy = match tag {
        NbtTag::String(value) => value.to_string(),
        _ => serde_json::to_string(&name.name).unwrap_or_else(|_| name.name.clone().get_text()),
    };
    (Some(legacy.clone()), Some((legacy, name)))
}

fn metadata_guard<T>(value: &Mutex<T>, packet_only: bool) -> Option<MutexGuard<'_, T>> {
    if packet_only {
        value.try_lock().ok()
    } else {
        Some(
            value
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }
}

fn canonical_guard<T>(
    value: &pumpkin_util::sync::Mutex<T, BeaconMenuMetadata>,
    packet_only: bool,
) -> Option<pumpkin_util::sync::MutexGuard<'_, T>> {
    if packet_only {
        value.try_lock().ok()
    } else {
        Some(
            value
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }
}

fn legacy_lock(name: String) -> Option<LockImpl> {
    if name.is_empty() {
        return None;
    }
    let mut components = NbtCompound::new();
    components.put_string("minecraft:custom_name", name);
    let mut predicate = NbtCompound::new();
    predicate.put_compound("components", components);
    Some(LockImpl { predicate })
}

impl BeaconBlockEntity {
    fn menu_name(&self, packet_only: bool) -> Option<CustomNameImpl> {
        let legacy_guard = metadata_guard(&self.custom_name, packet_only)?;
        let mut canonical = canonical_guard(&self.canonical_name, packet_only)?;
        let name = if let Some((source, name)) = canonical.as_ref()
            && legacy_guard.as_ref() == Some(source)
        {
            Some(name.clone())
        } else {
            *canonical = None;
            None
        };
        let legacy = legacy_guard.clone();
        drop(canonical);
        drop(legacy_guard);
        // Native plugins can still replace the legacy public field with a plain name.
        name.or_else(|| {
            legacy.map(|legacy| CustomNameImpl {
                name: TextComponent::text(legacy),
            })
        })
    }

    pub(crate) fn menu_display_name(&self) -> TextComponent {
        // BeaconBlockEntity.getDisplayName -> getName retains the complete component.
        self.menu_name(false).map_or_else(
            || {
                pumpkin_macros::translate_cross!(
                    pumpkin_data::translation::java::CONTAINER_BEACON,
                    pumpkin_data::translation::bedrock::CONTAINER_BEACON
                )
            },
            |name| name.name,
        )
    }

    pub(super) fn menu_lock(&self, packet_only: bool) -> Option<LockImpl> {
        self.menu_lock_snapshot(packet_only)?.1
    }

    fn menu_lock_snapshot(&self, packet_only: bool) -> Option<(Option<String>, Option<LockImpl>)> {
        let legacy_guard = metadata_guard(&self.lock_key, packet_only)?;
        let mut canonical = canonical_guard(&self.canonical_lock, packet_only)?;
        // A native write to the legacy field supersedes the loaded canonical predicate.
        let lock = if let Some((source, lock)) = canonical.as_ref()
            && source == &*legacy_guard
        {
            Some(lock.clone())
        } else {
            *canonical = None;
            None
        };
        let legacy = legacy_guard.clone();
        drop(canonical);
        drop(legacy_guard);
        let lock = lock.or_else(|| legacy.clone().and_then(legacy_lock));
        Some((legacy, lock))
    }

    pub(super) fn write_menu_nbt(&self, nbt: &mut NbtCompound, packet_only: bool) {
        // BeaconBlockEntity.saveAdditional uses the component codec and LockCode.addToTag.
        if let Some(name) = self.menu_name(packet_only) {
            nbt.put("CustomName", name.write_data());
        }
        if let Some((legacy, lock)) = self.menu_lock_snapshot(packet_only) {
            if let Some(lock) = lock {
                nbt.put("lock", lock.write_data());
            }
            if let Some(legacy) = legacy {
                nbt.put_string("Lock", legacy);
            }
        }
    }

    pub(super) fn collect_menu_components(&self, stack: &mut ItemStack) {
        if let Some(name) = self.menu_name(false) {
            stack.set_data_component(name);
        }
        if let Some(lock) = self.menu_lock(false) {
            stack.set_data_component(lock);
        }
    }

    pub(super) fn apply_menu_components(&self, stack: &ItemStack) {
        // BeaconBlockEntity.applyImplicitComponents replaces the name and lock on placement.
        let name = stack.get_data_component::<CustomNameImpl>().cloned();
        let legacy = name.as_ref().map(|name| match name.write_data() {
            NbtTag::String(value) => value.to_string(),
            _ => serde_json::to_string(&name.name).unwrap_or_else(|_| name.name.clone().get_text()),
        });
        let canonical_name = legacy.clone().zip(name);
        let canonical_lock = stack
            .get_data_component::<LockImpl>()
            .cloned()
            .map(|lock| (None, lock));
        let mut current_name = self
            .custom_name
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut current_canonical_name = self
            .canonical_name
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *current_name = legacy;
        *current_canonical_name = canonical_name;
        drop(current_canonical_name);
        drop(current_name);

        let mut current_key = self
            .lock_key
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut current_canonical_lock = self
            .canonical_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *current_key = None;
        *current_canonical_lock = canonical_lock;
    }
}
