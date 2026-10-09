use super::World;
use std::sync::Arc;

impl World {
    pub(super) fn tick_weather_and_sleep(self: &Arc<Self>) {
        // ServerLevel.tick: advance weather, handle sleep/reset, then tickTime.
        let (should_reset, cycle_enabled) = {
            let mut weather = self
                .weather
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            weather.tick_weather(self);
            (
                weather.is_raining(self),
                self.level_info.load().game_rules.advance_weather,
            )
        };
        if !self.should_skip_night() {
            return;
        }
        if self.level_info.load().game_rules.advance_time && !self.dimension.has_fixed_time {
            let time = {
                let mut time = self
                    .level_time
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let next_day = time.time_of_day + 24000;
                time.set_time(next_day - next_day % 24000);
                time.clone()
            };
            time.send_time(self);
        }
        for player in self.players.load().iter() {
            player.wake_up();
        }
        if cycle_enabled && should_reset {
            self.weather
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .reset_weather_cycle(self);
        }
    }
}
