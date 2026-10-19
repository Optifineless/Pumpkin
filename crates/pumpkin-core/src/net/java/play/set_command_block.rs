#[allow(clippy::wildcard_imports)]
use super::*;
use crate::block::blocks::command::CommandBlock;
use crate::block::entities::BlockEntity;
use crate::world::World;
use pumpkin_data::BlockStateId;
use pumpkin_util::math::position::BlockPos;

fn still_owns_command_block(
    world: &World,
    position: BlockPos,
    state_id: BlockStateId,
    previous: &Arc<dyn BlockEntity>,
    mode_changed: bool,
) -> bool {
    if world.get_block_state_id(&position) != state_id {
        return false;
    }
    let current = world.get_block_entity(&position);
    current
        .as_ref()
        .is_none_or(|current| Arc::ptr_eq(current, previous))
        && (mode_changed || current.is_some())
}

fn publish_command_block_entity(
    world: &World,
    position: BlockPos,
    state_id: BlockStateId,
    block_entity: &Arc<dyn BlockEntity>,
) -> bool {
    world.add_block_entity(Arc::clone(block_entity));
    world.get_block_state_id(&position) == state_id
        && world
            .get_block_entity(&position)
            .is_some_and(|current| Arc::ptr_eq(&current, block_entity))
}

struct CommandBlockSnapshot {
    automatic: bool,
    powered: bool,
    condition_met: bool,
    dirty: bool,
    last_output: String,
}

impl CommandBlockSnapshot {
    fn capture(command_block: &CommandBlockEntity) -> Self {
        Self {
            automatic: command_block.auto.load(Ordering::SeqCst),
            powered: command_block.powered.load(Ordering::SeqCst),
            condition_met: command_block.condition_met.load(Ordering::SeqCst),
            dirty: command_block.dirty.load(Ordering::SeqCst),
            last_output: command_block
                .last_output
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone(),
        }
    }

    fn into_entity(
        self,
        position: BlockPos,
        command: &SSetCommandBlock<'_>,
    ) -> Arc<CommandBlockEntity> {
        let mut command_text = command.command;
        if command_text.starts_with('/') {
            command_text = &command_text[1..];
        }
        Arc::new(CommandBlockEntity {
            position,
            powered: self.powered.into(),
            condition_met: self.condition_met.into(),
            auto: command.is_automatic().into(),
            dirty: self.dirty.into(),
            command: std::sync::Mutex::new(command_text.to_string()),
            last_output: std::sync::Mutex::new(self.last_output),
            track_output: command.track_output().into(),
            success_count: AtomicU32::new(0),
        })
    }
}

fn should_activate_command_block(
    old_block: &Block,
    new_block: &Block,
    snapshot: &CommandBlockSnapshot,
    automatic: bool,
) -> bool {
    let mode_changed = old_block.id != new_block.id;
    (!snapshot.automatic
        && automatic
        && !snapshot.powered
        && *new_block != Block::CHAIN_COMMAND_BLOCK)
        || (mode_changed
            && *new_block == Block::REPEATING_COMMAND_BLOCK
            && (snapshot.powered || automatic))
}

impl JavaClient {
    pub fn handle_set_command_block(&self, player: &Arc<Player>, command: &SSetCommandBlock<'_>) {
        if !player.is_creative() {
            return;
        }
        if player.permission_lvl.load() < PermissionLvl::Two {
            return;
        }
        let pos = command.pos;
        let world = player.world();
        let block_entity = world.get_block_entity(&pos);
        if let Some(block_entity) = block_entity {
            if block_entity.resource_location() != CommandBlockEntity::ID {
                warn!("Client tried to change Command block but not Command block entity found");
                return;
            }

            let Ok(command_block_mode) = CommandBlockMode::try_from(command.mode) else {
                self.try_kick(&TextComponent::text("Invalid Command block mode"));
                return;
            };

            let old_block = world.get_block(&pos);
            let old_state_id = world.get_block_state_id(&pos);
            let mut props = CommandBlockLikeProperties::from_state_id(old_state_id);

            let block_type: &'static Block = match command_block_mode {
                CommandBlockMode::Chain => &Block::CHAIN_COMMAND_BLOCK,
                CommandBlockMode::Repeating => &Block::REPEATING_COMMAND_BLOCK,
                CommandBlockMode::Impulse => &Block::COMMAND_BLOCK,
            };

            let Some(old_command_block) =
                block_entity.as_any().downcast_ref::<CommandBlockEntity>()
            else {
                return;
            };

            let snapshot = CommandBlockSnapshot::capture(old_command_block);
            let mode_changed = old_block.id != block_type.id;
            let should_activate = should_activate_command_block(
                old_block,
                block_type,
                &snapshot,
                command.is_automatic(),
            );

            props.conditional = command.is_conditional();

            let new_state_id = props.to_state_id(block_type);
            if world
                .set_block_state_if(
                    &command.pos,
                    new_state_id,
                    BlockFlags::NOTIFY_ALL | BlockFlags::SKIP_BLOCK_ADDED_CALLBACK,
                    |state_id| state_id == old_state_id,
                )
                .is_none()
            {
                return;
            }

            if !still_owns_command_block(&world, pos, new_state_id, &block_entity, mode_changed) {
                return;
            }

            let command_block = snapshot.into_entity(pos, command);

            if should_activate {
                CommandBlock::mark_condition_met(&world, &command_block, &pos);
            }

            let published_block_entity: Arc<dyn BlockEntity> = command_block;
            if !publish_command_block_entity(&world, pos, new_state_id, &published_block_entity) {
                return;
            }

            if should_activate {
                world.schedule_block_tick(
                    block_type,
                    pos,
                    1,
                    pumpkin_world::tick::TickPriority::Normal,
                );
            }

            player.send_system_message(&TextComponent::text(format!(
                "Command set: {}",
                command.command
            )));
        }
    }
}

#[cfg(test)]
mod tests;
