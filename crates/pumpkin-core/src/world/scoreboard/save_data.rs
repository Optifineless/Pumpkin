use pumpkin_data::scoreboard::ScoreboardDisplaySlot;
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_world::world_info::WorldInfoError;
use rustc_hash::{FxHashMap, FxHashSet};

use super::save_data_values::{self as values, DISPLAY_SLOTS};
use super::{Scoreboard, ScoreboardObjective, ScoreboardScore, Team};

#[derive(Clone, Debug)]
pub(super) struct SavedRecord<T> {
    raw: NbtCompound,
    baseline: T,
    replaced_fields: FxHashSet<&'static str>,
}

impl<T> SavedRecord<T> {
    pub(super) fn discard(&mut self, field: &'static str) -> bool {
        self.raw.child_tags.remove(field);
        self.replaced_fields.insert(field)
    }
}

impl SavedRecord<ScoreboardObjective> {
    pub(super) fn discard_changed_components(&mut self, current: &ScoreboardObjective) {
        if current.display_name != self.baseline.display_name {
            self.discard("DisplayName");
        }
        if current.number_format != self.baseline.number_format {
            self.discard("format");
        }
    }
}

impl SavedRecord<Team> {
    pub(super) fn discard_changed_components(&mut self, current: &Team) {
        if current.display_name != self.baseline.display_name {
            self.discard("DisplayName");
        }
        if current.player_prefix != self.baseline.player_prefix {
            self.discard("MemberNamePrefix");
        }
        if current.player_suffix != self.baseline.player_suffix {
            self.discard("MemberNameSuffix");
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct PersistentData {
    root: NbtCompound,
    pub objectives: FxHashMap<String, SavedRecord<ScoreboardObjective>>,
    pub scores: FxHashMap<(String, String), SavedRecord<ScoreboardScore>>,
    pub teams: FxHashMap<String, SavedRecord<Team>>,
    pub orphan_scores: FxHashMap<(String, String), NbtCompound>,
    pub orphan_display_slots: FxHashMap<ScoreboardDisplaySlot, String>,
}

impl PersistentData {
    pub(super) fn discard_objective_format(&mut self, name: &str) {
        if let Some(saved) = self.objectives.get_mut(name) {
            saved.discard("format");
        }
    }

    pub(super) fn discard_score_format(&mut self, entity_name: &str, objective_name: &str) {
        if let Some(saved) = self
            .scores
            .get_mut(&(entity_name.to_string(), objective_name.to_string()))
        {
            saved.discard("format");
        }
    }
}

impl Scoreboard {
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub const fn is_dirty(&self) -> bool {
        self.revision != self.saved_revision
    }

    pub const fn mark_saved(&mut self, revision: u64) {
        // A write can finish after another command has changed the live scoreboard.
        if self.revision == revision {
            self.saved_revision = revision;
        }
    }

    pub(super) const fn mark_dirty(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    /// Loads the data payload of vanilla's `ScoreboardSaveData.Packed` codec.
    pub fn from_save_data(data: &NbtCompound) -> Result<Self, WorldInfoError> {
        let mut scoreboard = Self::new();
        scoreboard.persistence.root = data.clone();
        for tag in values::list(data, "Objectives")? {
            let raw = values::compound(tag, "objective")?;
            let objective = values::read_objective(raw)?;
            let name = objective.name.clone();
            if scoreboard
                .objectives
                .insert(name.clone(), objective.clone())
                .is_some()
            {
                return Err(values::invalid("duplicate objective"));
            }
            scoreboard.persistence.objectives.insert(
                name,
                SavedRecord {
                    raw: raw.clone(),
                    baseline: objective,
                    replaced_fields: FxHashSet::default(),
                },
            );
        }
        scoreboard.load_scores(data)?;
        scoreboard.rebuild_bedrock_score_identities();
        scoreboard.load_display_slots(data)?;
        for tag in values::list(data, "Teams")? {
            let raw = values::compound(tag, "team")?;
            let team = values::read_team(raw)?;
            let name = team.name.clone();
            if scoreboard
                .teams
                .insert(name.clone(), team.clone())
                .is_some()
            {
                return Err(values::invalid("duplicate team"));
            }
            scoreboard.persistence.teams.insert(
                name,
                SavedRecord {
                    raw: raw.clone(),
                    baseline: team,
                    replaced_fields: FxHashSet::default(),
                },
            );
        }
        Ok(scoreboard)
    }

    fn load_scores(&mut self, data: &NbtCompound) -> Result<(), WorldInfoError> {
        for tag in values::list(data, "PlayerScores")? {
            let raw = values::compound(tag, "player score")?;
            let score = values::read_score(raw)?;
            let key = (score.entity_name.clone(), score.objective_name.clone());
            if !self.objectives.contains_key(&score.objective_name) {
                if self
                    .persistence
                    .orphan_scores
                    .insert(key, raw.clone())
                    .is_some()
                {
                    return Err(values::invalid("duplicate player score"));
                }
                continue;
            }
            if self
                .scores
                .entry(key.1.clone())
                .or_default()
                .insert(key.0.clone(), score.clone())
                .is_some()
            {
                return Err(values::invalid("duplicate player score"));
            }
            self.persistence.scores.insert(
                key,
                SavedRecord {
                    raw: raw.clone(),
                    baseline: score,
                    replaced_fields: FxHashSet::default(),
                },
            );
        }
        Ok(())
    }

    fn load_display_slots(&mut self, data: &NbtCompound) -> Result<(), WorldInfoError> {
        let Some(tag) = data.get("DisplaySlots") else {
            return Ok(());
        };
        for (key, value) in &values::compound(tag, "DisplaySlots")?.child_tags {
            let slot = DISPLAY_SLOTS
                .iter()
                .find(|(_, name)| *name == key.as_ref())
                .map(|(slot, _)| *slot)
                .ok_or_else(|| values::invalid("unknown display slot"))?;
            let objective = value
                .extract_string()
                .ok_or_else(|| values::invalid("display objective must be a string"))?;
            if !self.objectives.contains_key(objective) {
                self.persistence
                    .orphan_display_slots
                    .insert(slot, objective.to_string());
                continue;
            }
            self.display_slots.insert(slot, objective.to_string());
        }
        Ok(())
    }

    /// Packs runtime state while retaining component and style fields outside its text model.
    #[must_use]
    pub fn to_save_data(&self) -> NbtCompound {
        let mut data = self.persistence.root.clone();
        let mut objectives: Vec<_> = self.objectives.values().collect();
        objectives.sort_unstable_by(|a, b| a.name.cmp(&b.name));
        data.put_list(
            "Objectives",
            objectives
                .into_iter()
                .map(|value| {
                    pack_record(
                        value,
                        self.persistence.objectives.get(&value.name),
                        values::write_objective,
                    )
                })
                .collect(),
        );
        data.put_list("PlayerScores", self.packed_scores());
        let mut slots = NbtCompound::new();
        for (slot, name) in DISPLAY_SLOTS {
            if let Some(objective) = self
                .display_slots
                .get(&slot)
                .or_else(|| self.persistence.orphan_display_slots.get(&slot))
            {
                slots.put_string(name, objective.clone());
            }
        }
        data.put_compound("DisplaySlots", slots);
        let mut teams: Vec<_> = self.teams.values().collect();
        teams.sort_unstable_by(|a, b| a.name.cmp(&b.name));
        data.put_list(
            "Teams",
            teams
                .into_iter()
                .map(|value| {
                    pack_record(
                        value,
                        self.persistence.teams.get(&value.name),
                        values::write_team,
                    )
                })
                .collect(),
        );
        data
    }
    fn packed_scores(&self) -> Vec<NbtTag> {
        let mut scores: Vec<_> = self
            .scores
            .values()
            .flat_map(FxHashMap::values)
            .map(|value| {
                let key = (value.entity_name.clone(), value.objective_name.clone());
                let tag = pack_record(
                    value,
                    self.persistence.scores.get(&key),
                    values::write_score,
                );
                (key, tag)
            })
            .collect();
        scores.extend(
            self.persistence
                .orphan_scores
                .iter()
                .map(|(key, raw)| (key.clone(), NbtTag::Compound(raw.clone()))),
        );
        scores.sort_unstable_by(|(a, _), (b, _)| a.cmp(b));
        scores.into_iter().map(|(_, tag)| tag).collect()
    }
}

fn pack_record<T>(
    value: &T,
    saved: Option<&SavedRecord<T>>,
    encode: fn(&T) -> NbtCompound,
) -> NbtTag {
    let current = encode(value);
    let Some(saved) = saved else {
        return NbtTag::Compound(current);
    };
    let baseline = encode(&saved.baseline);
    let mut result = saved.raw.clone();
    for name in current.child_tags.keys().chain(baseline.child_tags.keys()) {
        if !saved.replaced_fields.contains(name.as_ref()) && current.get(name) == baseline.get(name)
        {
            continue;
        }
        if let Some(value) = current.get(name) {
            result.put(name, value.clone());
        } else {
            result.child_tags.remove(name);
        }
    }
    NbtTag::Compound(result)
}
