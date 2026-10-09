use super::data::{BlockDataAccessor, DataAccessor};
use crate::command::argument_types::block_state::BlockInput;
use crate::command::errors::command_syntax_error::CommandSyntaxError;
use crate::world::World;
use pumpkin_data::{Block, BlockStateId};
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;
use std::sync::Arc;

/// Places parsed state and block entity data using vanilla BlockInput.place's callbacks.
pub(super) fn place(input: &BlockInput, world: &Arc<World>, pos: &BlockPos, strict: bool) -> bool {
    let mut state = input.state;
    if !strict {
        state = world.update_from_neighbor_shapes(state, pos);
        if Block::from_state_id(state).is_air() {
            state = input.state;
        }
    }
    state = overwrite_with_defined_properties(input, state);
    let mut flags = BlockFlags::NOTIFY_LISTENERS | BlockFlags::SKIP_BLOCK_ENTITY_REPLACED_CALLBACK;
    if strict {
        flags |= BlockFlags::UPDATE_KNOWN_SHAPE
            | BlockFlags::SKIP_DROPS
            | BlockFlags::SKIP_BLOCK_ADDED_CALLBACK;
    }
    let old = world.set_block_state(pos, state, flags);
    // LevelChunk.setBlockState creates block entities even when onPlace is suppressed (flag 512).
    let placed = world.get_block_state(pos);
    if world.get_block_entity(pos).is_none()
        && let Some(entity) =
            crate::block::entities::create_block_entity(placed.block_entity_type, *pos)
    {
        world.add_block_entity(entity);
    }
    let mut affected = old != state;
    if let Some(tag) = &input.tag
        && let Ok(accessor) = BlockDataAccessor::new(*pos, world.clone())
    {
        // BlockInput.place reports data problems through ProblemReporter and keeps placing blocks.
        match apply_tag(&accessor, tag) {
            Ok(changed) => affected |= changed,
            Err(error) => tracing::warn!("Could not load block entity data at {pos:?}: {error:?}"),
        }
    }
    affected
}

// BlockInput.overwriteWithDefinedProperties uses BlockStateBase.copyProperty / trySetValue.
fn overwrite_with_defined_properties(input: &BlockInput, state: BlockStateId) -> BlockStateId {
    if state == input.state || input.properties.is_empty() {
        return state;
    }
    let block = Block::from_state_id(state);
    let Some(target) = block.properties(state) else {
        return state;
    };
    let Some(defined) = Block::from_state_id(input.state).properties(input.state) else {
        return state;
    };
    let mut properties = target.to_props();
    for (key, value) in defined.to_props() {
        if input.properties.contains(&key)
            && let Some(property) = properties.iter_mut().find(|(name, _)| *name == key)
        {
            property.1 = value;
        }
    }
    block.from_properties(&properties).to_state_id(block)
}

fn apply_tag(
    accessor: &dyn DataAccessor,
    tag: &pumpkin_nbt::compound::NbtCompound,
) -> Result<bool, CommandSyntaxError> {
    let before = accessor.get_data()?;
    accessor.set_data(tag)?;
    Ok(before != accessor.get_data()?)
}

