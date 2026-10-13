use super::*;
use crate::{
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::{
            player::player_statistic_increment::PlayerStatisticIncrementEvent,
            server::packet::PacketSentEvent,
        },
        player::item_held::PlayerItemHeldEvent,
    },
    server::Server,
};
use pumpkin_data::{packet::CURRENT_MC_VERSION, statistic::StatisticCategory};
use pumpkin_protocol::{
    MultiVersionJavaPacket,
    bedrock::{client::player_hotbar::CPlayerHotbar, server::SMobEquipment},
    java::client::play::CEntityStatus,
};
use pumpkin_util::version::JavaMinecraftVersion;
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};

struct SelectDuringDirectPublication {
    player: Arc<Player>,
    fired: AtomicBool,
}
impl EventHandler<PacketSentEvent> for SelectDuringDirectPublication {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PacketSentEvent,
    ) -> BoxFuture<'a, ()> {
        if event.packet_id == CSetEquipment::to_id(CURRENT_MC_VERSION)
            && !self.fired.swap(true, SeqCst)
        {
            self.player.inventory().set_selected_slot(4);
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn round4d_direct_equipment_corrects_callback_selection() -> TestResult {
    let mut fixture = HandUseFixture::new().await;
    let player = fixture.owner.player.clone();
    player.inventory().set_selected_slot(4);
    let live = ItemStack::new(3, &Item::DIAMOND);
    let sword = ItemStack::new(1, &Item::DIAMOND_SWORD);
    player.inventory().set_stack(4, live.clone());
    player.inventory().set_stack(8, sword.clone());
    let java = fixture.java_observer.client();
    java.set_player(fixture.java_observer.player.clone());
    java.version.store(JavaMinecraftVersion::V_26_2);
    let control = Arc::new(SelectDuringDirectPublication {
        player: player.clone(),
        fired: AtomicBool::new(false),
    });
    fixture
        .world
        .server
        .plugin_manager
        .register::<PacketSentEvent, _>(control.clone(), EventPriority::Normal, true);
    fixture.drain_packets();
    fixture.owner.client().handle_mob_equipment(
        &fixture.world.server,
        &player,
        &SMobEquipment {
            entity_runtime_id: (player.entity_id() as u64).into(),
            item: NetworkItemStackDescriptor::from(&sword),
            slot: 8,
            selected_slot: 8,
            container_id: 0,
        },
    );
    fixture
        .verify_and_close(|fixture| {
            assert!(control.fired.load(SeqCst));
            assert_eq!(player.inventory().get_selected_slot(), 4);
            assert_eq!(
                player
                    .living_entity
                    .get_attribute_value(&Attributes::ATTACK_DAMAGE),
                1.0
            );
            let hotbar = CPlayerHotbar {
                selected_slot: VarUInt(4),
                container_id: 0,
                should_select_slot: true,
            };
            assert!(
                fixture
                    .owner
                    .take_packets()
                    .contains(&fixture.owner.client().serialize_packet(&hotbar)?),
                "direct owner selection correction"
            );
            let java = CSetEquipment::new(
                player.entity_id().into(),
                vec![(
                    EquipmentSlot::MAIN_HAND.discriminant(),
                    ItemStackSerializer::from(live.clone()),
                )],
            );
            assert!(
                fixture
                    .java_observer
                    .take_packets()
                    .contains(&fixture.java_observer.client().serialize_packet(&java)?),
                "direct Java live hand"
            );
            let bedrock = CMobEquipment {
                target_runtime_id: (player.entity_id() as u64).into(),
                item: NetworkItemStackDescriptor::from(&live),
                slot: 4,
                selected_slot: 4,
                container_id: 0,
            };
            assert!(
                fixture.bedrock_observer.take_packets().contains(
                    &fixture
                        .bedrock_observer
                        .client()
                        .serialize_packet(&bedrock)?
                ),
                "direct Bedrock live hand"
            );
            Ok(())
        })
        .await
}

struct DenySelection;
impl EventHandler<PlayerItemHeldEvent> for DenySelection {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerItemHeldEvent,
    ) -> BoxFuture<'a, ()> {
        event.cancelled = true;
        Box::pin(async {})
    }
}

