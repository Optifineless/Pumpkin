use super::*;
use crate::command::context::command_source::ReturnValueCallable;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

struct ParentReturn {
    parent: Option<usize>,
    active: Arc<AtomicUsize>,
    max_depth: Arc<AtomicUsize>,
    results: Option<Arc<std::sync::Mutex<Vec<ReturnValue>>>>,
}

impl ReturnValueCallable for ParentReturn {
    fn returned_function_frame(&self) -> Option<usize> {
        self.parent
    }

    fn call(&self, value: ReturnValue) {
        if let Some(results) = &self.results {
            results
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(value);
        }
        let depth = self.active.fetch_add(1, AtomicOrdering::Relaxed) + 1;
        self.max_depth.fetch_max(depth, AtomicOrdering::Relaxed);
        if let Some(parent) = self.parent {
            return_from_function(parent, value);
        }
        self.active.fetch_sub(1, AtomicOrdering::Relaxed);
    }
}

#[test]
fn returned_fork_discards_later_function_calls() -> Result<(), FunctionRunError> {
    let manager = DatapackManager::new();
    insert(&manager, "test:outer", &["fork", "wrong"]);
    insert(&manager, "test:inner", &["return"]);
    let mut visited = Vec::new();
    let results = Arc::new(std::sync::Mutex::new(Vec::new()));
    manager.run_function(
        &CommandSource::dummy(),
        "test:outer",
        100,
        |source, line| {
            visited.push((line.to_string(), source.name.clone()));
            if line == "fork" {
                if let Some(id) = current_function_frame() {
                    discard_function_tail(id);
                    for name in ["first", "second"] {
                        let mut nested = source.clone();
                        nested.name = name.to_string();
                        nested.command_result_taker =
                            ResultValueTaker(vec![Arc::new(ParentReturn {
                                parent: Some(id),
                                active: Arc::new(AtomicUsize::new(0)),
                                max_depth: Arc::new(AtomicUsize::new(0)),
                                results: Some(results.clone()),
                            })]);
                        assert_eq!(
                            manager.run_function(&nested, "test:inner", 100, |_, _| {}),
                            Ok(0)
                        );
                    }
                }
            } else if let Some(id) = current_function_frame() {
                return_from_function(id, ReturnValue::Success(7));
            }
        },
    )?;
    assert_eq!(
        visited,
        [
            ("fork".into(), String::new()),
            ("return".into(), "first".into())
        ]
    );
    assert_eq!(
        *results
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        [ReturnValue::Success(7)]
    );
    Ok(())
}

#[test]
fn nested_returns_deliver_parent_callbacks_without_native_recursion() {
    let active = Arc::new(AtomicUsize::new(0));
    let max_depth = Arc::new(AtomicUsize::new(0));
    let frames = (0usize..32)
        .map(|id| Frame {
            id,
            lines: vec!["tail".to_string()].into(),
            index: 0,
            started: true,
            source: Arc::new(CommandSource::dummy()),
            result: FunctionResult::new(
                1,
                ResultValueTaker(vec![Arc::new(ParentReturn {
                    parent: id.checked_sub(1),
                    active: active.clone(),
                    max_depth: max_depth.clone(),
                    results: None,
                })]),
            ),
            returned: false,
        })
        .collect();
    ACTIVE_QUEUE.with(|queue| {
        *queue.borrow_mut() = Some(FunctionQueue {
            frames,
            ..FunctionQueue::new(0, 0, MAX_QUEUE_DEPTH)
        });
    });
    let _guard = QueueGuard;
    return_from_function(31, ReturnValue::Success(7));
    assert_eq!(max_depth.load(AtomicOrdering::Relaxed), 1);
    ACTIVE_QUEUE.with(|queue| {
        assert!(queue.borrow().as_ref().is_some_and(|queue| {
            queue
                .frames
                .iter()
                .all(|frame| frame.returned && frame.index == frame.lines.len())
        }));
    });
}

fn insert(manager: &DatapackManager, name: &str, lines: &[&str]) {
    manager
        .functions
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(
            name.to_string(),
            lines
                .iter()
                .map(|s| (*s).to_string())
                .collect::<Vec<_>>()
                .into(),
        );
}

#[test]
fn reloads_coalesce_until_a_game_tick_and_load_precedes_tick() {
    let manager = DatapackManager::new();
    let mut tags = Vec::new();
    manager.visit_tick_tags(|tag| tags.push(tag.to_string()));
    assert_eq!(tags, ["#minecraft:load", "#minecraft:tick"]);
    tags.clear();
    manager.mark_load_pending();
    manager.mark_load_pending();
    manager.visit_tick_tags(|tag| tags.push(tag.to_string()));
    manager.visit_tick_tags(|tag| tags.push(tag.to_string()));
    assert_eq!(
        tags,
        ["#minecraft:load", "#minecraft:tick", "#minecraft:tick"]
    );
}

