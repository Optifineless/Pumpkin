use super::{display_bounds::fixture, saved_components::*, *};
use crate::net::bedrock::combat_test_support::TestBedrockPlayer;
use pumpkin_nbt::tag::NbtTag;
use std::sync::atomic::Ordering;

fn repeated_translation(depth: usize, leaf: &str) -> NbtTag {
    let mut text = NbtTag::String(leaf.into());
    for _ in 0..depth {
        let mut component = NbtCompound::new();
        component.put_string("translate", "review.nonexistent".into());
        component.put_string("fallback", "%1$s%1$s".into());
        component.put_list("with", vec![text]);
        text = NbtTag::Compound(component);
    }
    text
}

fn translated_fixture() -> NbtCompound {
    let mut root = fixture(repeated_translation(40, "A"), true);
    let NbtTag::Compound(data) = root.child_tags.get_mut("data").unwrap() else {
        panic!("Missing fixture data")
    };
    let mut display = NbtCompound::new();
    display.put_string("text", String::new());
    display.put_list("extra", vec![repeated_translation(40, "AAAA"); 2]);
    let mut objective = NbtCompound::new();
    objective.put_string("Name", "composite".into());
    objective.put_compound("DisplayName", display);
    let mut objectives = data.get_list("Objectives").unwrap().to_vec();
    objectives.push(NbtTag::Compound(objective));
    data.put_list("Objectives", objectives);
    let NbtTag::Compound(slots) = data.child_tags.get_mut("DisplaySlots").unwrap() else {
        panic!("Missing fixture slots")
    };
    slots.put_string("list", "composite".into());
    root
}

async fn isolated_render_child() -> bool {
    const NAME: &str = "scoreboard_regression_t40_java_join_skips_bedrock_rendering";
    const CHILD: &str = "PUMPKIN_SCOREBOARD_RENDER_CHILD";
    if std::env::var(CHILD).is_ok_and(|value| value == NAME) {
        return true;
    }
    let directory = tempfile::tempdir().unwrap();
    let result = tokio::time::timeout(
        Duration::from_secs(30),
        tokio::process::Command::new(std::env::current_exe().unwrap())
            .args([NAME, "--nocapture", "--test-threads=1"])
            .env(CHILD, NAME)
            .current_dir(directory.path())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .expect("T40 startup and rendering exceeded 30 seconds")
    .unwrap();
    assert!(
        result.status.success(),
        "{}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    false
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoreboard_regression_t40_java_join_skips_bedrock_rendering() {
    if !isolated_render_child().await {
        return;
    }
    let root = translated_fixture();
    let directory = tempfile::tempdir().unwrap();
    write_fixture(directory.path(), root.clone());
    let (server, _) = startup(directory.path()).await;
    let renders = &crate::world::scoreboard::BEDROCK_TITLES_RENDERED;
    renders.store(0, Ordering::Relaxed);
    let packets = joining_packets(&server).await;
    assert_eq!(objective_components(&packets).len(), 2);
    assert!(packets.iter().all(|packet| packet.len() < 128 * 1024));
    assert_eq!(
        renders.load(Ordering::Relaxed),
        0,
        "Java login rendered a Bedrock title"
    );

    let world = server.worlds.load()[0].clone();
    let mut bedrock = TestBedrockPlayer::new(&world).await;
    bedrock.take_packets();
    bedrock.player.send_scoreboard();
    let packets = bedrock.take_packets();
    assert!(!packets.is_empty());
    assert!(
        renders.load(Ordering::Relaxed) > 0,
        "Bedrock sync skipped title rendering"
    );
    assert!(packets.iter().all(|packet| packet.len() <= 32 * 1024 + 128));
    let title = server
        .scoreboard
        .lock()
        .unwrap()
        .get_objective("imported")
        .unwrap()
        .display_name
        .clone()
        .get_text();
    assert!(!title.is_empty());
    assert!(title.len() <= 32 * 1024);
    assert!(title.bytes().all(|byte| byte == b'A'));
    let composite = server
        .scoreboard
        .lock()
        .unwrap()
        .get_objective("composite")
        .unwrap()
        .display_name
        .clone()
        .get_text();
    assert!(!composite.is_empty());
    assert!(composite.len() <= 32 * 1024);
    command(&server, &world, "scoreboard players set Holder imported 43");
    save(&server).await;
    let saved_root = read_scoreboard(directory.path());
    let saved = payload(&saved_root).get_list("Objectives").unwrap();
    for objective in payload(&root).get_list("Objectives").unwrap() {
        let name = objective.extract_compound().unwrap().get_string("Name");
        assert_eq!(
            saved
                .iter()
                .find(|record| record.extract_compound().unwrap().get_string("Name") == name),
            Some(objective)
        );
    }
    bedrock.close().await;
    world
        .entity_tracker
        .remove_entity(bedrock.player.as_ref(), &world);
    world.players.store(Arc::new(Vec::new()));
    drop(bedrock);
    server.shutdown().await;
    world.level.world_portal.store(Arc::new(None));
}