async fn rejected_before_placement(deny_selection: bool) -> TestResult {
    let mut fixture = HandUseFixture::new().await;
    let player = fixture.owner.player.clone();
    let (clicked, original) = prepare_hand_use(&fixture, BlockUse::Placement, Hand::Right, 2);
    let predicted = ItemStack::new(2, &Item::STONE);
    player.inventory().set_stack(8, predicted.clone());
    if deny_selection {
        fixture
            .world
            .server
            .plugin_manager
            .register::<PlayerItemHeldEvent, _>(
                Arc::new(DenySelection),
                EventPriority::Normal,
                true,
            );
    }
    let mut packet =
        click_block_packet(&player, HandSlot::Mainhand, clicked, &predicted, Vec::new());
    if deny_selection && let TransactionData::UseItem(data) = &mut packet.transaction_data {
        data.hot_bar_slot = VarInt(8);
    }
    fixture.drain_packets();
    fixture
        .owner
        .client()
        .handle_inventory_action(&player, packet);
    fixture
        .verify_and_close(|fixture| {
            assert_eq!(player.inventory().get_selected_slot(), 4);
            assert!(player.inventory().held_item().are_equal(&original));
            assert!(player.inventory().get_stack(8).are_equal(&predicted));
            assert_eq!(
                fixture.world.world().get_block(&BlockPos::new(8, 65, 8)),
                &Block::AIR
            );
            let owner = fixture.owner.take_packets();
            addendum::assert_block_corrections(fixture, &owner, clicked)
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn round4d_mismatched_descriptor_corrects_predicted_blocks() -> TestResult {
    rejected_before_placement(false).await
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn round4d_denied_selection_corrects_predicted_blocks() -> TestResult {
    rejected_before_placement(true).await
}

struct ReplaceDuringBrokenStatistic {
    replacement: ItemStack,
    fired: AtomicBool,
    saw_committed_break: AtomicBool,
}
impl EventHandler<PlayerStatisticIncrementEvent> for ReplaceDuringBrokenStatistic {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerStatisticIncrementEvent,
    ) -> BoxFuture<'a, ()> {
        if event.statistic_id == format!("Broken:{}", Item::FLINT_AND_STEEL.id) {
            self.fired.store(true, SeqCst);
            self.saw_committed_break
                .store(event.player.inventory().off_hand_item().is_empty(), SeqCst);
            event
                .player
                .inventory()
                .set_stack_in_hand(Hand::Left, self.replacement.clone());
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn round4d_broken_callback_observes_committed_final_durability_use() -> TestResult {
    let mut fixture = HandUseFixture::new().await;
    let player = fixture.owner.player.clone();
    let clicked = BlockPos::new(8, 64, 8);
    fixture.world.world().set_block_state(
        &clicked,
        Block::STONE.default_state.id,
        pumpkin_world::world::BlockFlags::FORCE_STATE,
    );
    let mut tool = ItemStack::new(1, &Item::FLINT_AND_STEEL);
    tool.set_damage(tool.get_max_damage().ok_or("damageable tool")? - 1);
    player
        .inventory()
        .set_stack_in_hand(Hand::Left, tool.clone());
    let replacement = ItemStack::new(3, &Item::DIAMOND);
    let control = Arc::new(ReplaceDuringBrokenStatistic {
        replacement: replacement.clone(),
        fired: AtomicBool::new(false),
        saw_committed_break: AtomicBool::new(false),
    });
    fixture
        .world
        .server
        .plugin_manager
        .register::<PlayerStatisticIncrementEvent, _>(control.clone(), EventPriority::Normal, true);
    fixture.drain_packets();
    fixture.owner.client().handle_inventory_action(
        &player,
        click_block_packet(&player, HandSlot::Offhand, clicked, &tool, Vec::new()),
    );
    fixture
        .verify_and_close(|fixture| {
            assert!(control.fired.load(SeqCst));
            assert!(
                control.saw_committed_break.load(SeqCst),
                "broken callback must follow successful atomic depletion"
            );
            assert_eq!(
                fixture.world.world().get_block(&BlockPos::new(8, 65, 8)),
                &Block::FIRE
            );
            let live = player.inventory().off_hand_item();
            assert!(live.are_equal(&replacement));
            assert_eq!(live.uid, replacement.uid);
            assert_eq!(
                player.stats.lock().unwrap().get(
                    StatisticCategory::Broken,
                    i32::from(Item::FLINT_AND_STEEL.id)
                ),
                1
            );
            let status = CEntityStatus::new(
                player.entity_id(),
                crate::entity::equipment_break_status(&EquipmentSlot::OFF_HAND) as i8,
            );
            let encoded = fixture.java_observer.client().serialize_packet(&status)?;
            let java = fixture.java_observer.take_packets();
            assert_eq!(
                java.iter().filter(|packet| **packet == encoded).count(),
                1,
                "one committed break animation"
            );
            let equipment = CSetEquipment::new(
                player.entity_id().into(),
                vec![(
                    EquipmentSlot::OFF_HAND.discriminant(),
                    ItemStackSerializer::from(replacement.clone()),
                )],
            );
            let equipment = fixture
                .java_observer
                .client()
                .serialize_packet(&equipment)?;
            assert!(
                java.iter().position(|packet| *packet == encoded)
                    < java.iter().position(|packet| *packet == equipment),
                "break precedes hand publication"
            );
            let owner = fixture.owner.take_packets();
            assert_owner_packet(fixture, &owner, Hand::Left, &replacement)
        })
        .await
}
