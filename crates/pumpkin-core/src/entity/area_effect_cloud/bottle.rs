use super::AreaEffectCloudEntity;
use crate::entity::{EntityBase, player::Player};
use pumpkin_data::{
    entity::EntityType,
    item::Item,
    item_stack::ItemStack,
    sound::{Sound, SoundCategory},
    statistic::StatisticCategory,
};
use pumpkin_util::{GameMode, Hand};

/// BottleItem.use's dragon-breath branch, before the ordinary water raycast.
pub fn try_bottle(player: &Player) -> bool {
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
    let (hand, mut held) = if player.inventory().held_item().item == &Item::GLASS_BOTTLE {
        (Hand::Right, player.inventory().held_item())
    } else {
        (Hand::Left, player.inventory().off_hand_item())
    };
    if held.is_empty() || held.item != &Item::GLASS_BOTTLE {
        return false;
    }
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
    let mut result = ItemStack::new(1, &Item::DRAGON_BREATH);
    // ItemUtils.createFilledResult limits creative inventory additions to one matching stack.
    if player.gamemode.load() == GameMode::Creative {
        if !player.inventory().contains_item(&Item::DRAGON_BREATH) {
            player.inventory().insert_stack_anywhere(&mut result);
        }
    } else if held.item_count == 1 {
        player.inventory().set_stack_in_hand(hand, result);
    } else {
        held.decrement_unless_creative(player.gamemode.load(), 1);
        player.inventory().set_stack_in_hand(hand, held);
        player.inventory().insert_stack_anywhere(&mut result);
        if !result.is_empty() {
            world.drop_stack(&player.position().to_block_pos(), result);
        }
    }
    true
}
