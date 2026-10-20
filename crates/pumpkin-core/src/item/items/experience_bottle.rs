use std::any::Any;

use crate::entity::player::Player;
use crate::entity::projectile::experience_bottle::ExperienceBottleEntity;
use crate::entity::{Entity, EntityBase};
use crate::item::{ItemBehaviour, ItemMetadata};
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
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
        let stack = player.inventory().get_stack_in_hand(hand);
        self.use_observed_stack(&stack, player, yaw, pitch, hand);
    }

    fn use_observed_stack(
        &self,
        stack: &ItemStack,
        player: &Player,
        yaw: f32,
        pitch: f32,
        hand: pumpkin_util::Hand,
    ) -> bool {
        // ExperienceBottleItem.use: claim the hand snapshot before projectile effects in Pumpkin's concurrent tick.
        let Some(receipt) =
            crate::item::projectile_stock::ProjectileStock::claim_hand(player, hand, stack)
        else {
            return false;
        };
        let world = player.world();
        let pos = player.position();

        let entity = Entity::new(world.clone(), pos, &EntityType::EXPERIENCE_BOTTLE);
        let bottle = ExperienceBottleEntity::new_shot(entity, player.get_entity());
        bottle.set_item_stack(stack);
        bottle.thrown.set_velocity_from(pitch, yaw, -20.0, 0.7, 1.0);
        crate::item::items::projectile_weapon::ProjectileWeaponItem::add_shooter_movement(
            bottle.get_entity(),
            player.get_entity(),
        );
        if !crate::entity::projectile::spawn_projectile(
            &world,
            Arc::new(bottle),
            Some(player.get_entity()),
        ) {
            receipt.refund(player);
            return false;
        }
        receipt.publish(player);
        world.play_sound_fine(
            Sound::EntityExperienceBottleThrow,
            SoundCategory::Neutral,
            &pos,
            0.5,
            0.4 / (rand::rng().random::<f32>() * 0.4 + 0.8),
        );

        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
