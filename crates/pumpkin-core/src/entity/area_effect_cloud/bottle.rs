use super::AreaEffectCloudEntity;
use crate::entity::{EntityBase, player::Player};
use pumpkin_data::{
    entity::EntityType,
    item::Item,
    item_stack::ItemStack,
    sound::{Sound, SoundCategory},
    statistic::StatisticCategory,
};
use pumpkin_util::Hand;

/// BottleItem.use's dragon-breath branch, before the ordinary water raycast.
pub fn try_bottle(player: &Player, hand: Hand) -> bool {
    let world = player.world();
    let candidates = world.get_all_at_box(&player.get_entity().bounding_box.load().expand_all(2.0));
    let Some(cloud) = candidates
        .iter()
        .filter_map(|entity| entity.cast_any().downcast_ref::<AreaEffectCloudEntity>())
        .find(|cloud| {
            cloud.entity.is_alive()
                && cloud.owner().is_some_and(|owner| {
                    owner.get_entity().entity_type == &EntityType::ENDER_DRAGON
                })
        })
    else {
        return false;
    };
    let held = player.inventory().get_stack_in_hand(hand);
    if held.is_empty() || held.item != &Item::GLASS_BOTTLE {
        return false;
    }
    let Some(receipt) = crate::item::item_utils::claim_item_in_hand(player, hand, &held) else {
        return false;
    };
    // BottleItem.use claims the bottle and shrinks the cloud in one serial operation.
    cloud.set_radius(cloud.radius() - 0.5);
    world.play_sound(
        Sound::ItemBottleFillDragonbreath,
        SoundCategory::Neutral,
        &player.position(),
    );
    world.emit_game_event(
        pumpkin_data::game_event::GameEvent::FluidPickup.name(),
        player.position(),
    );
    player.increment_stat(StatisticCategory::Used, i32::from(Item::GLASS_BOTTLE.id), 1);
    crate::item::item_utils::finish_claimed_result(
        player,
        hand,
        &held,
        &receipt,
        ItemStack::new(1, &Item::DRAGON_BREATH),
        true,
    );
    true
}
