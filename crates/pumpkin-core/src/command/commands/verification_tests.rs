use super::*;
use crate::command::argument_builder::{ArgumentBuilder, command};
use crate::command::context::command_context::CommandContext;
use crate::command::context::command_source::{ResultValueTaker, ReturnValue, ReturnValueCallable};
use std::sync::Mutex;

struct Positions(Arc<Mutex<Vec<Vector3<f64>>>>);

impl crate::command::node::CommandExecutor for Positions {
    fn execute(&self, context: &CommandContext) -> crate::command::node::CommandExecutorResult {
        use pumpkin_command::argument_types::coordinates::vec3::Vec3ArgumentType;
        self.0
            .lock()
            .unwrap()
            .push(Vec3ArgumentType::get_vector3(context, "pos")?);
        Ok(1)
    }
}

#[tokio::test]
async fn direct_player_local_coordinates_equal_execute_at_self() -> TestResult {
    let fixture = Fixture::new()?;
    let entity = fixture.alice.player.get_entity();
    entity.set_pos(Vector3::new(10.0, 20.0, 30.0));
    entity.pitch.store(30.0);
    entity.yaw.store(60.0);
    let positions = Arc::new(Mutex::new(Vec::new()));
    let mut dispatcher = (*fixture.server.command_dispatcher.load_full()).clone();
    dispatcher.register(
        command("position", "capture coordinates").then(
            crate::command::argument_builder::argument(
                "pos",
                crate::command::argument_types::coordinates::vec3::Vec3ArgumentType::default(),
            )
            .executes(Positions(positions.clone())),
        ),
    );
    let source = fixture.sources()[0].clone();
    for input in ["position ^ ^ ^1", "execute at @s run position ^ ^ ^1"] {
        dispatcher
            .execute_input(input, &source)
            .map_err(|error| format!("{error:?}"))?;
    }
    let positions = positions.lock().unwrap();
    assert_eq!(positions.len(), 2);
    assert_eq!(positions[0], positions[1]);
    assert!((positions[0].x - 9.25).abs() < 0.00001);
    assert!((positions[0].y - 19.5).abs() < 0.00001);
    assert!((positions[0].z - 30.4330127).abs() < 0.00001);
    Ok(())
}

struct Returns(Arc<Mutex<Vec<ReturnValue>>>);

impl ReturnValueCallable for Returns {
    fn call(&self, value: ReturnValue) {
        self.0.lock().unwrap().push(value);
    }
}

#[tokio::test]
async fn top_level_bare_return_discards_remaining_player_sources() -> TestResult {
    let fixture = Fixture::new()?;
    let returns = Arc::new(Mutex::new(Vec::new()));
    let source = fixture.sources()[1]
        .clone()
        .merge_command_result_taker(&ResultValueTaker(vec![Arc::new(Returns(returns.clone()))]));
    let result = fixture
        .server
        .command_dispatcher
        .load()
        .execute_input("execute as @a run return 1", &source)
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(result, 1);
    assert_eq!(*returns.lock().unwrap(), [ReturnValue::Success(1)]);
    Ok(())
}

#[tokio::test]
async fn chat_output_fallback_uses_bound_chat_type() -> TestResult {
    let fixture = Fixture::new()?;
    let output = Arc::new(
        crate::block::entities::command_block::CommandBlockEntity::new(
            pumpkin_util::math::position::BlockPos::new(0, 64, 0),
            true,
            false,
        ),
    );
    let mut source = fixture.sources()[1].clone();
    source.output = CommandSender::CommandBlock(output.clone(), fixture.world);
    let content = crate::net::chat::OutgoingChatMessage::Disguised {
        content: pumpkin_util::text::TextComponent::text("hello"),
    };
    source.send_chat_message(
        &content,
        pumpkin_data::chat_type::ChatType::MsgCommandIncoming,
        None,
    );
    let message = output.last_output.lock().unwrap().clone();
    assert!(
        message.ends_with("Server whispers to you: hello"),
        "{message}"
    );
    source.send_chat_message(
        &content,
        pumpkin_data::chat_type::ChatType::MsgCommandOutgoing,
        Some(&pumpkin_util::text::TextComponent::text("Recipient")),
    );
    assert!(
        output
            .last_output
            .lock()
            .unwrap()
            .ends_with("You whisper to Recipient: hello")
    );
    Ok(())
}
