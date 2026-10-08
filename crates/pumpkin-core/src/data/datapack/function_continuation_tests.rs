use super::*;
use pumpkin_data::entity::EntityType;
use pumpkin_util::math::vector3::Vector3;

fn insert(manager: &DatapackManager, name: &str, lines: &[&str]) {
    manager
        .functions
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(
            name.into(),
            lines
                .iter()
                .map(|line| (*line).into())
                .collect::<Vec<String>>()
                .into(),
        );
}

#[tokio::test]
async fn function_feedback_reports_scheduling_before_execution_and_explicit_result()
-> Result<(), Box<dyn std::error::Error>> {
    let (_directory, mut source) = source_with_entities(0)?;
    let output = Arc::new(
        crate::block::entities::command_block::CommandBlockEntity::new(
            pumpkin_util::math::position::BlockPos::new(0, 64, 0),
            true,
            false,
        ),
    );
    source.output =
        crate::command::CommandSender::CommandBlock(output.clone(), source.world().clone());
    source.silent = false;
    let server = source.server();
    insert(&server.datapack_manager, "test:feedback", &["return 7"]);
    let dispatcher = server.command_dispatcher.load();
    let output_text = || {
        output
            .last_output
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    };
    let result = run_in_context(
        10,
        max_command_forks(&source),
        MAX_QUEUE_DEPTH,
        || {
            let result = dispatcher.execute_input("function test:feedback", &source);
            assert!(
                output_text().ends_with("Running function test:feedback"),
                "{}",
                output_text()
            );
            result
        },
        |source, line| {
            dispatcher.handle_command(source, line);
        },
    )
    .0;
    result.map_err(|error| format!("{error:?}"))?;
    assert!(
        output_text().ends_with("Function test:feedback returned 7"),
        "{}",
        output_text()
    );
    Ok(())
}

fn source_with_entities(
    count: usize,
) -> Result<(tempfile::TempDir, CommandSource), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let server = crate::server::combat_test_support::server(directory.path());
    let world = crate::server::combat_test_support::world(&server, directory.path());
    server.worlds.store(Arc::new(vec![world.clone()]));
    for _ in 0..count {
        let entity = crate::entity::r#type::from_type(
            &EntityType::COW,
            Vector3::new(0.0, 64.0, 0.0),
            &world,
            uuid::Uuid::new_v4(),
        );
        world.spawn_entity(entity);
    }
    let mut source = CommandSource::dummy();
    source.server = Some(server);
    source.world = Some(world);
    source.silent = true;
    Ok((directory, source))
}

#[tokio::test]
async fn queue_regression_capped_recursive_fanout_stops_execution()
-> Result<(), Box<dyn std::error::Error>> {
    let (_directory, source) = source_with_entities(4)?;
    let manager = &source.server().datapack_manager;
    for name in ["test:left", "test:right"] {
        insert(manager, name, &["execute as @e run function #test:fanout"]);
    }
    manager
        .function_tags
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(
            "test:fanout".into(),
            vec!["test:left".into(), "test:right".into()],
        );
    let dispatcher = source.server().command_dispatcher.load();
    let mut overflowed = false;
    let (result, executed) = run_in_context(
        1_000,
        max_command_forks(&source),
        15,
        || dispatcher.execute_input("execute as @e run function #test:fanout", &source),
        |source, line| {
            // Overflow ends the context with no command error or result callback failure.
            assert_eq!(dispatcher.execute_input(line, source), Ok(0));
            ACTIVE_QUEUE.with(|active| {
                if let Some(queue) = active.borrow().as_ref() {
                    overflowed |= queue.queue_overflow;
                    if queue.queue_overflow {
                        assert_eq!(queue.queued_calls, 0);
                        assert!(queue.frames.is_empty());
                        assert!(queue.entries.is_empty());
                        assert!(queue.pending.is_empty());
                        assert!(queue.remaining > 0);
                    } else {
                        assert_eq!(queue.queued_calls, 15);
                        assert_eq!(queue.frames.len(), 1);
                    }
                }
            });
            if overflowed {
                assert!(!consume_command_cost());
            }
        },
    );
    assert_eq!(result, Ok(0));
    assert_eq!(executed, 2);
    assert!(overflowed);
    assert!(!is_nested_function_call());
    Ok(())
}

#[tokio::test]
async fn queue_regression_fork_limit_is_captured_for_the_whole_context()
-> Result<(), Box<dyn std::error::Error>> {
    let (_directory, source) = source_with_entities(4)?;
    let server = source.server();
    let dispatcher = server.command_dispatcher.load();
    dispatcher
        .execute_input("scoreboard objectives add quota dummy", &source)
        .map_err(|error| format!("{error:?}"))?;
    insert(
        &server.datapack_manager,
        "test:add",
        &["scoreboard players add count quota 1"],
    );
    let (result, _) = run_in_context(
        100,
        max_command_forks(&source),
        MAX_QUEUE_DEPTH,
        || {
            server.level_info.rcu(|info| {
                let mut info = (**info).clone();
                info.game_rules.max_command_forks = 2;
                info
            });
            dispatcher.execute_input("execute as @e run function test:add", &source)
        },
        |source, line| dispatcher.handle_command(source, line),
    );
    result.map_err(|error| format!("{error:?}"))?;
    assert_eq!(
        dispatcher.execute_input("execute as @e run function test:add", &source),
        Ok(0)
    );
    assert_eq!(
        source
            .world()
            .scoreboard
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_score("count", "quota")
            .map(|score| score.value.0),
        Some(4)
    );
    Ok(())
}

