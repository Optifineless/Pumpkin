use pumpkin_data::{Block, BlockStateId, translation};
use pumpkin_util::text::TextComponent;

use crate::argument_types::argument_type::{ArgumentType, JavaClientArgumentType};
use crate::argument_types::block::BlockArgumentType;
use crate::argument_types::block_predicate::{ERROR_NO_VALUE, ERROR_UNCLOSED_PROPERTIES};
use crate::context::command_context::CommandContext;
use crate::errors::command_syntax_error::CommandSyntaxError;
use crate::errors::error_types::CommandErrorType;
use crate::string_reader::StringReader;
use crate::suggestion::suggestions::{Suggestions, SuggestionsBuilder};

pub const ERROR_UNKNOWN_PROPERTY: CommandErrorType<2> = CommandErrorType::new(
    translation::java::ARGUMENT_BLOCK_PROPERTY_UNKNOWN,
    translation::java::ARGUMENT_BLOCK_PROPERTY_UNKNOWN,
);

pub const ERROR_DUPLICATE_PROPERTY: CommandErrorType<2> = CommandErrorType::new(
    translation::java::ARGUMENT_BLOCK_PROPERTY_DUPLICATE,
    translation::java::ARGUMENT_BLOCK_PROPERTY_DUPLICATE,
);

pub const ERROR_INVALID_VALUE: CommandErrorType<3> = CommandErrorType::new(
    translation::java::ARGUMENT_BLOCK_PROPERTY_INVALID,
    translation::java::ARGUMENT_BLOCK_PROPERTY_INVALID,
);

/// A block with optional `[property=value,...]` state, like vanilla's `BlockStateArgument`.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct BlockStateArgumentType;

impl<S: crate::source::CommandSource> ArgumentType<S> for BlockStateArgumentType {
    type Item = BlockInput;

    fn parse(&self, reader: &mut StringReader) -> Result<Self::Item, CommandSyntaxError> {
        let block: &'static Block =
            ArgumentType::<crate::source::DummySource>::parse(&BlockArgumentType, reader)?;
        let (state, properties) = if reader.peek() == Some('[') {
            read_properties(block, reader)?
        } else {
            (block.default_state.id, Vec::new())
        };
        let tag = if reader.peek() == Some('{') {
            Some(ArgumentType::<S>::parse(
                &crate::argument_types::nbt::NbtCompoundArgumentType,
                reader,
            )?)
        } else {
            None
        };
        Ok(BlockInput {
            state,
            properties,
            tag,
        })
    }

    fn client_side_parser(&'_ self) -> JavaClientArgumentType {
        JavaClientArgumentType::BlockState
    }

    fn examples(&self) -> Vec<String> {
        examples!("stone", "minecraft:stone", "stone[foo=bar]")
    }

    fn list_suggestions(
        &self,
        _context: &CommandContext<S>,
        builder: SuggestionsBuilder,
    ) -> Suggestions {
        builder.build()
    }
}

/// Mirrors vanilla `BlockStateParser.readProperties`, applying each property to the default state.
fn read_properties(
    block: &'static Block,
    reader: &mut StringReader,
) -> Result<(BlockStateId, Vec<&'static str>), CommandSyntaxError> {
    let block_name = || TextComponent::text(format!("minecraft:{}", block.name));
    let valid_properties: Vec<(&'static str, &'static str)> = block
        .states
        .iter()
        .filter_map(|state| block.properties(state.id))
        .flat_map(|props| props.to_props())
        .collect();
    let mut properties = block
        .properties(block.default_state.id)
        .map(|props| props.to_props())
        .unwrap_or_default();
    let mut seen: Vec<&'static str> = Vec::new();

    reader.skip();
    reader.skip_whitespace();
    while reader.can_read_char() && reader.peek() != Some(']') {
        reader.skip_whitespace();
        let key_start = reader.cursor();
        let key = reader.read_string()?;
        let Some(&(name, _)) = valid_properties.iter().find(|(name, _)| *name == key) else {
            reader.set_cursor(key_start);
            return Err(ERROR_UNKNOWN_PROPERTY.create(
                reader,
                block_name(),
                TextComponent::text(key),
            ));
        };
        if seen.contains(&name) {
            reader.set_cursor(key_start);
            return Err(ERROR_DUPLICATE_PROPERTY.create(
                reader,
                TextComponent::text(key),
                block_name(),
            ));
        }
        seen.push(name);

        reader.skip_whitespace();
        if reader.peek() != Some('=') {
            return Err(ERROR_NO_VALUE.create(reader, TextComponent::text(key)));
        }
        reader.skip();
        reader.skip_whitespace();

        let value_start = reader.cursor();
        let raw = reader.read_string()?;
        let Some(&(_, value)) = valid_properties
            .iter()
            .find(|(n, v)| *n == name && *v == raw)
        else {
            reader.set_cursor(value_start);
            return Err(ERROR_INVALID_VALUE.create(
                reader,
                block_name(),
                TextComponent::text(raw),
                TextComponent::text(name),
            ));
        };
        if let Some(entry) = properties.iter_mut().find(|(n, _)| *n == name) {
            entry.1 = value;
        }

        reader.skip_whitespace();
        if reader.can_read_char() {
            if reader.peek() != Some(',') {
                if reader.peek() != Some(']') {
                    return Err(ERROR_UNCLOSED_PROPERTIES.create(reader));
                }
                break;
            }
            reader.skip();
        }
    }

    if reader.can_read_char() {
        reader.skip();
    } else {
        return Err(ERROR_UNCLOSED_PROPERTIES.create(reader));
    }

    // BlockStateParser.readProperties leaves the default state for `[]`; the generated
    // from_properties panics on an empty or partial set, so never call it without one.
    if seen.is_empty() || properties.is_empty() {
        return Ok((block.default_state.id, seen));
    }
    Ok((block.from_properties(&properties).to_state_id(block), seen))
}

