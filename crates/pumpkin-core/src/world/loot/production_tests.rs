use super::*;
use crate::{entity::EntityBase, server::combat_test_support};
use pumpkin_data::{damage::DamageType, entity::EntityType, item_stack::ItemStack};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use serde_json::json;
use std::sync::{Arc, atomic::Ordering::Relaxed};

fn dropped(world: &crate::world::World) -> Vec<ItemStack> {
    world
        .entities
        .load()
        .iter()
        .filter_map(|entity| {
            entity
                .get_item_entity()
                .map(|item| item.get_item_stack().lock().unwrap().clone())
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lethal_hits_supply_projectile_and_wolf_credit_and_advance_named_streams() {
    use pumpkin_inventory::Inventory;
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    server.worlds.store(Arc::new(vec![world.clone()]));
    let client = crate::net::java::combat_test_support::TestPlayer::new(&world);
    let player = &client.player;
    let mut sword = ItemStack::new(1, &Item::DIAMOND_SWORD);
    sword.add_enchantment(&pumpkin_data::Enchantment::LOOTING, 3);
    player.inventory().set_stack(0, sword);
    let arrow = crate::entity::r#type::from_type(
        &EntityType::ARROW,
        Vector3::default(),
        &world,
        uuid::Uuid::new_v4(),
    );
    let wolf = crate::entity::r#type::from_type(
        &EntityType::WOLF,
        Vector3::default(),
        &world,
        uuid::Uuid::new_v4(),
    );
    let tame = wolf.get_mob().unwrap().as_tamable().unwrap();
    tame.set_tame(true);
    tame.set_owner(Some(player.gameprofile.id));
    for (cause, direct, attacker_type, direct_type, expected) in [
        (
            player.as_ref() as &dyn EntityBase,
            arrow.as_ref(),
            "minecraft:player",
            "minecraft:arrow",
            4,
        ),
        (
            wolf.as_ref(),
            wolf.as_ref(),
            "minecraft:wolf",
            "minecraft:wolf",
            1,
        ),
    ] {
        let table = super::tests::parse(
            &json!({"random_sequence":"test:death","pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:apple","condition":{"type":"minecraft:all_of","terms":[{"type":"minecraft:killed_by_player"},{"type":"minecraft:entity_properties","entity":"attacker","predicate":{"type":attacker_type}},{"type":"minecraft:entity_properties","entity":"direct_attacker","predicate":{"type":direct_type}},{"type":"minecraft:random_chance","chance":1}]},"modifier":{"type":"minecraft:enchanted_count_increase","enchantment":"minecraft:looting","count":1}}]}]}),
        );
        server
            .datapack_manager
            .insert_loot_table("minecraft:entities/cow".to_owned(), Arc::new(table));
        let id = pumpkin_util::identifier::Identifier::parse("test:death").unwrap();
        let before = server
            .random_sequences
            .lock()
            .unwrap()
            .get_or_create(&id, 0)
            .random()
            .state();
        let victim = crate::entity::r#type::from_type(
            &EntityType::COW,
            Vector3::new(0.0, 64.0, 0.0),
            &world,
            uuid::Uuid::new_v4(),
        );
        world.add_entity_silent(victim.clone());
        let previous = super::tests::count(&dropped(&world), &Item::APPLE);
        assert!(victim.damage_with_context(
            victim.as_ref(),
            f32::MAX,
            DamageType::ARROW,
            None,
            Some(direct),
            Some(cause)
        ));
        assert_eq!(
            super::tests::count(&dropped(&world), &Item::APPLE) - previous,
            expected
        );
        let after = server
            .random_sequences
            .lock()
            .unwrap()
            .get_or_create(&id, 0)
            .random()
            .state();
        assert_ne!(before, after);
    }
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn environmental_deaths_keep_player_credit_without_attacker_looting() {
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    server.worlds.store(Arc::new(vec![world.clone()]));
    let client = crate::net::java::combat_test_support::TestPlayer::new(&world);
    let player = &client.player;
    // Environmental death retains recent player credit without inventing an attacker/Looting.
    let table = super::tests::parse(
        &json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:diamond","condition":{"type":"minecraft:killed_by_player"},"modifier":{"type":"minecraft:enchanted_count_increase","enchantment":"minecraft:looting","count":1}}]}]}),
    );
    server
        .datapack_manager
        .insert_loot_table("minecraft:entities/cow".to_owned(), Arc::new(table));
    let victim = crate::entity::r#type::from_type(
        &EntityType::COW,
        Vector3::new(0.0, 64.0, 0.0),
        &world,
        uuid::Uuid::new_v4(),
    );
    world.add_entity_silent(victim.clone());
    assert!(victim.damage_with_context(
        victim.as_ref(),
        1.0,
        DamageType::PLAYER_ATTACK,
        None,
        Some(player.as_ref()),
        Some(player.as_ref())
    ));
    victim
        .get_living_entity()
        .unwrap()
        .hurt_cooldown
        .store(0, Relaxed);
    assert!(victim.damage_with_context(
        victim.as_ref(),
        f32::MAX,
        DamageType::GENERIC_KILL,
        None,
        None,
        None
    ));
    assert_eq!(super::tests::count(&dropped(&world), &Item::DIAMOND), 1);
    crate::server::fixture_lifecycle::finish().await;
}

