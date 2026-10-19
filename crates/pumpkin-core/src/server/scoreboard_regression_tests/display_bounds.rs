use super::{saved_components::*, *};
use pumpkin_nbt::tag::NbtTag;
use pumpkin_protocol::ser::NetworkReadExt;

fn fixture(display: NbtTag, displayed: bool) -> NbtCompound {
    let mut objective = NbtCompound::new();
    objective.put_string("Name", "imported".into());
    objective.put("DisplayName", display);
    let mut score = NbtCompound::new();
    score.put_string("Name", "Holder".into());
    score.put_string("Objective", "imported".into());
    score.put_int("Score", 42);
    let mut slots = NbtCompound::new();
    if displayed {
        slots.put_string("sidebar", "imported".into());
        slots.put_string("list", "imported".into());
    }
    let mut data = NbtCompound::new();
    data.put_list("Objectives", vec![NbtTag::Compound(objective)]);
    data.put_list("PlayerScores", vec![NbtTag::Compound(score)]);
    data.put_compound("DisplaySlots", slots);
    let mut root = NbtCompound::new();
    root.put_int("DataVersion", 5023);
    root.put_compound("data", data);
    root
}

fn container_depth(tag: &NbtTag) -> usize {
    match tag {
        NbtTag::Compound(data) => {
            1 + data
                .child_tags
                .values()
                .map(container_depth)
                .max()
                .unwrap_or(0)
        }
        NbtTag::List(children) => 1 + children.iter().map(container_depth).max().unwrap_or(0),
        _ => 0,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoreboard_regression_nested_lists_join_with_bounded_emitted_depth() {
    if !isolated_child("scoreboard_regression_nested_lists_join_with_bounded_emitted_depth") {
        return;
    }
    let mut display = NbtTag::List(vec![NbtTag::String("y".into())]);
    for _ in 0..300 {
        display = NbtTag::List(vec![
            NbtTag::List(vec![NbtTag::String("x".into())]),
            display,
        ]);
    }
    assert_eq!(container_depth(&display), 301);
    let root = fixture(display, true);
    let directory = tempfile::tempdir().unwrap();
    write_fixture(directory.path(), root.clone());
    let (server, _) = startup(directory.path()).await;
    let packets = joining_packets(&server).await;
    let components = objective_components(&packets);
    assert_eq!(
        components.len(),
        1,
        "Displayed objective must be deduplicated"
    );
    assert!(container_depth(&components[0].1) <= 128);
    let world = server.worlds.load()[0].clone();
    command(&server, &world, "scoreboard players set Holder imported 43");
    save(&server).await;
    assert_eq!(
        payload(&read_scoreboard(directory.path())).get_list("Objectives"),
        payload(&root).get_list("Objectives")
    );
    server.shutdown().await;
    world.level.world_portal.store(Arc::new(None));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoreboard_regression_display_slots_start_deduplicate_and_stop_tracking() {
    let directory = tempfile::tempdir().unwrap();
    with_worlds(directory.path(), None, |server, worlds| async move {
        let world = &worlds[0];
        let mut observer = TestPlayer::new(world);
        observer.player.send_scoreboard();
        observer.take_packets();
        command(&server, world, "scoreboard objectives add tracked dummy");
        command(&server, world, "scoreboard players set Holder tracked 1");
        assert!(observer.take_packets().is_empty());
        command(
            &server,
            world,
            "scoreboard objectives setdisplay sidebar tracked",
        );
        let packets = observer.take_packets();
        assert_eq!(objective_components(&packets).len(), 1);
        assert_eq!(
            packet_count(
                &packets,
                pumpkin_data::packet::clientbound::play::UPDATE_SCORE.0
            ),
            1
        );
        command(
            &server,
            world,
            "scoreboard objectives setdisplay list tracked",
        );
        let packets = observer.take_packets();
        assert_eq!(
            packet_count(
                &packets,
                pumpkin_data::packet::clientbound::play::SCOREBOARD_OBJECTIVE.0
            ),
            0
        );
        assert_eq!(
            packet_count(
                &packets,
                pumpkin_data::packet::clientbound::play::UPDATE_SCORE.0
            ),
            0
        );
        command(&server, world, "scoreboard objectives setdisplay sidebar");
        let packets = observer.take_packets();
        assert_eq!(
            packet_count(
                &packets,
                pumpkin_data::packet::clientbound::play::SCOREBOARD_OBJECTIVE.0
            ),
            0
        );
        command(&server, world, "scoreboard objectives setdisplay list");
        let packets = observer.take_packets();
        let mut packet = std::io::Cursor::new(packets[0].as_ref());
        assert_eq!(
            packet.get_var_int().unwrap().0,
            pumpkin_data::packet::clientbound::play::SCOREBOARD_OBJECTIVE.0
        );
        assert_eq!(&*packet.get_str().unwrap(), "tracked");
        assert_eq!(packet.get_u8().unwrap(), 1, "Last slot must stop tracking");
        command(&server, world, "scoreboard players set Holder tracked 2");
        assert!(observer.take_packets().is_empty());
        command(
            &server,
            world,
            "scoreboard objectives setdisplay sidebar tracked",
        );
        let packets = observer.take_packets();
        assert_eq!(objective_components(&packets).len(), 1);
        assert_eq!(
            packet_count(
                &packets,
                pumpkin_data::packet::clientbound::play::UPDATE_SCORE.0
            ),
            1
        );
    })
    .await;
}

fn oversized_display() -> NbtTag {
    let mut display = NbtCompound::new();
    display.put_string("text", String::new());
    display.put_list(
        "extra",
        vec![NbtTag::String("A".repeat(60_000).into()); 140],
    );
    NbtTag::Compound(display)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoreboard_regression_hidden_oversized_objective_joins_without_replay() {
    if !isolated_child("scoreboard_regression_hidden_oversized_objective_joins_without_replay") {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let root = fixture(oversized_display(), false);
    write_fixture(directory.path(), root.clone());
    let (server, _) = startup(directory.path()).await;
    let packets = joining_packets(&server).await;
    for id in [
        pumpkin_data::packet::clientbound::play::SCOREBOARD_OBJECTIVE.0,
        pumpkin_data::packet::clientbound::play::UPDATE_SCORE.0,
    ] {
        assert_eq!(packet_count(&packets, id), 0, "Hidden data reached login");
    }
    let world = server.worlds.load()[0].clone();
    let mut observer = TestPlayer::new(&world);
    observer.player.send_scoreboard();
    observer.take_packets();
    command(&server, &world, "scoreboard players set Holder imported 43");
    assert!(observer.take_packets().is_empty(), "Hidden score broadcast");
    command(
        &server,
        &world,
        "scoreboard objectives setdisplay sidebar imported",
    );
    let packets = observer.take_packets();
    assert_eq!(objective_components(&packets).len(), 1);
    assert_eq!(
        packet_count(
            &packets,
            pumpkin_data::packet::clientbound::play::UPDATE_SCORE.0
        ),
        1
    );
    assert!(packets.iter().all(|packet| packet.len() < 128 * 1024));
    assert!(
        packets
            .iter()
            .all(|packet| packet.len() < pumpkin_protocol::MAX_PACKET_DATA_SIZE)
    );
    save(&server).await;
    assert_eq!(
        payload(&read_scoreboard(directory.path())).get_list("Objectives"),
        payload(&root).get_list("Objectives")
    );
    observer.client().close();
    world
        .entity_tracker
        .remove_entity(observer.player.as_ref(), &world);
    observer.player.storage_session.lock().unwrap().take();
    world.players.store(Arc::new(Vec::new()));
    drop(observer);
    server.shutdown().await;
    world.level.world_portal.store(Arc::new(None));
}
