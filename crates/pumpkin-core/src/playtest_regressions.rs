use std::{sync::Arc, time::Duration};

use pumpkin_data::{
    Block, BlockDirection,
    block_properties::{DoubleBlockHalf, OakDoorLikeProperties},
    damage::DamageType,
    entity::EntityType,
    item::Item,
    item_stack::ItemStack,
    sound::Sound,
};
use pumpkin_inventory::{
    Inventory,
    screen_handler::{ScreenHandler, ScreenHandlerFactory},
};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::{
    VarInt,
    java::server::play::{SUseItemOn, SlotActionType},
    ser::NetworkReadExt,
};
use pumpkin_util::{
    GameMode,
    math::{position::BlockPos, vector2::Vector2, vector3::Vector3},
};
use pumpkin_world::{chunk::ChunkData, world::BlockFlags};

use crate::{
    entity::{EntityBase, passive::villager::VillagerEntity},
    net::java::combat_test_support::TestPlayer,
    server::combat_test_support,
};

#[derive(Default)]
struct BlockChangeEvents(std::sync::Mutex<Vec<String>>);

impl crate::plugin::EventHandler<crate::plugin::api::events::world::generic_game::GenericGameEvent>
    for BlockChangeEvents
{
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<crate::server::Server>,
        event: &'a mut crate::plugin::api::events::world::generic_game::GenericGameEvent,
    ) -> crate::plugin::BoxFuture<'a, ()> {
        self.0.lock().unwrap().push(event.event_key.clone());
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fletcher_selection_and_result_do_not_relock_the_menu() {
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    let fixture = TestPlayer::new(&world);
    let villager = crate::entity::r#type::from_type(
        &EntityType::VILLAGER,
        Vector3::new(0.0, 64.0, 0.0),
        &world,
        uuid::Uuid::new_v4(),
    );
    world.add_entity_silent(villager.clone());
    let merchant = villager
        .cast_any()
        .downcast_ref::<VillagerEntity>()
        .unwrap();
    let trade = &pumpkin_data::villager::TRADES_FLETCHER_LEVEL_1[0];
    *merchant.offers.lock().unwrap() = vec![pumpkin_protocol::java::client::play::MerchantOffer {
        base_cost_a: ItemStack::new(trade.wants.count as u8, trade.wants.item).into(),
        output: ItemStack::new(trade.gives.count as u8, trade.gives.item).into(),
        cost_b: None,
        reward_exp: true,
        uses: 0,
        max_uses: trade.max_uses,
        xp: trade.xp,
        special_price: 0,
        price_multiplier: trade.price_multiplier,
        demand: 0,
    }];
    let offer_index = 0;
    fixture
        .player
        .inventory()
        .set_stack(9, ItemStack::new(40, &Item::STICK));
    let handler = merchant
        .create_screen_handler(1, &fixture.player.inventory, fixture.player.as_ref())
        .unwrap();
    *fixture.player.current_screen_handler.lock().unwrap() = handler.clone();
    let player = fixture.player.clone();
    let runtime = tokio::runtime::Handle::current();
    let (send, receive) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _entered = runtime.enter();
        let mut guard = handler.lock().unwrap();
        let screen = guard.as_any_mut().downcast_mut::<
            pumpkin_inventory::merchant::merchant_screen_handler::MerchantScreenHandler,
        >().unwrap();
        screen.set_selected_offer(offer_index);
        assert_eq!(screen.inventory.get_stack(0).item_count, 40);
        assert_eq!(screen.inventory.get_stack(2).item, &Item::EMERALD);
        assert_eq!(screen.inventory.get_stack(2).item_count, 1);
        screen.on_slot_click(2, 0, SlotActionType::Pickup, player.as_ref());
        assert_eq!(screen.inventory.get_stack(0).item_count, 8);
        assert_eq!(screen.offers[offer_index].uses, 1);
        assert!(screen.inventory.get_stack(2).is_empty());
        assert_eq!(
            screen.get_behaviour().cursor_stack.lock().unwrap().item,
            &Item::EMERALD
        );
        send.send(()).unwrap();
    });
    receive.recv_timeout(Duration::from_secs(10)).unwrap();
    worker.join().unwrap();
    assert_eq!(merchant.offers.lock().unwrap()[offer_index].uses, 1);
    assert!(world.level.shutdown().await.is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn survival_boats_and_rafts_drop_their_item_once() {
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    let fixture = TestPlayer::new(&world);
    for (kind, item) in [
        (&EntityType::OAK_BOAT, &Item::OAK_BOAT),
        (&EntityType::OAK_CHEST_BOAT, &Item::OAK_CHEST_BOAT),
        (&EntityType::BAMBOO_RAFT, &Item::BAMBOO_RAFT),
        (&EntityType::BAMBOO_CHEST_RAFT, &Item::BAMBOO_CHEST_RAFT),
    ] {
        let boat = crate::entity::r#type::from_type(
            kind,
            Vector3::new(8.0, 64.0, 8.0),
            &world,
            uuid::Uuid::new_v4(),
        );
        world.add_entity_silent(boat.clone());
        for _ in 0..2 {
            boat.damage_with_context(
                boat.as_ref(),
                5.0,
                DamageType::PLAYER_ATTACK,
                None,
                Some(fixture.player.as_ref()),
                Some(fixture.player.as_ref()),
            );
        }
        let drops = world.entities.load_full();
        let count: u32 = drops
            .iter()
            .filter_map(|entity| entity.get_item_entity())
            .map(|drop| drop.get_item_stack().lock().unwrap().clone())
            .filter(|stack| stack.item == item)
            .map(|stack| u32::from(stack.item_count))
            .sum();
        assert_eq!(count, 1, "{}", kind.resource_name);
    }
    assert!(world.level.shutdown().await.is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chest_boat_contents_persist_and_drop_once_with_the_boat() {
    for (creative, entity_drops) in [(false, true), (true, true), (false, false)] {
        let dir = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(dir.path());
        let world = combat_test_support::world(&server, dir.path());
        let fixture = TestPlayer::new(&world);
        server.level_info.rcu(|info| {
            let mut info = (**info).clone();
            info.game_rules.entity_drops = entity_drops;
            info
        });
        fixture.player.gamemode.store(if creative {
            GameMode::Creative
        } else {
            GameMode::Survival
        });
        let boat = crate::entity::r#type::from_type(
            &EntityType::OAK_CHEST_BOAT,
            Vector3::new(8.0, 64.0, 8.0),
            &world,
            uuid::Uuid::new_v4(),
        );
        let mut stack = NbtCompound::new();
        ItemStack::new(3, &Item::DIAMOND).write_item_stack(&mut stack);
        stack.put_byte("Slot", 0);
        let mut loaded = NbtCompound::new();
        loaded.put(
            "Items",
            pumpkin_nbt::tag::NbtTag::List(vec![pumpkin_nbt::tag::NbtTag::Compound(stack)]),
        );
        boat.read_custom_nbt(&loaded);
        let mut saved = NbtCompound::new();
        boat.write_custom_nbt(&mut saved);
        assert_eq!(saved.get_list("Items").unwrap().len(), 1);
        world.add_entity_silent(boat.clone());
        for _ in 0..2 {
            boat.damage_with_context(
                boat.as_ref(),
                5.0,
                DamageType::PLAYER_ATTACK,
                None,
                Some(fixture.player.as_ref()),
                Some(fixture.player.as_ref()),
            );
        }
        let entities = world.entities.load_full();
        let first_drop = entities
            .iter()
            .find_map(|entity| entity.get_item_entity())
            .unwrap();
        assert_eq!(
            first_drop.get_item_stack().lock().unwrap().item.id,
            Item::DIAMOND.id
        );
        let diamonds: u32 = entities
            .iter()
            .filter_map(|entity| entity.get_item_entity())
            .map(|item| item.get_item_stack().lock().unwrap().clone())
            .filter(|stack| stack.item == &Item::DIAMOND)
            .map(|stack| u32::from(stack.item_count))
            .sum();
        assert_eq!(diamonds, 3);
        let boats: u32 = entities
            .iter()
            .filter_map(|entity| entity.get_item_entity())
            .map(|item| item.get_item_stack().lock().unwrap().clone())
            .filter(|stack| stack.item == &Item::OAK_CHEST_BOAT)
            .map(|stack| u32::from(stack.item_count))
            .sum();
        assert_eq!(boats, u32::from(!creative && entity_drops));
        assert!(world.level.shutdown().await.is_ok());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn boat_breaks_respect_creative_and_entity_drops() {
    for (creative, entity_drops) in [(true, true), (false, false)] {
        let dir = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(dir.path());
        let world = combat_test_support::world(&server, dir.path());
        server.level_info.rcu(|info| {
            let mut info = (**info).clone();
            info.game_rules.entity_drops = entity_drops;
            info
        });
        let fixture = TestPlayer::new(&world);
        fixture.player.gamemode.store(if creative {
            GameMode::Creative
        } else {
            GameMode::Survival
        });
        let boat = crate::entity::r#type::from_type(
            &EntityType::OAK_BOAT,
            Vector3::new(8.0, 64.0, 8.0),
            &world,
            uuid::Uuid::new_v4(),
        );
        world.add_entity_silent(boat.clone());
        assert!(boat.damage_with_context(
            boat.as_ref(),
            5.0,
            DamageType::PLAYER_ATTACK,
            None,
            Some(fixture.player.as_ref()),
            Some(fixture.player.as_ref())
        ));
        assert!(boat.get_entity().is_removed());
        assert!(
            world
                .entities
                .load()
                .iter()
                .all(|entity| entity.get_item_entity().is_none())
        );
        assert!(world.level.shutdown().await.is_ok());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn right_clicking_oak_log_strips_it_and_damages_the_axe_with_sound() {
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    world
        .level
        .loaded_chunks
        .insert(Vector2::new(0, 0), ChunkData::empty_sync(0, 0));
    let mut fixture = TestPlayer::new(&world);
    let mut observer = TestPlayer::new(&world);
    world.players.store(Arc::new(vec![
        fixture.player.clone(),
        observer.player.clone(),
    ]));
    let pos = BlockPos::new(0, 64, 1);
    fixture
        .player
        .get_entity()
        .set_pos(Vector3::new(0.5, 64.0, 0.5));
    observer
        .player
        .get_entity()
        .set_pos(Vector3::new(1.5, 64.0, 0.5));
    world.set_block_state(
        &pos,
        Block::OAK_LOG.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    fixture
        .player
        .inventory()
        .set_stack(0, ItemStack::new(1, &Item::IRON_AXE));
    fixture.take_packets();
    observer.take_packets();
    let events = Arc::new(BlockChangeEvents::default());
    server
        .plugin_manager
        .register(events.clone(), crate::plugin::EventPriority::Normal, true);
    fixture
        .client()
        .handle_use_item_on(
            &fixture.player,
            &SUseItemOn {
                hand: VarInt(0),
                position: pos,
                face: VarInt(BlockDirection::North as i32),
                cursor_pos: Vector3::new(0.5, 0.5, 0.0),
                inside_block: false,
                sequence: VarInt(1),
                is_against_world_border: false,
            },
            &server,
        )
        .unwrap();
    assert_eq!(world.get_block(&pos).id, Block::STRIPPED_OAK_LOG.id);
    assert_eq!(fixture.player.inventory().held_item().get_damage(), 1);
    assert_eq!(events.0.lock().unwrap().as_slice(), ["block_change"]);
    assert_eq!(
        fixture.player.stats.lock().unwrap().get(
            pumpkin_data::statistic::StatisticCategory::Used,
            i32::from(Item::IRON_AXE.id),
        ),
        1
    );
    let version = pumpkin_data::packet::CURRENT_MC_VERSION;
    let is_strip_sound = |packet: &bytes::Bytes| {
        let mut bytes = packet.as_ref();
        bytes.get_var_int().unwrap().0
            == pumpkin_data::packet::clientbound::play::SOUND.to_id(version)
            && bytes.get_var_int().unwrap().0 == i32::from(Sound::ItemAxeStrip as u16) + 1
    };
    assert!(observer.take_packets().iter().any(is_strip_sound));
    assert!(!fixture.take_packets().iter().any(is_strip_sound));
    assert!(world.level.shutdown().await.is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn waxed_copper_door_transforms_both_halves_without_drops() {
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    world
        .level
        .loaded_chunks
        .insert(Vector2::new(0, 0), ChunkData::empty_sync(0, 0));
    let fixture = TestPlayer::new(&world);
    let pos = BlockPos::new(0, 64, 1);
    fixture
        .player
        .get_entity()
        .set_pos(Vector3::new(0.5, 64.0, 0.5));
    fixture.player.get_entity().set_sneaking(true);
    world.set_block_state(
        &pos.down(),
        Block::STONE.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let mut properties = OakDoorLikeProperties::default(&Block::WAXED_COPPER_DOOR);
    for (position, half) in [
        (pos, DoubleBlockHalf::Lower),
        (pos.up(), DoubleBlockHalf::Upper),
    ] {
        properties.half = half;
        world.set_block_state(
            &position,
            properties.to_state_id(&Block::WAXED_COPPER_DOOR),
            BlockFlags::FORCE_STATE,
        );
    }
    fixture
        .player
        .inventory()
        .set_stack(0, ItemStack::new(1, &Item::IRON_AXE));
    fixture
        .client()
        .handle_use_item_on(
            &fixture.player,
            &SUseItemOn {
                hand: VarInt(0),
                position: pos,
                face: VarInt(BlockDirection::North as i32),
                cursor_pos: Vector3::new(0.5, 0.5, 0.0),
                inside_block: false,
                sequence: VarInt(1),
                is_against_world_border: false,
            },
            &server,
        )
        .unwrap();
    assert_eq!(world.get_block(&pos).id, Block::COPPER_DOOR.id);
    assert_eq!(world.get_block(&pos.up()).id, Block::COPPER_DOOR.id);
    assert_eq!(fixture.player.inventory().held_item().get_damage(), 1);
    assert!(
        world
            .entities
            .load()
            .iter()
            .all(|entity| entity.get_item_entity().is_none())
    );
    assert!(world.level.shutdown().await.is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn breaking_either_door_half_drops_one_in_survival_and_none_in_creative() {
    for creative in [false, true] {
        for half in [DoubleBlockHalf::Upper, DoubleBlockHalf::Lower] {
            let dir = tempfile::tempdir().unwrap();
            let server = combat_test_support::server(dir.path());
            let world = combat_test_support::world(&server, dir.path());
            world
                .level
                .loaded_chunks
                .insert(Vector2::new(0, 0), ChunkData::empty_sync(0, 0));
            let fixture = TestPlayer::new(&world);
            fixture
                .player
                .permission_lvl
                .store(pumpkin_util::permission::PermissionLvl::Four);
            fixture.player.gamemode.store(if creative {
                GameMode::Creative
            } else {
                GameMode::Survival
            });
            let pos = BlockPos::new(8, 64, 8);
            world.set_block_state(
                &pos.down(),
                Block::STONE.default_state.id,
                BlockFlags::FORCE_STATE,
            );
            let mut properties = OakDoorLikeProperties::default(&Block::OAK_DOOR);
            properties.half = DoubleBlockHalf::Lower;
            world.set_block_state(
                &pos,
                properties.to_state_id(&Block::OAK_DOOR),
                BlockFlags::FORCE_STATE,
            );
            properties.half = DoubleBlockHalf::Upper;
            world.set_block_state(
                &pos.up(),
                properties.to_state_id(&Block::OAK_DOOR),
                BlockFlags::FORCE_STATE,
            );
            let target = if half == DoubleBlockHalf::Lower {
                pos
            } else {
                pos.up()
            };
            let flags = if creative {
                BlockFlags::NOTIFY_ALL | BlockFlags::SKIP_DROPS
            } else {
                BlockFlags::NOTIFY_ALL
            };
            world
                .break_block(&target, Some(&fixture.player), flags)
                .unwrap();
            assert!(world.get_block_state(&pos).is_air());
            assert!(world.get_block_state(&pos.up()).is_air());
            let entities = world.entities.load_full();
            let count: u32 = entities
                .iter()
                .filter_map(|entity| entity.get_item_entity())
                .map(|drop| drop.get_item_stack().lock().unwrap().clone())
                .filter(|stack| stack.item == &Item::OAK_DOOR)
                .map(|stack| u32::from(stack.item_count))
                .sum();
            assert_eq!(
                count,
                u32::from(!creative),
                "creative={creative}, half={half:?}"
            );
            assert!(world.level.shutdown().await.is_ok());
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pvp_death_drops_start_fresh_and_survive_one_hundred_ticks() {
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    world
        .level
        .loaded_chunks
        .insert(Vector2::new(0, 0), ChunkData::empty_sync(0, 0));
    let victim = TestPlayer::new(&world);
    let killer = TestPlayer::new(&world);
    world
        .players
        .store(Arc::new(vec![victim.player.clone(), killer.player.clone()]));
    victim
        .player
        .get_entity()
        .set_pos(Vector3::new(8.0, 64.0, 8.0));
    world.set_block_state(
        &BlockPos::new(8, 63, 8),
        Block::STONE.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    victim
        .player
        .inventory()
        .set_stack(0, ItemStack::new(3, &Item::DIAMOND));
    victim.player.damage_with_context(
        victim.player.as_ref(),
        100.0,
        DamageType::PLAYER_ATTACK,
        None,
        Some(killer.player.as_ref()),
        Some(killer.player.as_ref()),
    );
    let drops: Vec<_> = world
        .entities
        .load_full()
        .iter()
        .filter(|entity| entity.get_item_entity().is_some())
        .cloned()
        .collect();
    assert_eq!(drops.len(), 1);
    let item = drops[0].get_item_entity().unwrap();
    let mut nbt = NbtCompound::new();
    item.write_custom_nbt(&mut nbt);
    assert_eq!(nbt.get_short("Age"), Some(0));
    assert_eq!(item.get_pickup_delay(), 40);
    world.players.store(Arc::new(Vec::new()));
    for _ in 0..100 {
        item.tick(item, &server);
    }
    item.write_custom_nbt(&mut nbt);
    assert_eq!(nbt.get_short("Age"), Some(100));
    assert_eq!(item.get_pickup_delay(), 0);
    assert!(item.get_entity().is_alive());
    assert_eq!(item.get_item_stack().lock().unwrap().item_count, 3);
    assert!(
        world
            .get_entity_by_id(item.get_entity().entity_id)
            .is_some()
    );
    assert_eq!(victim.player.living_entity.health.load(), 0.0);
    assert_eq!(victim.player.inventory().get_stack(0).item_count, 0);
    assert!(world.level.shutdown().await.is_ok());
}
