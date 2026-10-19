use super::world_info_save_tests::with_worlds;
use super::{Server, combat_test_support};
use crate::command::CommandSender;
use crate::net::java::combat_test_support::TestPlayer;
use crate::world::World;
use pumpkin_nbt::{compound::NbtCompound, nbt_compress::read_gzip_compound_tag};
use std::{fs, path::Path, sync::Arc, time::Duration};

mod death_visibility;
mod legacy;
mod lifecycle;
mod loaded;
mod login;
mod projection;
mod reentry;
mod save_paths;
mod saved_components;
mod vanilla;
mod versions;

fn command(server: &Arc<Server>, world: &Arc<World>, input: &str) -> i32 {
    let source = CommandSender::Console
        .into_source(server)
        .with_world(world.clone());
    server
        .command_dispatcher
        .load()
        .execute_input(input, &source)
        .unwrap_or_else(|error| panic!("Command {input:?} failed: {error:?}"))
}

async fn save(server: &Server) {
    tokio::time::timeout(Duration::from_secs(30), server.save_all())
        .await
        .expect("Real scoreboard save timed out")
        .unwrap();
}

fn read_scoreboard(path: &Path) -> NbtCompound {
    read_gzip_compound_tag(fs::File::open(path.join("data/minecraft/scoreboard.dat")).unwrap())
        .unwrap()
}

fn payload(root: &NbtCompound) -> &NbtCompound {
    root.get_compound("data").unwrap()
}

fn packet_count(packets: &[bytes::Bytes], expected: i32) -> usize {
    packets
        .iter()
        .filter(|packet| {
            let mut data = packet.as_ref();
            pumpkin_protocol::VarInt::decode(&mut data).unwrap().0 == expected
        })
        .count()
}

fn populate_base(server: &Arc<Server>, world: &Arc<World>) {
    for input in [
        "scoreboard objectives add tally dummy Tally",
        "scoreboard players set Holder tally 37",
        "team add red Crimson",
        "team modify red color red",
        "team modify red prefix Prefix",
        "team modify red suffix Suffix",
        "team modify red nametagVisibility hideForOtherTeams",
        "team modify red deathMessageVisibility hideForOwnTeam",
        "team modify red collisionRule pushOwnTeam",
    ] {
        command(server, world, input);
    }
    for (option, mask) in [("friendlyFire", 1), ("seeFriendlyInvisibles", 2)] {
        let enabled = world
            .scoreboard
            .lock()
            .unwrap()
            .get_team("red")
            .unwrap()
            .options
            & mask
            != 0;
        if !enabled {
            command(server, world, &format!("team modify red {option} true"));
        }
        command(server, world, &format!("team modify red {option} false"));
    }
}

