use crate::argument_types::FromStringReader;
use crate::argument_types::argument_type::{ArgumentType, JavaClientArgumentType};
use crate::context::command_context::CommandContext;
use crate::errors::command_syntax_error::CommandSyntaxError;
use crate::errors::error_types::CommandErrorType;
use crate::snbt::SnbtParser;
use crate::string_reader::StringReader;
use crate::suggestion::suggestions::{Suggestions, SuggestionsBuilder};
use pumpkin_data::particle::Particle;
use pumpkin_data::translation;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::identifier::Identifier;
use pumpkin_util::text::TextComponent;

#[path = "particle_options.rs"]
mod particle_options;

pub const ERROR_UNKNOWN_PARTICLE: CommandErrorType<1> = CommandErrorType::new(
    translation::java::PARTICLE_NOTFOUND,
    translation::bedrock::COMMANDS_PARTICLE_NOTFOUND,
);

pub struct ParticleArgumentType;

#[derive(Clone, Debug, PartialEq, Eq)]
/// A particle type and its validated 26.3 option payload, excluding the particle registry ID.
pub struct ParticleArgument {
    pub particle: Particle,
    pub data: Vec<u8>,
}

const ERROR_INVALID_OPTIONS: CommandErrorType<1> = CommandErrorType::new(
    translation::java::PARTICLE_INVALIDOPTIONS,
    translation::java::PARTICLE_INVALIDOPTIONS,
);

impl<S: crate::source::CommandSource> ArgumentType<S> for ParticleArgumentType {
    type Item = ParticleArgument;

    fn parse(&self, reader: &mut StringReader) -> Result<Self::Item, CommandSyntaxError> {
        let identifier = Identifier::from_reader(reader)?;
        let particle = Particle::from_name(identifier.path())
            .or_else(|| Particle::from_name(&identifier.to_string()))
            .ok_or_else(|| {
                ERROR_UNKNOWN_PARTICLE.create(reader, TextComponent::text(identifier.to_string()))
            })?;
        // ParticleArgument.readParticle parses the optional SNBT map before the coordinates.
        let nbt = if reader.peek() == Some('{') {
            SnbtParser::parse_for_commands(reader)?
                .extract_compound()
                .cloned()
                .ok_or_else(|| {
                    ERROR_INVALID_OPTIONS.create(reader, TextComponent::text("Expected compound"))
                })?
        } else {
            NbtCompound::new()
        };
        let data = particle_options::encode(particle, &nbt).ok_or_else(|| {
            ERROR_INVALID_OPTIONS.create(
                reader,
                TextComponent::text("Invalid or missing particle options"),
            )
        })?;
        Ok(ParticleArgument { particle, data })
    }

    fn client_side_parser(&self) -> JavaClientArgumentType {
        JavaClientArgumentType::Particle
    }

    fn examples(&self) -> Vec<String> {
        vec![
            "foo".to_string(),
            "foo:bar".to_string(),
            "particle".to_string(),
        ]
    }

    fn list_suggestions(
        &self,
        _context: &CommandContext<S>,
        builder: SuggestionsBuilder,
    ) -> Suggestions {
        let particles = (0..=124u16)
            .filter_map(Particle::from_id)
            .map(|p| format!("{p:?}").to_lowercase())
            .collect();
        builder.filter_and_suggest_lowercase(particles).build()
    }
}

impl ParticleArgumentType {
    pub fn get<'a, S: crate::source::CommandSource>(
        context: &'a CommandContext<S>,
        name: &str,
    ) -> Result<&'a ParticleArgument, CommandSyntaxError> {
        context.get_argument::<ParticleArgument>(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::DummySource;

    #[test]
    fn particle_argument_consumes_its_options_and_requires_mandatory_fields() {
        let mut reader = StringReader::new("dust{color:16711680,scale:1f} ~ ~ ~");
        ArgumentType::<DummySource>::parse(&ParticleArgumentType, &mut reader).unwrap();
        assert_eq!(reader.remaining_part(), " ~ ~ ~");
        assert!(
            ArgumentType::<DummySource>::parse(
                &ParticleArgumentType,
                &mut StringReader::new("dust")
            )
            .is_err()
        );
    }

    #[test]
    fn particle_argument_builds_vanilla_option_bytes() {
        for (input, expected) in [
            (
                "dust{color:16711680,scale:1f}",
                vec![0, 0xff, 0, 0, 0x3f, 0x80, 0, 0],
            ),
            ("effect", vec![0xff, 0xff, 0xff, 0xff, 0x3f, 0x80, 0, 0]),
            (
                // Vanilla Codec.INT/NbtOps narrows this long to -1.
                "effect{color:4294967295L,power:2f}",
                vec![0xff, 0xff, 0xff, 0xff, 0x40, 0, 0, 0],
            ),
            (
                "trail{target:[0.25d,65d,0.25d],color:16545810,duration:10}",
                vec![
                    0x3f, 0xd0, 0, 0, 0, 0, 0, 0, // x = .25
                    0x40, 0x50, 0x40, 0, 0, 0, 0, 0, // y = 65
                    0x3f, 0xd0, 0, 0, 0, 0, 0, 0, // z = .25
                    0, 0xfc, 0x78, 0x12, 10, // color and duration
                ],
            ),
        ] {
            let parsed = ArgumentType::<DummySource>::parse(
                &ParticleArgumentType,
                &mut StringReader::new(input),
            )
            .unwrap();
            assert_eq!(parsed.data, expected);
        }
    }
}