#[tokio::test]
async fn queue_regression_worldless_dispatch_uses_the_default_quota()
-> Result<(), Box<dyn std::error::Error>> {
    let (_directory, mut source) = source_with_entities(0)?;
    let server = source.server().clone();
    source.world = None;
    insert(&server.datapack_manager, "test:worldless", &["return 7"]);
    assert_eq!(
        server
            .command_dispatcher
            .load()
            .execute_input("function test:worldless", &source),
        Ok(0)
    );
    assert!(!is_nested_function_call());
    Ok(())
}

#[tokio::test]
async fn recursive_execute_as_fanout_keeps_one_continuation_per_depth()
-> Result<(), Box<dyn std::error::Error>> {
    let (_directory, source) = source_with_entities(128)?;
    let server = source.server();
    insert(
        &server.datapack_manager,
        "test:self",
        &["execute as @e run function test:self"],
    );
    let dispatcher = server.command_dispatcher.load();
    let mut dispatched = 0;
    let mut max_entries = 0;
    let mut max_depth = 0;
    let (_, executed) = run_in_context(
        100,
        max_command_forks(&source),
        MAX_QUEUE_DEPTH,
        || dispatcher.execute_input("execute as @e run function test:self", &source),
        |source, line| {
            dispatched += 1;
            assert!(dispatcher.execute_input(line, source).is_ok());
            ACTIVE_QUEUE.with(|active| {
                if let Some(queue) = active.borrow().as_ref() {
                    let entries = queue.entries.len() + queue.pending.len();
                    assert!(
                        entries <= 2 * queue.frames.len() + 1,
                        "{entries} queue entries at depth {}",
                        queue.frames.len()
                    );
                    max_entries = max_entries.max(entries);
                    max_depth = max_depth.max(queue.frames.len());
                }
            });
        },
    );
    assert_eq!(executed, 49);
    assert_eq!(dispatched, 49);
    assert_eq!(max_depth, 49);
    assert_eq!(max_entries, 99);
    assert!(!is_nested_function_call());
    Ok(())
}

#[tokio::test]
async fn top_level_execute_as_functions_share_one_quota() -> Result<(), Box<dyn std::error::Error>>
{
    let (_directory, source) = source_with_entities(16)?;
    let server = source.server();
    let dispatcher = server.command_dispatcher.load();
    dispatcher
        .execute_input("scoreboard objectives add quota dummy", &source)
        .map_err(|error| format!("{error:?}"))?;
    insert(
        &server.datapack_manager,
        "test:add",
        &[
            "scoreboard players add count quota 1",
            "scoreboard players add count quota 1",
            "scoreboard players add count quota 1",
        ],
    );
    server.level_info.rcu(|info| {
        let mut info = (**info).clone();
        info.game_rules.max_command_sequence_length = 10;
        info
    });
    dispatcher
        .execute_input("execute as @e run function test:add", &source)
        .map_err(|error| format!("{error:?}"))?;
    let scoreboard = source
        .world()
        .scoreboard
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // One redirect plus three CallFunctions plus six ordinary commands consume ten units.
    assert_eq!(
        scoreboard
            .get_score("count", "quota")
            .map(|score| score.value.0),
        Some(6)
    );
    assert!(!is_nested_function_call());
    Ok(())
}

#[tokio::test]
async fn return_run_does_not_consume_a_quota_unit() -> Result<(), Box<dyn std::error::Error>> {
    let (_directory, source) = source_with_entities(0)?;
    let server = source.server();
    let dispatcher = server.command_dispatcher.load();
    dispatcher
        .execute_input("scoreboard objectives add quota dummy", &source)
        .map_err(|error| format!("{error:?}"))?;
    insert(
        &server.datapack_manager,
        "test:return",
        &[
            "return run scoreboard players set result quota 7",
            "scoreboard players set tail quota 99",
        ],
    );
    server.level_info.rcu(|info| {
        let mut info = (**info).clone();
        info.game_rules.max_command_sequence_length = 2;
        info
    });
    dispatcher
        .execute_input("function test:return", &source)
        .map_err(|error| format!("{error:?}"))?;
    let scoreboard = source
        .world()
        .scoreboard
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_eq!(
        scoreboard
            .get_score("result", "quota")
            .map(|score| score.value.0),
        Some(7)
    );
    assert!(scoreboard.get_score("tail", "quota").is_none());
    Ok(())
}
