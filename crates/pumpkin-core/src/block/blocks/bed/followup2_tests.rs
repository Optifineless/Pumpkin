use super::test_support::PlayerFixture;
use super::*;
use crate::{
    net::java::combat_test_support::TestPlayer,
    world::spawn_test_support::{proto, publish},
};
use pumpkin_data::{
    biome::Biome,
    block_properties::HorizontalFacing,
    dimension::BedRuleOption,
    entity::{EntityPose, EntityType},
};
use pumpkin_util::{math::vector3::Vector3, text::TextComponent};

fn place(world: &Arc<World>) -> (BlockPos, BlockPos) {
    let foot = BlockPos::new(5, 64, 5);
    let mut props = BedProperties::default(&Block::RED_BED);
    props.facing = HorizontalFacing::East;
    world.set_block_state(
        &foot,
        props.to_state_id(&Block::RED_BED),
        BlockFlags::NOTIFY_ALL,
    );
    (foot, foot.offset(props.facing.to_offset()))
}

#[tokio::test]
async fn followup2_bed_uses_custom_rule_message_and_respects_absent_message() {
    for message in [Some("{\"text\":\"Sleep is disabled here\"}"), None] {
        let mut fixture = PlayerFixture::new();
        let rule = &mut Arc::get_mut(&mut fixture.world).unwrap().dimension.bed_rule;
        rule.can_sleep = BedRuleOption::Never;
        rule.can_set_spawn = BedRuleOption::Never;
        rule.error_message = message;
        publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
        let (foot, _) = place(&fixture.world);
        let mut client = TestPlayer::new(&fixture.world);
        client.player.get_entity().set_pos(foot.to_centered_f64());
        client.take_packets();
        BedBlock::use_bed(&fixture.world, &client.player, &Block::RED_BED, &foot);
        assert!(!client.player.is_sleeping());
        let packets = client.take_packets();
        if message.is_some() {
            let text = TextComponent::text("Sleep is disabled here");
            let expected = client
                .client()
                .serialize_packet(
                    &pumpkin_protocol::java::client::play::CSystemChatMessage::new(&text, true),
                )
                .unwrap();
            assert_eq!(
                packets.iter().filter(|packet| **packet == expected).count(),
                1
            );
        } else {
            assert!(packets.is_empty());
        }
        fixture.finish().await;
    }
}

#[tokio::test]
async fn followup3_default_bed_denial_uses_each_editions_translation() {
    let fixture = PlayerFixture::new();
    publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    fixture.world.level_time.lock().unwrap().time_of_day = 6_000;
    let (foot, _) = place(&fixture.world);
    let mut java = TestPlayer::new(&fixture.world);
    java.player.get_entity().set_pos(foot.to_centered_f64());
    BedBlock::use_bed(&fixture.world, &java.player, &Block::RED_BED, &foot);
    let expected = java
        .client()
        .serialize_packet(
            &pumpkin_protocol::java::client::play::CSystemChatMessage::new(
                &pumpkin_macros::translate_java!(translation::java::BLOCK_MINECRAFT_BED_NO_SLEEP),
                true,
            ),
        )
        .unwrap();
    assert!(java.take_packets().contains(&expected));
    assert!(!java.player.is_sleeping());

    let mut bedrock =
        crate::net::bedrock::combat_test_support::TestBedrockPlayer::new(&fixture.world).await;
    bedrock.player.get_entity().set_pos(foot.to_centered_f64());
    BedBlock::use_bed(&fixture.world, &bedrock.player, &Block::RED_BED, &foot);
    let expected = bedrock
        .client()
        .serialize_packet(
            &pumpkin_protocol::bedrock::server::text::SText::translation(
                translation::bedrock::TILE_BED_NOSLEEP.to_string(),
                vec![],
            ),
        )
        .unwrap();
    assert!(bedrock.take_packets().contains(&expected));
    assert!(!bedrock.player.is_sleeping());
    bedrock.close().await;
    fixture.finish().await;
}

#[tokio::test]
async fn followup2_bed_kicks_only_a_villager_intersecting_its_head() {
    let fixture = PlayerFixture::new();
    publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    let (foot, head) = place(&fixture.world);
    let client = TestPlayer::new(&fixture.world);
    client.player.get_entity().set_pos(foot.to_centered_f64());
    let villager =
        crate::entity::passive::villager::VillagerEntity::new(crate::entity::Entity::new(
            fixture.world.clone(),
            Vector3::new(12.5, 64.0, 5.5),
            &EntityType::VILLAGER,
        ));
    *villager.home_pos.lock().unwrap() = Some(head);
    villager.get_entity().set_pose(EntityPose::Sleeping);
    fixture.world.add_entity_silent(villager.clone());
    BedBlock::set_occupied(
        true,
        &fixture.world,
        &Block::RED_BED,
        &head,
        fixture.world.get_block_state_id(&head),
    );
    BedBlock::use_bed(&fixture.world, &client.player, &Block::RED_BED, &head);
    assert!(
        villager.get_entity().pose.load() == EntityPose::Sleeping,
        "a distant sleeper was kicked"
    );
    villager.get_entity().set_pos(head.to_centered_f64());
    BedBlock::use_bed(&fixture.world, &client.player, &Block::RED_BED, &head);
    assert!(villager.get_entity().pose.load() == EntityPose::Standing);
    fixture.finish().await;
}
