use crate::{
    block::entities::{BlockEntity, jukebox::JukeboxBlockEntity, sign::SignBlockEntity},
    entity::EntityBase,
    net::java::combat_test_support::TestPlayer,
    server::combat_test_support,
    world::spawn_test_support,
};
use pumpkin_data::{
    Block,
    biome::Biome,
    block_properties::{ComposterLikeProperties, SnowLikeProperties},
    data_component_impl::{
        BannerPatternLayer, BannerPatternsImpl, SuspiciousStewEffect, SuspiciousStewEffectsImpl,
    },
    dye_color::DyeColor,
    effect::StatusEffect,
    entity::EntityStatus,
    item::Item,
    item_stack::ItemStack,
    potion::Effect,
};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::{
    codec::var_int::VarInt,
    java::client::play::CEntityStatus,
    java::server::play::{SEditBook, SUpdateSign, SUseItemOn},
};
use pumpkin_util::{
    GameMode, Hand,
    math::{position::BlockPos, vector3::Vector3},
};
use std::{borrow::Cow, sync::Arc};

fn test_player(world: &Arc<crate::world::World>) -> TestPlayer {
    let fixture = TestPlayer::new(world);
    fixture
        .player
        .get_entity()
        .set_pos(Vector3::new(0.5, 64.0, 0.5));
    fixture
        .player
        .permission_lvl
        .store(pumpkin_util::permission::PermissionLvl::Four);
    fixture.player.advancements.try_lock().unwrap().player = Arc::downgrade(&fixture.player);
    fixture
}

