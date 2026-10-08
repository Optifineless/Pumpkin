use pumpkin_data::translation;
use pumpkin_util::PermissionLvl;
use pumpkin_util::permission::{Permission, PermissionDefault, PermissionRegistry};
use pumpkin_util::text::TextComponent;
use pumpkin_world::world::BlockFlags;

use crate::command::argument_builder::{ArgumentBuilder, argument, command, literal};
use crate::command::argument_types::block_state::BlockStateArgumentType;
use crate::command::argument_types::coordinates::block_pos::BlockPosArgumentType;
use crate::command::context::command_context::CommandContext;
use crate::command::errors::error_types::CommandErrorType;
use crate::command::node::dispatcher::CommandDispatcher;
use crate::command::node::{CommandExecutor, CommandExecutorResult};

const DESCRIPTION: &str = "Place a block.";
const PERMISSION: &str = "minecraft:command.setblock";

const ERROR_FAILED: CommandErrorType<0> = CommandErrorType::new(
    translation::java::COMMANDS_SETBLOCK_FAILED,
    translation::java::COMMANDS_SETBLOCK_FAILED,
);

#[derive(Clone, Copy)]
enum Mode {
    /// with particles + item drops
    Destroy,

    /// only replaces air
    Keep,

    /// default; without particles
    Replace,

    /// places block without triggering updates around it
    Strict,
}

struct SetBlockExecutor(Mode);

impl CommandExecutor for SetBlockExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let block = BlockStateArgumentType::get(context, "block")?;
        let world = context.source.world();
        let pos = BlockPosArgumentType::get_loaded_block_pos(context, "pos")?;
        // SetBlockCommand.setBlock checks keep, destroys with drops, then calls BlockInput.place.
        if matches!(self.0, Mode::Keep) && !world.get_block_state(&pos).is_air() {
            return Err(ERROR_FAILED.create_without_context());
        }
        let destroy = matches!(self.0, Mode::Destroy);
        if destroy {
            world.break_block(&pos, None, BlockFlags::NOTIFY_ALL);
        }
        let old = world.get_block_state_id(&pos);
        let strict = matches!(self.0, Mode::Strict);
        let place_needed = !destroy
            || !pumpkin_data::Block::from_state_id(block.state).is_air()
            || !pumpkin_data::Block::from_state_id(old).is_air();
        let success = !place_needed || super::block_input::place(block, world, &pos, strict);
        if success && !strict {
            super::block_input::update_neighbors(world, &pos, old);
        }

        if success {
            world.flush_block_updates();
            context.source.send_feedback(
                TextComponent::translate_cross(
                    pumpkin_data::translation::java::COMMANDS_SETBLOCK_SUCCESS,
                    pumpkin_data::translation::bedrock::COMMANDS_SETBLOCK_SUCCESS,
                    [
                        TextComponent::text(pos.0.x.to_string()),
                        TextComponent::text(pos.0.y.to_string()),
                        TextComponent::text(pos.0.z.to_string()),
                    ],
                ),
                true,
            );
            Ok(1)
        } else {
            Err(ERROR_FAILED.create_without_context())
        }
    }
}

pub fn register(dispatcher: &mut CommandDispatcher, registry: &PermissionRegistry) {
    registry.register_permission_or_panic(Permission::new(
        PERMISSION,
        DESCRIPTION,
        PermissionDefault::Op(PermissionLvl::Two),
    ));

    dispatcher.register(
        command("setblock", DESCRIPTION).requires(PERMISSION).then(
            argument("pos", BlockPosArgumentType).then(
                argument("block", BlockStateArgumentType)
                    .executes(SetBlockExecutor(Mode::Replace))
                    .then(literal("destroy").executes(SetBlockExecutor(Mode::Destroy)))
                    .then(literal("keep").executes(SetBlockExecutor(Mode::Keep)))
                    .then(literal("replace").executes(SetBlockExecutor(Mode::Replace)))
                    .then(literal("strict").executes(SetBlockExecutor(Mode::Strict))),
            ),
        ),
    );
}
