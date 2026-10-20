#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "Real-entry border regressions use successful bounded fixtures"
)]

use std::{sync::Arc, time::Duration};

use crate::{
    command::CommandSender,
    entity::{EntityBase, decoration::hanging_test_support::Fixture as HangingFixture},
    server::Server,
    world::{World, portal::end_return_test_support},
};
use pumpkin_data::{Block, BlockDirection, item::Item, painting_variant::PaintingVariant};
use pumpkin_util::math::position::BlockPos;

fn run_command(server: &Arc<Server>, world: &Arc<World>, command: &str) {
    let source = CommandSender::Console
        .into_source(server)
        .with_world(world.clone());
    server
        .command_dispatcher
        .load()
        .execute_input(command, &source)
        .unwrap_or_else(|error| panic!("command should succeed ({command}): {error:?}"));
}

async fn hanging_resize(expanding: bool) {
    let fixture = HangingFixture::new();
    fixture
        .server
        .worlds
        .store(Arc::new(vec![fixture.world.clone()]));
    run_command(
        &fixture.server,
        &fixture.world,
        "worldborder center 0.0 8.5",
    );
    run_command(&fixture.server, &fixture.world, "worldborder set 20");

    let support_x = if expanding { 6 } else { 4 };
    let support = BlockPos::new(support_x + 1, 65, 8);
    for x in support_x..support_x + 4 {
        for y in 64..68 {
            fixture.set_block(BlockPos::new(x, y, 8), &Block::STONE);
        }
    }
    fixture
        .player
        .player
        .get_entity()
        .set_pos(pumpkin_util::math::vector3::Vector3::new(
            f64::from(support_x) + 0.5,
            64.0,
            12.5,
        ));
    let wide_variant = PaintingVariant::all()
        .iter()
        .copied()
        .find(|variant| variant.width() == 4 && variant.height() == 4)
        .unwrap();
    let painting = fixture.place(
        &Item::PAINTING,
        support,
        BlockDirection::South,
        Some(wide_variant),
    );

    if expanding {
        run_command(&fixture.server, &fixture.world, "worldborder set 10");
        run_command(&fixture.server, &fixture.world, "worldborder set 20 202t");
        fixture.tick(101);
        assert!(
            painting.get_entity().is_removed(),
            "the current expanding extent still excludes the four-wide painting"
        );
    } else {
        run_command(&fixture.server, &fixture.world, "worldborder set 10 202t");
        fixture.tick(101);
        assert!(
            !painting.get_entity().is_removed(),
            "a shrinking border must retain the painting while the current extent includes it"
        );
        fixture.tick(101);
        assert!(
            painting.get_entity().is_removed(),
            "the painting retires when the shrinking current extent reaches it"
        );
    }
    fixture.finish().await;
}

async fn end_return_radius_uses_live_extent() {
    let fixture = end_return_test_support::Fixture::new().await;
    fixture.server.level_info.rcu(|info| {
        let mut info = (**info).clone();
        info.spawn_x = 7;
        info.spawn_z = 8;
        info.spawn_y = 200;
        info.game_rules.respawn_radius = 3;
        info
    });
    run_command(&fixture.server, &fixture.overworld, "worldborder set 20");
    run_command(
        &fixture.server,
        &fixture.overworld,
        "worldborder set 8 200t",
    );

    // Only (9, 8) has a dry column. Its block lies within the live radius and
    // current border, but outside the radius computed from the final diameter.
    for x in 5..=9 {
        for z in 6..=10 {
            if (x, z) != (9, 8) {
                fixture.overworld.set_block_state(
                    &BlockPos::new(x, 63, z),
                    Block::WATER.default_state.id,
                    crate::world::BlockFlags::FORCE_STATE,
                );
            }
        }
    }

    let mut client = fixture.returning_player();
    end_return_test_support::transfer(&fixture, &mut client).await;
    assert_eq!(
        client.player.position(),
        pumpkin_util::math::vector3::Vector3::new(9.5, 64.0, 8.5),
        "shared-spawn search must reach the candidate admitted by the live extent"
    );
    fixture.finish().await;
}

async fn end_return_border_spawn(spawn_x: i32, diameter: i32) {
    let mut fixture = end_return_test_support::Fixture::new().await;
    fixture
        .prepare_shared_spawn_floor(BlockPos::new(spawn_x, 64, 0))
        .await;
    fixture.server.level_info.rcu(|info| {
        let mut info = (**info).clone();
        info.spawn_x = spawn_x;
        info.spawn_y = 64;
        info.spawn_z = 0;
        info.game_rules.respawn_radius = 0;
        info
    });
    let border = fixture.overworld.worldborder.lock().unwrap();
    assert_eq!(
        (border.center_x, border.center_z),
        (0.0, 0.0),
        "the fixture border is centered at the requested origin"
    );
    drop(border);
    run_command(
        &fixture.server,
        &fixture.overworld,
        &format!("worldborder set {diameter}"),
    );

    let mut client = end_return_test_support::Fixture::returning_player(&fixture);
    let player = client.player.clone();
    let health = player.living_entity.health.load();
    end_return_test_support::transfer(&fixture, &mut client).await;

    let position = player.position();
    assert_eq!(position.y, 64.0, "End return must use the supported floor");
    let support = BlockPos::new(position.x.floor() as i32, 63, position.z.floor() as i32);
    assert!(
        fixture.overworld.get_block_state(&support).is_solid(),
        "End return must arrive above solid ground"
    );
    player.get_entity().tick(player.as_ref(), &fixture.server);
    assert_eq!(
        player.living_entity.health.load(),
        health,
        "safe-height End return must not deal fall damage"
    );
    assert_eq!(player.living_entity.fall_distance.load(), 0.0);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g10_end_return_border_exemption_keeps_supported_spawn() {
    // The shared spawn lies beyond the diameter-10 border, and the second case
    // is narrower than the player's body in either horizontal axis.
    end_return_border_spawn(6, 10).await;
    end_return_border_spawn(0, 1).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g10_hanging_survival_and_spawn_radius_use_live_extent() {
    hanging_resize(false).await;
    hanging_resize(true).await;
    tokio::time::timeout(
        Duration::from_secs(60),
        end_return_radius_uses_live_extent(),
    )
    .await
    .unwrap();
}
