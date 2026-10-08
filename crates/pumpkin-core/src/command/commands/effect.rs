use pumpkin_data::effect::StatusEffect;
use pumpkin_data::potion::Effect;
use pumpkin_data::translation;
use pumpkin_util::PermissionLvl;
use pumpkin_util::permission::{Permission, PermissionDefault, PermissionRegistry};
use pumpkin_util::text::TextComponent;

use crate::command::argument_builder::{ArgumentBuilder, argument, command, literal};
use crate::command::argument_types::core::bool::BoolArgumentType;
use crate::command::argument_types::core::integer::IntegerArgumentType;
use crate::command::argument_types::entity::EntityArgumentType;
use crate::command::argument_types::resource::{MOB_EFFECT_ARGUMENT, ResourceArgument};
use crate::command::context::command_context::CommandContext;
use crate::command::errors::error_types::CommandErrorType;
use crate::command::node::dispatcher::CommandDispatcher;
use crate::command::node::{CommandExecutor, CommandExecutorResult};

const DESCRIPTION: &str = "Adds or removes the status effects of players and other entities.";
const PERMISSION: &str = "minecraft:command.effect";

const ERROR_GIVE_FAILED: CommandErrorType<0> = CommandErrorType::new(
    translation::java::COMMANDS_EFFECT_GIVE_FAILED,
    translation::java::COMMANDS_EFFECT_GIVE_FAILED,
);

const ERROR_CLEAR_EVERYTHING_FAILED: CommandErrorType<0> = CommandErrorType::new(
    translation::java::COMMANDS_EFFECT_CLEAR_EVERYTHING_FAILED,
    translation::java::COMMANDS_EFFECT_CLEAR_EVERYTHING_FAILED,
);

const ERROR_CLEAR_SPECIFIC_FAILED: CommandErrorType<0> = CommandErrorType::new(
    translation::java::COMMANDS_EFFECT_CLEAR_SPECIFIC_FAILED,
    translation::java::COMMANDS_EFFECT_CLEAR_SPECIFIC_FAILED,
);

#[derive(Clone, Copy)]
enum Duration {
    Default,
    Specified,
    Infinite,
}

struct GiveExecutor {
    duration: Duration,
    has_amplifier: bool,
    has_hide_particles: bool,
}

impl CommandExecutor for GiveExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let targets = EntityArgumentType::get_entities(context, "targets")?;
        let effect = ResourceArgument::get_mob_effect(context, "effect")?;

        let seconds = match self.duration {
            Duration::Default => None,
            Duration::Specified => Some(IntegerArgumentType::get(context, "seconds")?),
            Duration::Infinite => Some(-1),
        };
        let duration_ticks = compute_duration_in_ticks(seconds, effect);

        let amplifier = if self.has_amplifier {
            IntegerArgumentType::get(context, "amplifier")? as u8
        } else {
            0
        };

        let hide_particles = if self.has_hide_particles {
            BoolArgumentType::get(context, "hideParticles")?
        } else {
            false
        };

        let mut successes = 0;

        let mut first_success = None;
        for target in &targets {
            let Some(living) = target.get_living_entity() else {
                continue;
            };
            let changed = apply_effect(
                living,
                Effect {
                    effect_type: effect,
                    duration: duration_ticks,
                    amplifier,
                    ambient: false,
                    show_particles: !hide_particles,
                    show_icon: !hide_particles,
                    blend: false,
                },
            );
            if changed {
                successes += 1;
                first_success.get_or_insert(target);
            }
        }

        let translation_name = TextComponent::translate_cross(
            effect.translation_key.to_string(),
            effect.translation_key.to_string(),
            [],
        );

        if successes == 0 {
            return Err(ERROR_GIVE_FAILED.create_without_context());
        }

        if successes == 1 {
            context.source.send_feedback(
                TextComponent::translate_cross(
                    translation::java::COMMANDS_EFFECT_GIVE_SUCCESS_SINGLE,
                    translation::java::COMMANDS_EFFECT_GIVE_SUCCESS_SINGLE,
                    [
                        translation_name,
                        first_success
                            .ok_or_else(|| ERROR_GIVE_FAILED.create_without_context())?
                            .get_display_name(),
                        TextComponent::text((duration_ticks / 20).to_string()),
                    ],
                ),
                true,
            );
        } else {
            context.source.send_feedback(
                TextComponent::translate_cross(
                    translation::java::COMMANDS_EFFECT_GIVE_SUCCESS_MULTIPLE,
                    translation::java::COMMANDS_EFFECT_GIVE_SUCCESS_MULTIPLE,
                    [
                        translation_name,
                        TextComponent::text(successes.to_string()),
                        TextComponent::text((duration_ticks / 20).to_string()),
                    ],
                ),
                true,
            );
        }

        Ok(successes)
    }
}

