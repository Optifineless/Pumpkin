pub use pumpkin_data::statistic::{CustomStatistic, StatisticCategory};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::tag::NbtTag;
use rustc_hash::FxHashMap;

#[derive(Default)]
pub struct Statistics {
    /// (Category ID, Statistic ID) -> Value
    pub stats: FxHashMap<(i32, i32), i32>,
}

impl Statistics {
    pub fn increment(&mut self, category: StatisticCategory, stat: i32, amount: i32) {
        let entry = self.stats.entry((category as i32, stat)).or_insert(0);
        *entry = entry.saturating_add(amount);
    }

    pub fn increment_custom(&mut self, stat: CustomStatistic, amount: i32) {
        self.increment(StatisticCategory::Custom, stat as i32, amount);
    }

    pub fn set(&mut self, category: StatisticCategory, stat: i32, value: i32) {
        self.stats.insert((category as i32, stat), value);
    }

    #[must_use]
    pub fn get(&self, category: StatisticCategory, stat: i32) -> i32 {
        *self.stats.get(&(category as i32, stat)).unwrap_or(&0)
    }

    pub fn write_nbt(&self, nbt: &mut NbtCompound) {
        let mut stats_compound = NbtCompound::new();
        for ((category, stat), value) in &self.stats {
            stats_compound.put_int(&format!("{category}:{stat}"), *value);
        }
        nbt.put_compound("Statistics", stats_compound);
    }

    pub fn read_nbt(&mut self, nbt: &NbtCompound) {
        if let Some(stats_compound) = nbt.get_compound("Statistics") {
            for (key, tag) in &stats_compound.child_tags {
                let parts: Vec<&str> = key.split(':').collect();
                if let (NbtTag::Int(value), [cat_str, stat_str]) = (tag, parts.as_slice())
                    && let (Ok(category), Ok(stat)) =
                        (cat_str.parse::<i32>(), stat_str.parse::<i32>())
                {
                    self.stats.insert((category, stat), *value);
                }
            }
        }
    }
}

impl super::Player {
    /// Awards the plugin-approved count to the statistics map and vanilla statistic objectives.
    pub fn award_stat(&self, category: StatisticCategory, stat: i32, amount: i32) {
        let final_amount = if let Some(player_arc) =
            self.world().get_player_by_uuid(self.gameprofile.id)
            && let Some(server) = self.world().server.upgrade()
        {
            let mut event = crate::plugin::api::events::player::player_statistic_increment::PlayerStatisticIncrementEvent {
                player: player_arc,
                statistic_id: format!("{category:?}:{stat}"),
                amount,
                cancelled: false,
            };
            server.plugin_manager.fire_blocking(&server, &mut event);
            if event.cancelled {
                return;
            }
            event.amount
        } else {
            amount
        };
        self.stats
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .increment(category, stat, final_amount);
        // ServerPlayer.awardStat updates statistic-backed objectives by the accepted count.
        if let Some(criterion) = statistic_criterion(category, stat) {
            let world = self.world();
            let mut scoreboard = world
                .scoreboard
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            award_stat_objectives(
                &mut scoreboard,
                &world,
                &self.gameprofile.name,
                &criterion,
                final_amount,
            );
        }
    }
}

fn statistic_criterion(category: StatisticCategory, id: i32) -> Option<String> {
    use pumpkin_data::{Block, data_component_impl::IDSetContent, entity::EntityType, item::Item};
    let name = match category {
        StatisticCategory::Custom => CustomStatistic::from_i32(id)?.registry_key(),
        StatisticCategory::Mined => {
            let block = pumpkin_data::BlockId::new(u16::try_from(id).ok()?)?;
            Block::from_id(block).name
        }
        StatisticCategory::Killed | StatisticCategory::KilledBy => {
            EntityType::from_id(u16::try_from(id).ok()?)?.resource_name
        }
        _ => Item::from_id(u16::try_from(id).ok()?)?.registry_key,
    };
    let name = if name.contains(':') {
        name.to_owned()
    } else {
        format!("minecraft:{name}")
    };
    Some(format!(
        "{}:{}",
        category.registry_key().replace(':', "."),
        name.replace(':', ".")
    ))
}

fn award_stat_objectives(
    scoreboard: &mut crate::world::scoreboard::Scoreboard,
    target: &impl crate::world::scoreboard::ScoreboardTarget,
    player: &str,
    criterion: &str,
    amount: i32,
) {
    let objectives: Vec<_> = scoreboard
        .get_objectives()
        .values()
        .filter(|objective| objective.criterion == criterion)
        .map(|objective| objective.name.clone())
        .collect();
    for objective in objectives {
        scoreboard.add_score(target, player, objective, amount);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::scoreboard::{NoTarget, Scoreboard, ScoreboardObjective};
    use pumpkin_protocol::java::client::play::RenderType;
    use pumpkin_util::text::TextComponent;

    #[test]
    fn accepted_statistics_increment_every_matching_objective() {
        let mut scoreboard = Scoreboard::default();
        for (name, criterion) in [
            (
                "blocks",
                "minecraft.custom:minecraft.damage_blocked_by_shield",
            ),
            (
                "also",
                "minecraft.custom:minecraft.damage_blocked_by_shield",
            ),
            ("other", "dummy"),
        ] {
            scoreboard.add_objective(
                &NoTarget,
                ScoreboardObjective::new(
                    name,
                    TextComponent::text(name),
                    RenderType::Integer,
                    None,
                    criterion,
                ),
            );
            scoreboard.set_score_value(&NoTarget, "Alex", name, 5);
        }
        let criterion = statistic_criterion(
            StatisticCategory::Custom,
            CustomStatistic::DamageBlockedByShield as i32,
        )
        .unwrap();
        award_stat_objectives(&mut scoreboard, &NoTarget, "Alex", &criterion, 3);
        assert_eq!(scoreboard.get_score_value("Alex", "blocks"), Some(8));
        assert_eq!(scoreboard.get_score_value("Alex", "also"), Some(8));
        assert_eq!(scoreboard.get_score_value("Alex", "other"), Some(5));
        assert_eq!(
            statistic_criterion(
                StatisticCategory::Used,
                i32::from(pumpkin_data::item::Item::SHIELD.id)
            )
            .unwrap(),
            "minecraft.used:minecraft.shield"
        );
    }
}