/// Parsed block state and optional block entity data, mirroring vanilla `BlockInput`.
#[derive(Debug, Clone)]
pub struct BlockInput {
    pub state: BlockStateId,
    pub properties: Vec<&'static str>,
    pub tag: Option<pumpkin_nbt::NbtCompound>,
}

impl BlockStateArgumentType {
    pub fn get<'a, S: crate::source::CommandSource>(
        context: &'a CommandContext<S>,
        name: &str,
    ) -> Result<&'a BlockInput, CommandSyntaxError> {
        context.get_argument(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::DummySource;

    fn parse(input: &str) -> Result<BlockInput, CommandSyntaxError> {
        ArgumentType::<DummySource>::parse(&BlockStateArgumentType, &mut StringReader::new(input))
    }

    #[test]
    fn properties_and_snbt_are_consumed_together() -> Result<(), CommandSyntaxError> {
        let mut reader = StringReader::new("minecraft:chest[facing=west]{CustomName:'Chest'} keep");
        let input = ArgumentType::<DummySource>::parse(&BlockStateArgumentType, &mut reader)?;
        let block = Block::from_state_id(input.state);
        assert!(
            block
                .properties(input.state)
                .is_some_and(|properties| properties.to_props().contains(&("facing", "west")))
        );
        assert_eq!(input.properties, ["facing"]);
        assert_eq!(
            input
                .tag
                .as_ref()
                .and_then(|tag| tag.get_string("CustomName")),
            Some("Chest")
        );
        assert_eq!(reader.remaining_part(), " keep");
        Ok(())
    }

    #[test]
    fn empty_property_lists_keep_the_default_state() -> Result<(), CommandSyntaxError> {
        // BlockStateParser.readProperties accepts `[]` and `[ ]` for any block.
        for (with_brackets, bare) in [
            ("stone[]", "stone"),
            ("stone[ ]", "stone"),
            ("air[]", "air"),
            ("oak_log[]", "oak_log"),
        ] {
            let parsed = parse(with_brackets)?;
            assert_eq!(parsed.state, parse(bare)?.state, "{with_brackets}");
            assert!(parsed.properties.is_empty(), "{with_brackets}");
        }
        Ok(())
    }

    #[test]
    fn invalid_properties_report_specific_syntax_errors() {
        for (input, key) in [
            (
                "birch_stairs[color=red]",
                translation::java::ARGUMENT_BLOCK_PROPERTY_UNKNOWN,
            ),
            (
                "birch_stairs[facing=up]",
                translation::java::ARGUMENT_BLOCK_PROPERTY_INVALID,
            ),
            (
                "birch_stairs[facing=west,facing=east]",
                translation::java::ARGUMENT_BLOCK_PROPERTY_DUPLICATE,
            ),
        ] {
            let Err(error) = parse(input) else {
                panic!("accepted {input}")
            };
            assert_eq!(
                serde_json::to_value(error.message)
                    .ok()
                    .and_then(|v| v.get("translate").cloned()),
                Some(serde_json::Value::String(key.to_string()))
            );
        }
        assert!(parse("birch_stairs[facing=west").is_err());
        assert!(parse("chest{CustomName:").is_err());
    }
}
