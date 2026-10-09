use crate::command::CommandSender;
use crate::server::combat_test_support::{server, world};

#[tokio::test]
async fn enabled_weather_cycle_expires_a_timed_rain() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let server = server(directory.path());
    let world = world(&server, directory.path());
    server
        .worlds
        .store(std::sync::Arc::new(vec![world.clone()]));
    let mut player = crate::net::java::combat_test_support::TestPlayer::new(&world);
    world
        .players
        .store(std::sync::Arc::new(vec![player.player.clone()]));
    world
        .weather
        .lock()
        .unwrap()
        .set_weather_parameters(&world, 0, 1, true, false);
    world.weather.lock().unwrap().rain_level = 0.5;
    player.take_packets();
    world.tick_environment();
    let weather = world.weather.lock().unwrap();
    assert_eq!(
        (weather.data().rain_time, weather.data().raining),
        (0, false)
    );
    drop(weather);
    let game_event = pumpkin_data::packet::clientbound::play::GAME_EVENT.0;
    let is_end_raining = |packet: &bytes::Bytes| {
        let mut bytes = packet.as_ref();
        let id = pumpkin_protocol::codec::var_int::VarInt::decode(&mut bytes)
            .unwrap()
            .0;
        id == game_event && bytes.first() == Some(&(super::GameEvent::EndRaining as u8))
    };
    assert!(!player.take_packets().iter().any(is_end_raining));
    for _ in 0..30 {
        world.tick_environment();
    }
    assert!(player.take_packets().iter().any(is_end_raining));
    Ok(())
}

#[tokio::test]
async fn disabled_weather_gamerule_preserves_timers_but_interpolates()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let server = server(directory.path());
    let world = world(&server, directory.path());
    server
        .worlds
        .store(std::sync::Arc::new(vec![world.clone()]));
    let source = CommandSender::Console.into_source(&server);
    let dispatcher = server.command_dispatcher.load();
    dispatcher
        .execute_input("gamerule advance_weather false", &source)
        .map_err(|error| format!("{error:?}"))?;
    world
        .weather
        .lock()
        .unwrap()
        .set_weather_parameters(&world, 0, 1, true, true);
    world.tick_environment();
    let weather = world.weather.lock().unwrap();
    assert_eq!(
        (
            weather.data().rain_time,
            weather.data().raining,
            weather.data().thunder_time,
            weather.data().thundering
        ),
        (1, true, 1, true)
    );
    assert!(!weather.weather_cycle_enabled);
    assert_eq!(weather.rain_level, 0.01);
    assert_eq!(weather.thunder_level, 0.01);
    drop(weather);
    dispatcher
        .execute_input("gamerule advance_weather true", &source)
        .map_err(|error| format!("{error:?}"))?;
    world.tick_environment();
    let weather = world.weather.lock().unwrap();
    assert_eq!(
        (
            weather.data().rain_time,
            weather.data().raining,
            weather.data().thunder_time,
            weather.data().thundering
        ),
        (0, false, 0, false)
    );
    Ok(())
}

#[tokio::test]
async fn weather_command_duration_expires_through_environment_tick()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let server = server(directory.path());
    let world = world(&server, directory.path());
    server
        .worlds
        .store(std::sync::Arc::new(vec![world.clone()]));
    let source = CommandSender::Console.into_source(&server);
    server
        .command_dispatcher
        .load()
        .execute_input("weather rain 2t", &source)
        .map_err(|error| format!("{error:?}"))?;
    world.tick_environment();
    let weather = world.weather.lock().unwrap();
    assert_eq!(
        (weather.data().rain_time, weather.data().raining),
        (1, true)
    );
    drop(weather);
    world.tick_environment();
    let weather = world.weather.lock().unwrap();
    assert_eq!(
        (weather.data().rain_time, weather.data().raining),
        (0, false)
    );
    Ok(())
}

fn game_events(packets: Vec<bytes::Bytes>) -> Vec<(u8, f32)> {
    packets
        .into_iter()
        .filter_map(|packet| {
            let mut data = packet.as_ref();
            let id = pumpkin_protocol::codec::var_int::VarInt::decode(&mut data)
                .unwrap()
                .0;
            (id == pumpkin_data::packet::clientbound::play::GAME_EVENT.0)
                .then(|| (data[0], f32::from_be_bytes(data[1..5].try_into().unwrap())))
        })
        .collect()
}

