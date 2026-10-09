use super::*;
use crate::entity::death_test_world::DeathTestWorld;
use pumpkin_data::entity::EntityType;

fn leads(world: &crate::world::World) -> u32 {
    world
        .entities
        .load()
        .iter()
        .filter_map(|entity| entity.get_item_entity())
        .map(|entity| {
            let stack = entity.get_item_stack().lock().unwrap();
            if stack.item == &Item::LEAD {
                u32::from(stack.item_count)
            } else {
                0
            }
        })
        .sum()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn leashed_mooshroom_first_click_only_snips_lead() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let player = fixture.player("SnipFirst");
    let mooshroom = fixture.mob(&EntityType::MOOSHROOM);
    mooshroom.get_entity().leash_to(player.clone());
    let mut tool = ItemStack::new(1, &Item::SHEARS);
    assert!(mooshroom.interact(&player, &mut tool));
    assert!(!mooshroom.get_entity().is_leashed());
    assert!(!mooshroom.get_entity().is_removed());
    assert_eq!(leads(&world), 1);
    let entities = world.entities.load_full();
    let lead = entities
        .iter()
        .find(|e| e.get_item_entity().is_some())
        .unwrap();
    assert_eq!(
        lead.get_entity().pos.load(),
        mooshroom.get_entity().pos.load()
    );
    assert_eq!(tool.get_damage(), 1);
    assert!(mooshroom.interact(&player, &mut tool));
    assert!(mooshroom.get_entity().is_removed());
    assert_eq!(tool.get_damage(), 2);
    assert_eq!(leads(&world), 1);
    assert_eq!(
        world
            .entities
            .load()
            .iter()
            .filter(|entity| entity.get_entity().entity_type == &EntityType::COW)
            .count(),
        1
    );
    fixture.server.shutdown().await;
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shearing_a_holder_cuts_all_nearby_outgoing_leashes() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let holder = fixture.mob(&EntityType::COW);
    let first = fixture.mob(&EntityType::SHEEP);
    let second = fixture.mob(&EntityType::MOOSHROOM);
    first.get_entity().leash_to(holder.clone());
    second.get_entity().leash_to(holder.clone());
    assert!(holder.get_entity().shear_off_all_leash_connections(None));
    assert!(!first.get_entity().is_leashed());
    assert!(!second.get_entity().is_leashed());
    assert_eq!(leads(&world), 2);
    assert!(!holder.get_entity().shear_off_all_leash_connections(None));
    assert_eq!(leads(&world), 2);
    fixture.server.shutdown().await;
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn simultaneous_leash_snips_drop_one_lead() {
    let fixture = DeathTestWorld::new().await;
    let mob = fixture.mob(&EntityType::COW);
    mob.get_entity().leash_to(fixture.player("Holder"));
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        for _ in 0..2 {
            scope.spawn(|| {
                barrier.wait();
                mob.get_entity().shear_off_all_leash_connections(None);
            });
        }
    });
    assert_eq!(leads(&fixture.world()), 1);
    fixture.server.shutdown().await;
    crate::server::fixture_lifecycle::finish().await;
}
