use std::any::Any;

use crate::entity::experience_orb::ExperienceOrbEntity;
use crate::entity::player::Player;
use crate::item::{ItemBehaviour, ItemMetadata};
use pumpkin_data::item::Item;
use pumpkin_data::sound::{Sound, SoundCategory};

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
        _yaw: f32,
        _pitch: f32,
        hand: pumpkin_util::Hand,
    ) {
        let world = player.world();
        let pos = player.eye_position();
        world.play_sound(
            Sound::EntityExperienceBottleThrow,
            SoundCategory::Players,
            &pos,
        );

        let amount = (rand::random::<u32>() % 9 + 3) as u32; // 3..=11 exp
        ExperienceOrbEntity::spawn(&world, pos, amount);

        let mut held = player.inventory().get_stack_in_hand(hand);
        held.decrement_unless_creative(player.gamemode.load(), 1);
        player.inventory().set_stack_in_hand(hand, held);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
