use super::*;

fn run(
    server: &Arc<Server>,
    world: &Arc<World>,
    player: &mut TestBedrockPlayer,
    input: &str,
) -> Vec<bytes::Bytes> {
    command(server, world, input);
    player.take_packets()
}

fn set(
    server: &Arc<Server>,
    world: &Arc<World>,
    player: &mut TestBedrockPlayer,
    holder: &str,
    objective: &str,
) -> Entry {
    command(
        server,
        world,
        &format!("scoreboard players set {holder} {objective} 5"),
    );
    let entry = one(player);
    changed(&entry, holder, objective, 5);
    entry
}

fn recreate(
    server: &Arc<Server>,
    world: &Arc<World>,
    player: &mut TestBedrockPlayer,
    prior: &Entry,
    holder: &str,
) -> Entry {
    let recreated = set(server, world, player, holder, &prior.objective);
    assert_ne!(
        recreated.id, prior.id,
        "Retired score identity was reused for a recreation"
    );
    recreated
}

fn reconstruct(
    server: &Arc<Server>,
    world: &Arc<World>,
    player: &mut TestBedrockPlayer,
    a3: &Entry,
    b2: &Entry,
    other: &Entry,
) {
    let mut lines = BTreeMap::from([
        (a3.id, ("a".into(), "Holder".into(), 5)),
        (b2.id, ("b".into(), "Holder".into(), 5)),
        (other.id, ("a".into(), "Other".into(), 5)),
    ]);
    apply(
        &mut lines,
        &run(
            server,
            world,
            player,
            "scoreboard objectives setdisplay sidebar",
        ),
    );
    let shown = run(
        server,
        world,
        player,
        "scoreboard objectives setdisplay sidebar a",
    );
    let shown_entries = entries(&shown);
    assert_eq!(shown_entries.len(), 2);
    assert_eq!(
        shown_entries
            .iter()
            .find(|e| e.change.as_ref().unwrap().0 == "Holder")
            .unwrap()
            .id,
        a3.id
    );
    assert_eq!(
        shown_entries
            .iter()
            .find(|e| e.change.as_ref().unwrap().0 == "Other")
            .unwrap()
            .id,
        other.id
    );
    apply(&mut lines, &shown);
    apply(
        &mut lines,
        &run(server, world, player, "scoreboard objectives remove a"),
    );
    command(server, world, "scoreboard objectives add a dummy");
    command(server, world, "scoreboard objectives setdisplay sidebar a");
    player.take_packets();
    command(server, world, "scoreboard players set Holder a 5");
    let packets = player.take_packets();
    let a4 = entries(&packets).pop().unwrap();
    assert_ne!(
        a4.id, a3.id,
        "Objective recreation retained an old score identity"
    );
    apply(&mut lines, &packets);
    assert_eq!(lines.len(), 2, "Reconstruction left a stale line");
    assert_eq!(lines[&b2.id], ("b".into(), "Holder".into(), 5));
    let board = server.scoreboard.lock().unwrap();
    assert!(board.get_score("Holder", "hidden").is_none());
    assert!(board.get_score("Other", "a").is_none());
    drop(board);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g05_score_ids_do_not_alias_objectives_or_retired_entries() {
    let directory = tempfile::tempdir().unwrap();
    scenario(directory.path(), |server, worlds| async move {
        let world = &worlds[0];
        let mut player = TestBedrockPlayer::new(world).await;
        player.player.send_scoreboard();
        for input in [
            "scoreboard objectives add a dummy",
            "scoreboard objectives add b dummy",
            "scoreboard objectives add hidden dummy",
            "scoreboard objectives setdisplay sidebar a",
            "scoreboard objectives setdisplay below_name b",
            "scoreboard players set Holder hidden 41",
        ] {
            command(&server, world, input);
        }
        assert!(
            entries(&player.take_packets()).is_empty(),
            "Untracked objective leaked a score"
        );
        let a = set(&server, world, &mut player, "Holder", "a");
        let b = set(&server, world, &mut player, "Holder", "b");
        let other = set(&server, world, &mut player, "Other", "a");
        assert_ne!(a.id, b.id, "Independent objectives shared a Bedrock entry");
        assert_ne!(a.id, other.id);
        assert_ne!(b.id, other.id);
        let absent = run(
            &server,
            world,
            &mut player,
            "scoreboard players reset Missing a",
        );
        assert!(
            entries(&absent).is_empty(),
            "Nonexistent score produced a removal identity"
        );
        assert_eq!(
            command(&server, world, "scoreboard players get Holder hidden"),
            41
        );
        let removal = run(
            &server,
            world,
            &mut player,
            "scoreboard players reset Holder a",
        );
        assert_eq!(entries(&removal).len(), 1);
        removed(&entries(&removal)[0], &a);
        let a2 = recreate(&server, world, &mut player, &a, "Holder");
        let reset = run(
            &server,
            world,
            &mut player,
            "scoreboard players reset Holder",
        );
        let reset = entries(&reset);
        assert_eq!(
            reset.len(),
            2,
            "Reset-all removed an unrelated/untracked line"
        );
        removed(
            reset.iter().find(|entry| entry.objective == "a").unwrap(),
            &a2,
        );
        removed(
            reset.iter().find(|entry| entry.objective == "b").unwrap(),
            &b,
        );
        let a3 = recreate(&server, world, &mut player, &a2, "Holder");
        let b2 = recreate(&server, world, &mut player, &b, "Holder");
        reconstruct(&server, world, &mut player, &a3, &b2, &other);
        player.close().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g05_java_hidden_and_nonexistent_controls() {
    let directory = tempfile::tempdir().unwrap();
    scenario(directory.path(), |server, worlds| async move {
        let mut bedrock = TestBedrockPlayer::new(&worlds[0]).await;
        let mut java = TestPlayer::new(&worlds[1]);
        bedrock.player.send_scoreboard();
        java.player.send_scoreboard();
        bedrock.take_packets();
        java.take_packets();
        for input in [
            "scoreboard objectives add a dummy",
            "scoreboard objectives add hidden dummy",
            "scoreboard objectives setdisplay sidebar a",
            "scoreboard players set Holder hidden 41",
            "scoreboard players set Other a 9",
            "scoreboard players reset Missing a",
        ] {
            command(&server, &worlds[2], input);
        }
        assert_eq!(
            java_scores(&java.take_packets()),
            [("Other".into(), "a".into(), 9)]
        );
        assert_eq!(
            command(&server, &worlds[1], "scoreboard players get Other a"),
            9
        );
        assert_eq!(
            command(&server, &worlds[1], "scoreboard players get Holder hidden"),
            41
        );
        command(
            &server,
            &worlds[2],
            "scoreboard players reset Holder hidden",
        );
        assert!(java_resets(&java.take_packets()).is_empty());
        command(&server, &worlds[2], "scoreboard players reset Other");
        assert_eq!(java_resets(&java.take_packets()), [("Other".into(), None)]);
        bedrock.close().await;
    })
    .await;
}
