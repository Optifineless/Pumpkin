use super::*;
use crate::command::context::command_source::ReturnValueCallable;
use crate::command::{CommandSender, CommandSource};

struct Capture(std::sync::Mutex<Vec<ReturnValue>>);

impl ReturnValueCallable for Capture {
    fn call(&self, value: ReturnValue) {
        self.0.lock().unwrap().push(value);
    }
}

#[tokio::test]
async fn function_return_fail_stops_before_later_mutation() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = tempfile::tempdir()?;
    let server = crate::server::combat_test_support::server(directory.path());
    let world = crate::server::combat_test_support::world(&server, directory.path());
    server.worlds.store(Arc::new(vec![world]));
    let function_dir = directory
        .path()
        .join("datapacks/review/data/review/function");
    std::fs::create_dir_all(&function_dir)?;
    for (name, contents) in [
        (
            "guarded",
            "return fail\ndata merge storage review:guard {reached:1}\n",
        ),
        (
            "redirected",
            "execute run return fail\ndata merge storage review:guard {reached:1}\n",
        ),
        (
            "valued",
            "return 7\ndata merge storage review:guard {reached:1}\n",
        ),
    ] {
        std::fs::write(function_dir.join(format!("{name}.mcfunction")), contents)?;
    }
    server.datapack_manager.load_all(
        directory.path(),
        &["file/review".into()],
        &server.recipe_manager,
    );
    let source = CommandSender::Console.into_source(&server);
    for function in ["review:guarded", "review:redirected", "review:valued"] {
        assert!(
            server
                .datapack_manager
                .get_functions()
                .contains_key(function)
        );
        assert_eq!(
            server
                .datapack_manager
                .execute_function(&server, &source, function)
                .map_err(|error| error.to_string())?,
            1
        );
        assert!(
            !server
                .command_storage
                .lock()
                .unwrap()
                .contains_key("review:guard")
        );
    }
    crate::server::fixture_lifecycle::finish().await;
    Ok(())
}

#[tokio::test]
async fn inner_return_fail_preserves_outer_tail_and_return_run_replaces_result()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let server = crate::server::combat_test_support::server(directory.path());
    let world = crate::server::combat_test_support::world(&server, directory.path());
    let mut source = CommandSource::dummy();
    source.server = Some(server.clone());
    source.world = Some(world);
    let capture = Arc::new(Capture(std::sync::Mutex::new(Vec::new())));
    source.command_result_taker = ResultValueTaker(vec![capture.clone()]);
    {
        let mut functions = server.datapack_manager.functions.write().unwrap();
        for (name, body) in [
            (
                "review:inner",
                vec!["return fail", "data merge storage review:wrong {reached:1}"],
            ),
            (
                "review:outer",
                vec![
                    "function review:inner",
                    "data merge storage review:outer {reached:1}",
                    "return 9",
                ],
            ),
            (
                "review:run",
                vec![
                    "return run data merge storage review:run {reached:1}",
                    "return 99",
                ],
            ),
        ] {
            functions.insert(
                name.into(),
                body.into_iter()
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
                    .into(),
            );
        }
    }
    server
        .datapack_manager
        .execute_function(&server, &source, "review:outer")
        .map_err(|error| error.to_string())?;
    assert!(
        server
            .command_storage
            .lock()
            .unwrap()
            .contains_key("review:outer")
    );
    assert!(
        !server
            .command_storage
            .lock()
            .unwrap()
            .contains_key("review:wrong")
    );
    assert_eq!(*capture.0.lock().unwrap(), vec![ReturnValue::Success(9)]);
    capture.0.lock().unwrap().clear();
    server
        .datapack_manager
        .execute_function(&server, &source, "review:run")
        .map_err(|error| error.to_string())?;
    assert_eq!(*capture.0.lock().unwrap(), vec![ReturnValue::Success(1)]);
    crate::server::fixture_lifecycle::finish().await;
    Ok(())
}

#[tokio::test]
async fn top_level_returned_function_tag_stops_at_first_explicit_result()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let server = crate::server::combat_test_support::server(directory.path());
    let world = crate::server::combat_test_support::world(&server, directory.path());
    server.worlds.store(Arc::new(vec![world]));
    let manager = &server.datapack_manager;
    {
        let mut functions = manager.functions.write().unwrap();
        for (name, lines) in [
            ("test:first", vec!["return 7"]),
            (
                "test:second",
                vec!["data merge storage test:wrong {reached:1}", "return 9"],
            ),
        ] {
            functions.insert(
                name.into(),
                lines
                    .into_iter()
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
                    .into(),
            );
        }
        manager.function_tags.write().unwrap().insert(
            "test:returns".into(),
            vec!["test:first".into(), "test:second".into()],
        )
    };
    let capture = Arc::new(Capture(std::sync::Mutex::new(Vec::new())));
    let source = CommandSender::Console
        .into_source(&server)
        .with_command_result_taker(ResultValueTaker(vec![capture.clone()]));
    server
        .command_dispatcher
        .load()
        .execute_input("return run function #test:returns", &source)
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(*capture.0.lock().unwrap(), vec![ReturnValue::Success(7)]);
    assert!(
        !server
            .command_storage
            .lock()
            .unwrap()
            .contains_key("test:wrong")
    );
    crate::server::fixture_lifecycle::finish().await;
    Ok(())
}

#[tokio::test]
async fn disabled_command_in_function_body_and_multiple_run_chains_stays_rejected()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let server = crate::server::combat_test_support::server(directory.path());
    let world = crate::server::combat_test_support::world(&server, directory.path());
    server.worlds.store(Arc::new(vec![world]));
    let mut dispatcher = (*server.command_dispatcher.load_full()).clone();
    dispatcher.disable_command("data");
    server.command_dispatcher.store(Arc::new(dispatcher));
    server.datapack_manager.functions.write().unwrap().insert(
        "test:disabled".into(),
        vec![
            "data merge storage test:wrong {reached:1}".into(),
            "execute run execute run data merge storage test:wrong {reached:2}".into(),
            "return 7".into(),
        ]
        .into(),
    );
    let capture = Arc::new(Capture(std::sync::Mutex::new(Vec::new())));
    let source = CommandSender::Console
        .into_source(&server)
        .with_command_result_taker(ResultValueTaker(vec![capture.clone()]));
    server
        .datapack_manager
        .execute_function(&server, &source, "test:disabled")
        .map_err(|error| error.to_string())?;
    assert!(
        !server
            .command_storage
            .lock()
            .unwrap()
            .contains_key("test:wrong")
    );
    assert_eq!(*capture.0.lock().unwrap(), vec![ReturnValue::Success(7)]);
    crate::server::fixture_lifecycle::finish().await;
    Ok(())
}
