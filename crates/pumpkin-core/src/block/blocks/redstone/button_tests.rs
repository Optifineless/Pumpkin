use super::*;
use crate::{
    net::java::combat_test_support::TestPlayer,
    server::combat_test_support,
    world::spawn_test_support::{proto, publish},
};
use pumpkin_data::biome::Biome;
use pumpkin_protocol::{
    java::client::play::CSoundEffect, packet::MultiVersionJavaPacket, ser::NetworkReadExt,
};

fn sound_count(player: &mut TestPlayer) -> usize {
    player
        .take_packets()
        .iter()
        .filter(|packet| {
            packet.as_ref().get_var_int().unwrap().0
                == CSoundEffect::to_id(pumpkin_data::packet::CURRENT_MC_VERSION)
        })
        .count()
}

async fn press(player_source: bool) {
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let mut presser = TestPlayer::new(&world);
    let mut neighbor = TestPlayer::new(&world);
    world.players.store(Arc::new(vec![
        presser.player.clone(),
        neighbor.player.clone(),
    ]));
    let position = BlockPos::new(8, 64, 8);
    let mut props = ButtonLikeProperties::from_state_id(Block::STONE_BUTTON.default_state.id);
    props.face = AttachFace::Floor;
    world.set_block_state(
        &position,
        props.to_state_id(&Block::STONE_BUTTON),
        BlockFlags::FORCE_STATE,
    );
    presser.take_packets();
    neighbor.take_packets();
    assert!(click_button(
        player_source.then_some(presser.player.as_ref()),
        &world,
        &position
    ));
    assert_eq!(sound_count(&mut presser), usize::from(!player_source));
    assert_eq!(sound_count(&mut neighbor), 1);
    assert!(!click_button(None, &world, &position));
    assert_eq!(sound_count(&mut presser), 0);
    assert_eq!(sound_count(&mut neighbor), 0);
    ButtonBlock.on_scheduled_tick(OnScheduledTickArgs {
        world: &world,
        block: &Block::STONE_BUTTON,
        position: &position,
    });
    assert_eq!(sound_count(&mut presser), 1);
    assert_eq!(sound_count(&mut neighbor), 1);
    world.level.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn button_press_excludes_only_presser() {
    press(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wind_charge_button_press_broadcasts() {
    press(false).await;
}