fn use_block(
    fixture: &TestPlayer,
    server: &Arc<crate::server::Server>,
    position: BlockPos,
    hand: Hand,
) {
    assert!(
        fixture
            .client()
            .handle_use_item_on(
                &fixture.player,
                &SUseItemOn {
                    hand: VarInt(i32::from(hand != Hand::Right)),
                    position,
                    face: VarInt(1),
                    cursor_pos: Vector3::new(0.5, 1.0, 0.5),
                    inside_block: false,
                    is_against_world_border: false,
                    sequence: VarInt(1),
                },
                server
            )
            .is_ok()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hand_use_jukebox_consumes_survival_disc_and_preserves_creative_disc() {
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    let pos = BlockPos::new(0, 64, 1);
    let mut chunk = spawn_test_support::proto(&Biome::PLAINS, &Block::STONE);
    chunk.set_block_state(0, 64, 1, Block::JUKEBOX.default_state);
    spawn_test_support::publish(&world, chunk);
    let fixture = test_player(&world);
    for mode in [GameMode::Survival, GameMode::Creative] {
        world.set_block_state(
            &pos,
            Block::JUKEBOX.default_state.id,
            pumpkin_world::world::BlockFlags::NOTIFY_ALL,
        );
        world.add_block_entity(Arc::new(JukeboxBlockEntity::new(pos)));
        fixture.player.gamemode.store(mode);
        fixture
            .player
            .inventory()
            .set_stack(40, ItemStack::new(1, &Item::MUSIC_DISC_CAT));
        use_block(&fixture, &server, pos, Hand::Left);
        let held = fixture.player.inventory().off_hand_item();
        assert_eq!(held.item_count, u8::from(mode == GameMode::Creative));
        let entity = world.get_block_entity(&pos).unwrap();
        let mut nbt = pumpkin_nbt::compound::NbtCompound::new();
        entity.write_nbt(&mut nbt);
        assert!(
            nbt.get_compound("RecordItem")
                .or_else(|| nbt.get_compound("record_item"))
                .is_some()
        );
    }
    assert!(world.level.shutdown().await.is_ok());
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hand_use_composter_waiting_level_preserves_input() {
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    let pos = BlockPos::new(0, 64, 1);
    let mut chunk = spawn_test_support::proto(&Biome::PLAINS, &Block::STONE);
    chunk.set_block_state(0, 64, 1, Block::COMPOSTER.default_state);
    spawn_test_support::publish(&world, chunk);
    let fixture = test_player(&world);
    fixture
        .player
        .inventory()
        .set_stack(0, ItemStack::new(10, &Item::WHEAT_SEEDS));
    use_block(&fixture, &server, pos, Hand::Right);
    assert_eq!(fixture.player.inventory().held_item().item_count, 9);
    assert_eq!(
        ComposterLikeProperties::from_state_id(world.get_block_state_id(&pos)).level,
        1
    );
    let mut props = ComposterLikeProperties::default(&Block::COMPOSTER);
    props.level = 7;
    world.set_block_state(
        &pos,
        props.to_state_id(&Block::COMPOSTER),
        pumpkin_world::world::BlockFlags::NOTIFY_ALL,
    );
    use_block(&fixture, &server, pos, Hand::Right);
    assert_eq!(fixture.player.inventory().held_item().item_count, 9);
    assert!(world.level.shutdown().await.is_ok());
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hand_use_banner_washes_one_pattern_from_one_banner() {
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    let pos = BlockPos::new(0, 64, 1);
    let mut chunk = spawn_test_support::proto(&Biome::PLAINS, &Block::STONE);
    chunk.set_block_state(0, 64, 1, Block::WATER_CAULDRON.default_state);
    spawn_test_support::publish(&world, chunk);
    let fixture = test_player(&world);
    let mut banner = ItemStack::new(2, &Item::WHITE_BANNER);
    banner.set_data_component(BannerPatternsImpl {
        layers: vec![
            BannerPatternLayer {
                pattern: "minecraft:stripe_top".into(),
                color: DyeColor::Red,
            },
            BannerPatternLayer {
                pattern: "minecraft:stripe_bottom".into(),
                color: DyeColor::Blue,
            },
        ],
    });
    fixture.player.inventory().set_stack(0, banner);
    use_block(&fixture, &server, pos, Hand::Right);
    let held = fixture.player.inventory().held_item();
    assert_eq!(held.item_count, 1);
    assert_eq!(
        held.get_data_component::<BannerPatternsImpl>()
            .unwrap()
            .layers
            .len(),
        2
    );
    let clean = (1..36)
        .map(|slot| fixture.player.inventory().get_stack(slot))
        .find(|stack| stack.item == &Item::WHITE_BANNER)
        .unwrap();
    assert_eq!(clean.item_count, 1);
    assert_eq!(
        clean
            .get_data_component::<BannerPatternsImpl>()
            .unwrap()
            .layers
            .len(),
        1
    );
    assert_eq!(
        fixture.player.stats.lock().unwrap().get(
            pumpkin_data::statistic::StatisticCategory::Used,
            i32::from(Item::WHITE_BANNER.id),
        ),
        0
    );
    assert!(world.level.shutdown().await.is_ok());
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hand_use_snow_placement_consumes_and_keeps_eight_layers() {
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    let pos = BlockPos::new(0, 64, 1);
    let mut chunk = spawn_test_support::proto(&Biome::PLAINS, &Block::STONE);
    chunk.set_block_state(0, 64, 1, Block::SNOW.default_state);
    spawn_test_support::publish(&world, chunk);
    let fixture = test_player(&world);
    let inventory = fixture.player.inventory();
    inventory.set_stack(0, ItemStack::new(10, &Item::SNOW));
    fixture.player.start_cooldown("snow".to_owned(), 100);
    use_block(&fixture, &server, pos, Hand::Right);
    assert_eq!(world.get_block_state_id(&pos), Block::SNOW.default_state.id);
    assert_eq!(inventory.held_item().item_count, 10);
    fixture.player.start_cooldown("snow".to_owned(), 0);
    for _ in 0..7 {
        use_block(&fixture, &server, pos, Hand::Right);
    }
    assert_eq!(inventory.held_item().item_count, 3);
    assert_eq!(world.get_block(&pos), &Block::SNOW);
    assert_eq!(
        SnowLikeProperties::from_state_id(world.get_block_state_id(&pos)).layers,
        8
    );
    use_block(&fixture, &server, pos, Hand::Right);
    assert_eq!(world.get_block(&pos), &Block::SNOW);
    assert!(world.level.shutdown().await.is_ok());
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hand_use_milk_and_stew_finish_in_the_active_hand() {
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    let mut fixture = test_player(&world);
    let player = fixture.player.clone();
    let inventory = player.inventory();
    inventory.set_stack(0, ItemStack::new(1, &Item::DIAMOND_SWORD));
    let milk = ItemStack::new(1, &Item::MILK_BUCKET);
    inventory.set_stack(40, milk.clone());
    player.living_entity.add_effect(Effect {
        effect_type: &StatusEffect::POISON,
        duration: 200,
        amplifier: 0,
        ambient: false,
        show_particles: true,
        show_icon: true,
        blend: false,
    });
    player
        .living_entity
        .set_active_hand(Hand::Left, milk.clone(), 32);
    server.item_registry.on_stopped_using(&milk, &player);
    player.living_entity.clear_active_hand();
    assert!(player.living_entity.has_effect(&StatusEffect::POISON));
    assert_eq!(inventory.off_hand_item().item, &Item::MILK_BUCKET);
    player.living_entity.set_active_hand(Hand::Left, milk, 1);
    player
        .living_entity
        .updating_using_item(player.as_ref(), &server);
    assert!(!player.living_entity.has_effect(&StatusEffect::POISON));
    assert_eq!(inventory.off_hand_item().item, &Item::BUCKET);
    let packet = CEntityStatus::new(player.entity_id(), EntityStatus::UseItemComplete as i8);
    let expected = fixture.client().serialize_packet(&packet).unwrap();
    assert!(fixture.take_packets().contains(&expected));
    assert_eq!(inventory.held_item().item, &Item::DIAMOND_SWORD);
    let mut stew = ItemStack::new(1, &Item::SUSPICIOUS_STEW);
    stew.set_data_component(SuspiciousStewEffectsImpl {
        effects: Cow::Owned(vec![SuspiciousStewEffect {
            effect: Cow::Borrowed("minecraft:night_vision"),
            duration: 123,
        }]),
    });
    inventory.set_stack(40, stew.clone());
    player.living_entity.set_active_hand(Hand::Left, stew, 1);
    player
        .living_entity
        .updating_using_item(player.as_ref(), &server);
    assert!(player.living_entity.has_effect(&StatusEffect::NIGHT_VISION));
    assert_eq!(inventory.off_hand_item().item, &Item::BOWL);
    assert!(world.level.shutdown().await.is_ok());
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hand_use_changed_item_never_finishes_saved_food() {
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    let fixture = test_player(&world);
    let player = &fixture.player;
    let inventory = player.inventory();
    for replacement in [&Item::APPLE, &Item::DIAMOND_SWORD] {
        let apple = ItemStack::new(1, &Item::APPLE);
        inventory.set_stack(0, apple.clone());
        player.living_entity.set_active_hand(Hand::Right, apple, 1);
        inventory.set_stack(0, ItemStack::new(1, replacement));
        player.living_entity.complete_using_item(player.as_ref());
        assert_eq!(inventory.held_item().item, replacement);
        assert_eq!(inventory.held_item().item_count, 1);
        assert!(player.living_entity.active_hand.lock().unwrap().is_none());
    }
    assert!(world.level.shutdown().await.is_ok());
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hand_use_offhand_book_signing_preserves_components() {
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    let fixture = test_player(&world);
    let mut book = ItemStack::new(1, &Item::WRITABLE_BOOK);
    book.set_custom_name("Keepsake".to_owned());
    fixture
        .player
        .inventory()
        .set_stack(0, ItemStack::new(1, &Item::DIAMOND_SWORD));
    fixture.player.inventory().set_stack(40, book);
    fixture.client().handle_edit_book(
        &fixture.player,
        &SEditBook {
            slot: VarInt(40),
            pages: vec!["page"],
            title: Some("Title"),
        },
    );
    let signed = fixture.player.inventory().off_hand_item();
    assert_eq!(signed.item, &Item::WRITTEN_BOOK);
    assert!(
        signed
            .get_data_component::<pumpkin_data::data_component_impl::CustomNameImpl>()
            .is_some()
    );
    assert_eq!(
        fixture.player.inventory().held_item().item,
        &Item::DIAMOND_SWORD
    );
    assert!(world.level.shutdown().await.is_ok());
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hand_use_sign_requires_session_and_expires_distant_editor() {
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    let fixture = test_player(&world);
    let pos = BlockPos::new(0, 64, 1);
    let sign = Arc::new(SignBlockEntity::empty(pos));
    world.add_block_entity(sign.clone());
    let update = SUpdateSign {
        location: pos,
        is_front_text: true,
        line_1: "changed",
        line_2: "",
        line_3: "",
        line_4: "",
    };
    fixture
        .client()
        .handle_sign_update(&fixture.player, &update);
    assert_eq!(sign.front_text.get_message(0, false).as_ref(), "");
    *sign.currently_editing_player.lock().unwrap() = Some(fixture.player.gameprofile.id);
    fixture
        .client()
        .handle_sign_update(&fixture.player, &update);
    assert_eq!(sign.front_text.get_message(0, false).as_ref(), "changed");
    assert!(sign.currently_editing_player.lock().unwrap().is_none());
    *sign.currently_editing_player.lock().unwrap() = Some(fixture.player.gameprofile.id);
    fixture
        .player
        .get_entity()
        .set_pos(Vector3::new(100.0, 64.0, 100.0));
    sign.tick(&world);
    assert!(sign.currently_editing_player.lock().unwrap().is_none());
    assert!(world.level.shutdown().await.is_ok());
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hand_use_flower_pot_returns_the_consumed_plant() {
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    let pos = BlockPos::new(0, 64, 1);
    let mut chunk = spawn_test_support::proto(&Biome::PLAINS, &Block::STONE);
    chunk.set_block_state(0, 64, 1, Block::FLOWER_POT.default_state);
    spawn_test_support::publish(&world, chunk);
    let fixture = test_player(&world);
    let inventory = fixture.player.inventory();
    inventory.set_stack(0, ItemStack::new(0, &Item::DANDELION));
    use_block(&fixture, &server, pos, Hand::Right);
    assert_eq!(world.get_block(&pos), &Block::FLOWER_POT);
    inventory.set_stack(0, ItemStack::new(1, &Item::DANDELION));
    use_block(&fixture, &server, pos, Hand::Right);
    assert!(inventory.held_item().is_empty());
    assert_eq!(world.get_block(&pos), &Block::POTTED_DANDELION);
    use_block(&fixture, &server, pos, Hand::Right);
    assert_eq!(world.get_block(&pos), &Block::FLOWER_POT);
    assert_eq!(inventory.held_item().item, &Item::DANDELION);
    assert!(world.level.shutdown().await.is_ok());
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hand_use_stacked_remainders_drop_when_full_and_skip_creative() {
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    spawn_test_support::publish(
        &world,
        spawn_test_support::proto(&Biome::PLAINS, &Block::STONE),
    );
    let fixture = test_player(&world);
    let player = &fixture.player;
    for slot in 0..36 {
        player
            .inventory()
            .set_stack(slot, ItemStack::new(64, &Item::STONE));
    }
    for mode in [GameMode::Survival, GameMode::Creative] {
        player.gamemode.store(mode);
        let honey = ItemStack::new(2, &Item::HONEY_BOTTLE);
        player.inventory().set_stack(40, honey.clone());
        player.living_entity.set_active_hand(Hand::Left, honey, 1);
        player
            .living_entity
            .updating_using_item(player.as_ref(), &server);
        assert_eq!(
            player.inventory().off_hand_item().item_count,
            if mode == GameMode::Creative { 2 } else { 1 }
        );
    }
    let drops = world.get_entities_at_box(
        &pumpkin_util::math::boundingbox::BoundingBox::from_block(&BlockPos::new(0, 64, 0))
            .expand_all(4.0),
    );
    let count: u32 = drops
        .iter()
        .filter_map(|entity| {
            entity
                .cast_any()
                .downcast_ref::<crate::entity::item::ItemEntity>()
        })
        .map(|entity| {
            let stack = entity.get_item_stack().lock().unwrap();
            assert_eq!(stack.item, &Item::GLASS_BOTTLE);
            u32::from(stack.item_count)
        })
        .sum();
    assert_eq!(count, 1);
    assert!(world.level.shutdown().await.is_ok());
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hand_use_seed_advancement_matches_generated_crop_predicates() {
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    let fixture = test_player(&world);
    fixture.player.trigger_advancement(
        crate::entity::player::advancement::trigger::AdvancementTrigger::PlacedBlock {
            block_id: "minecraft:torchflower_crop".into(),
        },
    );
    assert!(
        fixture
            .player
            .has_advancement(pumpkin_data::Advancement::HUSBANDRY_PLANT_SEED)
    );
    assert!(
        fixture
            .player
            .has_advancement(pumpkin_data::Advancement::HUSBANDRY_PLANT_ANY_SNIFFER_SEED)
    );
    assert!(world.level.shutdown().await.is_ok());
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hand_use_cauldron_accepts_only_water_potions() {
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    let pos = BlockPos::new(0, 64, 1);
    let mut chunk = spawn_test_support::proto(&Biome::PLAINS, &Block::STONE);
    chunk.set_block_state(0, 64, 1, Block::CAULDRON.default_state);
    spawn_test_support::publish(&world, chunk);
    let fixture = test_player(&world);
    let mut potion = ItemStack::new(1, &Item::POTION);
    potion.set_data_component(pumpkin_data::data_component_impl::PotionContentsImpl {
        potion_id: Some(i32::from(pumpkin_data::potion::Potion::HEALING.id)),
        custom_effects: Vec::new(),
        custom_color: None,
        custom_name: None,
    });
    fixture.player.inventory().set_stack(0, potion.clone());
    use_block(&fixture, &server, pos, Hand::Right);
    assert_eq!(world.get_block(&pos), &Block::CAULDRON);
    assert!(fixture.player.inventory().held_item().are_equal(&potion));
    fixture
        .player
        .inventory()
        .set_stack(0, crate::item::items::glass_bottle::water_bottle());
    use_block(&fixture, &server, pos, Hand::Right);
    assert_eq!(world.get_block(&pos), &Block::WATER_CAULDRON);
    assert_eq!(
        fixture.player.inventory().held_item().item,
        &Item::GLASS_BOTTLE
    );
    assert_eq!(
        fixture.player.stats.lock().unwrap().get(
            pumpkin_data::statistic::StatisticCategory::Used,
            i32::from(Item::POTION.id)
        ),
        1
    );
    assert!(world.level.shutdown().await.is_ok());
    crate::server::fixture_lifecycle::finish().await;
}

mod honey_harvest;