fn populate_formatting(server: &Arc<Server>, world: &Arc<World>) {
    for input in [
        r#"scoreboard objectives modify tally displayname {"text":"Total","color":"gold"}"#,
        "scoreboard objectives modify tally rendertype hearts",
        "scoreboard objectives modify tally displayautoupdate true",
        r#"scoreboard objectives modify tally numberformat fixed {"text":"many"}"#,
        "scoreboard objectives setdisplay sidebar.team.red tally",
        r#"scoreboard players display name Holder tally {"text":"Alias","italic":true}"#,
        "scoreboard players display numberformat Holder tally styled {color:'blue',bold:true}",
    ] {
        command(server, world, input);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoreboard_regression_commands_share_state_and_packets_across_dimensions() {
    let directory = tempfile::tempdir().unwrap();
    with_worlds(directory.path(), None, |server, worlds| async move {
        let mut overworld = TestPlayer::new(&worlds[0]);
        let mut nether = TestPlayer::new(&worlds[1]);
        overworld.player.send_scoreboard();
        nether.player.send_scoreboard();
        overworld.take_packets();
        nether.take_packets();
        command(
            &server,
            &worlds[1],
            "scoreboard objectives add shared dummy",
        );
        command(
            &server,
            &worlds[1],
            "scoreboard players set Holder shared 37",
        );
        command(&server, &worlds[1], "team add shared_team");
        assert_eq!(
            command(&server, &worlds[0], "scoreboard players get Holder shared"),
            37,
            "An objective created in the Nether was invisible to the Overworld dispatcher"
        );
        assert_eq!(command(&server, &worlds[0], "team list"), 1);
        for packets in [overworld.take_packets(), nether.take_packets()] {
            for id in [
                pumpkin_data::packet::clientbound::play::SCOREBOARD_OBJECTIVE.0,
                pumpkin_data::packet::clientbound::play::UPDATE_SCORE.0,
                pumpkin_data::packet::clientbound::play::SET_PLAYER_TEAM.0,
            ] {
                assert_eq!(
                    packet_count(&packets, id),
                    1,
                    "Missing or repeated packet {id}"
                );
            }
            assert_eq!(
                packet_count(
                    &packets,
                    pumpkin_data::packet::clientbound::play::RESET_SCORE.0
                ),
                0,
                "Creating a score unexpectedly reset another score"
            );
        }
    })
    .await;
}

fn assert_command_fields(data: &NbtCompound) {
    let objective = data
        .get_list("Objectives")
        .unwrap()
        .iter()
        .filter_map(pumpkin_nbt::tag::NbtTag::extract_compound)
        .find(|objective| {
            objective
                .get_string("Name")
                .is_some_and(|name| name == "tally")
        })
        .unwrap();
    assert_eq!(objective.get_string("CriteriaName"), Some("dummy"));
    assert_eq!(objective.get_string("RenderType"), Some("hearts"));
    assert_eq!(objective.get_bool("display_auto_update"), Some(true));
    assert!(objective.get_compound("format").is_some());
    let scores = data.get_list("PlayerScores").unwrap();
    let holder = scores
        .iter()
        .filter_map(pumpkin_nbt::tag::NbtTag::extract_compound)
        .find(|score| {
            score
                .get_string("Name")
                .is_some_and(|name| name == "Holder")
        })
        .unwrap();
    assert_eq!(holder.get_int("Score"), Some(37));
    assert!(holder.get("display").is_some());
    assert!(holder.get_compound("format").is_some());
    assert_eq!(holder.get_bool("Locked"), Some(true));
    assert_eq!(
        data.get_compound("DisplaySlots")
            .unwrap()
            .get_string("sidebar.team.red"),
        Some("tally")
    );
    let team = data.get_list("Teams").unwrap()[0]
        .extract_compound()
        .unwrap();
    assert_eq!(team.get_string("TeamColor"), Some("red"));
    assert_eq!(team.get_bool("AllowFriendlyFire"), Some(false));
    assert_eq!(team.get_bool("SeeFriendlyInvisibles"), Some(false));
    assert_eq!(
        team.get_string("NameTagVisibility"),
        Some("hideForOtherTeams")
    );
    assert_eq!(
        team.get_string("DeathMessageVisibility"),
        Some("hideForOwnTeam")
    );
    assert_eq!(team.get_string("CollisionRule"), Some("pushOwnTeam"));
    assert_eq!(team.get_list("Players").unwrap().len(), 1);
    assert!(team.get("MemberNamePrefix").is_some());
    assert!(team.get("MemberNameSuffix").is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoreboard_regression_commands_save_and_reload_every_field() {
    let directory = tempfile::tempdir().unwrap();
    with_worlds(directory.path(), None, |server, worlds| async move {
        populate_base(&server, &worlds[1]);
        let player = TestPlayer::new(&worlds[1]);
        let source = player.player.get_command_source(&server);
        command(&server, &worlds[1], "team join red @a");
        // This first save is also a baseline-compatible negative control: the old
        // dispatcher can populate these fields, but its save path writes a stub.
        save(&server).await;
        let root = read_scoreboard(&server.basic_config.get_world_path());
        assert_eq!(
            payload(&root).get_list("Objectives").map(<[_]>::len),
            Some(1)
        );
        populate_formatting(&server, &worlds[1]);
        command(
            &server,
            &worlds[1],
            "scoreboard objectives add trigger_value trigger",
        );
        command(
            &server,
            &worlds[1],
            "scoreboard players set @a trigger_value 9",
        );
        command(
            &server,
            &worlds[1],
            "scoreboard players enable @a trigger_value",
        );
        server
            .command_dispatcher
            .load()
            .execute_input("trigger trigger_value", &source)
            .unwrap();
        save(&server).await;
        let path = server.basic_config.get_world_path();
        let root = read_scoreboard(&path);
        let data = payload(&root);
        assert_command_fields(data);
        let destination = path.join("data/minecraft/scoreboard.dat");
        let sentinel = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1);
        fs::File::open(&destination)
            .unwrap()
            .set_modified(sentinel)
            .unwrap();
        command(
            &server,
            &worlds[1],
            "scoreboard players set Holder tally 37",
        );
        save(&server).await;
        assert_eq!(
            fs::metadata(&destination).unwrap().modified().unwrap(),
            sentinel,
            "An unchanged scoreboard was rewritten"
        );
        fs::write(
            path.join("expected-scoreboard.nbt"),
            pumpkin_nbt::nbt_compress::write_gzip_compound_tag_to_bytes(root).unwrap(),
        )
        .unwrap();
    })
    .await;
    with_worlds(directory.path(), None, |server, worlds| async move {
        loaded::assert_command_fields(&worlds[2]);
        assert_eq!(
            command(&server, &worlds[2], "scoreboard players get Holder tally"),
            37
        );
        assert_eq!(command(&server, &worlds[2], "team list red"), 1);
        let path = server.basic_config.get_world_path();
        let expected =
            read_gzip_compound_tag(fs::File::open(path.join("expected-scoreboard.nbt")).unwrap())
                .unwrap();
        save(&server).await;
        vanilla::assert_semantic_payload_eq(payload(&expected), payload(&read_scoreboard(&path)));
    })
    .await;
}