#[test]
fn recursion_uses_the_configured_quota_and_releases_the_queue() -> Result<(), FunctionRunError> {
    let manager = DatapackManager::new();
    let source = CommandSource::dummy();
    insert(&manager, "test:loop", &["function test:loop"]);
    let count = manager.run_function(&source, "test:loop", 20_000, |source, _| {
        assert_eq!(
            manager.run_function(source, "test:loop", 20_000, |_, _| {}),
            Ok(0)
        );
    })?;
    assert_eq!(count, 19_999);
    assert!(!is_nested_function_call());
    assert_eq!(
        manager.run_function(&source, "test:loop", 2, |_, _| {}),
        Ok(1)
    );
    Ok(())
}

#[test]
fn queued_calls_run_before_the_callers_tail_with_their_own_source() -> Result<(), FunctionRunError>
{
    let manager = DatapackManager::new();
    insert(&manager, "test:outer", &["call", "tail"]);
    insert(&manager, "test:inner", &["inner"]);
    let mut visited = Vec::new();
    manager.run_function(
        &CommandSource::dummy(),
        "test:outer",
        100,
        |source, line| {
            assert!(manager.functions.try_write().is_ok());
            visited.push((line.to_string(), source.name.clone()));
            assert!(consume_command_cost());
            if line == "call" {
                let mut nested = source.clone();
                nested.name = "nested".to_string();
                assert_eq!(
                    manager.run_function(&nested, "test:inner", 100, |_, _| {}),
                    Ok(0)
                );
            }
        },
    )?;
    assert_eq!(
        visited,
        [
            ("call".into(), String::new()),
            ("inner".into(), "nested".into()),
            ("tail".into(), String::new())
        ]
    );
    Ok(())
}

#[test]
fn unknown_functions_fail_but_optional_tag_entries_are_skipped() {
    let manager = DatapackManager::new();
    let source = CommandSource::dummy();
    assert!(matches!(
        manager.run_function(&source, "missing", 10, |_, _| {}),
        Err(FunctionRunError::Unknown(_))
    ));
    assert!(matches!(
        manager.run_function(&source, "#missing", 10, |_, _| {}),
        Err(FunctionRunError::Unknown(_))
    ));
    insert(&manager, "test:present", &["one"]);
    manager
        .function_tags
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(
            "test:tag".into(),
            vec!["test:absent".into(), "test:present".into()],
        );
    assert_eq!(
        manager.run_function(&source, "#test:tag", 10, |_, _| {}),
        Ok(1)
    );
}

#[tokio::test]
async fn explicit_returns_discard_tails_and_store_function_results()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let server = crate::server::combat_test_support::server(directory.path());
    let world = crate::server::combat_test_support::world(&server, directory.path());
    let mut source = CommandSource::dummy();
    source.server = Some(server.clone());
    source.world = Some(world.clone());
    source.silent = true;
    let dispatcher = server.command_dispatcher.load();
    dispatcher
        .execute_input("scoreboard objectives add repro dummy", &source)
        .map_err(|error| format!("{error:?}"))?;
    insert(
        &server.datapack_manager,
        "test:child",
        &["return 7", "scoreboard players set wrong repro 99"],
    );
    insert(
        &server.datapack_manager,
        "test:outer",
        &[
            "execute store result score value repro run function test:child",
            "scoreboard players set tail repro 9",
        ],
    );
    dispatcher
        .execute_input("function test:outer", &source)
        .map_err(|error| format!("{error:?}"))?;
    insert(
        &server.datapack_manager,
        "test:return",
        &[
            "return run function test:child",
            "scoreboard players set wrong repro 88",
        ],
    );
    dispatcher
        .execute_input(
            "execute store result score returned repro run function test:return",
            &source,
        )
        .map_err(|error| format!("{error:?}"))?;
    server
        .datapack_manager
        .function_tags
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(
            "test:returns".into(),
            vec!["test:child".into(), "test:child".into()],
        );
    dispatcher
        .execute_input(
            "execute store result score tagged repro run function #test:returns",
            &source,
        )
        .map_err(|error| format!("{error:?}"))?;
    let scoreboard = world
        .scoreboard
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_eq!(
        scoreboard
            .get_score("value", "repro")
            .map(|score| score.value.0),
        Some(7)
    );
    assert_eq!(
        scoreboard
            .get_score("returned", "repro")
            .map(|score| score.value.0),
        Some(7)
    );
    assert_eq!(
        scoreboard
            .get_score("tail", "repro")
            .map(|score| score.value.0),
        Some(9)
    );
    assert_eq!(
        scoreboard
            .get_score("tagged", "repro")
            .map(|score| score.value.0),
        Some(7)
    );
    assert!(scoreboard.get_score("wrong", "repro").is_none());
    Ok(())
}