/// Sends ServerLevel.updateNeighboursOnBlockSet's deferred notifications after placement.
pub(super) fn update_neighbors(world: &Arc<World>, pos: &BlockPos, old: BlockStateId) {
    let current = world.get_block_state_id(pos);
    let block = Block::from_state_id(current);
    let old_block = Block::from_state_id(old);
    if block.id != old_block.id {
        world
            .block_registry
            .on_state_replaced(world, old_block, pos, old, false);
    }
    world.update_neighbors_at(pos, block, None);
    if current.has_analog_output_signal() {
        world.update_neighbour_for_output_signal(pos, block);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::context::command_source::CommandSource;
    use pumpkin_data::entity::EntityType;
    use pumpkin_util::math::vector2::Vector2;
    use pumpkin_util::math::vector3::Vector3;

    #[test]
    fn placement_regression_defined_properties_preserve_target_state() {
        let input = BlockInput {
            state: Block::OAK_STAIRS
                .from_properties(&[("facing", "east")])
                .to_state_id(&Block::OAK_STAIRS),
            properties: vec!["facing"],
            tag: None,
        };
        let target_properties = [
            ("facing", "north"),
            ("half", "top"),
            ("shape", "inner_left"),
            ("waterlogged", "true"),
        ];
        let target = Block::OAK_STAIRS
            .from_properties(&target_properties)
            .to_state_id(&Block::OAK_STAIRS);
        let mut expected_properties = target_properties;
        expected_properties[0].1 = "east";
        let expected = Block::OAK_STAIRS
            .from_properties(&expected_properties)
            .to_state_id(&Block::OAK_STAIRS);
        assert_eq!(overwrite_with_defined_properties(&input, target), expected);
        assert_eq!(
            overwrite_with_defined_properties(&input, Block::STONE.default_state.id),
            Block::STONE.default_state.id
        );
        assert_eq!(
            overwrite_with_defined_properties(&input, Block::OAK_LOG.default_state.id),
            Block::OAK_LOG.default_state.id
        );
        let unspecified = BlockInput {
            properties: Vec::new(),
            ..input
        };
        assert_eq!(
            overwrite_with_defined_properties(&unspecified, target),
            target
        );
    }

    fn placement_source() -> Result<(tempfile::TempDir, CommandSource), std::io::Error> {
        let directory = tempfile::tempdir()?;
        let server = crate::server::combat_test_support::server(directory.path());
        let world = crate::server::combat_test_support::world(&server, directory.path());
        world.level.loaded_chunks.insert(
            Vector2::new(0, 0),
            pumpkin_world::chunk::ChunkData::empty_sync(0, 0),
        );
        let mut source = CommandSource::dummy();
        source.server = Some(server);
        source.world = Some(world);
        source.position = Vector3::new(8.0, 64.0, 8.0);
        source.silent = true;
        Ok((directory, source))
    }

    #[tokio::test]
    async fn placement_regression_setblock_stone() -> Result<(), Box<dyn std::error::Error>> {
        let (_directory, source) = placement_source()?;
        assert_eq!(
            source
                .server()
                .command_dispatcher
                .load()
                .execute_input("setblock ~ ~ ~ stone", &source),
            Ok(1)
        );
        assert_eq!(
            source.world().get_block(&BlockPos::new(8, 64, 8)),
            &Block::STONE
        );
        crate::server::fixture_lifecycle::finish().await;
        Ok(())
    }

    #[tokio::test]
    async fn placement_regression_fill_stone() -> Result<(), Box<dyn std::error::Error>> {
        let (_directory, source) = placement_source()?;
        assert_eq!(
            source
                .server()
                .command_dispatcher
                .load()
                .execute_input("fill 8 64 8 9 64 8 stone", &source),
            Ok(2)
        );
        for x in [8, 9] {
            assert_eq!(
                source.world().get_block(&BlockPos::new(x, 64, 8)),
                &Block::STONE
            );
        }
        crate::server::fixture_lifecycle::finish().await;
        Ok(())
    }

    #[tokio::test]
    async fn placement_regression_fill_air() -> Result<(), Box<dyn std::error::Error>> {
        let (_directory, source) = placement_source()?;
        let chunk = source
            .world()
            .level
            .loaded_chunks
            .get(&Vector2::new(0, 0))
            .map(|chunk| chunk.clone())
            .ok_or("missing test chunk")?;
        for x in [8, 9] {
            chunk.set_block_absolute_y(x, 64, 8, Block::STONE.default_state.id);
        }
        assert_eq!(
            source
                .server()
                .command_dispatcher
                .load()
                .execute_input("fill 8 64 8 9 64 8 air", &source),
            Ok(2)
        );
        for x in [8, 9] {
            assert_eq!(
                source.world().get_block(&BlockPos::new(x, 64, 8)),
                &Block::AIR
            );
        }
        crate::server::fixture_lifecycle::finish().await;
        Ok(())
    }

    #[tokio::test]
    async fn placement_regression_fill_hollow() -> Result<(), Box<dyn std::error::Error>> {
        let (_directory, source) = placement_source()?;
        let chunk = source
            .world()
            .level
            .loaded_chunks
            .get(&Vector2::new(0, 0))
            .map(|chunk| chunk.clone())
            .ok_or("missing test chunk")?;
        chunk.set_block_absolute_y(9, 65, 9, Block::OAK_LOG.default_state.id);
        assert_eq!(
            source
                .server()
                .command_dispatcher
                .load()
                .execute_input("fill 8 64 8 10 66 10 stone hollow", &source),
            Ok(27)
        );
        for x in 8..=10 {
            for y in 64..=66 {
                for z in 8..=10 {
                    let expected = if (x, y, z) == (9, 65, 9) {
                        &Block::AIR
                    } else {
                        &Block::STONE
                    };
                    assert_eq!(source.world().get_block(&BlockPos::new(x, y, z)), expected);
                }
            }
        }
        crate::server::fixture_lifecycle::finish().await;
        Ok(())
    }

    #[tokio::test]
    async fn placement_regression_setblock_log_without_properties()
    -> Result<(), Box<dyn std::error::Error>> {
        let (_directory, source) = placement_source()?;
        assert_eq!(
            source
                .server()
                .command_dispatcher
                .load()
                .execute_input("setblock ~ ~ ~ oak_log", &source),
            Ok(1)
        );
        assert_eq!(
            source.world().get_block_state_id(&BlockPos::new(8, 64, 8)),
            Block::OAK_LOG.default_state.id
        );
        crate::server::fixture_lifecycle::finish().await;
        Ok(())
    }

    #[tokio::test]
    async fn strict_setblock_creates_chest_with_items() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let server = crate::server::combat_test_support::server(directory.path());
        let world = crate::server::combat_test_support::world(&server, directory.path());
        world.level.loaded_chunks.insert(
            Vector2::new(0, 0),
            pumpkin_world::chunk::ChunkData::empty_sync(0, 0),
        );
        let mut source = CommandSource::dummy();
        source.server = Some(server.clone());
        source.world = Some(world.clone());
        source.silent = true;
        let dispatcher = server.command_dispatcher.load();
        assert_eq!(
            dispatcher.execute_input(
                "setblock 8 64 8 chest{Items:[{Slot:0b,id:\"minecraft:diamond\",count:3}]} strict",
                &source,
            ),
            Ok(1)
        );
        assert_eq!(
            dispatcher.execute_input("data get block 8 64 8 Items[0].count", &source),
            Ok(3)
        );
        assert_eq!(
            dispatcher.execute_input("setblock 9 64 8 chest strict", &source),
            Ok(1)
        );
        assert!(world.get_block_entity(&BlockPos::new(9, 64, 8)).is_some());
        crate::server::fixture_lifecycle::finish().await;
        Ok(())
    }

    struct UnreadableBlockEntity(BlockPos);

    impl crate::block::entities::BlockEntity for UnreadableBlockEntity {
        fn write_nbt(&self, _: &mut pumpkin_nbt::compound::NbtCompound) {}
        fn from_nbt(_: &pumpkin_nbt::compound::NbtCompound, pos: BlockPos) -> Self {
            Self(pos)
        }
        fn resource_location(&self) -> &'static str {
            "test:unreadable"
        }
        fn get_position(&self) -> BlockPos {
            self.0
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    #[tokio::test]
    async fn fill_and_setblock_continue_after_block_entity_data_errors()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let server = crate::server::combat_test_support::server(directory.path());
        let world = crate::server::combat_test_support::world(&server, directory.path());
        world.level.loaded_chunks.insert(
            Vector2::new(0, 0),
            pumpkin_world::chunk::ChunkData::empty_sync(0, 0),
        );
        let mut source = CommandSource::dummy();
        source.server = Some(server.clone());
        source.world = Some(world.clone());
        source.silent = true;
        // Simulate a block entity whose data loader is unavailable; the accessor returns an error.
        world.add_block_entity(Arc::new(UnreadableBlockEntity(BlockPos::new(8, 64, 8))));
        assert!(
            BlockDataAccessor::new(BlockPos::new(8, 64, 8), world.clone())
                .map_err(|error| format!("{error:?}"))?
                .set_data(&pumpkin_nbt::compound::NbtCompound::new())
                .is_err()
        );
        let dispatcher = server.command_dispatcher.load();
        assert_eq!(
            dispatcher.execute_input("fill 8 64 8 9 64 8 oak_log{test:1}", &source),
            Ok(2)
        );
        assert_eq!(world.get_block(&BlockPos::new(9, 64, 8)), &Block::OAK_LOG);
        world.add_block_entity(Arc::new(UnreadableBlockEntity(BlockPos::new(10, 64, 8))));
        assert_eq!(
            dispatcher.execute_input("setblock 10 64 8 oak_log{test:1}", &source),
            Ok(1)
        );
        crate::server::fixture_lifecycle::finish().await;
        Ok(())
    }

    #[tokio::test]
    async fn fill_runs_golem_callbacks_except_in_strict_mode()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let server = crate::server::combat_test_support::server(directory.path());
        let world = crate::server::combat_test_support::world(&server, directory.path());
        let chunk = pumpkin_world::chunk::ChunkData::empty_sync(0, 0);
        for y in [64, 65] {
            chunk.set_block_absolute_y(8, y, 8, Block::SNOW_BLOCK.default_state.id);
        }
        world
            .level
            .loaded_chunks
            .insert(Vector2::new(0, 0), chunk.clone());
        let mut source = CommandSource::dummy();
        source.server = Some(server.clone());
        source.world = Some(world.clone());
        source.silent = true;
        let dispatcher = server.command_dispatcher.load();
        assert_eq!(
            dispatcher.execute_input("fill 8 66 8 8 66 8 carved_pumpkin strict", &source),
            Ok(1)
        );
        assert!(world.entities.load().is_empty());
        assert_eq!(
            world.get_block(&BlockPos::new(8, 65, 8)),
            &Block::SNOW_BLOCK
        );
        chunk.set_block_absolute_y(8, 66, 8, BlockStateId::AIR);
        assert_eq!(
            dispatcher.execute_input("fill 8 66 8 8 66 8 carved_pumpkin", &source),
            Ok(1)
        );
        assert_eq!(
            world
                .entities
                .load()
                .iter()
                .filter(|entity| entity.get_entity().entity_type == &EntityType::SNOW_GOLEM)
                .count(),
            1
        );
        for y in [64, 65, 66] {
            assert_eq!(world.get_block(&BlockPos::new(8, y, 8)), &Block::AIR);
        }
        crate::server::fixture_lifecycle::finish().await;
        Ok(())
    }

    #[tokio::test]
    async fn setblock_and_data_merge_update_live_container_nbt()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let server = crate::server::combat_test_support::server(directory.path());
        let world = crate::server::combat_test_support::world(&server, directory.path());
        world.level.loaded_chunks.insert(
            Vector2::new(0, 0),
            pumpkin_world::chunk::ChunkData::empty_sync(0, 0),
        );
        let mut source = CommandSource::dummy();
        source.server = Some(server.clone());
        source.world = Some(world);
        source.silent = true;
        let dispatcher = server.command_dispatcher.load();
        assert_eq!(
            dispatcher.execute_input(
                "setblock 8 64 8 chest[facing=east]{Items:[{Slot:0b,id:\"minecraft:diamond\",count:3}]}",
                &source,
            ),
            Ok(1)
        );
        assert_eq!(
            dispatcher.execute_input("data get block 8 64 8 Items[0].count", &source),
            Ok(3)
        );
        dispatcher
            .execute_input(
                "data merge block 8 64 8 {Items:[{Slot:0b,id:\"minecraft:diamond\",count:7}]}",
                &source,
            )
            .map_err(|error| format!("{error:?}"))?;
        assert_eq!(
            dispatcher.execute_input("data get block 8 64 8 Items[0].count", &source),
            Ok(7)
        );
        crate::server::fixture_lifecycle::finish().await;
        Ok(())
    }
}
