use pumpkin_data::data_component_impl::{
    BundleContentsImpl, ChargedProjectilesImpl, ContainerImpl, DamageImpl,
};
use pumpkin_data::{item::Item, item_stack::ItemStack};

use crate::ser::ReadingError;

/// Applies ItemStack.validatedStreamCodec's persistent count and component bounds.
pub(super) fn validate_persistent(stack: &ItemStack) -> Result<(), ReadingError> {
    if stack.is_empty() {
        return Ok(());
    }
    let invalid = || ReadingError::Message("Invalid incoming item stack".into());
    let mut pending = vec![stack];
    while let Some(stack) = pending.pop() {
        let max = stack.get_max_stack_size();
        if !(1..=Item::ABSOLUTE_MAX_STACK_SIZE).contains(&stack.item_count)
            || !(1..=Item::ABSOLUTE_MAX_STACK_SIZE).contains(&max)
            || stack.get_max_damage().is_some_and(|damage| damage <= 0)
            || stack
                .get_data_component::<DamageImpl>()
                .is_some_and(|damage| damage.damage < 0)
        {
            return Err(invalid());
        }
        if let Some(container) = stack.get_data_component::<ContainerImpl>() {
            pending.extend(
                container
                    .items
                    .iter()
                    .map(|(_, item)| item)
                    .filter(|item| !item.is_empty()),
            );
        }
        if let Some(bundle) = stack.get_data_component::<BundleContentsImpl>() {
            pending.extend(bundle.items.iter().filter(|item| !item.is_empty()));
        }
        if let Some(projectiles) = stack.get_data_component::<ChargedProjectilesImpl>() {
            for projectile in &projectiles.projectiles {
                if ItemStack::read_item_stack(projectile).is_none() {
                    return Err(invalid());
                }
            }
        }
    }
    Ok(())
}
