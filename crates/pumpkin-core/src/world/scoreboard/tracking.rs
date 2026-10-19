use pumpkin_data::scoreboard::ScoreboardDisplaySlot;
use pumpkin_protocol::{
    BClientPacket, Packet,
    bedrock::client::{
        remove_objective::CRemoveObjective as BRemoveObjective,
        set_display_objective::CSetDisplayObjective as BSetDisplayObjective,
        set_score::{CSetScore as BSetScore, ScoreEntry as BScoreEntry},
    },
    codec::var_int::VarInt,
    java::client::play::{
        CDisplayObjective, CSetPlayerTeam, CUpdateObjectives, CUpdateScore, Mode, RenderType,
        TeamMethod, TeamParameters,
    },
};
use pumpkin_util::text::{TextComponent, display_projection};
use rustc_hash::FxHashSet;

use super::{DISPLAY_SLOTS, Scoreboard, ScoreboardObjective, ScoreboardScore, ScoreboardTarget};
use crate::entity::player::Player;

#[cfg(test)]
pub static BEDROCK_TITLES_RENDERED: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

// ScoreboardTarget serializes only the recipient edition. Defer the title until then.
struct BedrockDisplay<F>(F);

impl<F> Packet for BedrockDisplay<F> {
    const PACKET_ID: i32 = BSetDisplayObjective::PACKET_ID;
}

impl<F: Fn() -> BSetDisplayObjective> BClientPacket for BedrockDisplay<F> {
    fn write_packet(&self, writer: impl std::io::Write) -> Result<(), std::io::Error> {
        (self.0)().write_packet(writer)
    }
}

fn bedrock_title(component: &TextComponent, name: &str) -> String {
    #[cfg(test)]
    BEDROCK_TITLES_RENDERED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    display_projection::component(component, name).get_text()
}

impl Scoreboard {
    pub(super) fn is_tracked(&self, name: &str) -> bool {
        self.display_slots
            .values()
            .any(|objective| objective == name)
    }

    pub(super) fn send_objective(
        &self,
        target: &impl ScoreboardTarget,
        objective: &ScoreboardObjective,
        mode: Mode,
    ) {
        let java = CUpdateObjectives::new(
            objective.name.clone(),
            mode,
            objective.display_name.clone(),
            objective.render_type,
            objective.number_format.clone(),
        );
        let bedrock = BedrockDisplay(|| BSetDisplayObjective {
            display_slot_name: "sidebar".into(),
            objective_name: objective.name.clone(),
            objective_display_name: bedrock_title(&objective.display_name, &objective.name),
            criteria_name: "dummy".into(),
            sort_order: VarInt(0),
        });
        self.effects
            .capture(|| target.send_editioned(&java, &bedrock));
    }

    pub(super) fn send_display(
        &self,
        target: &impl ScoreboardTarget,
        slot: ScoreboardDisplaySlot,
        name: Option<&str>,
    ) {
        let name = name.unwrap_or("");
        let java = CDisplayObjective::new(slot, name.to_owned());
        let bedrock = BedrockDisplay(|| BSetDisplayObjective {
            display_slot_name: match slot {
                ScoreboardDisplaySlot::List => "list",
                ScoreboardDisplaySlot::BelowName => "belowname",
                _ => "sidebar",
            }
            .into(),
            objective_name: name.to_owned(),
            objective_display_name: self.objectives.get(name).map_or_else(
                || name.to_owned(),
                |objective| bedrock_title(&objective.display_name, name),
            ),
            criteria_name: "dummy".into(),
            sort_order: VarInt(0),
        });
        self.effects
            .capture(|| target.send_editioned(&java, &bedrock));
    }

    pub(super) fn send_score(&self, target: &impl ScoreboardTarget, score: &ScoreboardScore) {
        let java = CUpdateScore::new(
            score.entity_name.clone(),
            score.objective_name.clone(),
            score.value,
            score.display_name.clone(),
            score.number_format.clone(),
        );
        let bedrock = BSetScore {
            action: VarInt(0),
            entries: vec![BScoreEntry {
                scoreboard_id: score.entity_name.as_ptr() as i64,
                objective_name: score.objective_name.clone(),
                score: score.value,
                entry_type: VarInt(3),
                entity_unique_id: 0,
                custom_name: score.entity_name.clone(),
            }],
        };
        self.effects
            .capture(|| target.send_editioned(&java, &bedrock));
    }

    // ServerScoreboard.getStartTrackingPackets: create, slots, then existing scores.
    pub(super) fn start_tracking(&self, target: &impl ScoreboardTarget, name: &str) {
        let Some(objective) = self.objectives.get(name) else {
            return;
        };
        self.send_objective(target, objective, Mode::Add);
        for (slot, _) in DISPLAY_SLOTS {
            if self.get_display_objective(slot) == Some(name) {
                self.send_display(target, slot, Some(name));
            }
        }
        if let Some(scores) = self.scores.get(name) {
            for score in scores.values() {
                self.send_score(target, score);
            }
        }
    }

    pub(super) fn stop_tracking(&self, target: &impl ScoreboardTarget, name: &str) {
        let java = CUpdateObjectives::new(
            name.to_owned(),
            Mode::Remove,
            TextComponent::empty(),
            RenderType::Integer,
            None,
        );
        let bedrock = BRemoveObjective {
            objective_name: name.to_owned(),
        };
        self.effects
            .capture(|| target.send_editioned(&java, &bedrock));
    }

    pub fn send_to_player(&self, player: &Player) {
        // PlayerList.updateEntireScoreboard sends teams before deduplicated objectives.
        for team in self.teams.values() {
            let parameters = TeamParameters {
                display_name: &team.display_name,
                options: team.options,
                nametag_visibility: team.nametag_visibility.to_str(),
                collision_rule: team.collision_rule.to_str(),
                color: team.color.map_or(-1, |color| color as i32),
                player_prefix: &team.player_prefix,
                player_suffix: &team.player_suffix,
            };
            let packet = CSetPlayerTeam {
                team_name: team.name.clone(),
                method: TeamMethod::Create,
                parameters: Some(parameters),
                players: team.players.clone().into(),
            };
            self.effects
                .capture(|| ScoreboardTarget::send_je(player, &packet));
        }
        let mut sent = FxHashSet::default();
        for (slot, _) in DISPLAY_SLOTS {
            if let Some(name) = self.get_display_objective(slot)
                && sent.insert(name)
            {
                self.start_tracking(player, name);
            }
        }
    }
}
