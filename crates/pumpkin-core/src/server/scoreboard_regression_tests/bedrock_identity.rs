use super::*;
use crate::net::bedrock::combat_test_support::TestBedrockPlayer;
use pumpkin_protocol::{
    Packet, RawPacket,
    bedrock::{
        BEDROCK_GAME_PACKET,
        client::{remove_objective::CRemoveObjective, set_score::CSetScore},
        packet_decoder::BedrockBatchDecoder,
    },
    codec::{var_long::VarLong, var_uint::VarUInt},
    ser::NetworkReadExt,
    serial::PacketRead,
};
use std::collections::BTreeMap;
use std::io::Cursor;

mod lifecycle;
mod loaded;

async fn scenario<F: std::future::Future<Output = ()>>(
    path: &Path,
    check: impl FnOnce(Arc<Server>, Vec<Arc<World>>) -> F,
) {
    tokio::time::timeout(Duration::from_secs(60), with_worlds(path, None, check))
        .await
        .expect("Scoreboard fixture or publication exceeded its bounded deadline");
}

#[derive(Debug, PartialEq, Eq)]
struct Entry {
    id: i64,
    objective: String,
    change: Option<(String, i32)>,
}

fn game_packet(frame: &bytes::Bytes) -> RawPacket {
    // TestBedrockPlayer uses the normal pre-compression batch encoder.
    assert_eq!(frame[0], BEDROCK_GAME_PACKET);
    let mut input = Cursor::new(frame[1..].to_vec());
    let packet = BedrockBatchDecoder::new()
        .get_game_packet(&mut input)
        .unwrap();
    assert_eq!(input.position() as usize, input.get_ref().len());
    packet
}

fn entries(packets: &[bytes::Bytes]) -> Vec<Entry> {
    assert!(packets.len() < 128, "Unbounded scoreboard packet capture");
    let mut entries = Vec::new();
    for packet in packets {
        let packet = game_packet(packet);
        if packet.id != CSetScore::PACKET_ID {
            continue;
        }
        let mut input = packet.payload.as_ref();
        let count = VarUInt::read(&mut input).unwrap().0;
        assert!(count < 32);
        for _ in 0..count {
            let kind = VarUInt::read(&mut input).unwrap().0;
            let label = String::read(&mut input).unwrap();
            assert_eq!(
                label,
                if kind == 0 {
                    "remove"
                } else {
                    "changefakeplayer"
                }
            );
            let id = VarLong::read(&mut input).unwrap().0;
            let objective = if kind != 0 || bool::read(&mut input).unwrap() {
                String::read(&mut input).unwrap()
            } else {
                String::new()
            };
            let change = if kind == 0 {
                None
            } else {
                assert_eq!(kind, 3);
                let value = i32::read(&mut input).unwrap();
                Some((String::read(&mut input).unwrap(), value))
            };
            entries.push(Entry {
                id,
                objective,
                change,
            });
        }
        assert!(input.is_empty(), "Unsupported trailing Bedrock score data");
    }
    entries
}

fn one(player: &mut TestBedrockPlayer) -> Entry {
    let mut entries = entries(&player.take_packets());
    assert_eq!(
        entries.len(),
        1,
        "Expected exactly one score entry: {entries:?}"
    );
    entries.pop().unwrap()
}

fn changed(entry: &Entry, holder: &str, objective: &str, value: i32) {
    assert_eq!(entry.objective, objective);
    assert_eq!(entry.change, Some((holder.to_owned(), value)));
}

fn removed(entry: &Entry, prior: &Entry) {
    assert_eq!(entry.change, None);
    assert_eq!(entry.objective, prior.objective);
    assert_eq!(
        entry.id, prior.id,
        "Removal did not address the published live entry"
    );
}

fn java_scores(packets: &[bytes::Bytes]) -> Vec<(String, String, i32)> {
    packets
        .iter()
        .filter_map(|packet| {
            let mut input = packet.as_ref();
            if input.get_var_int().unwrap().0
                != pumpkin_data::packet::clientbound::play::UPDATE_SCORE.0
            {
                return None;
            }
            Some((
                input.get_str().unwrap().into_string(),
                input.get_str().unwrap().into_string(),
                input.get_var_int().unwrap().0,
            ))
        })
        .collect()
}

