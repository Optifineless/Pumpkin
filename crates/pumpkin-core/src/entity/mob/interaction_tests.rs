use super::MobInteraction;
use crate::entity::{
    Entity, EntityBase,
    death_test_world::DeathTestWorld,
    mob::{Mob, MobEntity},
};
use crate::net::java::combat_test_support::TestPlayer;
use pumpkin_data::{entity::EntityType, item::Item, item_stack::ItemStack};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::{VarInt, java::server::play::SInteract};
use pumpkin_util::{Hand, math::vector3::Vector3};
use std::sync::Arc;

#[derive(Clone, Copy)]
enum ProbeAction {
    SwitchSelectedSlot,
    ReplaceSource,
    DeleteSource,
}

struct InteractionProbeMob {
    mob: MobEntity,
    action: ProbeAction,
}

impl InteractionProbeMob {
    fn new(world: &Arc<crate::world::World>, action: ProbeAction) -> Arc<Self> {
        let entity = Entity::new(
            world.clone(),
            Vector3::new(0.0, 64.0, 0.0),
            &EntityType::COW,
        );
        let mob = Arc::new(Self {
            mob: MobEntity::new(entity),
            action,
        });
        world.add_entity_silent(mob.clone());
        mob
    }
}

impl Mob for InteractionProbeMob {
    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob
    }

    fn mob_interact_with_hand(
        &self,
        player: &Arc<crate::entity::player::Player>,
        item_stack: &mut ItemStack,
        interaction: &mut MobInteraction,
    ) -> bool {
        match self.action {
            ProbeAction::SwitchSelectedSlot => player.inventory().set_selected_slot(1),
            ProbeAction::ReplaceSource => player.inventory().set_stack(
                interaction.source_slot(),
                ItemStack::new(2, &Item::HONEY_BOTTLE),
            ),
            ProbeAction::DeleteSource => player
                .inventory()
                .set_stack(interaction.source_slot(), ItemStack::EMPTY.clone()),
        }
        interaction.use_player_item(player, item_stack);
        true
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interaction_result_returns_to_the_captured_hotbar_slot() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let player = TestPlayer::new(&world);
    let before = ItemStack::new(1, &Item::BUCKET);
    player.player.inventory().set_stack(0, before.clone());
    player.player.inventory().set_selected_slot(0);

    let interaction = MobInteraction::new(&player.player, &before, Hand::Right);
    let after = ItemStack::new(1, &Item::MILK_BUCKET);
    let other_slot = ItemStack::new(1, &Item::DIAMOND);
    player.player.inventory().set_stack(1, other_slot.clone());
    player.player.inventory().set_selected_slot(1);

    assert!(interaction.finish(&player.player, &after));
    assert!(player.player.inventory().get_stack(0).are_equal(&after));
    assert!(
        player
            .player
            .inventory()
            .get_stack(1)
            .are_equal(&other_slot)
    );
    fixture.server.shutdown().await;
}

fn interact_with_mob(
    fixture: &DeathTestWorld,
    player: &TestPlayer,
    mob: &InteractionProbeMob,
    hand: Hand,
) {
    player.client().handle_interact(
        &player.player,
        &SInteract {
            entity_id: VarInt(mob.get_entity().entity_id),
            r#type: VarInt(0),
            target_position: None,
            hand: Some(VarInt(i32::from(hand == Hand::Left))),
            sneaking: false,
        },
        &fixture.server,
    );
}

fn count_item(player: &crate::entity::player::Player, item: &Item) -> u32 {
    (0..player.inventory().size())
        .map(|slot| player.inventory().get_stack(slot))
        .filter(|stack| stack.item == item)
        .map(|stack| u32::from(stack.item_count))
        .sum()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn packet_mob_interaction_consumes_from_the_captured_hotbar_slot_after_selection_changes() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let player = TestPlayer::new(&world);
    let mob = InteractionProbeMob::new(&world, ProbeAction::SwitchSelectedSlot);
    player
        .player
        .inventory()
        .set_stack(0, ItemStack::new(2, &Item::HONEY_BOTTLE));
    let diamond = ItemStack::new(1, &Item::DIAMOND);
    player.player.inventory().set_stack(1, diamond.clone());
    player.player.inventory().set_selected_slot(0);

    // ServerGamePacketListenerImpl.handleInteract reaches Mob.interact with the captured hand.
    interact_with_mob(&fixture, &player, &mob, Hand::Right);

    assert_eq!(player.player.inventory().get_selected_slot(), 1);
    assert_eq!(
        player.player.inventory().get_stack(0).item,
        &Item::HONEY_BOTTLE
    );
    assert_eq!(player.player.inventory().get_stack(0).item_count, 1);
    assert!(player.player.inventory().get_stack(1).are_equal(&diamond));
    assert_eq!(count_item(&player.player, &Item::DIAMOND), 1);
    assert_eq!(count_item(&player.player, &Item::GLASS_BOTTLE), 1);
    assert_eq!(count_item(&player.player, &Item::HONEY_BOTTLE), 1);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn packet_offhand_mob_interaction_keeps_the_captured_offhand_after_selection_changes() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let player = TestPlayer::new(&world);
    let mob = InteractionProbeMob::new(&world, ProbeAction::SwitchSelectedSlot);
    player
        .player
        .inventory()
        .set_stack(40, ItemStack::new(2, &Item::HONEY_BOTTLE));
    let diamond = ItemStack::new(1, &Item::DIAMOND);
    player.player.inventory().set_stack(1, diamond.clone());
    player.player.inventory().set_selected_slot(0);

    interact_with_mob(&fixture, &player, &mob, Hand::Left);

    assert_eq!(player.player.inventory().get_selected_slot(), 1);
    assert_eq!(
        player.player.inventory().get_stack(40).item,
        &Item::HONEY_BOTTLE
    );
    assert_eq!(player.player.inventory().get_stack(40).item_count, 1);
    assert!(player.player.inventory().get_stack(1).are_equal(&diamond));
    assert_eq!(count_item(&player.player, &Item::DIAMOND), 1);
    assert_eq!(count_item(&player.player, &Item::GLASS_BOTTLE), 1);
    assert_eq!(count_item(&player.player, &Item::HONEY_BOTTLE), 1);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn packet_mob_interaction_does_not_consume_or_grant_remainder_after_source_replacement() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let player = TestPlayer::new(&world);
    let mob = InteractionProbeMob::new(&world, ProbeAction::ReplaceSource);
    player
        .player
        .inventory()
        .set_stack(0, ItemStack::new(2, &Item::HONEY_BOTTLE));
    player.player.inventory().set_selected_slot(0);
    let original = player.player.inventory().get_stack(0);

    interact_with_mob(&fixture, &player, &mob, Hand::Right);

    let current = player.player.inventory().get_stack(0);
    assert_ne!(current.uid, original.uid);
    assert!(current.are_equal(&original));
    assert_eq!(count_item(&player.player, &Item::GLASS_BOTTLE), 0);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn packet_mob_interaction_does_not_consume_or_grant_remainder_after_source_deletion() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let player = TestPlayer::new(&world);
    let mob = InteractionProbeMob::new(&world, ProbeAction::DeleteSource);
    player
        .player
        .inventory()
        .set_stack(0, ItemStack::new(2, &Item::HONEY_BOTTLE));
    player.player.inventory().set_selected_slot(0);

    interact_with_mob(&fixture, &player, &mob, Hand::Right);

    assert!(player.player.inventory().get_stack(0).is_empty());
    assert_eq!(count_item(&player.player, &Item::GLASS_BOTTLE), 0);
    fixture.server.shutdown().await;
}
