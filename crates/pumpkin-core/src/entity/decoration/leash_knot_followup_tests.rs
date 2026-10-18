use super::*;

struct CancelUnleashForEntity(i32);

impl EventHandler<crate::plugin::api::events::entity::entity_unleash::EntityUnleashEvent>
    for CancelUnleashForEntity
{
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut crate::plugin::api::events::entity::entity_unleash::EntityUnleashEvent,
    ) -> BoxFuture<'a, ()> {
        if event.entity_id == self.0 {
            event.cancelled = true;
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_knot_unleash_is_handled_without_transfer_or_shears_wear() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let player = TestPlayer::new(&world);
    player_near_fence(&player);
    player
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::new(1, &Item::SHEARS));
    let pos = BlockPos::new(0, 64, 0);
    make_fence(&world, pos);
    let knot = LeashKnotEntity::create_knot(&world, pos);
    let cow = fixture.mob(&EntityType::COW);
    cow.get_entity().set_pos(Vector3::new(0.0, 64.0, 0.0));
    cow.get_entity()
        .leash_to(knot.clone() as Arc<dyn EntityBase>);
    fixture
        .server
        .plugin_manager
        .register::<crate::plugin::api::events::entity::entity_unleash::EntityUnleashEvent, _>(
        Arc::new(CancelUnleashForEntity(cow.get_entity().entity_id)),
        EventPriority::Normal,
        true,
    );

    player.client().handle_interact(
        &player.player,
        &right_click(knot.get_entity().entity_id),
        &fixture.server,
    );

    assert_eq!(
        leash_holder_id(cow.get_entity()),
        Some(knot.get_entity().entity_id)
    );
    assert_eq!(player.player.inventory().held_item().get_damage(), 0);
    assert_eq!(lead_count(&world), 0);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn can_be_leashed_matches_implemented_vanilla_species_overrides() {
    let fixture = DeathTestWorld::new().await;
    let should_leash: &[(&'static EntityType, bool)] = &[
        (&EntityType::PANDA, false),
        (&EntityType::TURTLE, false),
        (&EntityType::BAT, false),
        (&EntityType::COD, false),
        (&EntityType::GLOW_SQUID, true),
        (&EntityType::VILLAGER, false),
        (&EntityType::WANDERING_TRADER, false),
        (&EntityType::CREEPER, false),
        (&EntityType::HOGLIN, true),
        (&EntityType::ZOGLIN, true),
        (&EntityType::SQUID, true),
        (&EntityType::DOLPHIN, true),
        (&EntityType::AXOLOTL, true),
        (&EntityType::NAUTILUS, true),
        (&EntityType::ZOMBIE_NAUTILUS, true),
        (&EntityType::ZOMBIE_HORSE, true),
        (&EntityType::WOLF, true),
        (&EntityType::COW, true),
    ];
    for &(entity_type, expected) in should_leash {
        let mob = fixture.mob(entity_type);
        assert_eq!(
            mob.get_mob().unwrap().can_be_leashed(),
            expected,
            "{} canBeLeashed",
            entity_type.resource_name
        );
    }

    let wolf = fixture.mob(&EntityType::WOLF);
    wolf.get_mob()
        .unwrap()
        .as_neutral()
        .unwrap()
        .set_time_to_remain_angry(20);
    assert!(!wolf.get_mob().unwrap().can_be_leashed());

    let nautilus = fixture.mob(&EntityType::NAUTILUS);
    let attack_target = fixture.mob(&EntityType::COW);
    *nautilus
        .get_mob()
        .unwrap()
        .get_mob_entity()
        .target
        .lock()
        .unwrap() = Some(attack_target);
    assert!(!nautilus.get_mob().unwrap().can_be_leashed());

    let zombie_horse = fixture.mob(&EntityType::ZOMBIE_HORSE);
    let mob_passenger = fixture.mob(&EntityType::COW);
    zombie_horse
        .get_entity()
        .passengers
        .lock()
        .unwrap()
        .push(mob_passenger);
    assert!(!zombie_horse.get_mob().unwrap().can_be_leashed());

    let zombie_nautilus = fixture.mob(&EntityType::ZOMBIE_NAUTILUS);
    let mob_passenger = fixture.mob(&EntityType::COW);
    zombie_nautilus
        .get_entity()
        .passengers
        .lock()
        .unwrap()
        .push(mob_passenger);
    assert!(!zombie_nautilus.get_mob().unwrap().can_be_leashed());
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn panda_does_not_accept_a_lead_in_real_player_interaction() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let player = TestPlayer::new(&world);
    player_near_fence(&player);
    player
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::new(1, &Item::LEAD));
    let panda = fixture.mob(&EntityType::PANDA);

    player.client().handle_interact(
        &player.player,
        &right_click(panda.get_entity().entity_id),
        &fixture.server,
    );

    assert!(!panda.get_entity().is_leashed());
    assert_eq!(player.player.inventory().held_item().item_count, 1);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn glow_squid_accepts_a_lead_in_real_player_interaction() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let player = TestPlayer::new(&world);
    player_near_fence(&player);
    player
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::new(1, &Item::LEAD));
    let glow_squid = fixture.mob(&EntityType::GLOW_SQUID);

    player.client().handle_interact(
        &player.player,
        &right_click(glow_squid.get_entity().entity_id),
        &fixture.server,
    );

    assert_eq!(
        leash_holder_id(glow_squid.get_entity()),
        Some(player.player.entity_id())
    );
    assert_eq!(player.player.inventory().held_item().item_count, 0);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prospective_knot_range_uses_its_bounding_box_center() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let player = TestPlayer::new(&world);
    player_near_fence(&player);
    player
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::new(1, &Item::LEAD));
    let pos = BlockPos::new(0, 64, 0);
    make_fence(&world, pos);
    let knot_spawns = track_knot_spawns(&fixture.server);
    let cow = fixture.mob(&EntityType::COW);
    let knot_center = LeashKnotEntity::bounding_box_center_at(pos);
    let cow_height = f64::from(cow.get_entity().entity_dimension.load().height);
    let cow_center_y = knot_center.y + cow.get_entity().leash_snap_distance() - 0.1;
    cow.get_entity()
        .set_pos(Vector3::new(0.5, cow_center_y - cow_height * 0.5, 0.5));
    cow.get_entity()
        .leash_to(player.player.clone() as Arc<dyn EntityBase>);

    use_on_fence(&player, pos, &fixture.server);

    let knot = LeashKnotEntity::get_knot(&world, pos).unwrap();
    assert_eq!(
        leash_holder_id(cow.get_entity()),
        Some(knot.get_entity().entity_id)
    );
    assert_eq!(knot_spawns.load(Ordering::Relaxed), 1);
    fixture.server.shutdown().await;
}
