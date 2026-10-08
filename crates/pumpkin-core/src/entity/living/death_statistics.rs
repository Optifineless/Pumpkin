use super::LivingEntity;
use crate::entity::{
    EntityBase,
    player::{
        advancement::trigger::AdvancementTrigger,
        statistics::{CustomStatistic, StatisticCategory},
    },
};

impl LivingEntity {
    // ServerPlayer.die / LivingEntity.die: stop the old death after a plugin resets its life.
    pub(in crate::entity) fn update_death_stats(
        &self,
        dyn_self: &dyn EntityBase,
        cause: Option<&dyn EntityBase>,
        lifecycle: u64,
    ) -> bool {
        if let Some(victim_player) = dyn_self.get_player() {
            victim_player.increment_custom_stat(CustomStatistic::Deaths, 1);
            if !self.death_lifecycle_current(lifecycle) {
                return false;
            }
            victim_player.set_stat(
                StatisticCategory::Custom,
                CustomStatistic::TimeSinceDeath as i32,
                0,
            );
            victim_player.set_stat(
                StatisticCategory::Custom,
                CustomStatistic::TimeSinceRest as i32,
                0,
            );
            if let Some(killer_entity) = cause.map(EntityBase::get_entity) {
                victim_player.increment_stat(
                    StatisticCategory::KilledBy,
                    killer_entity.entity_type.id as i32,
                    1,
                );
                if !self.death_lifecycle_current(lifecycle) {
                    return false;
                }
            }
        }

        self.award_kill_score(dyn_self, cause, lifecycle)
    }

    // LivingEntity.die / ServerPlayer.awardKillScore; each statistic can dispatch a plugin.
    fn award_kill_score(
        &self,
        dyn_self: &dyn EntityBase,
        cause: Option<&dyn EntityBase>,
        lifecycle: u64,
    ) -> bool {
        if let Some(killer_player) = cause.and_then(|c| c.get_player()) {
            killer_player.increment_stat(
                StatisticCategory::Killed,
                self.entity.entity_type.id as i32,
                1,
            );
            if !self.death_lifecycle_current(lifecycle) {
                return false;
            }
            if dyn_self.get_player().is_some() {
                killer_player.increment_stat(
                    StatisticCategory::Custom,
                    CustomStatistic::PlayerKills as i32,
                    1,
                );
                if !self.death_lifecycle_current(lifecycle) {
                    return false;
                }
            } else {
                killer_player.increment_stat(
                    StatisticCategory::Custom,
                    CustomStatistic::MobKills as i32,
                    1,
                );
                if !self.death_lifecycle_current(lifecycle) {
                    return false;
                }

                let resource_name = self.entity.entity_type.resource_name;
                let criterion_key = format!("minecraft:{resource_name}");
                killer_player.trigger_advancement(AdvancementTrigger::PlayerKilledEntity {
                    entity_type_resource: criterion_key,
                });
                if !self.death_lifecycle_current(lifecycle) {
                    return false;
                }

                if resource_name == "skeleton" {
                    let distance_sq = killer_player
                        .position()
                        .squared_distance_to_vec(&self.entity.pos.load());
                    if distance_sq >= 2500.0 {
                        killer_player.trigger_advancement(AdvancementTrigger::SniperDuel);
                        if !self.death_lifecycle_current(lifecycle) {
                            return false;
                        }
                    }
                }

                if resource_name == "phantom" {
                    killer_player.trigger_advancement(AdvancementTrigger::TwoBirdsOneArrow);
                    if !self.death_lifecycle_current(lifecycle) {
                        return false;
                    }
                }

                let held_item = killer_player.inventory().held_item();
                let is_crossbow = held_item.item.registry_key == "crossbow";
                if is_crossbow {
                    killer_player.trigger_advancement(AdvancementTrigger::Arbalistic);
                    if !self.death_lifecycle_current(lifecycle) {
                        return false;
                    }
                }
            }
        }
        self.death_lifecycle_current(lifecycle)
    }
}