fn dimension_world(
    server: &std::sync::Arc<crate::server::Server>,
    path: &std::path::Path,
    dimension: pumpkin_data::dimension::Dimension,
) -> std::sync::Arc<super::World> {
    std::sync::Arc::new(super::World::load(
        pumpkin_world::level::Level::from_root_folder(
            &pumpkin_config::world::LevelConfig::default(),
            path.to_path_buf(),
            0,
            dimension.clone(),
        ),
        server.level_info.clone(),
        dimension,
        server.block_registry.clone(),
        std::sync::Arc::downgrade(server),
    ))
}

#[tokio::test]
async fn weather_restart_round_trip_preserves_active_duration_and_prepares_visual_levels()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    {
        let server = server(directory.path());
        let world = world(&server, directory.path());
        server
            .worlds
            .store(std::sync::Arc::new(vec![world.clone()]));
        let source = CommandSender::Console.into_source(&server);
        server
            .command_dispatcher
            .load()
            .execute_input("weather thunder 123t", &source)
            .map_err(|error| format!("{error:?}"))?;
        world.tick_environment();
        server.save_world_info()?;
        // MinecraftServer.stopServer uses the same live-data writer as autosaving.
        world.tick_environment();
        server.shutdown().await;
    };
    let server = server(directory.path());
    let world = world(&server, directory.path());
    let weather = world.weather.lock().unwrap();
    let data = weather.data();
    assert_eq!(
        (
            data.clear_weather_time,
            data.rain_time,
            data.thunder_time,
            data.raining,
            data.thundering
        ),
        (0, 121, 121, true, true)
    );
    assert_eq!((weather.rain_level, weather.thunder_level), (1.0, 1.0));
    drop(weather);
    world.tick_environment();
    assert_eq!(world.weather.lock().unwrap().data().rain_time, 120);
    Ok(())
}

#[tokio::test]
async fn excluded_dimensions_do_not_advance_shared_weather_or_visuals_and_nether_commands_set_overworld()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let server = server(directory.path());
    let overworld = world(&server, directory.path());
    let nether = dimension_world(
        &server,
        directory.path(),
        pumpkin_data::dimension::Dimension::THE_NETHER,
    );
    let end = dimension_world(
        &server,
        directory.path(),
        pumpkin_data::dimension::Dimension::THE_END,
    );
    server.worlds.store(std::sync::Arc::new(vec![
        overworld.clone(),
        nether.clone(),
        end.clone(),
    ]));
    let mut nether_player = crate::net::java::combat_test_support::TestPlayer::new(&nether);
    let mut end_player = crate::net::java::combat_test_support::TestPlayer::new(&end);
    nether_player.take_packets();
    end_player.take_packets();
    let source = CommandSender::Console.into_source(&server);
    server
        .command_dispatcher
        .load()
        .execute_input(
            "execute in minecraft:the_nether run weather rain 100t",
            &source,
        )
        .map_err(|error| format!("{error:?}"))?;
    let before = overworld.weather.lock().unwrap().data();
    assert_eq!((before.rain_time, before.raining), (100, true));
    assert!(game_events(nether_player.take_packets()).is_empty());
    for excluded in [&nether, &end] {
        for _ in 0..3 {
            excluded.tick_environment();
        }
        let weather = excluded.weather.lock().unwrap();
        assert_eq!(weather.data(), before);
        assert_eq!((weather.rain_level, weather.thunder_level), (0.0, 0.0));
    }
    assert!(game_events(nether_player.take_packets()).is_empty());
    assert!(game_events(end_player.take_packets()).is_empty());
    overworld.tick_environment();
    assert_eq!(nether.weather.lock().unwrap().data().rain_time, 99);
    Ok(())
}

