use super::World;
use pumpkin_protocol::java::client::play::{CGameEvent, GameEvent};
use pumpkin_world::world_info::data_files::WeatherData;
use rand::RngExt;

// Weather timing constants
const RAIN_DELAY_MIN: i32 = 12_000;
const RAIN_DELAY_MAX: i32 = 180_000;
const RAIN_DURATION_MIN: i32 = 12_000;
const RAIN_DURATION_MAX: i32 = 24_000;
const THUNDER_DELAY_MIN: i32 = 12_000;
const THUNDER_DELAY_MAX: i32 = 180_000;
const THUNDER_DURATION_MIN: i32 = 3_600;
const THUNDER_DURATION_MAX: i32 = 15_600;

const WEATHER_TRANSITION_SPEED: f32 = 0.01;

#[derive(Clone)]
pub struct Weather {
    data: std::sync::Arc<std::sync::Mutex<WeatherData>>,
    pub rain_level: f32,
    pub old_rain_level: f32,
    pub thunder_level: f32,
    pub old_thunder_level: f32,
    pub weather_cycle_enabled: bool,
}

impl Default for Weather {
    fn default() -> Self {
        Self::new()
    }
}

impl Weather {
    #[must_use]
    pub fn new() -> Self {
        Self::from_shared_data(std::sync::Arc::new(std::sync::Mutex::new(
            WeatherData::default(),
        )))
    }

    pub(super) fn from_world_data(
        info: &pumpkin_world::world_info::LevelData,
        dimension: &pumpkin_data::dimension::Dimension,
        server: &std::sync::Weak<crate::server::Server>,
    ) -> Self {
        let data = server.upgrade().map_or_else(
            || std::sync::Arc::new(std::sync::Mutex::new(WeatherData::from_level_data(info))),
            |server| server.weather_data.clone(),
        );
        let mut weather = Self::from_shared_data(data);
        // ServerLevel's constructor only calls prepareWeather in weather-bearing dimensions.
        if !Self::can_have_weather(dimension) {
            weather.rain_level = 0.0;
            weather.thunder_level = 0.0;
        }
        weather
    }

    fn from_shared_data(data: std::sync::Arc<std::sync::Mutex<WeatherData>>) -> Self {
        // ServerLevel.prepareWeather initializes visual levels from the saved flags.
        let saved = data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        Self {
            data,
            rain_level: if saved.raining { 1.0 } else { 0.0 },
            old_rain_level: 0.0,
            thunder_level: if saved.raining && saved.thundering {
                1.0
            } else {
                0.0
            },
            old_thunder_level: 0.0,
            weather_cycle_enabled: true,
        }
    }

