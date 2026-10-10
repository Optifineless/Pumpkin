use super::{tests::hook, *};
use crate::{
    net::java::combat_test_support::TestPlayer,
    server::combat_test_support::{server, world},
    world::spawn_test_support::{proto, publish},
};
use pumpkin_data::{Block, biome::Biome};
use pumpkin_util::math::position::BlockPos;

#[tokio::test]
async fn fishing_seeded_bite_kick_matches_vanilla_float_bits() {
    // FishingHook.onSyncedDataUpdated -> Mth.nextFloat, LegacyRandomSource seed 3.
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let owner = TestPlayer::new(&world);
    publish(&world, proto(&Biome::PLAINS, &Block::AIR));
    let mut bobber = hook(&world, &owner.player);
    // FishingHook.tick seeds with UUID low bits XOR game time: 4 XOR 7 = 3.
    bobber.entity.entity_uuid = uuid::Uuid::from_u128(4);
    world.level_time.lock().unwrap().world_age = 7;
    bobber.hook_countdown.store(1, Relaxed);
    let mut velocity = Vector3::default();
    bobber.catching_fish(&world, &BlockPos::new(0, 64, 0), &mut velocity);
    assert_eq!((velocity.y as f32).to_bits(), 0xbeb6_c4aa);
    assert_eq!(velocity.y, -0.356_969_177_722_930_9);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn fishing_seeded_biting_pull_uses_two_vanilla_float_draws() {
    // FishingHook.tick's two LegacyRandomSource seed-3 draws, widened before multiplication.
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let owner = TestPlayer::new(&world);
    let mut bobber = hook(&world, &owner.player);
    bobber.entity.entity_uuid = uuid::Uuid::from_u128(4);
    world.level_time.lock().unwrap().world_age = 7;
    // Exact water equilibrium eliminates the unsynchronized spring draw.
    bobber.entity.set_pos(Vector3::new(0.0, 64.5, 0.0));
    bobber.bite_countdown.store(20, Relaxed);
    let velocity = bobber.bob_tick(&world, &BlockPos::new(0, 64, 0), 0.5, Vector3::default());
    assert_eq!(velocity.y.to_bits(), 0xbfa5_0d10_b00f_c633);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn fishing_process_tick_moves_before_applying_inertia() {
    // FishingHook.tick moves with unscaled velocity, then applies 0.92 inertia.
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let owner = TestPlayer::new(&world);
    publish(&world, proto(&Biome::PLAINS, &Block::AIR));
    owner
        .player
        .get_entity()
        .set_pos(Vector3::new(4.0, 64.0, 8.5));
    let bobber = hook(&world, &owner.player);
    let start = Vector3::new(8.5, 70.0, 8.5);
    bobber.entity.set_pos(start);
    bobber.entity.velocity.store(Vector3::new(1.0, 0.5, 0.0));
    bobber.process_tick(&bobber);
    assert_eq!(bobber.entity.pos.load().x, 9.5);
    // FishingHook.getDefaultGravity widens Java's 0.03F before subtraction.
    assert_eq!(bobber.entity.pos.load().y, 70.470_000_000_670_55);
    assert_eq!(bobber.entity.velocity.load().x, 0.92);
    assert_eq!(bobber.entity.velocity.load().y, 0.432_400_000_616_908_1);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn fishing_external_removal_clears_owner_immediately_with_a_retained_arc() {
    // FishingHook.remove/updateOwnerInfo must not rely on the next tick or Drop.
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let owner = TestPlayer::new(&world);
    let bobber = Arc::new(hook(&world, &owner.player));
    world.spawn_entity(bobber.clone());
    owner
        .player
        .fishing_bobber
        .store(bobber.entity.entity_id, Relaxed);
    // Entity.remove passes only the base Entity; World must still resolve the concrete hook.
    bobber.entity.remove();
    assert!(bobber.entity.is_removed());
    assert!(world.get_entity_by_id(bobber.entity.entity_id).is_none());
    assert_eq!(owner.player.fishing_bobber.load(Relaxed), -1);

    let old_hook = Arc::new(hook(&world, &owner.player));
    let new_hook = Arc::new(hook(&world, &owner.player));
    world.spawn_entity(old_hook.clone());
    world.spawn_entity(new_hook.clone());
    owner
        .player
        .fishing_bobber
        .store(new_hook.entity.entity_id, Relaxed);
    world.remove_entity(old_hook.as_ref());
    assert_eq!(
        owner.player.fishing_bobber.load(Relaxed),
        new_hook.entity.entity_id
    );
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn fishing_offhand_packet_uses_its_enchantments_and_durability_with_two_rods() {
    // FishingRodItem.use reads the packet's hand for enchantments and hurtAndBreak.
    use pumpkin_data::{Enchantment, item_stack::ItemStack};
    use pumpkin_protocol::{VarInt, java::server::play::SUseItem};
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let owner = TestPlayer::new(&world);
    let mut main_rod = ItemStack::new(1, &Item::FISHING_ROD);
    main_rod.add_enchantment(&Enchantment::LUCK_OF_THE_SEA, 1);
    main_rod.add_enchantment(&Enchantment::LURE, 3);
    main_rod.set_damage(7);
    let mut off_rod = ItemStack::new(1, &Item::FISHING_ROD);
    off_rod.add_enchantment(&Enchantment::LUCK_OF_THE_SEA, 3);
    off_rod.add_enchantment(&Enchantment::LURE, 1);
    off_rod.set_damage(11);
    owner
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, main_rod.clone());
    owner
        .player
        .inventory()
        .set_stack_in_hand(Hand::Left, off_rod);
    let packet = SUseItem {
        hand: VarInt(1),
        sequence: VarInt(1),
        yaw: 0.0,
        pitch: 0.0,
    };
    owner
        .client()
        .handle_use_item(&owner.player, &packet, &server);
    let entity = world
        .get_entity_by_id(owner.player.fishing_bobber.load(Relaxed))
        .unwrap();
    let bobber = entity
        .cast_any()
        .downcast_ref::<FishingBobberEntity>()
        .unwrap();
    assert_eq!(bobber.luck, 3);
    assert_eq!(bobber.lure_speed, 100);
    assert_eq!(bobber.hand, Hand::Left);
    bobber.entity.on_ground.store(true, Relaxed);
    owner
        .client()
        .handle_use_item(&owner.player, &packet, &server);
    assert_eq!(owner.player.fishing_bobber.load(Relaxed), -1);
    assert!(
        owner
            .player
            .inventory()
            .held_item()
            .are_items_and_components_equal(&main_rod)
    );
    assert_eq!(owner.player.inventory().off_hand_item().get_damage(), 13);
    crate::server::fixture_lifecycle::finish().await;
}
