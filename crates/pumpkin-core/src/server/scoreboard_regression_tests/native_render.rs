use super::{display_bounds::fixture, saved_components::*, *};
use pumpkin_nbt::tag::NbtTag;
use pumpkin_util::text::TextContent;

const NAME: &str = "scoreboard_regression_native_s20_console_query_is_bounded";
const CHILD: &str = "PUMPKIN_SCOREBOARD_NATIVE_RENDER_CHILD";

fn assert_native_entity_saves_are_lossless(world: &Arc<World>) {
    use crate::entity::{Entity, EntityBase, decoration::display::TextDisplayEntity};
    use pumpkin_util::math::vector3::Vector3;
    let text = "A".repeat(40_000);
    let mut input = NbtCompound::new();
    input.put_string("CustomName", text.clone());
    input.put_string(
        "text",
        serde_json::json!({"text": text.as_str()}).to_string(),
    );
    let display = TextDisplayEntity::new(Entity::new(
        world.clone(),
        Vector3::default(),
        &pumpkin_data::entity::EntityType::TEXT_DISPLAY,
    ));
    EntityBase::read_nbt_non_mut(display.as_ref(), &input);
    let mut output = NbtCompound::new();
    EntityBase::write_nbt(display.as_ref(), &mut output);
    assert_eq!(
        output.get_string("CustomName").map(str::len),
        Some(text.len())
    );
    let saved: serde_json::Value =
        serde_json::from_str(output.get_string("text").unwrap()).unwrap();
    assert_eq!(saved["text"].as_str(), Some(text.as_str()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoreboard_regression_native_s20_console_query_is_bounded() {
    if !std::env::var(CHILD).is_ok_and(|value| value == NAME) {
        let directory = tempfile::tempdir().unwrap();
        let output = tokio::time::timeout(
            Duration::from_secs(30),
            tokio::process::Command::new(std::env::current_exe().unwrap())
                .args([NAME, "--nocapture", "--test-threads=1"])
                .env(CHILD, NAME)
                .current_dir(directory.path())
                .kill_on_drop(true)
                .output(),
        )
        .await
        .expect("Native translation startup/Console query exceeded 30 seconds")
        .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.stdout.len() <= 2 * 32 * 1024 + 2048,
            "Console rendered {} bytes",
            output.stdout.len()
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("Holder has 42 "));
        return;
    }

    // Only S5 is used for the pre-fix negative control; never run unbounded S20.
    let depth = std::env::var("PUMPKIN_SCOREBOARD_NATIVE_RENDER_DEPTH")
        .map_or(20, |value| value.parse::<usize>().unwrap());
    let mut display = NbtTag::String("A".into());
    for _ in 0..depth {
        let mut component = NbtCompound::new();
        component.put_string("translate", "translation.test.complex".into());
        component.put_list("with", vec![display]);
        display = NbtTag::Compound(component);
    }
    let root = fixture(display, true);
    let directory = tempfile::tempdir().unwrap();
    write_fixture(directory.path(), root.clone());
    let (server, _) = startup(directory.path()).await;
    let world = server.worlds.load()[0].clone();
    assert_native_entity_saves_are_lossless(&world);
    assert!(
        matches!(
            server
                .scoreboard
                .lock()
                .unwrap()
                .get_objective("imported")
                .unwrap()
                .display_name
                .0
                .content
                .as_ref(),
            TextContent::Translate { .. }
        ),
        "Fixture bypassed the native translation renderer"
    );
    assert_eq!(
        objective_components(&joining_packets(&server).await).len(),
        1
    );
    assert_eq!(
        command(&server, &world, "scoreboard players get Holder imported"),
        42
    );
    command(&server, &world, "scoreboard players set Holder imported 43");
    save(&server).await;
    assert_eq!(
        payload(&read_scoreboard(directory.path())).get_list("Objectives"),
        payload(&root).get_list("Objectives")
    );
    server.shutdown().await;
    world.level.world_portal.store(Arc::new(None));
}
