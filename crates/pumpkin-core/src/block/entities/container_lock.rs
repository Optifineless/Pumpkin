use super::BlockEntity;
use crate::{entity::player::Player, world::World};
use pumpkin_data::{
    data_component_impl::{CustomNameImpl, DataComponentImpl, LockImpl},
    item_stack::ItemStack,
};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_util::{GameMode, text::TextComponent};
use std::sync::Mutex;

#[derive(Default)]
pub struct ContainerLock(Mutex<Option<LockImpl>>);
impl ContainerLock {
    pub fn from_nbt(nbt: &NbtCompound) -> Self {
        // BaseContainerBlockEntity.loadAdditional -> LockCode.fromTag uses lowercase "lock".
        Self(Mutex::new(nbt.get("lock").and_then(LockImpl::read_data)))
    }
    pub fn get(&self) -> Option<LockImpl> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
    pub fn write_nbt(&self, nbt: &mut NbtCompound) {
        if let Some(lock) = self.get() {
            nbt.put("lock", lock.write_data());
        }
    }
    pub fn apply(&self, stack: &ItemStack) {
        // BaseContainerBlockEntity.applyImplicitComponents retains the item's lock.
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            stack.get_data_component::<LockImpl>().cloned();
    }
    pub fn collect(&self, stack: &mut ItemStack) {
        if let Some(lock) = self.get() {
            stack.set_data_component(lock);
        }
    }
}

/// Checks locks before loot generation or menu creation; spectators cannot unpack loot.
pub fn can_open(
    entity: &dyn BlockEntity,
    player: &Player,
    world: &World,
    name: &'static str,
) -> bool {
    // RandomizableContainerBlockEntity.canOpen and LockCode.canUnlock.
    let spectator = player.gamemode.load() == GameMode::Spectator;
    if spectator {
        return !entity.has_loot_table();
    }
    let allowed = entity
        .container_lock()
        .is_none_or(|lock| unlocks_with(&lock, &player.inventory.held_item()));
    if !allowed {
        // BaseContainerBlockEntity.sendChestLockedNotifications.
        player.send_system_message_raw(
            &TextComponent::translate("container.isLocked", [TextComponent::translate(name, [])]),
            true,
        );
        world.play_sound(
            pumpkin_data::sound::Sound::BlockChestLocked,
            pumpkin_data::sound::SoundCategory::Blocks,
            &entity.get_position().to_centered_f64(),
        );
    }
    allowed
}

fn unlocks_with(lock: &LockImpl, stack: &ItemStack) -> bool {
    let mut predicate = lock.predicate.clone();
    if let Some(NbtTag::Compound(components)) = predicate.child_tags.get_mut("components") {
        for name in ["minecraft:custom_name", "custom_name"] {
            if let Some(expected) = components.child_tags.remove(name) {
                // DataComponentExactPredicate compares the complete text, including its style.
                let expected = CustomNameImpl::read_data(&expected);
                if stack.is_empty()
                    || stack
                        .get_data_component::<CustomNameImpl>()
                        .is_none_or(|actual| expected.as_ref() != Some(actual))
                {
                    return false;
                }
            }
        }
    }
    crate::world::loot::matches_container_lock(&nbt_json(&NbtTag::Compound(predicate)), stack)
}

