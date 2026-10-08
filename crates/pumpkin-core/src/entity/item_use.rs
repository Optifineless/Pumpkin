use pumpkin_data::item_stack::ItemStack;

/// Resolves the cooldown group shared by starting use and disabling a blocking item.
#[must_use]
pub fn cooldown_group(item: &ItemStack) -> &str {
    // ItemCooldowns.getCooldownGroup: items without USE_COOLDOWN use their own id.
    item.get_use_cooldown()
        .and_then(|cooldown| cooldown.cooldown_group.as_deref())
        .unwrap_or(item.item.registry_key)
}

/// Checks `ItemCooldowns` before any active-hand or equipment mutation.
pub fn item_use_allowed(item: &ItemStack, is_on_cooldown: impl FnOnce(&str) -> bool) -> bool {
    !is_on_cooldown(cooldown_group(item))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pumpkin_data::{data_component_impl::UseCooldownImpl, item::Item};
    #[test]
    fn cooldown_rejects_shield_use_without_a_use_cooldown_component() {
        let shield = ItemStack::new(1, &Item::SHIELD);
        assert!(shield.get_use_cooldown().is_none());
        assert!(!item_use_allowed(&shield, |group| group == "shield"));
        assert!(item_use_allowed(&shield, |_| false));
        let mut custom = shield;
        custom.set_data_component(UseCooldownImpl {
            seconds: 1.0,
            cooldown_group: Some("test:shared".into()),
        });
        assert!(!item_use_allowed(&custom, |group| group == "test:shared"));
    }
}
