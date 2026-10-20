use super::*;
use crate::plugin::{
    BoxFuture, EventHandler, EventPriority, api::events::server::packet::PacketSentEvent,
};
use pumpkin_nbt::tag::NbtTag;
use pumpkin_world::world_info::data_files::write_saved_data;
use std::sync::atomic::{AtomicBool, Ordering};

fn old_save(path: &Path) -> NbtCompound {
    let mut root = vanilla::fixture();
    root.put_int("DataVersion", 4903);
    let NbtTag::Compound(data) = root.child_tags.get_mut("data").unwrap() else {
        panic!("Expected a vanilla scoreboard compound/list");
    };
    data.put_string("UnknownSavedField", "keep this opaque data".into());
    let NbtTag::List(scores) = data.child_tags.get_mut("PlayerScores").unwrap() else {
        panic!("Expected a vanilla scoreboard compound/list")
    };
    let NbtTag::Compound(score) = &mut scores[0] else {
        panic!("Expected a vanilla scoreboard compound/list")
    };
    score.put_long("UnknownScoreField", 718);
    let destination = path.join("data/scoreboard.dat");
    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    write_saved_data(&destination, root.clone()).unwrap();
    root
}

fn expected_value(mut data: NbtCompound, value: i32) -> NbtCompound {
    let NbtTag::List(scores) = data.child_tags.get_mut("PlayerScores").unwrap() else {
        panic!("Expected a vanilla scoreboard compound/list")
    };
    let NbtTag::Compound(score) = &mut scores[0] else {
        panic!("Expected a vanilla scoreboard compound/list")
    };
    score.put_int("Score", value);
    data
}

struct RecreateDuringRemoval(AtomicBool);

impl EventHandler<PacketSentEvent> for RecreateDuringRemoval {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        event: &'a mut PacketSentEvent,
    ) -> BoxFuture<'a, ()> {
        if event.packet_id == pumpkin_data::packet::clientbound::play::RESET_SCORE.0
            && !self.0.swap(true, Ordering::Relaxed)
        {
            assert!(
                server.scoreboard.try_lock().is_ok(),
                "Packet callback retained the scoreboard guard"
            );
            let world = event.player.world();
            command(
                server,
                &world,
                "scoreboard players set ImportedHolder imported 77",
            );
            command(
                server,
                &world,
                "scoreboard players set ImportedHolder imported 84",
            );
        }
        Box::pin(async {})
    }
}

