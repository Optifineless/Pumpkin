use std::any::Any;

use crate::entity::player::Player;
use crate::entity::projectile::experience_bottle::ExperienceBottleEntity;
use crate::entity::{Entity, EntityBase};
use crate::item::{ItemBehaviour, ItemMetadata};
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::sound::{Sound, SoundCategory};
use rand::RngExt;
use std::sync::Arc;

pub struct ExperienceBottleItem;

impl ItemMetadata for ExperienceBottleItem {
    fn ids() -> Box<[u16]> {
        Box::new([Item::EXPERIENCE_BOTTLE.id])
    }
}

impl ItemBehaviour for ExperienceBottleItem {
    fn normal_use(&self, item: &Item, player: &Player) {
        let (yaw, pitch) = player.rotation();
        self.normal_use_with_hand(item, player, yaw, pitch, pumpkin_util::Hand::Right);
    }

    // ExperienceBottleItem.use reads and consumes player.getItemInHand(hand).
    fn normal_use_with_hand(
        &self,
        _item: &Item,
        player: &Player,
        yaw: f32,
        pitch: f32,
        hand: pumpkin_util::Hand,
    ) {
        let world = player.world();
        let pos = player.position();
        world.play_sound_fine(
            Sound::EntityExperienceBottleThrow,
            SoundCategory::Neutral,
            &pos,
            0.5,
            0.4 / (rand::rng().random::<f32>() * 0.4 + 0.8),
        );

        let mut held = player.inventory().get_stack_in_hand(hand);
        let entity = Entity::new(world.clone(), pos, &EntityType::EXPERIENCE_BOTTLE);
        let bottle = ExperienceBottleEntity::new_shot(entity, player.get_entity());
        bottle.set_item_stack(&held);
        bottle.thrown.set_velocity_from(pitch, yaw, -20.0, 0.7, 1.0);
        crate::item::items::projectile_weapon::ProjectileWeaponItem::add_shooter_movement(
            bottle.get_entity(),
            player.get_entity(),
        );
        world.spawn_entity(Arc::new(bottle));

        held.decrement_unless_creative(player.gamemode.load(), 1);
        player.inventory().set_stack_in_hand(hand, held);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