struct RemoveContainer;
impl crate::plugin::EventHandler<crate::plugin::block::block_drop_item::BlockDropItemEvent>
    for RemoveContainer
{
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<crate::server::Server>,
        event: &'a mut crate::plugin::block::block_drop_item::BlockDropItemEvent,
    ) -> crate::plugin::BoxFuture<'a, ()> {
        for stack in &mut event.items {
            assert!(
                stack
                    .get_data_component::<pumpkin_data::data_component_impl::ContainerImpl>()
                    .is_some()
            );
            stack.patch.clear();
            stack
                .patch
                .push((pumpkin_data::data_component::DataComponent::Container, None));
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn block_drop_events_run_after_component_copy_and_their_edits_survive() {
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    let pos = BlockPos::new(0, 64, 0);
    let shulker = crate::block::entities::shulker_box::ShulkerBoxBlockEntity::new(pos);
    shulker.items.write().unwrap()[0] = ItemStack::new(5, &Item::DIAMOND);
    world.add_block_entity(Arc::new(shulker));
    server
        .plugin_manager
        .register::<crate::plugin::block::block_drop_item::BlockDropItemEvent, _>(
            Arc::new(RemoveContainer),
            crate::plugin::EventPriority::Normal,
            true,
        );
    crate::block::drop_loot(
        &world,
        &pumpkin_data::Block::SHULKER_BOX,
        &pos,
        false,
        &LootContextParameters::default(),
    );
    let items = dropped(&world);
    assert_eq!(items.len(), 1);
    assert!(
        items[0]
            .get_data_component::<pumpkin_data::data_component_impl::ContainerImpl>()
            .is_none()
    );
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn loot_kill_command_credits_the_executing_player_without_target_hurt_memory() {
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    server.worlds.store(Arc::new(vec![world.clone()]));
    let client = crate::net::java::combat_test_support::TestPlayer::new(&world);
    client
        .player
        .permission_lvl
        .store(pumpkin_util::PermissionLvl::Four);
    let victim = crate::entity::r#type::from_type(
        &EntityType::COW,
        Vector3::new(0.0, 64.0, 0.0),
        &world,
        uuid::Uuid::new_v4(),
    );
    world.add_entity_silent(victim.clone());
    let table = super::tests::parse(
        &json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:emerald","condition":{"type":"minecraft:all_of","terms":[{"type":"minecraft:killed_by_player"},{"type":"minecraft:entity_properties","entity":"attacker","predicate":{"type":"minecraft:player"}},{"type":"minecraft:entity_properties","entity":"direct_attacker","predicate":{"type":"minecraft:player"}},{"type":"minecraft:entity_properties","entity":"this","predicate":{"type":"minecraft:cow"}}]}}]}]}),
    );
    server
        .datapack_manager
        .insert_loot_table("minecraft:entities/cow".to_owned(), Arc::new(table));
    let source = crate::command::CommandSender::Player(client.player).into_source(&server);
    let command = format!("loot spawn 0 64 0 kill {}", victim.get_entity().entity_uuid);
    assert!(
        server
            .command_dispatcher
            .load()
            .execute_input(&command, &source)
            .is_ok()
    );
    assert_eq!(super::tests::count(&dropped(&world), &Item::EMERALD), 1);
    assert!(!victim.get_living_entity().unwrap().dead.load(Relaxed));
    crate::server::fixture_lifecycle::finish().await;
}