fn nbt_json(tag: &NbtTag) -> serde_json::Value {
    use serde_json::Value;
    match tag {
        NbtTag::End => Value::Null,
        NbtTag::Byte(v) => (*v).into(),
        NbtTag::Short(v) => (*v).into(),
        NbtTag::Int(v) => (*v).into(),
        NbtTag::Long(v) => (*v).into(),
        NbtTag::Float(v) => (*v).into(),
        NbtTag::Double(v) => (*v).into(),
        NbtTag::String(v) => v.to_string().into(),
        NbtTag::List(v) => Value::Array(v.iter().map(nbt_json).collect()),
        NbtTag::Compound(v) => Value::Object(
            v.child_tags
                .iter()
                .map(|(k, v)| (k.to_string(), nbt_json(v)))
                .collect(),
        ),
        NbtTag::ByteArray(v) => Value::Array(v.iter().map(|v| Value::from(*v)).collect()),
        NbtTag::IntArray(v) => Value::Array(v.iter().map(|v| Value::from(*v)).collect()),
        NbtTag::LongArray(v) => Value::Array(v.iter().map(|v| Value::from(*v)).collect()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pumpkin_data::item::Item;

    #[test]
    fn lock_component_equality_normalizes_rgb_and_retains_fallback_style_children() {
        fn name(color: &str, fallback: &str, bold: bool, child: &str) -> NbtTag {
            let mut name = NbtCompound::new();
            name.put_string("translate", "example".into());
            name.put_string("fallback", fallback.into());
            name.put_string("color", color.into());
            name.put_bool("bold", bold);
            name.put_list("extra", vec![NbtTag::String(child.into())]);
            NbtTag::Compound(name)
        }
        let mut components = NbtCompound::new();
        components.put("minecraft:custom_name", name("red", "Vault", true, "!"));
        let mut predicate = NbtCompound::new();
        predicate.put_compound("components", components);
        let lock = LockImpl { predicate };
        for (color, fallback, bold, child, expected) in [
            ("#FF5555", "Vault", true, "!", true),
            ("red", "Different", true, "!", false),
            ("red", "Vault", false, "!", false),
            ("red", "Vault", true, "?", false),
        ] {
            let mut key = ItemStack::new(1, &Item::TRIPWIRE_HOOK);
            key.set_data_component(
                CustomNameImpl::read_data(&name(color, fallback, bold, child)).unwrap(),
            );
            assert_eq!(unlocks_with(&lock, &key), expected);
        }
    }

    #[test]
    fn deep_review_key_and_lock_survive_nbt_and_require_exact_style() {
        let mut components = NbtCompound::new();
        components.put_string("minecraft:custom_name", "Vault key".to_owned());
        let mut predicate = NbtCompound::new();
        predicate.put_compound("components", components);
        let mut nbt = NbtCompound::new();
        nbt.put_compound("lock", predicate);
        let lock = ContainerLock::from_nbt(&nbt);
        let mut saved = NbtCompound::new();
        lock.write_nbt(&mut saved);
        assert_eq!(saved, nbt);
        let lock = ContainerLock::from_nbt(&saved).get().unwrap();
        let mut key = ItemStack::new(1, &Item::TRIPWIRE_HOOK);
        assert!(!unlocks_with(&lock, &key));
        key.set_data_component(CustomNameImpl {
            name: TextComponent::text("Vault key extra"),
        });
        assert!(!unlocks_with(&lock, &key));
        key.set_data_component(CustomNameImpl {
            name: TextComponent::text("Vault key"),
        });
        assert!(unlocks_with(&lock, &key));
        key.set_data_component(CustomNameImpl {
            name: TextComponent::text("Vault key").bold(),
        });
        assert!(!unlocks_with(&lock, &key));
        let mut styled = NbtCompound::new();
        styled.put_string("text", "Vault key".to_owned());
        styled.put_bool("bold", true);
        let mut components = NbtCompound::new();
        components.put_compound("minecraft:custom_name", styled);
        let mut predicate = NbtCompound::new();
        predicate.put_compound("components", components);
        let mut locked_container = ItemStack::new(1, &Item::CHEST);
        locked_container.set_data_component(LockImpl { predicate });
        let mut saved_container = NbtCompound::new();
        locked_container.write_item_stack(&mut saved_container);
        let restored_container = ItemStack::read_item_stack(&saved_container).unwrap();
        let restored_lock = restored_container.get_data_component::<LockImpl>().unwrap();
        let mut saved_key = NbtCompound::new();
        key.write_item_stack(&mut saved_key);
        let mut restored_key = ItemStack::read_item_stack(&saved_key).unwrap();
        assert!(unlocks_with(restored_lock, &restored_key));
        restored_key.set_data_component(CustomNameImpl {
            name: TextComponent::text("Vault key").italic(),
        });
        assert!(!unlocks_with(restored_lock, &restored_key));
    }
}
