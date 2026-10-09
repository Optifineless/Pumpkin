use super::{Player, Text, TextComponent, World};
use crate::command::{CommandSender, context::command_source::CommandSource};
use crate::entity::EntityBase;
use crate::plugin::api::events::dialog::dialog_click_action::DialogClickActionEvent;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::{
    IdOr,
    java::client::{dialog::DialogNBT, play::CPlayShowDialog},
    ser::NetworkWriteExt,
};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector2::Vector2;
use std::sync::Arc;

// SignBlockEntity.executeClickCommandsIfPresent checks all three server click actions.
pub(super) fn execute_click_commands_if_present(
    world: &Arc<World>,
    player: &Arc<Player>,
    position: &BlockPos,
    text: &Text,
    allow_op_features: bool,
) -> bool {
    let Some(server) = world.server.upgrade() else {
        return false;
    };
    let mut has_click_action = false;
    for component in text.get_messages(player.is_text_filtering_enabled()) {
        // The component codec retains ShowDialog/Custom in opaque component NBT.
        let message = component.0.to_nbt_compound();
        let Some(event) = message.get_compound("click_event") else {
            continue;
        };
        let Some(action @ ("run_command" | "show_dialog" | "custom")) = event.get_string("action")
        else {
            continue;
        };
        has_click_action = true;
        if !allow_op_features {
            continue;
        }
        match action {
            "run_command" => {
                if let Some(command) = event.get_string("command") {
                    let source = CommandSource::new(
                        CommandSender::Dummy,
                        world.clone(),
                        Some(player.clone()),
                        position.to_centered_f64(),
                        Vector2::new(0.0, 0.0),
                        player.gameprofile.name.clone(),
                        player.get_display_name(),
                        server.clone(),
                    );
                    server
                        .command_dispatcher
                        .load()
                        .handle_command(&source, command.strip_prefix('/').unwrap_or(command));
                }
            }
            "show_dialog" => {
                if let Some(dialog) = event.get_compound("dialog") {
                    player.try_send_client_packet(&CPlayShowDialog::new(IdOr::Value(
                        DialogNBT::from_nbt(dialog),
                    )));
                }
            }
            "custom" => execute_custom_click_action(&server, player, event),
            _ => {}
        }
    }
    if has_click_action && !allow_op_features {
        player.send_system_message_raw(
            &TextComponent::translate("sign.click_actions_disabled", [])
                .color_named(pumpkin_util::text::color::NamedColor::Red),
            true,
        );
    }
    has_click_action
}

// MinecraftServer.handleCustomClickAction; use the existing plugin event contract.
fn execute_custom_click_action(
    server: &Arc<crate::server::Server>,
    player: &Arc<Player>,
    event: &NbtCompound,
) {
    let Some(id) = event.get_string("id") else {
        return;
    };
    let payload = if let Some(tag) = event.get("payload") {
        let mut bytes = Vec::new();
        if let Err(error) = bytes.write_nbt_with_version(
            Some(tag),
            &pumpkin_util::version::JavaMinecraftVersion::V_26_3,
        ) {
            tracing::warn!("Failed to encode sign click payload: {error}");
            return;
        }
        Some(bytes::Bytes::from(bytes))
    } else {
        None
    };
    let mut event = DialogClickActionEvent::new(player.clone(), id.to_string(), payload);
    server.plugin_manager.fire_blocking(server, &mut event);
}
