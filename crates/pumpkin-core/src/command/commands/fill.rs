use pumpkin_data::BlockStateId;
use pumpkin_data::translation;
use pumpkin_util::PermissionLvl;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::permission::{Permission, PermissionDefault, PermissionRegistry};
use pumpkin_util::text::TextComponent;
use pumpkin_world::world::BlockFlags;

use crate::command::argument_builder::{
    ArgumentBuilder, RequiredArgumentBuilder, argument, command, literal,
};
use crate::command::argument_types::block_predicate::{BlockPredicate, BlockPredicateArgumentType};
use crate::command::argument_types::block_state::{BlockInput, BlockStateArgumentType};
use crate::command::argument_types::coordinates::block_pos::BlockPosArgumentType;
use crate::command::context::command_context::CommandContext;
use crate::command::context::command_source::CommandSource;
use crate::command::errors::command_syntax_error::CommandSyntaxError;
use crate::command::errors::error_types::CommandErrorType;
use crate::command::node::dispatcher::CommandDispatcher;
use crate::command::node::{CommandExecutor, CommandExecutorResult};

const DESCRIPTION: &str = "Fills all or parts of a region with a specific block.";
const PERMISSION: &str = "minecraft:command.fill";

const ERROR_AREA_TOO_LARGE: CommandErrorType<2> = CommandErrorType::new(
    translation::java::COMMANDS_FILL_TOOBIG,
    translation::java::COMMANDS_FILL_TOOBIG,
);

const ERROR_FAILED: CommandErrorType<0> = CommandErrorType::new(
    translation::java::COMMANDS_FILL_FAILED,
    translation::java::COMMANDS_FILL_FAILED,
);

#[derive(Clone, Copy, PartialEq, Eq)]
enum FillMode {
    Replace,
    Outline,
    Hollow,
    Destroy,
    Keep,
}

#[derive(Clone, Copy)]
enum FilterMode {
    None,
    WithFilter,
    KeepAir,
}

// FillCommand.fillBlocks places synchronously, with MAX_BLOCK_MODIFICATIONS as its bound.
fn fill_volume(from: BlockPos, to: BlockPos) -> i128 {
    let span = |a: i32, b: i32| (i128::from(a) - i128::from(b)).abs() + 1;
    span(from.0.x, to.0.x) * span(from.0.y, to.0.y) * span(from.0.z, to.0.z)
}

fn check_region_loaded(
    source: &CommandSource,
    min: Vector3<i32>,
    max: Vector3<i32>,
) -> Result<(), CommandSyntaxError> {
    use pumpkin_command::source::CommandSource as _;
    // Pumpkin has no synchronous chunk-loading read; reject unloaded interior chunks before writes.
    for x in (min.x >> 4)..=(max.x >> 4) {
        for z in (min.z >> 4)..=(max.z >> 4) {
            source.check_block_loaded(&BlockPos::new(
                (x << 4).max(min.x),
                min.y,
                (z << 4).max(min.z),
            ))?;
        }
    }
    Ok(())
}

fn fill_blocks(
    source: &CommandSource,
    from: BlockPos,
    to: BlockPos,
    target: &BlockInput,
    mode: FillMode,
    filter: Option<&BlockPredicate>,
    strict: bool,
) -> Result<i32, CommandSyntaxError> {
    let area = fill_volume(from, to);
    let world = source.world();
    let limit = world.level_info.load().game_rules.max_block_modifications;
    if area > i128::from(limit) {
        return Err(ERROR_AREA_TOO_LARGE.create_without_context(
            TextComponent::text(limit.to_string()),
            TextComponent::text(area.to_string()),
        ));
    }
    let min = Vector3::new(
        from.0.x.min(to.0.x),
        from.0.y.min(to.0.y),
        from.0.z.min(to.0.z),
    );
    let max = Vector3::new(
        from.0.x.max(to.0.x),
        from.0.y.max(to.0.y),
        from.0.z.max(to.0.z),
    );
    check_region_loaded(source, min, max)?;
    let air = BlockInput {
        state: BlockStateId::AIR,
        properties: Vec::new(),
        tag: None,
    };
    let mut updates = Vec::new();
    let mut count = 0;
    // BlockPos.betweenClosed increments X first, then Y, then Z.
    for z in min.z..=max.z {
        for y in min.y..=max.y {
            for x in min.x..=max.x {
                let pos = BlockPos(Vector3::new(x, y, z));
                let old = world.get_block_state_id(&pos);
                if matches!(mode, FillMode::Keep)
                    && !pumpkin_data::Block::from_state_id(old).is_air()
                    || filter.is_some_and(|f| !f.test(pumpkin_data::Block::from_state_id(old)))
                {
                    continue;
                }
                let edge = x == min.x
                    || x == max.x
                    || y == min.y
                    || y == max.y
                    || z == min.z
                    || z == max.z;
                if matches!(mode, FillMode::Outline) && !edge {
                    continue;
                }
                let destroyed = matches!(mode, FillMode::Destroy)
                    && world
                        .break_block(&pos, None, BlockFlags::NOTIFY_ALL)
                        .is_some();
                let input = if matches!(mode, FillMode::Hollow) && !edge {
                    &air
                } else {
                    target
                };
                let placed = super::block_input::place(input, world, &pos, strict);
                if placed || destroyed {
                    count += 1;
                }
                if placed && !strict {
                    updates.push((pos, old));
                }
            }
        }
    }
    for (pos, old) in updates {
        super::block_input::update_neighbors(world, &pos, old);
    }
    world.flush_block_updates();
    if count == 0 {
        return Err(ERROR_FAILED.create_without_context());
    }
    source.send_feedback(
        TextComponent::translate_cross(
            translation::java::COMMANDS_FILL_SUCCESS,
            translation::java::COMMANDS_FILL_SUCCESS,
            [TextComponent::text(count.to_string())],
        ),
        true,
    );
    Ok(count)
}

