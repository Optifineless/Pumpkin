use super::*;
use crate::{
    block::entities::BlockEntity,
    command::{CommandSender, context::command_source::CommandSource},
    net::java::combat_test_support::TestPlayer,
    server::combat_test_support,
    world::spawn_test_support,
};
use pumpkin_data::{biome::Biome, item::Item, item_stack::ItemStack};
use pumpkin_inventory::Inventory;
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_protocol::{
    codec::var_int::VarInt,
    java::server::play::{SUpdateSign, SUseItemOn},
};
use pumpkin_util::math::vector2::Vector2;
use pumpkin_util::{GameMode, permission::PermissionLvl};

const PAYLOAD: &str =
    r#"{"text":"x","click_event":{"action":"run_command","command":"gamemode creative @s"}}"#;

#[path = "sign_load_tests.rs"]
mod sign_load_tests;

fn test_world(path: &std::path::Path) -> (Arc<crate::server::Server>, Arc<World>) {
    let mut server = combat_test_support::server(path);
    Arc::get_mut(&mut server)
        .unwrap()
        .basic_config
        .spawn_protection = 0;
    let world = combat_test_support::world(&server, path);
    server.worlds.store(Arc::new(vec![world.clone()]));
    (server, world)
}

fn click(fixture: &TestPlayer, server: &Arc<crate::server::Server>, pos: BlockPos) {
    fixture
        .player
        .inventory()
        .set_stack(0, ItemStack::EMPTY.clone());
    fixture
        .client()
        .handle_use_item_on(
            &fixture.player,
            &SUseItemOn {
                hand: VarInt(0),
                position: pos,
                face: VarInt(3),
                cursor_pos: Vector3::new(0.5, 0.5, 1.0),
                inside_block: false,
                is_against_world_border: false,
                sequence: VarInt(2),
            },
            server,
        )
        .unwrap();
}

