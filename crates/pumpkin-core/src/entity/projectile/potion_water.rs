use super::ProjectileHit;
use crate::{entity::EntityBase, world::World};
use pumpkin_data::{
    Block, BlockDirection,
    block_properties::{CampfireLikeProperties, CandleCakeProperties, CandleLikeProperties},
    data_component_impl::PotionContentsImpl,
    entity::EntityType,
    item_stack::ItemStack,
    tag::{self, Tag, Taggable},
};
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;
use std::sync::Arc;

fn has_potion_tag(stack: &ItemStack, tag: &Tag) -> bool {
    stack
        .get_data_component::<PotionContentsImpl>()
        .and_then(|contents| contents.potion_id)
        .is_some_and(|id| tag.1.contains(&(id as u16)))
}

// AbstractThrownPotion.onHitBlock / affectEntitiesAround: shared by splash and lingering bottles.
pub(super) fn on_hit(projectile: &dyn EntityBase, stack: &ItemStack, hit: &ProjectileHit) {
    let entity = projectile.get_entity();
    let world = entity.world.load();
    if has_potion_tag(stack, &tag::Potion::MINECRAFT_DOUSES_FIRE)
        && let ProjectileHit::Block { pos, face, .. } = hit
    {
        let effect_pos = pos.offset(face.to_offset());
        douse_fire(&world, &effect_pos);
        douse_fire(&world, pos);
        for direction in [
            BlockDirection::North,
            BlockDirection::South,
            BlockDirection::West,
            BlockDirection::East,
        ] {
            douse_fire(&world, &effect_pos.offset(direction.to_offset()));
        }
    }
    let hurts = has_potion_tag(
        stack,
        &tag::Potion::MINECRAFT_HURTS_WATER_SENSITIVE_ENTITIES,
    );
    let extinguishes = has_potion_tag(stack, &tag::Potion::MINECRAFT_EXTINGUISHES_ENTITIES);
    if !hurts && !extinguishes {
        return;
    }
    let owner = projectile.projectile_owner();
    for target in world.get_all_at_box(&entity.bounding_box.load().expand(4.0, 2.0, 4.0)) {
        if target.get_living_entity().is_none() || target.is_spectator() {
            continue;
        }
        let other = target.get_entity();
        if other.pos.load().squared_distance_to_vec(&entity.pos.load()) >= 16.0 {
            continue;
        }
        // isSensitiveToWater overrides in Blaze, Enderman, SnowGolem and Strider.
        if hurts
            && [
                &EntityType::BLAZE,
                &EntityType::ENDERMAN,
                &EntityType::SNOW_GOLEM,
                &EntityType::STRIDER,
            ]
            .contains(&other.entity_type)
        {
            super::damage::hurt_entity(
                target.as_ref(),
                1.0,
                pumpkin_data::damage::DamageType::INDIRECT_MAGIC,
                projectile,
                owner.as_deref(),
            );
        }
        if extinguishes && other.is_on_fire() && other.is_alive() {
            other.extinguish();
        }
    }
}

pub fn douse_fire(world: &Arc<World>, pos: &BlockPos) {
    let state = world.get_block_state(pos);
    let block = state.id.to_block();
    if block.has_tag(&tag::Block::MINECRAFT_FIRE) {
        world.break_block(pos, None, BlockFlags::NOTIFY_ALL | BlockFlags::SKIP_DROPS);
        return;
    }
    let (state_id, campfire) = if block.has_tag(&tag::Block::MINECRAFT_CANDLES) {
        let mut props = CandleLikeProperties::from_state_id(state.id);
        if !props.lit {
            return;
        }
        props.lit = false;
        (props.to_state_id(block), false)
    } else if block.has_tag(&tag::Block::MINECRAFT_CANDLE_CAKES) {
        let mut props = CandleCakeProperties::from_state_id(state.id);
        if !props.lit {
            return;
        }
        props.lit = false;
        (props.to_state_id(block), false)
    } else if block == &Block::CAMPFIRE || block == &Block::SOUL_CAMPFIRE {
        let mut props = CampfireLikeProperties::from_state_id(state.id);
        if !props.lit {
            return;
        }
        props.lit = false;
        (props.to_state_id(block), true)
    } else {
        return;
    };
    world.set_block_state(pos, state_id, BlockFlags::NOTIFY_ALL);
    if campfire {
        world.broadcast_to_chunk(
            pos.chunk_and_chunk_relative_position().0,
            &pumpkin_protocol::java::client::play::CWorldEvent::new(1009, *pos, 0, false),
        );
    } else {
        world.play_sound(
            pumpkin_data::sound::Sound::BlockCandleExtinguish,
            pumpkin_data::sound::SoundCategory::Blocks,
            &pos.to_centered_f64(),
        );
    }
    world.emit_game_event(
        pumpkin_data::game_event::GameEvent::BlockChange.name(),
        pos.to_centered_f64(),
    );
}
