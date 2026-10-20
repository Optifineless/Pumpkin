#![expect(
    clippy::unwrap_used,
    reason = "Focused item regressions use valid in-memory server fixtures"
)]

use crate::{
    entity::{
        EntityBase, death_test_world::DeathTestWorld, projectile::ender_pearl::EnderPearlEntity,
    },
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        player::player_interact_entity_event::PlayerInteractEntityEvent,
    },
    server::Server,
};
use pumpkin_data::{
    data_component_impl::{CustomNameImpl, UseCooldownImpl},
    entity::EntityType,
    item::Item,
    item_stack::ItemStack,
    statistic::StatisticCategory,
};
use pumpkin_protocol::{
    codec::var_int::VarInt,
    java::{
        client::play::CItemCooldown,
        server::play::{SInteract, SUseItem},
    },
};
use pumpkin_util::{
    GameMode, Hand,
    math::{vector2::Vector2, vector3::Vector3},
    text::TextComponent,
};
use std::sync::{Arc, atomic::Ordering};

const PEARL_GROUP: &str = "test:entity_use_pearl";
const NAME_TAG_GROUP: &str = "test:entity_use_name_tag";

struct JavaEntityUseFixture {
    owner: DeathTestWorld,
    player: TestPlayer,
    target: Arc<dyn EntityBase>,
}

impl JavaEntityUseFixture {
    async fn new(target_type: &'static EntityType) -> Self {
        let owner = DeathTestWorld::new().await;
        let world = owner.world();
        crate::server::combat_test_support::publish_empty_chunk(&world, Vector2::new(0, 0));
        let player = TestPlayer::new(&world);
        player
            .player
            .get_entity()
            .set_pos(Vector3::new(8.5, 64.0, 8.5));
        let target = owner.mob(target_type);
        target.get_entity().set_pos(Vector3::new(8.5, 64.0, 10.5));
        Self {
            owner,
            player,
            target,
        }
    }

    fn server(&self) -> &Arc<Server> {
        &self.owner.server
    }

    fn interact(&self, hand: Hand, action: i32) {
        self.player.client().handle_interact(
            &self.player.player,
            &SInteract {
                entity_id: VarInt(self.target.get_entity().entity_id),
                r#type: VarInt(action),
                target_position: (action == 2).then_some(Vector3::new(0.0, 0.0, 0.0)),
                hand: Some(VarInt(i32::from(hand == Hand::Left))),
                sneaking: false,
            },
            self.server(),
        );
    }

    fn use_item(&self, hand: Hand) {
        self.player.client().handle_use_item(
            &self.player.player,
            &SUseItem {
                hand: VarInt(i32::from(hand == Hand::Left)),
                sequence: VarInt(1),
                yaw: 0.0,
                pitch: 0.0,
            },
            self.server(),
        );
    }

    fn projectile_count(&self) -> usize {
        self.owner
            .world()
            .entities
            .load()
            .iter()
            .filter(|entity| entity.cast_any().is::<EnderPearlEntity>())
            .count()
    }

    fn cooldown_packet(&self, group: &str) -> bytes::Bytes {
        self.player
            .client()
            .serialize_packet(&CItemCooldown::new(group.to_owned(), VarInt(20)))
            .unwrap()
    }

    fn take_cooldown_packets(&mut self, expected: &bytes::Bytes) -> usize {
        self.player
            .take_packets()
            .iter()
            .filter(|packet| *packet == expected)
            .count()
    }

    fn used_stat(&self, item: &Item) -> i32 {
        self.player
            .player
            .stats
            .lock()
            .unwrap()
            .get(StatisticCategory::Used, i32::from(item.id))
    }

    fn clear_cooldown(&self, group: &str) {
        self.player
            .player
            .item_cooldowns
            .lock()
            .unwrap()
            .remove(group);
    }

    async fn finish(self) {
        self.owner.server.shutdown().await;
    }
}

fn pearl_stack(count: u8, group: &str) -> ItemStack {
    let mut stack = ItemStack::new(count, &Item::ENDER_PEARL);
    stack.set_data_component(UseCooldownImpl::new(1.0, Some(group.to_owned())));
    stack
}

fn name_tag_stack(count: u8, name: Option<&str>, group: &str) -> ItemStack {
    let mut stack = ItemStack::new(count, &Item::NAME_TAG);
    if let Some(name) = name {
        stack.set_data_component(CustomNameImpl {
            name: TextComponent::text(name.to_owned()),
        });
    }
    stack.set_data_component(UseCooldownImpl::new(1.0, Some(group.to_owned())));
    stack
}