// EffectCommands.giveEffect delegates upgrade and immunity decisions to LivingEntity.addEffect.
fn apply_effect(living: &crate::entity::living::LivingEntity, effect: Effect) -> bool {
    let kind = effect.effect_type;
    let before = living.get_effect(kind);
    living.add_effect(effect);
    let after = living.get_effect(kind);
    match (before, after) {
        (None, Some(_)) => true,
        (Some(before), Some(after)) => {
            before.duration != after.duration
                || before.amplifier != after.amplifier
                || before.ambient != after.ambient
                || before.show_particles != after.show_particles
                || before.show_icon != after.show_icon
        }
        // add_effect returns no admission result for an instant effect rejected by entity logic.
        _ => is_instantaneous(kind),
    }
}

enum ClearMode {
    SelfAll,
    TargetsAll,
    TargetsSpecific,
}

struct ClearExecutor(ClearMode);

impl CommandExecutor for ClearExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let targets = match self.0 {
            ClearMode::SelfAll => vec![context.source.entity_or_err()?],
            ClearMode::TargetsAll | ClearMode::TargetsSpecific => {
                EntityArgumentType::get_entities(context, "targets")?
            }
        };
        let effect = if matches!(self.0, ClearMode::TargetsSpecific) {
            Some(ResourceArgument::get_mob_effect(context, "effect")?)
        } else {
            None
        };
        let mut successes = 0;
        let mut first = None;
        // EffectCommands.clearEffect / clearEffects ignore non-living targets and count actual removals.
        for target in &targets {
            let Some(living) = target.get_living_entity() else {
                continue;
            };
            let changed = effect.map_or_else(
                || {
                    let has_effects = !living
                        .active_effects
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .is_empty();
                    living.reset_effects_and_attributes();
                    has_effects
                },
                |effect| living.remove_effect(effect),
            );
            if changed {
                successes += 1;
                first.get_or_insert(target);
            }
        }
        let (failure, single_key, multiple_key) = if effect.is_some() {
            (
                &ERROR_CLEAR_SPECIFIC_FAILED,
                translation::java::COMMANDS_EFFECT_CLEAR_SPECIFIC_SUCCESS_SINGLE,
                translation::java::COMMANDS_EFFECT_CLEAR_SPECIFIC_SUCCESS_MULTIPLE,
            )
        } else {
            (
                &ERROR_CLEAR_EVERYTHING_FAILED,
                translation::java::COMMANDS_EFFECT_CLEAR_EVERYTHING_SUCCESS_SINGLE,
                translation::java::COMMANDS_EFFECT_CLEAR_EVERYTHING_SUCCESS_MULTIPLE,
            )
        };
        if successes == 0 {
            return Err(failure.create_without_context());
        }
        let target = if successes == 1 {
            first
                .ok_or_else(|| failure.create_without_context())?
                .get_display_name()
        } else {
            TextComponent::text(successes.to_string())
        };
        let mut args = Vec::new();
        if let Some(effect) = effect {
            args.push(TextComponent::translate_cross(
                effect.translation_key,
                effect.translation_key,
                [],
            ));
        }
        args.push(target);
        let key = if successes == 1 {
            single_key
        } else {
            multiple_key
        };
        context
            .source
            .send_feedback(TextComponent::translate_cross(key, key, args), true);
        Ok(successes)
    }
}

