use super::default_dispatcher;
use crate::command::CommandSource;
use crate::command::node::attached::NodeId;
use crate::command::node::tree::ROOT_NODE_ID;
use pumpkin_config::CommandsConfig;
use pumpkin_util::permission::PermissionManager;
use rand::{RngExt, SeedableRng, rngs::StdRng};
use std::sync::Arc;

#[tokio::test]
async fn every_registered_command_accepts_seeded_parser_fuzz_without_panicking()
-> Result<(), Box<dyn std::error::Error>> {
    let dispatcher = default_dispatcher(&PermissionManager::new(), &CommandsConfig::default());
    let directory = tempfile::tempdir()?;
    let server = crate::server::combat_test_support::server(directory.path());
    let world = crate::server::combat_test_support::world(&server, directory.path());
    let mut source = CommandSource::dummy();
    source.server = Some(server);
    source.world = Some(world);
    let source = Arc::new(source);
    let mut random = StdRng::seed_from_u64(0x3759_3779_2016);
    let fragments = [
        "",
        " ",
        "@s",
        "@e[limit=0]",
        "[",
        "[]",
        "[ ]",
        "{",
        "}",
        "-2147483649",
        "2147483648",
        "NaN",
        "\"",
        "é",
        "😀",
        "İ",
        "\0",
        "~",
        "^",
        "=",
        "\n",
    ];
    let mut commands = dispatcher.tree.get_children(ROOT_NODE_ID);
    commands.sort_by_key(|id| dispatcher.tree[*id].name());
    assert!(!commands.is_empty());
    for command in commands {
        for _ in 0..128 {
            let mut node: NodeId = command;
            let mut input = dispatcher.tree[node].name();
            // Walk real registered grammar and argument examples, including executable paths.
            for _ in 0..12 {
                let mut children = dispatcher.tree.get_children(node);
                children.sort_by_key(|id| dispatcher.tree[*id].name());
                if children.is_empty() {
                    break;
                }
                node = children[random.random_range(0..children.len())];
                let examples = dispatcher.tree[node].examples();
                if let Some(example) = examples.get(random.random_range(0..examples.len().max(1))) {
                    input.push(' ');
                    input.push_str(example);
                }
                if dispatcher.tree[node].command().is_some() {
                    break;
                }
            }
            let parsed = dispatcher.parse_input(&input, &source);
            let _ = dispatcher.get_completion_suggestions_at_end(parsed);
            for _ in 0..4 {
                let boundaries: Vec<usize> = input
                    .char_indices()
                    .map(|(i, _)| i)
                    .chain([input.len()])
                    .collect();
                let cursor = boundaries[random.random_range(0..boundaries.len())];
                let mut malformed = input.clone();
                malformed.insert_str(cursor, fragments[random.random_range(0..fragments.len())]);
                let parsed = dispatcher.parse_input(&malformed, &source);
                let _ = dispatcher.get_completion_suggestions(
                    dispatcher.parse_input(&malformed, &source),
                    cursor,
                );
                let _ = dispatcher
                    .get_completion_suggestions_at_end(dispatcher.parse_input(&malformed, &source));
                // Execute syntax failures through the dispatcher; valid commands need a live world.
                if !parsed.errors.is_empty() && parsed.reader.can_read_char() {
                    assert!(dispatcher.execute(parsed).is_err(), "{malformed}");
                }
            }
        }
    }
    crate::server::fixture_lifecycle::finish().await;
    Ok(())
}