fn held_hand(player: &crate::entity::player::Player, hand: Hand) -> ItemStack {
    player.inventory().get_stack_in_hand(hand)
}

fn name_of(entity: &dyn EntityBase) -> Option<String> {
    entity
        .get_entity()
        .custom_name
        .load()
        .as_ref()
        .as_ref()
        .map(|name| name.clone().get_text())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g03_passing_entity_interaction_allows_one_pearl_throw() {
    let mut fixture = JavaEntityUseFixture::new(&EntityType::COW).await;
    let player = fixture.player.player.clone();
    let expected_cooldown = fixture.cooldown_packet(PEARL_GROUP);
    let mut failures = Vec::new();

    for action in [0, 2] {
        for hand in [Hand::Right, Hand::Left] {
            for mode in [GameMode::Survival, GameMode::Creative] {
                fixture.clear_cooldown(PEARL_GROUP);
                player.gamemode.store(mode);
                let pearl = pearl_stack(2, PEARL_GROUP);
                let control = ItemStack::new(3, &Item::STONE);
                let other_hand = if hand == Hand::Right {
                    Hand::Left
                } else {
                    Hand::Right
                };
                player.inventory().set_stack_in_hand(hand, pearl.clone());
                player
                    .inventory()
                    .set_stack_in_hand(other_hand, control.clone());
                let stat_before = fixture.used_stat(&Item::ENDER_PEARL);
                let projectiles_before = fixture.projectile_count();

                fixture.interact(hand, action);
                let entity_only = (
                    fixture.projectile_count() == projectiles_before,
                    held_hand(&player, hand).are_equal(&pearl),
                    held_hand(&player, other_hand).are_equal(&control),
                    !player.is_on_cooldown(PEARL_GROUP),
                    fixture.used_stat(&Item::ENDER_PEARL) == stat_before,
                );
                let interaction_cooldown_packets =
                    fixture.take_cooldown_packets(&expected_cooldown);

                fixture.use_item(hand);
                let throw_result = (
                    fixture.projectile_count() == projectiles_before + 1,
                    held_hand(&player, hand).item_count
                        == if mode == GameMode::Survival { 1 } else { 2 },
                    held_hand(&player, other_hand).are_equal(&control),
                    player.is_on_cooldown(PEARL_GROUP),
                    fixture.used_stat(&Item::ENDER_PEARL) == stat_before + 1,
                );
                let throw_cooldown_packets = fixture.take_cooldown_packets(&expected_cooldown);

                if !entity_only.0
                    || !entity_only.1
                    || !entity_only.2
                    || !entity_only.3
                    || !entity_only.4
                    || interaction_cooldown_packets != 0
                {
                    failures.push(format!(
                        "entity stage action={action} hand={hand:?} mode={mode:?}: {entity_only:?}, cooldown packets={interaction_cooldown_packets}"
                    ));
                }
                if !throw_result.0
                    || !throw_result.1
                    || !throw_result.2
                    || !throw_result.3
                    || !throw_result.4
                    || throw_cooldown_packets != 1
                {
                    failures.push(format!(
                        "throw stage action={action} hand={hand:?} mode={mode:?}: {throw_result:?}, cooldown packets={throw_cooldown_packets}"
                    ));
                }
            }
        }
    }

    assert!(
        failures.is_empty(),
        "pearl fallback mismatch: {failures:#?}"
    );
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g03_air_aimed_pearl_throw_keeps_ordinary_cooldown() {
    let mut fixture = JavaEntityUseFixture::new(&EntityType::COW).await;
    let player = fixture.player.player.clone();
    fixture
        .target
        .get_entity()
        .set_pos(Vector3::new(40.0, 64.0, 40.0));
    player
        .inventory()
        .set_stack_in_hand(Hand::Right, pearl_stack(2, PEARL_GROUP));
    let expected_cooldown = fixture.cooldown_packet(PEARL_GROUP);

    fixture.use_item(Hand::Right);

    assert_eq!(fixture.projectile_count(), 1, "air use inserts one pearl");
    assert_eq!(held_hand(&player, Hand::Right).item_count, 1);
    assert!(player.is_on_cooldown(PEARL_GROUP));
    assert_eq!(fixture.used_stat(&Item::ENDER_PEARL), 1);
    assert_eq!(fixture.take_cooldown_packets(&expected_cooldown), 1);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g03_successful_entity_hook_does_not_invent_use_cooldown() {
    let mut fixture = JavaEntityUseFixture::new(&EntityType::COW).await;
    let player = fixture.player.player.clone();
    let mut failures = Vec::new();

    for mode in [GameMode::Survival, GameMode::Creative] {
        fixture.clear_cooldown(NAME_TAG_GROUP);
        player.gamemode.store(mode);
        let target = fixture.owner.mob(&EntityType::COW);
        target.get_entity().set_pos(Vector3::new(8.5, 64.0, 10.5));
        let stack = name_tag_stack(2, Some("Named by hook"), NAME_TAG_GROUP);
        player
            .inventory()
            .set_stack_in_hand(Hand::Right, stack.clone());
        let stat_before = fixture.used_stat(&Item::NAME_TAG);

        fixture.target = target.clone();
        fixture.interact(Hand::Right, 2);

        let named = name_of(target.as_ref()).as_deref() == Some("Named by hook");
        let persistent = target.get_mob().is_some_and(|mob| {
            mob.get_mob_entity()
                .persistence_required
                .load(Ordering::Relaxed)
        });
        let paid_count = held_hand(&player, Hand::Right).item_count;
        let expected_count = if mode == GameMode::Survival { 1 } else { 2 };
        let expected_stat = stat_before + i32::from(mode == GameMode::Survival);
        let cooldown_started = player.is_on_cooldown(NAME_TAG_GROUP);
        if !named
            || !persistent
            || paid_count != expected_count
            || fixture.used_stat(&Item::NAME_TAG) != expected_stat
            || cooldown_started
        {
            failures.push(format!(
                "successful name tag mode={mode:?}: named={named}, persistent={persistent}, count={paid_count}, stat={}, cooldown={cooldown_started}",
                fixture.used_stat(&Item::NAME_TAG)
            ));
        }
    }

    for (target_type, name, case) in [
        (&EntityType::COW, None, "unnamed tag"),
        (
            &EntityType::FISHING_BOBBER,
            Some("Must not apply"),
            "ineligible target",
        ),
    ] {
        fixture.clear_cooldown(NAME_TAG_GROUP);
        let target = fixture.owner.mob(target_type);
        let stack = name_tag_stack(2, name, NAME_TAG_GROUP);
        player
            .inventory()
            .set_stack_in_hand(Hand::Right, stack.clone());
        let stat_before = fixture.used_stat(&Item::NAME_TAG);
        fixture.target = target.clone();
        fixture.interact(Hand::Right, 0);

        let stock_preserved = held_hand(&player, Hand::Right).are_equal(&stack);
        let target_unnamed = name_of(target.as_ref()).is_none();
        let cooldown_started = player.is_on_cooldown(NAME_TAG_GROUP);
        let stat_delta = fixture.used_stat(&Item::NAME_TAG) - stat_before;
        if !stock_preserved || !target_unnamed || cooldown_started || stat_delta != 0 {
            failures.push(format!(
                "{case}: stock_preserved={stock_preserved}, target_unnamed={target_unnamed}, cooldown_started={cooldown_started}, stat_delta={stat_delta}"
            ));
        }
    }

    assert!(failures.is_empty(), "entity hook mismatch: {failures:#?}");
    fixture.finish().await;
}

struct CancelEntityInteraction;

impl EventHandler<PlayerInteractEntityEvent> for CancelEntityInteraction {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerInteractEntityEvent,
    ) -> BoxFuture<'a, ()> {
        event.cancelled = true;
        Box::pin(async {})
    }
}

fn active_cooldown_denies_second_pearl(fixture: &JavaEntityUseFixture, group: &str) {
    let player = &fixture.player.player;
    player
        .inventory()
        .set_stack_in_hand(Hand::Right, pearl_stack(2, group));
    fixture.use_item(Hand::Right);
    let projectile_count = fixture.projectile_count();
    let stock = held_hand(player, Hand::Right).item_count;
    let active = player.is_on_cooldown(group);
    fixture.use_item(Hand::Right);

    assert!(
        active,
        "successful ordinary throw starts the configured cooldown"
    );
    assert_eq!(projectile_count, 1, "first throw inserts one pearl");
    assert_eq!(stock, 1, "first Survival throw pays one pearl");
    assert_eq!(fixture.projectile_count(), projectile_count);
    assert_eq!(held_hand(player, Hand::Right).item_count, stock);
    assert_eq!(fixture.used_stat(&Item::ENDER_PEARL), 1);
}

fn cancelled_entity_interaction_has_no_item_effects(
    fixture: &mut JavaEntityUseFixture,
) -> Arc<dyn EntityBase> {
    let player = &fixture.player.player;
    fixture.clear_cooldown(NAME_TAG_GROUP);
    fixture
        .owner
        .server
        .plugin_manager
        .register::<PlayerInteractEntityEvent, _>(
            Arc::new(CancelEntityInteraction),
            EventPriority::Normal,
            true,
        );
    let tag = name_tag_stack(2, Some("Cancelled"), NAME_TAG_GROUP);
    player
        .inventory()
        .set_stack_in_hand(Hand::Right, tag.clone());
    let target = fixture.owner.mob(&EntityType::COW);
    fixture.target = target.clone();
    let stat_before = fixture.used_stat(&Item::NAME_TAG);
    fixture.interact(Hand::Right, 2);

    assert!(
        name_of(target.as_ref()).is_none(),
        "cancelled name tag has no effect"
    );
    assert!(held_hand(player, Hand::Right).are_equal(&tag));
    assert!(!player.is_on_cooldown(NAME_TAG_GROUP));
    assert_eq!(fixture.used_stat(&Item::NAME_TAG), stat_before);
    target
}

fn close_pearl_impact_preserves_same_world_teleport(
    fixture: &JavaEntityUseFixture,
    target: &dyn EntityBase,
    original_target: &dyn EntityBase,
    group: &str,
) {
    let player = &fixture.player.player;
    fixture.clear_cooldown(group);
    original_target
        .get_entity()
        .set_pos(Vector3::new(40.0, 64.0, 40.0));
    let start = Vector3::new(8.5, 64.0, 8.5);
    player.get_entity().set_pos(start);
    player.get_entity().set_rotation(0.0, 0.0);
    target.get_entity().set_pos(Vector3::new(8.5, 64.5, 10.8));
    player
        .inventory()
        .set_stack_in_hand(Hand::Right, pearl_stack(2, group));
    let before_ids: Vec<_> = fixture
        .owner
        .world()
        .entities
        .load()
        .iter()
        .map(|entity| entity.get_entity().entity_id)
        .collect();
    let projectile_count_before = fixture.projectile_count();
    fixture.use_item(Hand::Right);
    let inserted_projectiles = fixture.projectile_count() - projectile_count_before;
    let stock_after_throw = held_hand(player, Hand::Right).item_count;
    let cooldown_after_throw = player.is_on_cooldown(group);
    let projectile = fixture
        .owner
        .world()
        .entities
        .load()
        .iter()
        .find(|entity| {
            entity.cast_any().is::<EnderPearlEntity>()
                && !before_ids.contains(&entity.get_entity().entity_id)
        })
        .cloned()
        .unwrap();
    for _ in 0..4 {
        if projectile.get_entity().is_removed() {
            break;
        }
        projectile.tick(projectile.as_ref(), &fixture.owner.server);
    }
    let pearl = projectile
        .cast_any()
        .downcast_ref::<EnderPearlEntity>()
        .unwrap();
    let hit = pearl.thrown.has_hit.load(Ordering::Relaxed);
    let teleported = player.get_entity().pos.load() != start;
    let impact_health = player.living_entity.health.load();

    assert_eq!(inserted_projectiles, 1, "real use inserts one impact pearl");
    assert_eq!(stock_after_throw, 1, "impact throw pays exactly one pearl");
    assert!(
        cooldown_after_throw,
        "cooldown starts on the ordinary throw"
    );
    assert!(
        hit,
        "nearby target receives the normal pearl impact; projectile={:?}, target={:?}",
        projectile.get_entity().pos.load(),
        target.get_entity().bounding_box.load()
    );
    assert!(teleported, "same-world impact teleports the pearl owner");
    assert_eq!(impact_health, 15.0, "same-world impact keeps pearl damage");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g03_entity_use_preserves_active_cooldown_cancel_and_immediate_impact() {
    let mut fixture = JavaEntityUseFixture::new(&EntityType::COW).await;
    let original_target = fixture.target.clone();
    let pearl_group = "test:active_pearl";
    active_cooldown_denies_second_pearl(&fixture, pearl_group);
    let impact_target = cancelled_entity_interaction_has_no_item_effects(&mut fixture);
    close_pearl_impact_preserves_same_world_teleport(
        &fixture,
        impact_target.as_ref(),
        original_target.as_ref(),
        pearl_group,
    );
    fixture.finish().await;
}