async fn first_session(path: &Path, original: NbtCompound) {
    scenario(path, |server, worlds| async move {
        let mut bedrock = TestBedrockPlayer::new(&worlds[0]).await;
        bedrock.player.send_scoreboard();
        let snapshot = one(&mut bedrock);
        changed(&snapshot, "ImportedHolder", "imported", 42);
        command(
            &server,
            &worlds[1],
            "scoreboard players set ImportedHolder imported 43",
        );
        let update = one(&mut bedrock);
        changed(&update, "ImportedHolder", "imported", 43);
        assert_eq!(
            update.id, snapshot.id,
            "Loaded score changed identity on its first update"
        );
        save(&server).await;
        let saved = read_scoreboard(&server.basic_config.get_world_path());
        vanilla::assert_semantic_payload_eq(
            &expected_value(payload(&original).clone(), 43),
            payload(&saved),
        );
        command(
            &server,
            &worlds[2],
            "scoreboard players reset ImportedHolder imported",
        );
        removed(&one(&mut bedrock), &update);
        command(
            &server,
            &worlds[2],
            "scoreboard players set ImportedHolder imported 43",
        );
        assert_ne!(
            one(&mut bedrock).id,
            snapshot.id,
            "Loaded holder recreation reused a retired identity"
        );
        save(&server).await;
        bedrock.close().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g05_loaded_scores_receive_stable_runtime_identities() {
    let directory = tempfile::tempdir().unwrap();
    let original = old_save(directory.path());
    first_session(directory.path(), original).await;
    scenario(directory.path(), |server, worlds| async move {
        let mut java = TestPlayer::new(&worlds[1]);
        java.client().set_player(java.player.clone());
        java.client()
            .version
            .store(pumpkin_util::version::JavaMinecraftVersion::V_1_21_2);
        java.player.send_scoreboard();
        java.take_packets();
        let mut bedrock = TestBedrockPlayer::new(&worlds[0]).await;
        bedrock.player.send_scoreboard();
        let snapshot = one(&mut bedrock);
        changed(&snapshot, "ImportedHolder", "imported", 43);
        let hook = Arc::new(RecreateDuringRemoval(AtomicBool::new(false)));
        server.plugin_manager.register::<PacketSentEvent, _>(
            hook.clone(),
            EventPriority::Normal,
            true,
        );
        command(
            &server,
            &worlds[2],
            "scoreboard players reset ImportedHolder imported",
        );
        assert!(
            hook.0.load(Ordering::Relaxed),
            "Real packet callback was not exercised"
        );
        let packets = bedrock.take_packets();
        let effects = entries(&packets);
        assert_eq!(effects.len(), 3);
        removed(&effects[0], &snapshot);
        changed(&effects[1], "ImportedHolder", "imported", 77);
        changed(&effects[2], "ImportedHolder", "imported", 84);
        assert_ne!(
            effects[1].id, snapshot.id,
            "Reentrant recreation reused the removed identity"
        );
        assert_eq!(
            effects[1].id, effects[2].id,
            "Reentrant updates changed identity"
        );
        let mut lines = BTreeMap::from([(
            snapshot.id,
            ("imported".into(), "ImportedHolder".into(), 43),
        )]);
        apply(&mut lines, &packets);
        assert_eq!(
            lines.values().collect::<Vec<_>>(),
            [&("imported".into(), "ImportedHolder".into(), 84)]
        );
        assert_eq!(
            java_resets(&java.take_packets()),
            [("ImportedHolder".into(), Some("imported".into()))]
        );
        save(&server).await;
        assert_eq!(
            payload(&read_scoreboard(&server.basic_config.get_world_path()))
                .get_string("UnknownSavedField"),
            Some("keep this opaque data")
        );
        bedrock.close().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g05_unknown_saved_fields_control() {
    let directory = tempfile::tempdir().unwrap();
    let original = old_save(directory.path());
    scenario(directory.path(), |server, worlds| async move {
        command(
            &server,
            &worlds[0],
            "scoreboard objectives add scratch dummy",
        );
        save(&server).await;
        command(&server, &worlds[0], "scoreboard objectives remove scratch");
        save(&server).await;
        vanilla::assert_semantic_payload_eq(
            payload(&original),
            payload(&read_scoreboard(&server.basic_config.get_world_path())),
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g05_identity_exhaustion_keeps_existing_and_java_scores() {
    let directory = tempfile::tempdir().unwrap();
    scenario(directory.path(), |server, worlds| async move {
        let mut bedrock = TestBedrockPlayer::new(&worlds[0]).await;
        let mut java = TestPlayer::new(&worlds[1]);
        bedrock.player.send_scoreboard();
        java.player.send_scoreboard();
        command(&server, &worlds[0], "scoreboard objectives add tally dummy");
        command(
            &server,
            &worlds[0],
            "scoreboard objectives setdisplay sidebar tally",
        );
        server
            .scoreboard
            .lock()
            .unwrap()
            .exhaust_bedrock_score_identities_after_next();
        command(&server, &worlds[0], "scoreboard players set Last tally 1");
        let last = one(&mut bedrock);
        assert_eq!(last.id, i64::MAX);
        command(
            &server,
            &worlds[0],
            "scoreboard players set Omitted tally 2",
        );
        assert!(
            entries(&bedrock.take_packets()).is_empty(),
            "Exhausted allocator wrapped into a live ID"
        );
        command(&server, &worlds[0], "scoreboard players set Last tally 3");
        assert_eq!(one(&mut bedrock).id, last.id);
        assert_eq!(
            java_scores(&java.take_packets()),
            [
                ("Last".into(), "tally".into(), 1),
                ("Omitted".into(), "tally".into(), 2),
                ("Last".into(), "tally".into(), 3)
            ]
        );
        command(&server, &worlds[0], "scoreboard players reset Last tally");
        removed(&one(&mut bedrock), &last);
        command(&server, &worlds[0], "scoreboard players set Last tally 4");
        assert!(
            entries(&bedrock.take_packets()).is_empty(),
            "Retired IDs were reused after exhaustion"
        );
        assert_eq!(
            command(&server, &worlds[0], "scoreboard players get Last tally"),
            4
        );
        bedrock.close().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g05_builder_scores_publish_the_live_identity() {
    use crate::world::scoreboard::ScoreboardBuilder;
    use pumpkin_data::scoreboard::ScoreboardDisplaySlot;
    scenario(
        tempfile::tempdir().unwrap().path(),
        |server, worlds| async move {
            *server.scoreboard.lock().unwrap() = ScoreboardBuilder::new()
                .objective("built", pumpkin_util::text::TextComponent::text("Built"))
                .score("Holder", "built", 8)
                .display_slot(ScoreboardDisplaySlot::Sidebar, "built")
                .build();
            let mut bedrock = TestBedrockPlayer::new(&worlds[0]).await;
            bedrock.player.send_scoreboard();
            let snapshot = one(&mut bedrock);
            changed(&snapshot, "Holder", "built", 8);
            command(&server, &worlds[1], "scoreboard players set Holder built 9");
            let update = one(&mut bedrock);
            changed(&update, "Holder", "built", 9);
            assert_eq!(
                snapshot.id, update.id,
                "Builder snapshot and live update disagreed on identity"
            );
            bedrock.close().await;
        },
    )
    .await;
}