#[tokio::test]
async fn weather_commands_send_only_tick_ordered_level_and_transition_packets_with_either_cycle_rule()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let server = server(directory.path());
    let world = world(&server, directory.path());
    server
        .worlds
        .store(std::sync::Arc::new(vec![world.clone()]));
    let mut viewer = crate::net::java::combat_test_support::TestPlayer::new(&world);
    let source = CommandSender::Console.into_source(&server);
    let dispatcher = server.command_dispatcher.load();
    for cycle in [true, false] {
        dispatcher
            .execute_input(&format!("gamerule advance_weather {cycle}"), &source)
            .map_err(|error| format!("{error:?}"))?;
        world.weather.lock().unwrap().rain_level = 0.2;
        viewer.take_packets();
        dispatcher
            .execute_input("weather rain 100t", &source)
            .map_err(|error| format!("{error:?}"))?;
        assert!(
            game_events(viewer.take_packets()).is_empty(),
            "setter must not broadcast"
        );
        world.tick_environment();
        let events = game_events(viewer.take_packets());
        assert_eq!(
            events.iter().map(|e| e.0).collect::<Vec<_>>(),
            vec![7, 1, 7, 8]
        );
        assert!((events[0].1 - 0.21).abs() < 0.00001);
        assert_eq!(events[0].1, events[2].1);
        assert_eq!(events[1].1, 0.0);
        assert_eq!(events[3].1, 0.0);
        assert_eq!(
            world.weather.lock().unwrap().data().rain_time,
            if cycle { 99 } else { 100 }
        );
        // Stop at an exact visual threshold, independently of the boolean setter.
        world.weather.lock().unwrap().rain_level = 0.21;
        dispatcher
            .execute_input("weather clear 50t", &source)
            .map_err(|error| format!("{error:?}"))?;
        assert!(game_events(viewer.take_packets()).is_empty());
        world.tick_environment();
        let events = game_events(viewer.take_packets());
        assert_eq!(
            events.iter().map(|e| e.0).collect::<Vec<_>>(),
            vec![7, 2, 7, 8]
        );
        assert!((events[0].1 - 0.2).abs() < 0.00001);
    }
    Ok(())
}

#[tokio::test]
async fn sleep_resets_only_visual_rain_after_weather_tick_and_before_time_tick()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let server = server(directory.path());
    let world = world(&server, directory.path());
    server
        .worlds
        .store(std::sync::Arc::new(vec![world.clone()]));
    let sleeper = crate::net::java::combat_test_support::TestPlayer::new(&world);
    for (level, cycle, expect_reset) in
        [(0.18, true, false), (0.2, true, true), (0.2, false, false)]
    {
        server.level_info.rcu(|info| {
            let mut info = (**info).clone();
            info.game_rules.advance_weather = cycle;
            info
        });
        world.set_time_of_day(23999);
        sleeper.player.sleeping_since.store(Some(100));
        {
            let mut weather = world.weather.lock().unwrap();
            weather.set_weather_parameters(&world, 0, 100, true, false);
            weather.rain_level = level;
        };
        world.tick_environment();
        let data = world.weather.lock().unwrap().data();
        assert_eq!(data.raining, !expect_reset, "level={level}, cycle={cycle}");
        assert_eq!(
            data.rain_time,
            if expect_reset {
                0
            } else if cycle {
                99
            } else {
                100
            }
        );
        assert_eq!(world.get_time_of_day(), 24001);
        assert_eq!(sleeper.player.sleeping_since.load(), None);
    }
    server.level_info.rcu(|info| {
        let mut info = (**info).clone();
        info.game_rules.advance_weather = true;
        info
    });
    world.set_time_of_day(23999);
    sleeper.player.sleeping_since.store(Some(100));
    // Clear timer must survive a sleep reset while lingering visual rain remains.
    {
        let mut weather = world.weather.lock().unwrap();
        weather.set_weather_parameters(&world, 42, 17, false, true);
        weather.rain_level = 0.5;
    };
    world.tick_environment();
    {
        let weather = world.weather.lock().unwrap();
        let data = weather.data();
        assert_eq!(
            (
                data.clear_weather_time,
                data.rain_time,
                data.thunder_time,
                data.raining,
                data.thundering
            ),
            (41, 0, 0, false, false)
        );
    };
    Ok(())
}
