use super::test_support::PlayerFixture;
use super::*;
use crate::{
    entity::{
        Entity,
        mob::{neutral::NeutralMob, zombified_piglin::ZombifiedPiglinEntity},
    },
    net::java::combat_test_support::TestPlayer,
    world::spawn_test_support::{proto, publish},
};
use pumpkin_data::{
    biome::Biome,
    block_properties::HorizontalFacing,
    dimension::BedRuleOption,
    entity::{EntityPose, EntityType},
};
use pumpkin_util::{gamemode::GameMode, math::vector3::Vector3};

fn place(world: &Arc<World>, block: &Block) -> (BlockPos, BlockPos) {
    let foot = BlockPos::new(5, 64, 5);
    let mut props = BedProperties::default(block);
    props.facing = HorizontalFacing::East;
    props.part = BedPart::Foot;
    world.set_block_state(&foot, props.to_state_id(block), BlockFlags::NOTIFY_ALL);
    (foot, foot.offset(props.facing.to_offset()))
}

#[tokio::test]
async fn missing_bed_wakes_player_before_night_skip_or_sleep_timer() {
    let fixture = PlayerFixture::new();
    publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    let (foot, head) = place(&fixture.world, &Block::RED_BED);
    fixture.world.level_time.lock().unwrap().time_of_day = 13_000;
    let client = TestPlayer::new(&fixture.world);
    client.player.sleep(head);
    client.player.sleeping_since.store(Some(100));
    fixture
        .world
        .break_block(&foot, None, BlockFlags::NOTIFY_ALL);
    client.player.tick(&fixture.world.server.upgrade().unwrap());
    assert!(client.player.get_entity().pose.load() == EntityPose::Standing);
    assert!(client.player.sleeping_since.load().is_none());
    assert!(!fixture.world.should_skip_night());
    let (_, head) = place(&fixture.world, &Block::RED_BED);
    client.player.sleep(head);
    client.player.sleeping_since.store(Some(100));
    fixture.world.level_time.lock().unwrap().time_of_day = 6_000;
    assert!(!fixture.world.should_skip_night());
    assert!(client.player.sleeping_bed_pos.load().is_none());
    fixture.finish().await;
}

#[tokio::test]
async fn player_wake_finds_safe_standing_position_and_corrects_client() {
    for bed in [&Block::RED_BED, &Block::STRAW_BED] {
        let fixture = PlayerFixture::new();
        publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
        let (foot, head) = place(&fixture.world, bed);
        for pos in [head.up().up(), foot.up().up()] {
            fixture.world.set_block_state(
                &pos,
                Block::STONE.default_state.id,
                BlockFlags::NOTIFY_ALL,
            );
        }
        let client = TestPlayer::new(&fixture.world);
        client.player.sleep(head);
        let lying = client.player.position();
        client.player.wake_up();
        let standing = client.player.position();
        assert_ne!(standing, lying);
        assert!(
            fixture
                .world
                .is_space_empty(client.player.get_entity().bounding_box.load()),
            "unsafe wake position for {}",
            bed.name
        );
        assert!(
            client
                .player
                .awaiting_teleport
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|(_, pos)| *pos == standing)
        );
        assert_eq!(
            fixture.world.get_block(&head).is_air(),
            bed == &Block::STRAW_BED
        );
        fixture.finish().await;
    }
}

#[tokio::test]
async fn monster_rest_predicate_exempts_ocelot_neutral_piglin_and_creative() {
    let fixture = PlayerFixture::new();
    publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    let (_, head) = place(&fixture.world, &Block::RED_BED);
    let client = TestPlayer::new(&fixture.world);
    let ocelot = Arc::new(Entity::new(
        fixture.world.clone(),
        head.to_centered_f64(),
        &EntityType::OCELOT,
    ));
    fixture.world.add_entity_silent(ocelot);
    let piglin = ZombifiedPiglinEntity::new(Entity::new(
        fixture.world.clone(),
        head.to_centered_f64(),
        &EntityType::ZOMBIFIED_PIGLIN,
    ));
    fixture.world.add_entity_silent(piglin.clone());
    assert!(!super::super::abstract_bed::monsters_prevent_sleep(
        &fixture.world,
        &client.player,
        head
    ));
    piglin.set_persistent_anger_target(Some(client.player.gameprofile.id));
    piglin.set_time_to_remain_angry(100);
    assert!(super::super::abstract_bed::monsters_prevent_sleep(
        &fixture.world,
        &client.player,
        head
    ));
    client.player.gamemode.store(GameMode::Creative);
    assert!(!super::super::abstract_bed::monsters_prevent_sleep(
        &fixture.world,
        &client.player,
        head
    ));
    fixture.finish().await;
}

