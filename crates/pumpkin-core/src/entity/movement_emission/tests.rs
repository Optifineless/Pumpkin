mod controls;
mod flapping;
mod nonplayers;
mod packets;
mod support;

use crate::{entity::EntityBase, world::game_events::test_support::Scenario};
use pumpkin_data::{Block, packet::CURRENT_MC_VERSION};
use pumpkin_protocol::{
    ClientPacket, MultiVersionJavaPacket, RawPacket, ServerPacket,
    java::server::play::{SConfirmTeleport, SPlayerPosition, SPlayerPositionRotation},
};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};

const START: Vector3<f64> = Vector3::new(8.0, 65.0, 8.0);

fn scenario(movement: bool, sculk: bool) -> Scenario {
    let scenario = Scenario::with_movement(sculk, movement);
    scenario
        .owner
        .client()
        .set_player(scenario.owner.player.clone());
    for x in 0..16 {
        for z in 0..16 {
            place(
                &scenario,
                &BlockPos::new(x, 64, z),
                Block::STONE.default_state.id,
            );
        }
    }
    scenario.owner.player.get_entity().set_pos(START);
    scenario.capture(None);
    scenario
}

fn send<'a, P: ClientPacket + ServerPacket<'a> + MultiVersionJavaPacket>(s: &Scenario, packet: &P) {
    let mut payload = Vec::new();
    packet
        .write_packet_data(&mut payload, &CURRENT_MC_VERSION)
        .unwrap();
    s.owner
        .client()
        .handle_play_packet(
            &s.owner.player,
            &s.server,
            &RawPacket {
                id: P::to_id(CURRENT_MC_VERSION),
                payload: payload.into(),
            },
        )
        .unwrap();
}

fn position(s: &Scenario, turn: bool, pos: Vector3<f64>, ground: bool) {
    move_player(&s.owner.player, &s.server, turn, pos, ground);
}

fn move_player(
    player: &std::sync::Arc<crate::entity::player::Player>,
    server: &std::sync::Arc<crate::server::Server>,
    turn: bool,
    pos: Vector3<f64>,
    ground: bool,
) {
    let client = player.client.java().unwrap();
    let mut payload = Vec::new();
    let id = if turn {
        let packet = SPlayerPositionRotation {
            position: pos,
            yaw: 23.0,
            pitch: 0.0,
            collision: u8::from(ground),
        };
        packet
            .write_packet_data(&mut payload, &CURRENT_MC_VERSION)
            .unwrap();
        SPlayerPositionRotation::to_id(CURRENT_MC_VERSION)
    } else {
        let packet = SPlayerPosition {
            position: pos,
            collision: u8::from(ground),
        };
        packet
            .write_packet_data(&mut payload, &CURRENT_MC_VERSION)
            .unwrap();
        SPlayerPosition::to_id(CURRENT_MC_VERSION)
    };
    client
        .handle_play_packet(
            player,
            server,
            &RawPacket {
                id,
                payload: payload.into(),
            },
        )
        .unwrap();
}

fn at(dx: f64) -> Vector3<f64> {
    Vector3::new(START.x + dx, START.y, START.z)
}

fn confirm(s: &Scenario) {
    let pending = *s.owner.player.awaiting_teleport.lock().unwrap();
    if let Some((teleport_id, position)) = pending {
        send(
            s,
            &SConfirmTeleport {
                teleport_id,
                position,
                yaw: 0.0,
                pitch: 0.0,
            },
        );
    }
}

fn counts(s: &Scenario) -> (usize, usize) {
    let sink = s.sink.lock().unwrap();
    (sink.plugins.len(), sink.forwarded.len())
}

fn place(s: &Scenario, pos: &BlockPos, state: pumpkin_data::BlockStateId) {
    let (coordinate, relative) = pos.chunk_and_chunk_relative_position();
    let chunk = s
        .world
        .level
        .loaded_chunks
        .get(&coordinate)
        .unwrap()
        .clone();
    chunk.set_block_absolute_y(relative.x as usize, relative.y, relative.z as usize, state);
}
