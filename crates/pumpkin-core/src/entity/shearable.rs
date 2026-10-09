use std::sync::Arc;

use pumpkin_data::game_event::GameEvent;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::SoundCategory;
use pumpkin_util::Hand;

use crate::entity::Entity;
use crate::entity::mob::Mob;
use crate::entity::player::Player;
use crate::plugin::api::events::player::player_shear_entity::PlayerShearEntityEvent;
use crate::world::loot::LootContextParameters;

/// Vanilla `Shearable`: a mob that players and dispensers can shear.
pub trait Shearable: Mob {
    /// Shears this mob. Returns `false` if it was already sheared or a plugin stopped it.
    ///
    /// Vanilla's `shear` returns nothing, but because Pumpkin handles players' interactions in parallel, so
    /// two of them can pass [`Self::ready_for_shearing`] at once.
    /// IMPORTANT: Implementations must claim their shear state
    /// with a single atomic swap so only one of them goes on to drop loot.
    /// See [`SnowGolemEntity::shear`]
    fn shear(&self, sound_category: SoundCategory, tool: &ItemStack) -> bool;

    fn ready_for_shearing(&self) -> bool;
}

/// The shears branch every shearable mob has in vanilla `mobInteract`.
///
/// Returns `false` if a plugin cancels the shearing or another shear got there first.
pub fn shear_by_player(
    shearable: &dyn Shearable,
    player: &Arc<Player>,
    tool: &mut ItemStack,
) -> bool {
    shear_by_player_with_hand(shearable, player, tool, Hand::Right)
}

fn shear_by_player_with_hand(
    shearable: &dyn Shearable,
    player: &Arc<Player>,
    tool: &mut ItemStack,
    hand: Hand,
) -> bool {
    let entity = &shearable.get_mob_entity().living_entity.entity;
    let world = entity.world.load();
    if let Some(server) = world.server.upgrade() {
        let mut event = PlayerShearEntityEvent {
            player: player.clone(),
            entity_id: entity.entity_id,
            hand: u8::from(hand == Hand::Left),
            cancelled: false,
        };
        server.plugin_manager.fire_blocking(&server, &mut event);
        if event.cancelled {
            return false;
        }
    }

    let pos = entity.pos.load();
    if !shearable.shear(SoundCategory::Players, tool) {
        return false;
    }
    world.emit_game_event(GameEvent::Shear.name(), pos);
    player.damage_detached_item(tool, 1);
    true
}

// LivingEntity.dropFromShearingLootTable evaluates the root through reloadable registries.
#[must_use]
pub fn shearing_loot(
    entity: &Entity,
    loot_key: &str,
    params: &LootContextParameters,
) -> Vec<ItemStack> {
    let Some(loot_table) = entity.world.load().get_loot_table(loot_key) else {
        return Vec::new();
    };
    loot_table.generate_loot_with_context(0, params)
}

#[cfg(test)]
#[path = "shearing_loot_tests.rs"]
mod loot_tests;

/// Handles the authoritative shearing hand; `None` preserves species-specific fallback.
pub(crate) fn interact_with_hand(
    mob: &dyn Mob,
    player: &Arc<Player>,
    tool: &mut ItemStack,
    hand: Hand,
) -> Option<bool> {
    // Mob.interact -> Entity.interact -> Sheep/SnowGolem/Bogged/MushroomCow.mobInteract.
    if !mob.get_entity().is_alive() || mob.get_mob_entity().living_entity.health.load() <= 0.0 {
        return Some(false);
    }
    if tool.item != &pumpkin_data::item::Item::SHEARS {
        return None;
    }
    if crate::entity::leash_shearing::shear_leashes_by_player(mob.get_entity(), player, tool) {
        return Some(true);
    }
    let shearable = mob.as_shearable().filter(|mob| mob.ready_for_shearing())?;
    let result = shear_by_player_with_hand(shearable, player, tool, hand);
    if result
        && mob
            .cast_any()
            .is::<crate::entity::passive::sheep::SheepEntity>()
    {
        player.swing_hand(hand, true);
    }
    Some(result)
}

#[cfg(test)]
#[path = "shearing_hand_tests.rs"]
mod hand_tests;
