use super::data::TargetKind;
use crate::command::argument_builder::{
    ArgumentBuilder, CommandArgumentBuilder, LiteralArgumentBuilder, argument, literal,
};
use crate::command::argument_types::coordinates::block_pos::BlockPosArgumentType;
use crate::command::argument_types::core::double::DoubleArgumentType;
use crate::command::argument_types::entity::EntityArgumentType;
use crate::command::argument_types::identifier::IdentifierArgumentType;
use crate::command::argument_types::nbt_path::{NbtPath, NbtPathArgumentType};
use crate::command::argument_types::objective::ObjectiveArgumentType;
use crate::command::argument_types::score_holder::ScoreHolderArgumentType;
use crate::command::context::command_context::CommandContext;
use crate::command::context::command_source::{ResultValueTaker, ReturnValue, ReturnValueCallable};
use crate::command::errors::error_types::CommandErrorType;
use crate::command::node::{RedirectModifier, RedirectModifierResult, Redirection};
use pumpkin_nbt::tag::NbtTag;
use pumpkin_util::text::TextComponent;
use std::sync::Arc;

const ERROR_UNKNOWN_BOSSBAR: CommandErrorType<1> =
    CommandErrorType::new("commands.bossbar.unknown", "commands.bossbar.unknown");

struct StoreCallback<F>(F);

impl<F: Fn(ReturnValue) + Send + Sync> ReturnValueCallable for StoreCallback<F> {
    fn call(&self, result: ReturnValue) {
        (self.0)(result);
    }
}

fn with_callback(
    context: &CommandContext,
    callback: impl Fn(ReturnValue) + Send + Sync + 'static,
) -> Vec<Arc<crate::command::CommandSource>> {
    let taker = ResultValueTaker(vec![Arc::new(StoreCallback(callback))]);
    vec![Arc::new(
        context
            .source
            .as_ref()
            .clone()
            .merge_command_result_taker(&taker),
    )]
}

const fn stored_value(result: ReturnValue, store_result: bool) -> i32 {
    if store_result {
        result.result_value()
    } else {
        result.success_value() as i32
    }
}

fn store_score(context: &CommandContext, store_result: bool) -> RedirectModifierResult {
    let holders = ScoreHolderArgumentType::get_score_holders(context, "store_holders")?;
    let objective = ObjectiveArgumentType::get(context, "store_objective")?.to_string();
    let world = context.world().clone();
    {
        let scoreboard = world
            .scoreboard
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        ObjectiveArgumentType::objective_or_error(&scoreboard, &objective)?
    };
    // ExecuteCommand.storeValue chains callbacks, including failure (which stores zero).
    Ok(with_callback(context, move |result| {
        let mut scoreboard = world
            .scoreboard
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for holder in &holders {
            scoreboard.set_score_value(
                &world,
                holder.name.clone(),
                objective.clone(),
                stored_value(result, store_result),
            );
        }
    }))
}

fn store_bossbar(
    context: &CommandContext,
    store_result: bool,
    into_value: bool,
) -> RedirectModifierResult {
    let id = IdentifierArgumentType::get(context, "store_id")?.to_string();
    let server = context.server().clone();
    if !server
        .bossbars
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .has_bossbar(&id)
    {
        return Err(ERROR_UNKNOWN_BOSSBAR.create_without_context(TextComponent::text(id)));
    }
    Ok(with_callback(context, move |result| {
        let mut bars = server
            .bossbars
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let value = stored_value(result, store_result);
        if into_value {
            let _ = bars.update_value(&server, id.clone(), value);
        } else {
            let _ = bars.update_max(&server, id.clone(), value);
        }
    }))
}

#[derive(Clone, Copy)]
enum NumberType {
    Byte,
    Short,
    Int,
    Long,
    Float,
    Double,
}