struct FillExecutor {
    mode: FillMode,
    filter_mode: FilterMode,
    strict: bool,
}

impl CommandExecutor for FillExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let from = BlockPosArgumentType::get_loaded_block_pos(context, "from")?;
        let to = BlockPosArgumentType::get_loaded_block_pos(context, "to")?;
        let block = BlockStateArgumentType::get(context, "block")?;

        let filter = if matches!(self.filter_mode, FilterMode::WithFilter) {
            Some(BlockPredicateArgumentType::get(context, "filter")?)
        } else {
            None
        };

        fill_blocks(
            &context.source,
            from,
            to,
            block,
            self.mode,
            filter.as_ref(),
            self.strict,
        )
    }
}

fn wrap_with_mode(builder: RequiredArgumentBuilder, has_filter: bool) -> RequiredArgumentBuilder {
    let filter_mode = if has_filter {
        FilterMode::WithFilter
    } else {
        FilterMode::None
    };

    builder
        .executes(FillExecutor {
            mode: FillMode::Replace,
            filter_mode,
            strict: false,
        })
        .then(literal("outline").executes(FillExecutor {
            mode: FillMode::Outline,
            filter_mode,
            strict: false,
        }))
        .then(literal("hollow").executes(FillExecutor {
            mode: FillMode::Hollow,
            filter_mode,
            strict: false,
        }))
        .then(literal("destroy").executes(FillExecutor {
            mode: FillMode::Destroy,
            filter_mode,
            strict: false,
        }))
        .then(literal("strict").executes(FillExecutor {
            mode: FillMode::Replace,
            filter_mode,
            strict: true,
        }))
}

pub fn register(dispatcher: &mut CommandDispatcher, registry: &PermissionRegistry) {
    registry.register_permission_or_panic(Permission::new(
        PERMISSION,
        DESCRIPTION,
        PermissionDefault::Op(PermissionLvl::Two),
    ));

    let filter_arg = wrap_with_mode(argument("filter", BlockPredicateArgumentType), true);

    let replace_literal = literal("replace")
        .executes(FillExecutor {
            mode: FillMode::Replace,
            filter_mode: FilterMode::None,
            strict: false,
        })
        .then(filter_arg);

    let keep_literal = literal("keep").executes(FillExecutor {
        mode: FillMode::Keep,
        filter_mode: FilterMode::KeepAir,
        strict: false,
    });

    let block_arg = wrap_with_mode(argument("block", BlockStateArgumentType), false)
        .then(replace_literal)
        .then(keep_literal);

    dispatcher.register(
        command("fill", DESCRIPTION).requires(PERMISSION).then(
            argument("from", BlockPosArgumentType)
                .then(argument("to", BlockPosArgumentType).then(block_arg)),
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_counts_inclusive_bounds_without_overflow() {
        assert_eq!(
            fill_volume(BlockPos::new(0, 0, 0), BlockPos::new(39, 39, 39)),
            64_000
        );
        assert_eq!(
            fill_volume(BlockPos::new(15, 15, 15), BlockPos::new(0, 0, 0)),
            4096
        );
        assert_eq!(
            fill_volume(
                BlockPos::new(i32::MIN, i32::MIN, i32::MIN),
                BlockPos::new(i32::MAX, i32::MAX, i32::MAX)
            ),
            79_228_162_514_264_337_593_543_950_336
        );
    }
}
