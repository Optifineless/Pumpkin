use super::*;
use crate::entity::{EntityBase, death_test_world::DeathTestWorld};
use crate::{
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::entity::entity_unleash::EntityUnleashEvent,
    },
    server::Server,
};
use pumpkin_data::entity::EntityType;
use pumpkin_util::math::vector3::Vector3;
use std::sync::Arc;

struct CancelEntityUnleash {
    entity_id: i32,
}

impl EventHandler<EntityUnleashEvent> for CancelEntityUnleash {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut EntityUnleashEvent,
    ) -> BoxFuture<'a, ()> {
        if event.entity_id == self.entity_id {
            event.cancelled = true;
        }
        Box::pin(async {})
    }
}

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
    assert!(
        holder
            .get_entity()
            .shear_off_all_leash_connections(None)
            .detached
    );
    assert!(!first.get_entity().is_leashed());
    assert!(!second.get_entity().is_leashed());
    assert_eq!(leads(&world), 2);
    let empty = holder.get_entity().shear_off_all_leash_connections(None);
    assert!(!empty.attempted);
    assert!(!empty.detached);
    assert_eq!(leads(&world), 2);
    fixture.server.shutdown().await;
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
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn snapping_leash_drops_lead_only_after_unleash_succeeds() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let cancel_holder = fixture.player("CancelSnap");
    let cancelled_cow = fixture.mob(&EntityType::COW);
    cancel_holder
        .get_entity()
        .set_pos(Vector3::new(0.0, 64.0, 0.0));
    cancelled_cow
        .get_entity()
        .set_pos(Vector3::new(13.0, 64.2, 0.0));
    cancelled_cow.get_entity().leash_to(cancel_holder.clone());
    fixture
        .server
        .plugin_manager
        .register::<EntityUnleashEvent, _>(
            Arc::new(CancelEntityUnleash {
                entity_id: cancelled_cow.get_entity().entity_id,
            }),
            EventPriority::Normal,
            true,
        );

    for _ in 0..3 {
        cancelled_cow
            .get_entity()
            .tick_leash_with_living(cancelled_cow.get_living_entity().unwrap());
    }
    assert!(cancelled_cow.get_entity().is_leashed());
    assert_eq!(leads(&world), 0);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn snapping_leash_without_cancelling_listener_drops_one_lead() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let holder = fixture.player("OrdinarySnap");
    let cow = fixture.mob(&EntityType::COW);
    holder.get_entity().set_pos(Vector3::new(0.0, 64.0, 0.0));
    cow.get_entity().set_pos(Vector3::new(13.0, 64.2, 0.0));
    cow.get_entity().leash_to(holder);

    cow.get_entity()
        .tick_leash_with_living(cow.get_living_entity().unwrap());

    assert!(!cow.get_entity().is_leashed());
    assert_eq!(leads(&world), 1);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn removed_holder_unlinks_without_dropping_when_entity_drops_are_disabled() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let holder = fixture.mob(&EntityType::COW);
    let cow = fixture.mob(&EntityType::COW);
    cow.get_entity().leash_to(holder.clone());
    holder.get_entity().remove();
    fixture.server.level_info.rcu(|info| {
        let mut next = (**info).clone();
        next.game_rules.entity_drops = false;
        next
    });

    cow.get_entity()
        .tick_leash_with_living(cow.get_living_entity().unwrap());

    assert!(!cow.get_entity().is_leashed());
    assert_eq!(leads(&world), 0);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_holder_transfer_to_another_world_skips_distance_and_physics() {
    let fixture = DeathTestWorld::new().await;
    let destination_fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let destination = destination_fixture.world();
    let holder = fixture.mob(&EntityType::COW);
    let cow = fixture.mob(&EntityType::COW);
    holder.get_entity().set_pos(Vector3::new(30.0, 64.0, 0.0));
    cow.get_entity().set_pos(Vector3::new(0.0, 64.0, 0.0));
    cow.get_entity().velocity.store(Vector3::default());
    cow.get_entity().leash_to(holder.clone());

    // Entity.set_world is the same transfer hook used by dimension changes.
    holder.get_entity().set_world(destination);

    cow.get_entity()
        .tick_leash_with_living(cow.get_living_entity().unwrap());

    assert_eq!(
        crate::entity::decoration::leash_knot::leash_holder_id(cow.get_entity()),
        Some(holder.get_entity().entity_id)
    );
    assert_eq!(cow.get_entity().velocity.load(), Vector3::default());
    assert_eq!(leads(&world), 0);
    fixture.server.shutdown().await;
    destination_fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dead_living_leashee_settles_and_drops_one_lead() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let holder = fixture.mob(&EntityType::COW);
    let cow = fixture.mob(&EntityType::COW);
    cow.get_entity().leash_to(holder);
    cow.get_living_entity().unwrap().set_health(0.0);

    cow.get_entity()
        .tick_leash_with_living(cow.get_living_entity().unwrap());

    assert!(!cow.get_entity().is_leashed());
    assert_eq!(leads(&world), 1);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spectator_player_cannot_remain_a_leash_holder() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let holder = fixture.player("SpectatorLeashHolder");
    let cow = fixture.mob(&EntityType::COW);
    cow.get_entity()
        .leash_to(holder.clone() as Arc<dyn EntityBase>);
    assert!(cow.get_entity().is_leashed());
    holder.gamemode.store(pumpkin_util::GameMode::Spectator);

    cow.get_entity()
        .tick_leash_with_living(cow.get_living_entity().unwrap());

    assert!(!cow.get_entity().is_leashed());
    assert_eq!(leads(&world), 1);
    fixture.server.shutdown().await;
}