fn command_sign_nbt(trusted: bool) -> NbtCompound {
    let mut text = NbtCompound::new();
    text.put_list(
        "messages",
        vec![
            NbtTag::Compound(
                serde_json::from_str::<TextComponent>(PAYLOAD)
                    .unwrap()
                    .0
                    .to_nbt_compound(),
            );
            Text::LINES
        ],
    );
    let mut nbt = NbtCompound::new();
    nbt.put_compound("front_text", text.clone());
    nbt.put_compound("back_text", text);
    nbt.put_bool("allow_op_features", trusted);
    nbt
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn player_written_sign_json_remains_literal_and_cannot_run_commands() {
    let dir = tempfile::tempdir().unwrap();
    let (server, world) = test_world(dir.path());
    spawn_test_support::publish(
        &world,
        spawn_test_support::proto(&Biome::PLAINS, &Block::STONE),
    );
    let fixture = TestPlayer::new(&world);
    let player = &fixture.player;
    player.get_entity().set_pos(Vector3::new(0.5, 64.0, 3.5));
    player.permission_lvl.store(PermissionLvl::Zero);
    let source = CommandSource::new(
        CommandSender::Player(player.clone()),
        world.clone(),
        Some(player.clone()),
        player.position(),
        Vector2::default(),
        player.gameprofile.name.clone(),
        player.get_display_name(),
        server.clone(),
    );
    server
        .command_dispatcher
        .load()
        .handle_command(&source, "gamemode creative @s");
    assert_eq!(player.gamemode.load(), GameMode::Survival);
    player
        .inventory()
        .set_stack(0, ItemStack::new(1, &Item::OAK_SIGN));
    fixture
        .client()
        .handle_use_item_on(
            player,
            &SUseItemOn {
                hand: VarInt(0),
                position: BlockPos::new(0, 63, 1),
                face: VarInt(1),
                cursor_pos: Vector3::new(0.5, 1.0, 0.5),
                inside_block: false,
                is_against_world_border: false,
                sequence: VarInt(1),
            },
            &server,
        )
        .unwrap();
    let pos = BlockPos::new(0, 64, 1);
    let entity = world.get_block_entity(&pos).unwrap();
    let sign = SignEntityRef::from_block_entity(&*entity).unwrap();
    assert_eq!(
        *sign.currently_editing_player().lock().unwrap(),
        Some(player.gameprofile.id)
    );
    let is_front_text = is_facing_front_text(&world, &pos, world.get_block(&pos), player);
    fixture.client().handle_sign_update(
        player,
        &SUpdateSign {
            location: pos,
            is_front_text,
            line_1: PAYLOAD,
            line_2: "",
            line_3: "",
            line_4: "",
        },
    );
    player.inventory().set_stack(0, ItemStack::EMPTY.clone());
    click(&fixture, &server, pos);
    assert_eq!(player.gamemode.load(), GameMode::Survival);
    assert_eq!(
        sign.get_text(is_front_text).get_message(0, false).as_ref(),
        PAYLOAD
    );
    assert!(!sign.get_text(is_front_text).has_any_click_commands(false));
    let mut saved = NbtCompound::new();
    entity.write_nbt(&mut saved);
    let loaded = SignBlockEntity::from_nbt(&saved, pos);
    assert!(!loaded.front_text.has_any_click_commands(false));
    assert!(!loaded.back_text.has_any_click_commands(false));
    world.level.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trusted_sign_commands_require_persisted_flag_independent_of_wax() {
    let dir = tempfile::tempdir().unwrap();
    let (server, world) = test_world(dir.path());
    let mut chunk = spawn_test_support::proto(&Biome::PLAINS, &Block::STONE);
    let state = Block::OAK_SIGN
        .from_properties(&[("rotation", "0")])
        .to_state_id(&Block::OAK_SIGN);
    chunk.set_block_state(0, 64, 1, state.to_state());
    chunk.set_block_state(0, 65, 1, Block::STONE.default_state);
    spawn_test_support::publish(&world, chunk);
    let fixture = TestPlayer::new(&world);
    fixture
        .player
        .get_entity()
        .set_pos(Vector3::new(0.5, 64.0, 3.5));
    fixture.player.permission_lvl.store(PermissionLvl::Zero);
    let pos = BlockPos::new(0, 64, 1);
    for waxed in [false, true] {
        for trusted in [false, true] {
            fixture.player.gamemode.store(GameMode::Survival);
            let mut nbt = command_sign_nbt(trusted);
            nbt.put_bool("is_waxed", waxed);
            for hanging in [false, true] {
                fixture.player.gamemode.store(GameMode::Survival);
                let block = if hanging {
                    &Block::OAK_HANGING_SIGN
                } else {
                    &Block::OAK_SIGN
                };
                world.set_block_state(
                    &pos,
                    block.default_state.id,
                    pumpkin_world::world::BlockFlags::NOTIFY_ALL,
                );
                let entity: Arc<dyn BlockEntity> = if hanging {
                    Arc::new(HangingSignBlockEntity::from_nbt(&nbt, pos))
                } else {
                    Arc::new(SignBlockEntity::from_nbt(&nbt, pos))
                };
                world.add_block_entity(entity);
                click(&fixture, &server, pos);
                assert_eq!(
                    fixture.player.gamemode.load(),
                    if trusted {
                        GameMode::Creative
                    } else {
                        GameMode::Survival
                    },
                    "trusted={trusted}, waxed={waxed}, hanging={hanging}"
                );
            }
        }
    }
    world.level.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sign_item_nbt_requires_creative_gamemaster_and_preserves_trust() {
    use pumpkin_data::data_component_impl::BlockEntityDataImpl;
    let dir = tempfile::tempdir().unwrap();
    let (server, world) = test_world(dir.path());
    spawn_test_support::publish(
        &world,
        spawn_test_support::proto(&Biome::PLAINS, &Block::STONE),
    );
    let fixture = TestPlayer::new(&world);
    fixture
        .player
        .get_entity()
        .set_pos(Vector3::new(0.5, 64.0, 3.5));
    for (x, mode, permission, allowed) in [
        (0, GameMode::Survival, PermissionLvl::Zero, false),
        (1, GameMode::Creative, PermissionLvl::Zero, false),
        (2, GameMode::Survival, PermissionLvl::Two, false),
        (3, GameMode::Creative, PermissionLvl::Two, true),
    ] {
        fixture.player.gamemode.store(mode);
        fixture.player.permission_lvl.store(permission);
        let mut nbt = command_sign_nbt(true);
        nbt.put_string("id", SignBlockEntity::ID.to_string());
        let mut stack = ItemStack::new(1, &Item::OAK_SIGN);
        stack.set_data_component(BlockEntityDataImpl { nbt });
        fixture.player.inventory().set_stack(0, stack);
        fixture
            .client()
            .handle_use_item_on(
                &fixture.player,
                &SUseItemOn {
                    hand: VarInt(0),
                    position: BlockPos::new(x, 63, 1),
                    face: VarInt(1),
                    cursor_pos: Vector3::new(0.5, 1.0, 0.5),
                    inside_block: false,
                    is_against_world_border: false,
                    sequence: VarInt(x + 1),
                },
                &server,
            )
            .unwrap();
        let pos = BlockPos::new(x, 64, 1);
        let entity = world.get_block_entity(&pos).unwrap();
        let mut saved = NbtCompound::new();
        entity.write_nbt(&mut saved);
        assert_eq!(
            saved.get_bool("allow_op_features").unwrap_or(false),
            allowed
        );
        let sign = SignEntityRef::from_block_entity(&*entity).unwrap();
        assert_eq!(sign.front_text().has_any_click_commands(false), allowed);
    }
    world.level.shutdown().await.unwrap();
}

#[test]
fn sign_allow_op_features_round_trips_through_nbt() {
    let pos = BlockPos::new(1, 64, 2);
    for trusted in [false, true] {
        let nbt = command_sign_nbt(trusted);
        let entities: [Arc<dyn BlockEntity>; 2] = [
            Arc::new(SignBlockEntity::from_nbt(&nbt, pos)),
            Arc::new(HangingSignBlockEntity::from_nbt(&nbt, pos)),
        ];
        for entity in entities {
            let mut saved = NbtCompound::new();
            entity.write_nbt(&mut saved);
            assert_eq!(
                saved.get_bool("allow_op_features").unwrap_or(false),
                trusted
            );
            assert_eq!(
                entity
                    .chunk_data_nbt()
                    .unwrap()
                    .get_bool("allow_op_features")
                    .unwrap_or(false),
                trusted
            );
            assert_eq!(saved.get_bool("is_waxed"), Some(false));
            let loaded = SignBlockEntity::from_nbt(&saved, pos);
            assert!(loaded.front_text.has_any_click_commands(false));
        }
    }
}

#[test]
fn sign_packet_lines_stay_literal_with_original_filtered_style_after_reload() {
    use pumpkin_util::text::color::NamedColor;
    let raw = TextComponent::text("raw").color_named(NamedColor::Red);
    let filtered = TextComponent::text("filtered").color_named(NamedColor::Blue);
    let mut nbt = NbtCompound::new();
    nbt.put_list(
        "messages",
        vec![NbtTag::Compound(raw.0.to_nbt_compound()); Text::LINES],
    );
    nbt.put_list(
        "filtered_messages",
        vec![NbtTag::Compound(filtered.0.to_nbt_compound()); Text::LINES],
    );
    let text = Text::from(NbtTag::Compound(nbt));
    text.update_messages([PAYLOAD, "", "", ""], true);
    assert_eq!(text.get_message(0, false).as_ref(), PAYLOAD);
    assert_eq!(text.get_messages(false), text.get_messages(true));
    assert_eq!(text.get_messages(false)[0].0.style, filtered.0.style);
    assert!(!text.has_any_click_commands(false));
    let saved = NbtTag::from(text);
    let loaded = Text::from(saved);
    assert_eq!(loaded.get_message(0, true).as_ref(), PAYLOAD);
    assert!(!loaded.has_any_click_commands(true));
}