#[test]
fn executor_costs_share_the_function_quota() -> Result<(), FunctionRunError> {
    let manager = DatapackManager::new();
    insert(&manager, "test:quota", &["fork", "tail"]);
    let mut executed = 0;
    manager.run_function(&CommandSource::dummy(), "test:quota", 4, |_, _| {
        for _ in 0..10 {
            if consume_command_cost() {
                executed += 1;
            }
        }
    })?;
    assert_eq!(executed, 3);
    Ok(())
}

#[tokio::test]
async fn returned_tags_stop_at_the_first_explicit_return() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = tempfile::tempdir()?;
    let server = crate::server::combat_test_support::server(directory.path());
    let world = crate::server::combat_test_support::world(&server, directory.path());
    let mut source = CommandSource::dummy();
    source.server = Some(server.clone());
    source.world = Some(world.clone());
    source.silent = true;
    let dispatcher = server.command_dispatcher.load();
    dispatcher
        .execute_input("scoreboard objectives add repro dummy", &source)
        .map_err(|error| format!("{error:?}"))?;
    let manager = &server.datapack_manager;
    insert(manager, "test:first", &["return 7"]);
    insert(
        manager,
        "test:second",
        &["scoreboard players set wrong repro 99", "return 9"],
    );
    insert(
        manager,
        "test:outer",
        &[
            "return run function #test:returns",
            "scoreboard players set wrong repro 88",
        ],
    );
    manager
        .function_tags
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(
            "test:returns".into(),
            vec!["test:first".into(), "test:second".into()],
        );
    dispatcher
        .execute_input(
            "execute store result score value repro run function test:outer",
            &source,
        )
        .map_err(|error| format!("{error:?}"))?;
    let scoreboard = world
        .scoreboard
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_eq!(
        scoreboard
            .get_score("value", "repro")
            .map(|score| score.value.0),
        Some(7)
    );
    assert!(scoreboard.get_score("wrong", "repro").is_none());
    Ok(())
}

#[tokio::test]
async fn returned_empty_branches_and_functions_report_failure()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let server = crate::server::combat_test_support::server(directory.path());
    let world = crate::server::combat_test_support::world(&server, directory.path());
    let mut source = CommandSource::dummy();
    source.server = Some(server.clone());
    source.world = Some(world.clone());
    source.silent = true;
    let dispatcher = server.command_dispatcher.load();
    dispatcher
        .execute_input("scoreboard objectives add repro dummy", &source)
        .map_err(|error| format!("{error:?}"))?;
    let manager = &server.datapack_manager;
    insert(manager, "test:body", &[]);
    manager
        .function_tags
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert("test:empty_tag".into(), Vec::new());
    let commands = [
        "return run execute if entity @e[type=minecraft:cow] run return 3",
        "return run function test:body",
        "return run function #test:empty_tag",
    ];
    for (index, command) in commands.into_iter().enumerate() {
        let name = format!("test:empty_{index}");
        insert(
            manager,
            &name,
            &[command, "scoreboard players set wrong repro 99"],
        );
        dispatcher
            .execute_input(
                &format!("execute store result score value{index} repro run function {name}"),
                &source,
            )
            .map_err(|error| format!("{error:?}"))?;
        assert_eq!(
            world
                .scoreboard
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get_score(&format!("value{index}"), "repro")
                .map(|score| score.value.0),
            Some(0)
        );
    }
    assert!(
        world
            .scoreboard
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_score("wrong", "repro")
            .is_none()
    );
    Ok(())
}

#[tokio::test]
async fn frozen_load_waits_and_tick_functions_have_independent_quotas()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let server = crate::server::combat_test_support::server(directory.path());
    let world = crate::server::combat_test_support::world(&server, directory.path());
    server.level_info.rcu(|info| {
        let mut info = (**info).clone();
        info.game_rules.max_command_sequence_length = 3;
        info
    });
    let mut source = CommandSource::dummy();
    source.server = Some(server.clone());
    source.world = Some(world.clone());
    source.silent = true;
    server
        .command_dispatcher
        .load()
        .execute_input("scoreboard objectives add repro dummy", &source)
        .map_err(|error| format!("{error:?}"))?;
    let manager = &server.datapack_manager;
    for name in ["test:first", "test:second"] {
        insert(manager, name, &["scoreboard players add value repro 1"]);
    }
    manager
        .function_tags
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(
            "minecraft:tick".to_string(),
            vec!["test:first".into(), "test:second".into()],
        );
    manager
        .function_tags
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert("minecraft:load".into(), vec!["test:first".into()]);
    server.tick_rate_manager.set_frozen(&server, true);
    server.tick_rate_manager.tick();
    manager.tick_functions(&server, &source);
    assert!(
        world
            .scoreboard
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_score("value", "repro")
            .is_none()
    );
    server.tick_rate_manager.set_frozen(&server, false);
    server.tick_rate_manager.tick();
    manager.tick_functions(&server, &source);
    manager.tick_functions(&server, &source);
    let scoreboard = world
        .scoreboard
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_eq!(
        scoreboard
            .get_score("value", "repro")
            .map(|score| score.value.0),
        Some(5)
    );
    Ok(())
}
