use pumpkin_data::block_properties::StickyPistonLikeProperties;

use super::*;
use crate::block::entities::shulker_box::ShulkerBoxBlockEntity;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_bed_admission_rules_piston_suffocation() {
    for (block, item) in BEDS {
        let mut fixture = Fixture::new(Dimension::OVERWORLD);
        for piston in [&Block::PISTON, &Block::STICKY_PISTON] {
            for extended in [false, true] {
                fixture.reset();
                fixture.place(block, item);
                let mut properties = StickyPistonLikeProperties::default(piston);
                properties.extended = extended;
                fixture.world.set_block_state(
                    &HEAD.up(),
                    properties.to_state_id(piston),
                    BlockFlags::NOTIFY_LISTENERS | BlockFlags::UPDATE_KNOWN_SHAPE,
                );
                fixture.use_on(HEAD);
                assert_eq!(
                    fixture.client.player.sleeping_bed_pos.load(),
                    extended.then_some(HEAD),
                    "{} extended={extended} over {}",
                    piston.name,
                    block.name
                );
            }
        }
        fixture.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_bed_admission_rules_shulker_suffocation() {
    for (block, item) in BEDS {
        let mut fixture = Fixture::new(Dimension::OVERWORLD);
        for open in [false, true] {
            fixture.reset();
            fixture.place(block, item);
            let ceiling = HEAD.up();
            fixture.world.set_block_state(
                &ceiling,
                Block::SHULKER_BOX.default_state.id,
                BlockFlags::NOTIFY_LISTENERS | BlockFlags::UPDATE_KNOWN_SHAPE,
            );
            let shulker = Arc::new(ShulkerBoxBlockEntity::new(ceiling));
            fixture.world.add_block_entity(shulker.clone());
            if open {
                fixture.use_on(ceiling);
            }
            assert_eq!(shulker.viewers.current.load(Relaxed), u16::from(open));
            fixture.use_on(HEAD);
            assert_eq!(
                fixture.client.player.sleeping_bed_pos.load(),
                open.then_some(HEAD),
                "shulker open={open} over {}",
                block.name
            );
            if open {
                fixture.client.player.close_handled_screen();
            }
        }
        fixture.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_bed_admission_rules_revalidate_spawn_callback() {
    for (block, item) in BEDS {
        for change in 0..3 {
            let mut dimension = Dimension::OVERWORLD;
            dimension.straw_bed_rule.can_set_spawn = BedRuleOption::Always;
            let mut fixture = Fixture::new(dimension);
            fixture.place(block, item);
            let callbacks = Arc::new(AtomicUsize::new(0));
            let seen = callbacks.clone();
            let world = fixture.world.clone();
            fixture.register::<PlayerSpawnChangeEvent>(move |event| {
                seen.fetch_add(1, Relaxed);
                match change {
                    0 => event
                        .player
                        .get_entity()
                        .set_pos(Vector3::new(20.5, 64.0, 5.5)),
                    1 => {
                        world.set_block_state(
                            &HEAD.up(),
                            Block::STONE.default_state.id,
                            BlockFlags::NOTIFY_ALL,
                        );
                    }
                    _ => world.set_time_of_day(1_000),
                }
            });
            let enters = Arc::new(AtomicUsize::new(0));
            let seen = enters.clone();
            fixture.register::<PlayerBedEnterEvent>(move |_| {
                seen.fetch_add(1, Relaxed);
            });
            fixture.use_on(HEAD);
            assert_eq!(callbacks.load(Relaxed), 1);
            fixture.awake();
            assert_eq!(
                enters.load(Relaxed),
                0,
                "spawn callback invalidated admission case {change}"
            );
            assert_eq!(
                fixture.spawn(),
                Some(HEAD),
                "completed spawn change remains valid"
            );
            for position in [FOOT, HEAD] {
                assert!(
                    !WhiteBedLikeProperties::from_state_id(
                        fixture.world.get_block_state_id(&position)
                    )
                    .occupied
                );
            }
            fixture.finish().await;
        }
    }
}