    /// Returns a snapshot of the server's shared weather flags and timers.
    #[must_use]
    pub fn data(&self) -> WeatherData {
        self.data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub fn set_weather_parameters(
        &mut self,
        _world: &World,
        clear_time: i32,
        rain_time: i32,
        raining: bool,
        thundering: bool,
    ) {
        // MinecraftServer.setWeatherParameters only changes data; tick sends transitions.
        let mut data = self
            .data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        data.clear_weather_time = clear_time;
        data.rain_time = rain_time;
        data.thunder_time = rain_time;
        data.raining = raining;
        data.thundering = thundering;
    }

    pub fn tick_weather(&mut self, world: &World) {
        let was_raining = self.is_raining(world);
        // ServerLevel.advanceWeatherCycle gates timers AND interpolation on canHaveWeather.
        if !Self::can_have_weather(&world.dimension) {
            return;
        }
        self.weather_cycle_enabled = world.level_info.load().game_rules.advance_weather;
        let data = {
            let mut data = self
                .data
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if self.weather_cycle_enabled {
                Self::advance_weather_cycle(&mut data);
            }
            data.clone()
        };

        // Update visual transitions
        self.old_rain_level = self.rain_level;
        self.old_thunder_level = self.thunder_level;

        if data.raining {
            self.rain_level = (self.rain_level + WEATHER_TRANSITION_SPEED).min(1.0);
        } else {
            self.rain_level = (self.rain_level - WEATHER_TRANSITION_SPEED).max(0.0);
        }

        if data.thundering {
            self.thunder_level = (self.thunder_level + WEATHER_TRANSITION_SPEED).min(1.0);
        } else {
            self.thunder_level = (self.thunder_level - WEATHER_TRANSITION_SPEED).max(0.0);
        }

        // Broadcast level changes if needed
        if self.old_rain_level != self.rain_level {
            world.broadcast_packet_all(&CGameEvent::new(
                GameEvent::RainLevelChange,
                self.rain_level,
            ));
        }

        if self.old_thunder_level != self.thunder_level {
            world.broadcast_packet_all(&CGameEvent::new(
                GameEvent::ThunderLevelChange,
                self.thunder_level,
            ));
        }

        // ServerLevel.advanceWeatherCycle also announces natural rain start/stop transitions.
        if was_raining != self.is_raining(world) {
            let event = if was_raining {
                GameEvent::EndRaining
            } else {
                GameEvent::BeginRaining
            };
            world.broadcast_weather_transition(&CGameEvent::new(event, 0.0));
            world.broadcast_weather_transition(&CGameEvent::new(
                GameEvent::RainLevelChange,
                self.rain_level,
            ));
            world.broadcast_weather_transition(&CGameEvent::new(
                GameEvent::ThunderLevelChange,
                self.thunder_level,
            ));
        }
    }

    fn can_have_weather(dimension: &pumpkin_data::dimension::Dimension) -> bool {
        // Level.canHaveWeather (vanilla 26.3, line 935).
        dimension.has_skylight
            && !dimension.has_ceiling
            && dimension.minecraft_name
                != pumpkin_data::dimension::Dimension::THE_END.minecraft_name
    }

    pub fn is_raining(&self, world: &World) -> bool {
        Self::can_have_weather(&world.dimension) && self.rain_level > 0.2
    }

    pub fn is_thundering(&self, world: &World) -> bool {
        Self::can_have_weather(&world.dimension) && self.thunder_level * self.rain_level > 0.9
    }

    fn advance_weather_cycle(data: &mut WeatherData) {
        // ServerLevel.advanceWeatherCycle updates the server-wide WeatherData timers.
        if data.clear_weather_time > 0 {
            data.clear_weather_time -= 1;
            data.thunder_time = i32::from(!data.thundering);
            data.rain_time = i32::from(!data.raining);
            data.thundering = false;
            data.raining = false;
        } else {
            // Handle thunder timing
            if data.thunder_time > 0 {
                data.thunder_time -= 1;
                if data.thunder_time == 0 {
                    data.thundering = !data.thundering;
                }
            } else if data.thundering {
                data.thunder_time =
                    rand::rng().random_range(THUNDER_DURATION_MIN..=THUNDER_DURATION_MAX);
            } else {
                data.thunder_time = rand::rng().random_range(THUNDER_DELAY_MIN..=THUNDER_DELAY_MAX);
            }

            // Handle rain timing
            if data.rain_time > 0 {
                data.rain_time -= 1;
                if data.rain_time == 0 {
                    data.raining = !data.raining;
                }
            } else if data.raining {
                data.rain_time = rand::rng().random_range(RAIN_DURATION_MIN..=RAIN_DURATION_MAX);
            } else {
                data.rain_time = rand::rng().random_range(RAIN_DELAY_MIN..=RAIN_DELAY_MAX);
            }
        }
    }

    pub fn reset_weather_cycle(&mut self, _world: &World) {
        // ServerLevel.resetWeatherCycle leaves the forced-clear timer alone.
        let mut data = self
            .data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        data.rain_time = 0;
        data.raining = false;
        data.thunder_time = 0;
        data.thundering = false;
    }
}

impl World {
    fn broadcast_weather_transition(&self, packet: &CGameEvent) {
        if let Some(server) = self.server.upgrade() {
            for world in server.worlds.load().iter() {
                world.broadcast_packet_all(packet);
            }
        } else {
            self.broadcast_packet_all(packet);
        }
    }
}

#[cfg(test)]
#[path = "weather_tests.rs"]
mod tests;