impl NumberType {
    const ALL: [(&'static str, Self); 6] = [
        ("byte", Self::Byte),
        ("short", Self::Short),
        ("int", Self::Int),
        ("long", Self::Long),
        ("float", Self::Float),
        ("double", Self::Double),
    ];

    fn tag(self, value: i32, scale: f64) -> NbtTag {
        let value = f64::from(value) * scale;
        // ExecuteCommand.wrapStores (26.3) truncates, then Java narrows byte/short after int conversion.
        match self {
            Self::Byte => NbtTag::Byte(value as i32 as i8),
            Self::Short => NbtTag::Short(value as i32 as i16),
            Self::Int => NbtTag::Int(value as i32),
            Self::Long => NbtTag::Long(value as i64),
            Self::Float => NbtTag::Float(value as f32),
            Self::Double => NbtTag::Double(value),
        }
    }
}

fn store_data(
    context: &CommandContext,
    kind: TargetKind,
    number: NumberType,
    store_result: bool,
) -> RedirectModifierResult {
    let accessor = kind.access(context, "store")?;
    let path = context.get_argument::<NbtPath>("store_path")?.clone();
    let scale = DoubleArgumentType::get(context, "store_scale")?;
    // ExecuteCommand.storeData silently ignores data-access/path errors in its result callback.
    Ok(with_callback(context, move |result| {
        if let Ok(data) = accessor.get_data() {
            let mut data = NbtTag::Compound(data);
            if path
                .set(
                    &mut data,
                    number.tag(stored_value(result, store_result), scale),
                )
                .is_ok()
                && let NbtTag::Compound(data) = data
            {
                let _ = accessor.set_data(&data);
            }
        }
    }))
}

fn data_types(
    kind: TargetKind,
    store_result: bool,
) -> crate::command::argument_builder::RequiredArgumentBuilder {
    let mut path = argument("store_path", NbtPathArgumentType);
    for (name, number) in NumberType::ALL {
        path = path.then(literal(name).then(
            argument("store_scale", DoubleArgumentType::any()).redirect_with_modifier(
                Redirection::Root,
                RedirectModifier::Custom(Arc::new(move |context| {
                    store_data(context, kind, number, store_result)
                })),
            ),
        ));
    }
    path
}

fn stores(name: &'static str, store_result: bool) -> LiteralArgumentBuilder {
    let mut bossbar = argument("store_id", IdentifierArgumentType);
    for (name, into_value) in [("value", true), ("max", false)] {
        bossbar = bossbar.then(literal(name).redirect_with_modifier(
            Redirection::Root,
            RedirectModifier::Custom(Arc::new(move |context| {
                store_bossbar(context, store_result, into_value)
            })),
        ));
    }
    literal(name)
        .then(literal("score").then(
            argument("store_holders", ScoreHolderArgumentType::Multiple).then(
                argument("store_objective", ObjectiveArgumentType).redirect_with_modifier(
                    Redirection::Root,
                    RedirectModifier::Custom(Arc::new(move |context| {
                        store_score(context, store_result)
                    })),
                ),
            ),
        ))
        .then(literal("bossbar").then(bossbar))
        .then(
            literal("storage").then(
                argument("store_target", IdentifierArgumentType)
                    .then(data_types(TargetKind::Storage, store_result)),
            ),
        )
        .then(
            literal("block").then(
                argument("store_target", BlockPosArgumentType)
                    .then(data_types(TargetKind::Block, store_result)),
            ),
        )
        .then(
            literal("entity").then(
                argument("store_target", EntityArgumentType::Entity)
                    .then(data_types(TargetKind::Entity, store_result)),
            ),
        )
}

pub(super) fn add_store(builder: CommandArgumentBuilder) -> CommandArgumentBuilder {
    builder.then(
        literal("store")
            .then(stores("result", true))
            .then(stores("success", false)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::CommandSource;
    use crate::command::argument_builder::command;
    use crate::command::node::dispatcher::CommandDispatcher;

    struct Probe;
    impl crate::command::node::CommandExecutor for Probe {
        fn execute(&self, _: &CommandContext) -> crate::command::node::CommandExecutorResult {
            Ok(7)
        }
    }

    #[test]
    fn store_grammar_accepts_all_vanilla_destinations() {
        let mut dispatcher = CommandDispatcher::new();
        dispatcher.register(command("probe", "probe").executes(Probe));
        let mut builder =
            command("execute", "execute").then(literal("run").redirect(Redirection::Root));
        builder = add_store(builder);
        let id = dispatcher.register(builder);
        super::super::execute::set_redirects_to_execute(&mut dispatcher.tree, id.into(), id);
        let source = Arc::new(CommandSource::dummy());
        for mode in ["result", "success"] {
            let mut destinations = vec![
                "score value repro".to_string(),
                "bossbar repro:bar value".to_string(),
                "bossbar repro:bar max".to_string(),
            ];
            for target in ["storage repro:state", "block 0 64 0", "entity @s"] {
                for (number, _) in NumberType::ALL {
                    destinations.push(format!("{target} value {number} 0.5"));
                }
            }
            for destination in destinations {
                let input = format!("execute store {mode} {destination} run probe");
                let parsed = dispatcher.parse_input(&input, &source);
                assert!(parsed.errors.is_empty(), "{input}: {:?}", parsed.errors);
                assert_eq!(parsed.reader.remaining_length(), 0, "{input}");
            }
        }
    }

    #[test]
    fn integral_stores_truncate_and_narrow_like_java() {
        assert_eq!(NumberType::Int.tag(-1, 0.5), NbtTag::Int(0));
        assert_eq!(NumberType::Byte.tag(255, 1.0), NbtTag::Byte(-1));
        assert_eq!(stored_value(ReturnValue::Failure, true), 0);
        assert_eq!(stored_value(ReturnValue::Success(0), false), 1);
    }

    #[tokio::test]
    async fn store_callbacks_write_command_results_and_failures()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        // Existing service fixtures construct a world without listeners, ticks or startup tasks.
        let server = crate::server::combat_test_support::server(directory.path());
        let world = crate::server::combat_test_support::world(&server, directory.path());
        let mut source = CommandSource::dummy();
        source.server = Some(server.clone());
        source.world = Some(world.clone());
        source.silent = true;
        let dispatcher = server.command_dispatcher.load();
        assert_eq!(
            dispatcher.execute_input("scoreboard objectives add repro dummy", &source),
            Ok(1)
        );
        dispatcher
            .execute_input("time set 100", &source)
            .map_err(|error| format!("{error:?}"))?;
        dispatcher
            .execute_input(
                "execute store result score value repro run time query daytime",
                &source,
            )
            .map_err(|error| format!("{error:?}"))?;
        let score = || {
            world
                .scoreboard
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get_score("value", "repro")
                .map(|score| score.value.0)
        };
        assert_eq!(score(), Some(100));
        let failed = dispatcher.execute_input(
            "execute store success score value repro run clear @a",
            &source,
        );
        assert!(failed.is_err());
        assert_eq!(score(), Some(0));
        dispatcher
            .execute_input(
                "execute store result storage repro:state value int -0.005 run time query daytime",
                &source,
            )
            .map_err(|error| format!("{error:?}"))?;
        {
            let storage = server
                .command_storage
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            assert_eq!(
                storage
                    .get("repro:state")
                    .and_then(|nbt| nbt.get_int("value")),
                Some(0)
            );
        };
        crate::server::fixture_lifecycle::finish().await;
        Ok(())
    }
}