#[tokio::test]
async fn bed_admission_uses_bottom_centres_vertical_two_and_suffocation() {
    let fixture = PlayerFixture::new();
    publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    let (foot, head) = place(&fixture.world, &Block::RED_BED);
    let client = TestPlayer::new(&fixture.world);
    fixture.world.level_time.lock().unwrap().time_of_day = 13_000;
    client
        .player
        .get_entity()
        .set_pos(Vector3::new(9.5, 64.0, 5.5));
    fixture.world.set_block_state(
        &head.up(),
        Block::GLASS.default_state.id,
        BlockFlags::NOTIFY_ALL,
    );
    assert!(
        super::super::abstract_bed::sleep_admission(&fixture.world, &client.player, head, foot)
            .is_none()
    );
    client
        .player
        .get_entity()
        .set_pos(Vector3::new(6.5, 66.01, 5.5));
    BedBlock::use_bed(&fixture.world, &client.player, &Block::RED_BED, &foot);
    assert!(client.player.sleeping_bed_pos.load().is_none());
    assert!(client.player.respawn_point.lock().unwrap().is_none());
    client
        .player
        .get_entity()
        .set_pos(Vector3::new(6.5, 64.0, 5.5));
    fixture.world.set_block_state(
        &head.up(),
        Block::STONE.default_state.id,
        BlockFlags::NOTIFY_ALL,
    );
    assert!(
        super::super::abstract_bed::sleep_admission(&fixture.world, &client.player, head, foot)
            .is_some()
    );
    fixture.finish().await;
}

#[tokio::test]
async fn bed_placement_rejects_head_across_border_for_both_species() {
    let fixture = PlayerFixture::new();
    publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    fixture.world.worldborder.lock().unwrap().center_x = 5.0;
    fixture.world.worldborder.lock().unwrap().center_z = 5.0;
    fixture.world.worldborder.lock().unwrap().new_diameter = 2.0;
    let client = TestPlayer::new(&fixture.world);
    client.player.get_entity().set_rotation(-90.0, 0.0);
    let foot = BlockPos::new(5, 64, 5);
    for block in [&Block::RED_BED, &Block::STRAW_BED] {
        let args = CanPlaceAtArgs {
            server: None,
            world: Some(&fixture.world),
            block_accessor: fixture.world.as_ref(),
            block,
            state: block.default_state,
            position: &foot,
            direction: None,
            player: Some(&client.player),
            use_item_on: None,
        };
        assert!(
            !fixture
                .world
                .block_registry
                .get_pumpkin_block(block.id)
                .unwrap()
                .can_place_at(args)
        );
        assert!(fixture.world.get_block(&foot).is_air());
    }
    fixture.finish().await;
}

#[tokio::test]
async fn straw_rule_independently_controls_sleep_spawn_and_leave_destruction() {
    let mut fixture = PlayerFixture::new();
    let dimension = &mut Arc::get_mut(&mut fixture.world).unwrap().dimension;
    dimension.bed_rule.can_sleep = BedRuleOption::Never;
    dimension.straw_bed_rule.can_sleep = BedRuleOption::Always;
    dimension.straw_bed_rule.can_set_spawn = BedRuleOption::Always;
    dimension.straw_bed_rule.destroy_on_leave = false;
    publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    let (foot, head) = place(&fixture.world, &Block::STRAW_BED);
    let client = TestPlayer::new(&fixture.world);
    client.player.get_entity().set_pos(foot.to_centered_f64());
    BedBlock::use_bed(&fixture.world, &client.player, &Block::STRAW_BED, &foot);
    assert_eq!(client.player.sleeping_bed_pos.load(), Some(head));
    assert!(client.player.respawn_point.lock().unwrap().is_some());
    client.player.wake_up();
    assert_eq!(fixture.world.get_block(&head), &Block::STRAW_BED);
    fixture.finish().await;
}