// MobEffects registers these with InstantaneousMobEffect subclasses, rather than duration effects.
fn is_instantaneous(effect: &StatusEffect) -> bool {
    effect == &StatusEffect::INSTANT_HEALTH
        || effect == &StatusEffect::INSTANT_DAMAGE
        || effect == &StatusEffect::SATURATION
}

// EffectCommands.computeDurationInTicks: instant durations are ticks, including the default of one.
fn compute_duration_in_ticks(seconds: Option<i32>, effect: &StatusEffect) -> i32 {
    match seconds {
        Some(seconds) if is_instantaneous(effect) || seconds == -1 => seconds,
        Some(seconds) => seconds * 20,
        None if is_instantaneous(effect) => 1,
        None => 600,
    }
}

pub fn register(dispatcher: &mut CommandDispatcher, registry: &PermissionRegistry) {
    registry.register_permission_or_panic(Permission::new(
        PERMISSION,
        DESCRIPTION,
        PermissionDefault::Op(PermissionLvl::Two),
    ));

    let seconds_node = argument("seconds", IntegerArgumentType::new(1, 1_000_000))
        .executes(GiveExecutor {
            duration: Duration::Specified,
            has_amplifier: false,
            has_hide_particles: false,
        })
        .then(
            argument("amplifier", IntegerArgumentType::new(0, 255))
                .executes(GiveExecutor {
                    duration: Duration::Specified,
                    has_amplifier: true,
                    has_hide_particles: false,
                })
                .then(
                    argument("hideParticles", BoolArgumentType).executes(GiveExecutor {
                        duration: Duration::Specified,
                        has_amplifier: true,
                        has_hide_particles: true,
                    }),
                ),
        );

    let infinite_node = literal("infinite")
        .executes(GiveExecutor {
            duration: Duration::Infinite,
            has_amplifier: false,
            has_hide_particles: false,
        })
        .then(
            argument("amplifier", IntegerArgumentType::new(0, 255))
                .executes(GiveExecutor {
                    duration: Duration::Infinite,
                    has_amplifier: true,
                    has_hide_particles: false,
                })
                .then(
                    argument("hideParticles", BoolArgumentType).executes(GiveExecutor {
                        duration: Duration::Infinite,
                        has_amplifier: true,
                        has_hide_particles: true,
                    }),
                ),
        );

    let give_node = literal("give").then(
        argument("targets", EntityArgumentType::Entities).then(
            argument("effect", MOB_EFFECT_ARGUMENT.clone())
                .executes(GiveExecutor {
                    duration: Duration::Default,
                    has_amplifier: false,
                    has_hide_particles: false,
                })
                .then(seconds_node)
                .then(infinite_node),
        ),
    );

    let clear_node = literal("clear")
        .executes(ClearExecutor(ClearMode::SelfAll))
        .then(
            argument("targets", EntityArgumentType::Entities)
                .executes(ClearExecutor(ClearMode::TargetsAll))
                .then(
                    argument("effect", MOB_EFFECT_ARGUMENT.clone())
                        .executes(ClearExecutor(ClearMode::TargetsSpecific)),
                ),
        );

    dispatcher.register(
        command("effect", DESCRIPTION)
            .requires(PERMISSION)
            .then(clear_node)
            .then(give_node),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instant_effect_duration_is_not_multiplied_by_twenty() {
        for instant in [
            &StatusEffect::INSTANT_HEALTH,
            &StatusEffect::INSTANT_DAMAGE,
            &StatusEffect::SATURATION,
        ] {
            assert_eq!(compute_duration_in_ticks(None, instant), 1);
            assert_eq!(compute_duration_in_ticks(Some(2), instant), 2);
            assert_eq!(compute_duration_in_ticks(Some(-1), instant), -1);
        }
        assert_eq!(compute_duration_in_ticks(None, &StatusEffect::SPEED), 600);
        assert_eq!(compute_duration_in_ticks(Some(2), &StatusEffect::SPEED), 40);
        assert_eq!(
            compute_duration_in_ticks(Some(-1), &StatusEffect::SPEED),
            -1
        );
    }
}
