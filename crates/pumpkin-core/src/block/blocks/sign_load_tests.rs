use super::*;
use crate::block::entities::sign::apply_trusted_item_data;
use crate::plugin::api::events::dialog::dialog_click_action::DialogClickActionEvent;
use crate::plugin::{EventHandler, EventPriority};
use pumpkin_data::data_component_impl::BlockEntityDataImpl;
use pumpkin_protocol::ser::NetworkReadExt;
use pumpkin_util::version::JavaMinecraftVersion;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

fn publish_sign(world: &Arc<World>) -> BlockPos {
    let mut chunk = spawn_test_support::proto(&Biome::PLAINS, &Block::STONE);
    chunk.set_block_state(0, 64, 1, Block::OAK_SIGN.default_state);
    chunk.set_block_state(0, 65, 1, Block::STONE.default_state);
    spawn_test_support::publish(world, chunk);
    BlockPos::new(0, 64, 1)
}

#[test]
fn sign_nbt_string_payload_is_literal_before_any_world_interaction() {
    let mut nbt = NbtCompound::new();
    nbt.put_list(
        "messages",
        vec![NbtTag::String(PAYLOAD.into()); Text::LINES],
    );
    let text = Text::from(NbtTag::Compound(nbt));
    for filtered in [false, true] {
        assert_eq!(text.get_message(0, filtered).as_ref(), PAYLOAD);
        assert!(!text.has_any_click_commands(filtered));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trusted_sign_nbt_string_payload_is_literal_and_cannot_run_commands() {
    let dir = tempfile::tempdir().unwrap();
    let (server, world) = test_world(dir.path());
    let pos = publish_sign(&world);
    let fixture = TestPlayer::new(&world);
    fixture
        .player
        .get_entity()
        .set_pos(Vector3::new(0.5, 64.0, 3.5));
    fixture.player.permission_lvl.store(PermissionLvl::Zero);
    let mut text = NbtCompound::new();
    text.put_list(
        "messages",
        vec![NbtTag::String(PAYLOAD.into()); Text::LINES],
    );
    text.put_list(
        "filtered_messages",
        vec![NbtTag::String(PAYLOAD.into()); Text::LINES],
    );
    for hanging in [false, true] {
        for trusted in [true, false] {
            let mut nbt = NbtCompound::new();
            nbt.put_compound("front_text", text.clone());
            nbt.put_compound("back_text", text.clone());
            if trusted {
                nbt.put_bool("allow_op_features", true);
            }
            let entity: Arc<dyn BlockEntity> = if hanging {
                Arc::new(HangingSignBlockEntity::from_nbt(&nbt, pos))
            } else {
                Arc::new(SignBlockEntity::from_nbt(&nbt, pos))
            };
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
            world.add_block_entity(entity.clone());
            click(&fixture, &server, pos);
            assert_eq!(fixture.player.gamemode.load(), GameMode::Survival);
            let sign = SignEntityRef::from_block_entity(&*entity).unwrap();
            assert_eq!(sign.allow_op_features(), trusted);
            for face in [sign.front_text(), sign.back_text()] {
                for filtered in [false, true] {
                    assert_eq!(face.get_message(0, filtered).as_ref(), PAYLOAD);
                    assert!(!face.has_any_click_commands(filtered));
                }
            }
        }
    }
    world.level.shutdown().await.unwrap();
}

#[derive(Default)]
struct ClickHandler(AtomicUsize);

impl EventHandler<DialogClickActionEvent> for ClickHandler {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<crate::server::Server>,
        event: &'a mut DialogClickActionEvent,
    ) -> futures::future::BoxFuture<'a, ()> {
        Box::pin(async move {
            assert_eq!(event.id, "minecraft:sign_test");
            assert_eq!(
                event.payload.as_deref(),
                Some(&[8, 0, 7, b'p', b'a', b'y', b'l', b'o', b'a', b'd'][..])
            );
            self.0.fetch_add(1, Relaxed);
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sign_dialog_and_custom_actions_require_trust_and_consume_the_click() {
    let dir = tempfile::tempdir().unwrap();
    let (server, world) = test_world(dir.path());
    let pos = publish_sign(&world);
    let mut fixture = TestPlayer::new(&world);
    fixture
        .player
        .get_entity()
        .set_pos(Vector3::new(0.5, 64.0, 3.5));
    let handler = Arc::new(ClickHandler::default());
    server.plugin_manager.register::<DialogClickActionEvent, _>(
        handler.clone(),
        EventPriority::Normal,
        true,
    );
    let mut dialog = NbtCompound::new();
    dialog.put_string("type", "minecraft:notice".to_string());
    dialog.put_string("title", "Sign dialog".to_string());
    let mut show = NbtCompound::new();
    show.put_string("action", "show_dialog".to_string());
    show.put_compound("dialog", dialog.clone());
    let mut custom = NbtCompound::new();
    custom.put_string("action", "custom".to_string());
    custom.put_string("id", "minecraft:sign_test".to_string());
    custom.put_string("payload", "payload".to_string());
    let lines = [show, custom].map(|event| {
        let mut line = NbtCompound::new();
        line.put_string("text", "click".to_string());
        line.put_compound("click_event", event);
        NbtTag::Compound(line)
    });
    let mut text = NbtCompound::new();
    text.put_list(
        "messages",
        vec![
            lines[0].clone(),
            lines[1].clone(),
            NbtTag::String("".into()),
            NbtTag::String("".into()),
        ],
    );
    for trusted in [false, true] {
        let mut nbt = NbtCompound::new();
        nbt.put_compound("front_text", text.clone());
        nbt.put_compound("back_text", text.clone());
        nbt.put_bool("allow_op_features", trusted);
        world.add_block_entity(Arc::new(SignBlockEntity::from_nbt(&nbt, pos)));
        fixture.take_packets();
        click(&fixture, &server, pos);
        let packets = fixture.take_packets();
        let mut overlays = 0;
        let mut dialogs = 0;
        for packet in packets {
            let mut bytes = packet.as_ref();
            let id = bytes.get_var_int().unwrap().0;
            if id == pumpkin_data::packet::clientbound::play::SYSTEM_CHAT.0 {
                let component = bytes
                    .get_compound_nbt_with_version(&JavaMinecraftVersion::V_26_3)
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    TextComponent::from_nbt(&NbtTag::Compound(component.clone())).get_text(),
                    "Click actions are disabled for this sign",
                );
                assert_eq!(component.get_string("color"), Some("red"));
                assert!(bytes.get_bool().unwrap());
                overlays += 1;
            } else if id == pumpkin_data::packet::clientbound::play::SHOW_DIALOG.0 {
                assert_eq!(bytes.get_var_int().unwrap().0, 0);
                assert_eq!(
                    bytes
                        .get_compound_nbt_with_version(&JavaMinecraftVersion::V_26_3)
                        .unwrap()
                        .unwrap(),
                    dialog
                );
                dialogs += 1;
            }
        }
        assert_eq!(overlays, usize::from(!trusted));
        assert_eq!(dialogs, usize::from(trusted));
        assert_eq!(handler.0.load(Relaxed), usize::from(trusted));
    }
    world.level.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sign_item_data_load_into_preserves_merged_fields_and_editor() {
    let dir = tempfile::tempdir().unwrap();
    let (server, world) = test_world(dir.path());
    let pos = publish_sign(&world);
    let fixture = TestPlayer::new(&world);
    fixture.player.gamemode.store(GameMode::Creative);
    fixture.player.permission_lvl.store(PermissionLvl::Two);
    for hanging in [false, true] {
        let mut nbt = command_sign_nbt(true);
        nbt.put_bool("is_waxed", true);
        let mut front = nbt.get_compound("front_text").unwrap().clone();
        front.put_string("color", "red".to_string());
        front.put_bool("has_glowing_text", true);
        nbt.put_compound("front_text", front);
        let entity: Arc<dyn BlockEntity> = if hanging {
            Arc::new(HangingSignBlockEntity::from_nbt(&nbt, pos))
        } else {
            Arc::new(SignBlockEntity::from_nbt(&nbt, pos))
        };
        let sign = SignEntityRef::from_block_entity(&*entity).unwrap();
        *sign.currently_editing_player().lock().unwrap() = Some(fixture.player.gameprofile.id);
        world.add_block_entity(entity.clone());
        let mut custom_data = NbtCompound::new();
        custom_data.put_string("id", entity.resource_location().to_string());
        let mut front = NbtCompound::new();
        front.put_list(
            "messages",
            vec![NbtTag::String("replacement".into()); Text::LINES],
        );
        custom_data.put_compound("front_text", front);
        let mut stack = ItemStack::new(1, &Item::OAK_SIGN);
        stack.set_data_component(BlockEntityDataImpl { nbt: custom_data });
        apply_trusted_item_data(&entity, &stack, &fixture.player, &world);
        let loaded = world.get_block_entity(&pos).unwrap();
        let loaded = SignEntityRef::from_block_entity(&*loaded).unwrap();
        assert_eq!(
            *loaded.currently_editing_player().lock().unwrap(),
            Some(fixture.player.gameprofile.id)
        );
        assert_eq!(
            loaded.front_text().get_message(0, false).as_ref(),
            "replacement"
        );
        assert_eq!(
            loaded.front_text().get_color(),
            pumpkin_data::dye_color::DyeColor::Red
        );
        assert!(loaded.front_text().has_glowing_text());
        assert!(loaded.back_text().has_any_click_commands(false));
        assert!(loaded.allow_op_features());
        assert!(loaded.is_waxed());
    }
    drop(server);
    world.level.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sign_item_unchanged_custom_data_does_not_reload() {
    let dir = tempfile::tempdir().unwrap();
    let (_server, world) = test_world(dir.path());
    let pos = publish_sign(&world);
    let fixture = TestPlayer::new(&world);
    fixture.player.gamemode.store(GameMode::Creative);
    fixture.player.permission_lvl.store(PermissionLvl::Two);
    let entity: Arc<dyn BlockEntity> = Arc::new(SignBlockEntity::empty(pos));
    world.add_block_entity(entity.clone());
    let mut nbt = NbtCompound::new();
    nbt.put_string("id", SignBlockEntity::ID.to_string());
    let mut stack = ItemStack::new(1, &Item::OAK_SIGN);
    stack.set_data_component(BlockEntityDataImpl { nbt });
    apply_trusted_item_data(&entity, &stack, &fixture.player, &world);
    assert!(Arc::ptr_eq(&entity, &world.get_block_entity(&pos).unwrap()));
    world.level.shutdown().await.unwrap();
}
