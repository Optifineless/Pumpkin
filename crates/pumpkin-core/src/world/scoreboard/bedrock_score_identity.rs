use pumpkin_protocol::{
    bedrock::client::set_score::{CSetScore, ScoreEntry},
    codec::var_int::VarInt,
};
use rustc_hash::FxHashMap;

use super::Scoreboard;

// Runtime translation state only. Cloned save snapshots retain the live IDs.
#[derive(Clone, Debug)]
pub(super) struct BedrockScoreIdentities {
    objectives: FxHashMap<String, FxHashMap<String, i64>>,
    next: Option<i64>,
}

impl Default for BedrockScoreIdentities {
    fn default() -> Self {
        Self {
            objectives: FxHashMap::default(),
            next: Some(1),
        }
    }
}

impl BedrockScoreIdentities {
    pub(super) fn get(&self, holder: &str, objective: &str) -> Option<i64> {
        self.objectives.get(objective)?.get(holder).copied()
    }

    pub(super) fn ensure(&mut self, holder: &str, objective: &str) {
        if self.get(holder, objective).is_some() {
            return;
        }
        let Some(id) = self.next else {
            tracing::error!("Bedrock scoreboard identity space exhausted; score omitted");
            return;
        };
        // Never recycle retired IDs: a queued removal must not address a recreation.
        self.next = id.checked_add(1);
        self.objectives
            .entry(objective.to_owned())
            .or_default()
            .insert(holder.to_owned(), id);
    }

    pub(super) fn remove(&mut self, holder: &str, objective: &str) {
        if let Some(holders) = self.objectives.get_mut(objective) {
            holders.remove(holder);
            if holders.is_empty() {
                self.objectives.remove(objective);
            }
        }
    }

    pub(super) fn remove_holder(&mut self, holder: &str) {
        self.objectives.retain(|_, holders| {
            holders.remove(holder);
            !holders.is_empty()
        });
    }

    pub(super) fn remove_objective(&mut self, objective: &str) {
        self.objectives.remove(objective);
    }
}

impl Scoreboard {
    pub(super) fn rebuild_bedrock_score_identities(&mut self) {
        for (objective, holders) in &self.scores {
            for holder in holders.keys() {
                self.bedrock_score_ids.ensure(holder, objective);
            }
        }
    }

    // ServerScoreboard resets refer to the existing logical record, never a new ID.
    pub(super) fn bedrock_score_removal(&self, holder: &str, objective: Option<&str>) -> CSetScore {
        let entries = self
            .scores
            .iter()
            .filter_map(|(name, holders)| {
                if objective.is_some_and(|objective| objective != name)
                    || !self.is_tracked(name)
                    || !holders.contains_key(holder)
                {
                    return None;
                }
                Some(ScoreEntry {
                    scoreboard_id: self.bedrock_score_ids.get(holder, name)?,
                    objective_name: name.clone(),
                    score: VarInt(0),
                    entry_type: VarInt(3),
                    entity_unique_id: 0,
                    custom_name: holder.to_owned(),
                })
            })
            .collect();
        CSetScore {
            action: VarInt(1),
            entries,
        }
    }

    #[cfg(test)]
    pub(crate) const fn exhaust_bedrock_score_identities_after_next(&mut self) {
        self.bedrock_score_ids.next = Some(i64::MAX);
    }
}
