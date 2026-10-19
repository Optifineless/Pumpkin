use pumpkin_protocol::{NumberFormat, codec::var_int::VarInt};
use tracing::warn;

use super::{Scoreboard, ScoreboardObjective, ScoreboardScore, ScoreboardTarget};

impl Scoreboard {
    /// Sets an objective's number format, replacing any preserved unsupported style fields.
    pub fn set_objective_number_format(
        &mut self,
        target: &impl ScoreboardTarget,
        name: &str,
        number_format: Option<NumberFormat>,
    ) {
        let Some(mut objective) = self.get_objective(name).cloned() else {
            warn!("Tried to update an objective which does not exist: {name}");
            return;
        };
        objective.number_format = number_format;
        self.update_objective_with_number_format(target, objective);
    }

    /// Updates objective settings with an explicit number-format replacement.
    pub fn update_objective_with_number_format(
        &mut self,
        target: &impl ScoreboardTarget,
        objective: ScoreboardObjective,
    ) {
        self.persistence.discard_objective_format(&objective.name);
        self.update_objective(target, objective);
    }

    /// Sets a score's number format, creating a zero score when necessary.
    pub fn set_score_number_format(
        &mut self,
        target: &impl ScoreboardTarget,
        entity_name: &str,
        objective_name: &str,
        number_format: Option<NumberFormat>,
    ) {
        let mut score = self
            .get_score(entity_name, objective_name)
            .cloned()
            .unwrap_or_else(|| {
                ScoreboardScore::new(entity_name, objective_name, VarInt(0), None, None)
            });
        score.number_format = number_format;
        self.update_score_with_number_format(target, score);
    }

    /// Updates score fields with an explicit number-format replacement.
    pub fn update_score_with_number_format(
        &mut self,
        target: &impl ScoreboardTarget,
        score: ScoreboardScore,
    ) {
        self.persistence
            .discard_score_format(&score.entity_name, &score.objective_name);
        self.update_score(target, score);
    }
}
