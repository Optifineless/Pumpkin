use std::sync::Arc;

pub fn armor_test_world(path: &std::path::Path) -> Arc<crate::world::World> {
    use arc_swap::ArcSwap;
    use pumpkin_config::world::LevelConfig;
    use pumpkin_data::dimension::Dimension;
    use pumpkin_util::world_seed::Seed;
    use pumpkin_world::level::Level;

    let world = Arc::new(crate::world::World::load(
        Level::from_root_folder(
            &LevelConfig::default(),
            path.to_path_buf(),
            0,
            Dimension::OVERWORLD,
        ),
        Arc::new(ArcSwap::from_pointee(crate::world::LevelData::default(
            Seed(0),
        ))),
        Dimension::OVERWORLD,
        Arc::new(crate::block::registry::BlockRegistry::default()),
        std::sync::Weak::new(),
    ));
    crate::server::fixture_lifecycle::track_world(&world);
    world
}
