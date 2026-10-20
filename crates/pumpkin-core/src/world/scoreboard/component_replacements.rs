use super::{Scoreboard, ScoreboardObjective, ScoreboardScore, ScoreboardTarget, Team};

impl Scoreboard {
    pub(super) fn replace_objective_components(&mut self, objective: &ScoreboardObjective) {
        let Some(existing) = self.objectives.get(&objective.name) else {
            return;
        };
        if let Some(saved) = self.persistence.objectives.get_mut(&objective.name) {
            if existing.display_name != objective.display_name {
                saved.discard("DisplayName");
            }
            if existing.number_format != objective.number_format {
                saved.discard("format");
            }
        }
    }

    pub(super) fn replace_score_components(&mut self, score: &ScoreboardScore) {
        let Some(existing) = self.get_score(&score.entity_name, &score.objective_name) else {
            return;
        };
        let display_changed = existing.display_name != score.display_name;
        let format_changed = existing.number_format != score.number_format;
        let key = (score.entity_name.clone(), score.objective_name.clone());
        if let Some(saved) = self.persistence.scores.get_mut(&key) {
            if display_changed {
                saved.discard("display");
            }
            if format_changed {
                saved.discard("format");
            }
        }
    }

    pub(super) fn replace_team_components(&mut self, team: &Team) {
        let Some(existing) = self.teams.get(&team.name) else {
            return;
        };
        if let Some(saved) = self.persistence.teams.get_mut(&team.name) {
            for (field, old, new) in [
                ("DisplayName", &existing.display_name, &team.display_name),
                (
                    "MemberNamePrefix",
                    &existing.player_prefix,
                    &team.player_prefix,
                ),
                (
                    "MemberNameSuffix",
                    &existing.player_suffix,
                    &team.player_suffix,
                ),
            ] {
                if old != new {
                    saved.discard(field);
                }
            }
        }
    }

    pub(crate) fn discard_team_component(&mut self, name: &str, field: &'static str) -> bool {
        self.persistence
            .teams
            .get_mut(name)
            .is_some_and(|saved| saved.discard(field))
    }

    pub(crate) fn discard_objective_display_name(&mut self, name: &str) -> bool {
        self.persistence
            .objectives
            .get_mut(name)
            .is_some_and(|saved| saved.discard("DisplayName"))
    }

    pub(crate) fn update_score_with_display_name(
        &mut self,
        target: &impl ScoreboardTarget,
        score: ScoreboardScore,
    ) {
        let key = (score.entity_name.clone(), score.objective_name.clone());
        if let Some(saved) = self.persistence.scores.get_mut(&key) {
            saved.discard("display");
        }
        self.update_score(target, score);
    }
}
