use crate::block::registry::BlockActionResult;
use crate::entity::player::Player;
use crate::entity::{
    decoration::leash_knot::{LeashKnotEntity, leash_holder_id},
    leashable::{can_have_a_leash_attached_to_position, leash_search_box},
};
use crate::item::{ItemBehaviour, ItemMetadata};
use crate::server::Server;
use pumpkin_data::Block;
use pumpkin_data::BlockDirection;
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::Taggable;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

pub struct LeadItem;

impl ItemMetadata for LeadItem {
    fn ids() -> Box<[u16]> {
        [Item::LEAD.id].into()
    }
}

impl ItemBehaviour for LeadItem {
    fn use_on_block(
        &self,
        _item: &mut ItemStack,
        player: &Player,
        location: BlockPos,
        _face: BlockDirection,
        _cursor_pos: Vector3<f32>,
        block: &Block,
        _server: &Server,
    ) -> BlockActionResult {
        if !block.has_tag(&pumpkin_data::tag::Block::MINECRAFT_FENCES) {
            return BlockActionResult::Pass;
        }

        let world = player.world();
        let center = Vector3::new(
            f64::from(location.0.x) + 0.5,
            f64::from(location.0.y) + 0.5,
            f64::from(location.0.z) + 0.5,
        );
        let knot_center = LeashKnotEntity::bounding_box_center_at(location);

        // LeadItem.bindPlayerMobs uses Leashable.leashableInArea's centered 32-block cube.
        let search_box = leash_search_box(center);

        let player_id = player.entity_id();
        let entities = world.get_entities_at_box(&search_box);

        let player_leashed_mobs: Vec<_> = entities
            .iter()
            .filter(|entity_base| {
                leash_holder_id(entity_base.get_entity()) == Some(player_id)
                    && can_have_a_leash_attached_to_position(entity_base.as_ref(), knot_center)
            })
            .collect();
        if player_leashed_mobs.is_empty() {
            return BlockActionResult::Pass;
        }

        let existing_knot = LeashKnotEntity::get_knot(&world, location);
        let created_knot = existing_knot.is_none();
        let mut knot =
            existing_knot.unwrap_or_else(|| LeashKnotEntity::get_or_create(&world, location));
        let mut any_leashed = false;
        for mob in player_leashed_mobs {
            any_leashed |=
                LeashKnotEntity::attach_to_knot(&world, location, mob.as_ref(), &mut knot);
        }

        if any_leashed {
            // LeadItem.bindPlayerMobs transfers existing leashes without consuming a lead.
            world.play_sound(Sound::ItemLeadTied, SoundCategory::Neutral, &center);
            world.emit_game_event(GameEvent::BlockAttach.name(), center);
            BlockActionResult::Success
        } else {
            if created_knot && let Some(knot) = knot.cast_any().downcast_ref::<LeashKnotEntity>() {
                knot.remove_if_empty(&world);
            }
            BlockActionResult::Pass
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