fn java_resets(packets: &[bytes::Bytes]) -> Vec<(String, Option<String>)> {
    packets
        .iter()
        .filter_map(|packet| {
            let mut input = packet.as_ref();
            if input.get_var_int().unwrap().0
                != pumpkin_data::packet::clientbound::play::RESET_SCORE.0
            {
                return None;
            }
            let holder = input.get_str().unwrap().into_string();
            let objective = input
                .get_bool()
                .unwrap()
                .then(|| input.get_str().unwrap().into_string());
            assert!(input.is_empty());
            Some((holder, objective))
        })
        .collect()
}

// Apply the supported codec as a client would, including objective hide/removal.
fn apply(lines: &mut BTreeMap<i64, (String, String, i32)>, packets: &[bytes::Bytes]) {
    for packet in packets {
        let decoded = game_packet(packet);
        let mut input = decoded.payload.as_ref();
        if decoded.id == CRemoveObjective::PACKET_ID {
            let objective = String::read(&mut input).unwrap();
            lines.retain(|_, (name, _, _)| *name != objective);
        }
        for entry in entries(std::slice::from_ref(packet)) {
            if let Some((holder, value)) = entry.change {
                lines.insert(entry.id, (entry.objective, holder, value));
            } else {
                let old = lines
                    .remove(&entry.id)
                    .expect("Removal targeted a missing or unrelated line");
                assert_eq!(old.0, entry.objective);
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g05_score_update_snapshot_and_remove_share_identity() {
    let directory = tempfile::tempdir().unwrap();
    scenario(directory.path(), |server, worlds| async move {
        let mut bedrock = TestBedrockPlayer::new(&worlds[0]).await;
        let mut java = TestPlayer::new(&worlds[1]);
        bedrock.player.send_scoreboard();
        java.player.send_scoreboard();
        bedrock.take_packets();
        java.take_packets();
        command(&server, &worlds[1], "scoreboard objectives add tally dummy");
        command(
            &server,
            &worlds[1],
            "scoreboard objectives setdisplay sidebar tally",
        );
        let inputs: Vec<_> = [7, 11, 19]
            .into_iter()
            .map(|value| {
                format!(
                    "scoreboard players set {} {} {value}",
                    String::from("Holder"),
                    String::from("tally")
                )
            })
            .collect();
        let mut updates = Vec::new();
        for (input, value) in inputs.iter().zip([7, 11, 19]) {
            command(&server, &worlds[1], input);
            let update = one(&mut bedrock);
            changed(&update, "Holder", "tally", value);
            updates.push(update);
        }
        assert_eq!(
            java_scores(&java.take_packets()),
            vec![
                ("Holder".into(), "tally".into(), 7),
                ("Holder".into(), "tally".into(), 11),
                ("Holder".into(), "tally".into(), 19)
            ]
        );
        let mut joining = TestBedrockPlayer::new(&worlds[2]).await;
        joining.player.send_scoreboard();
        let snapshot = one(&mut joining);
        changed(&snapshot, "Holder", "tally", 19);
        // A save/snapshot clone must keep the owner's existing identities.
        let clone = server.scoreboard.lock().unwrap().clone();
        let effects = crate::world::guard_effects::GuardEffects::new();
        clone.send_to_player(&joining.player);
        drop(effects);
        let cloned = one(&mut joining);
        command(&server, &worlds[1], "scoreboard players reset Holder tally");
        let removal = one(&mut bedrock);
        assert_eq!(one(&mut joining), removal);
        assert_eq!(
            java_resets(&java.take_packets()),
            [("Holder".into(), Some("tally".into()))]
        );
        for update in &updates {
            assert_eq!(
                update.id, snapshot.id,
                "Live updates and join snapshot changed identity"
            );
        }
        assert_eq!(cloned.id, snapshot.id, "Cloned snapshot changed identity");
        removed(&removal, &snapshot);
        bedrock.close().await;
        joining.close().await;
    })
    .await;
}
