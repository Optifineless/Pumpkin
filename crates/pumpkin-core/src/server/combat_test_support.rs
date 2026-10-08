//! Combat fixtures with real server services, without listeners, world ticking or startup tasks.
use super::*;
use std::sync::RwLock;

pub fn server(path: &std::path::Path) -> Arc<Server> {
    let session_lock = pumpkin_world::session_lock::acquire(path).unwrap();
    let basic_config = BasicConfiguration::default();
    let advanced_config = AdvancedConfiguration::default();
    let telemetry_config = TelemetryConfig::default();
    let vanilla_data = VanillaData {
        banned_ip_list: RwLock::default(),
        banned_player_list: RwLock::default(),
        operator_config: RwLock::default(),
        user_cache: RwLock::default(),
        whitelist_config: RwLock::default(),
    };
    let management_settings = Arc::new(crate::net::management::hub::ManagementSettings::new(
        &basic_config,
        &advanced_config,
        &advanced_config.networking.management,
    ));
    let management_hub = Arc::new(crate::net::management::hub::ManagementHub::new(
        management_settings,
    ));
    let plugin_loaders = Vec::new();
    let permission_manager = Arc::new(PermissionManager::new());
    let command_dispatcher = ArcSwap::from_pointee(default_dispatcher(
        &permission_manager,
        &advanced_config.commands,
    ));
    let block_registry = crate::block::registry::default_registry();
    let level_info = Arc::new(ArcSwap::from_pointee(LevelData::default(
        pumpkin_util::world_seed::Seed(0),
    )));
    let listing = std::sync::Mutex::new(CachedStatus::new(
        &basic_config,
        &advanced_config.networking.java.motd,
        advanced_config.networking.java.max_players,
    ));
    let defaultgamemode = std::sync::Mutex::new(DefaultGamemode {
        gamemode: basic_config.default_gamemode,
    });
    let player_data_storage =
        ServerPlayerData::new(path.join("players"), Duration::from_secs(300), false);
    let advancement_manager = Arc::new(AdvancementManager::new(path.join("advancements"), false));
    let white_list = AtomicBool::new(false);
    let tick_rate_manager = Arc::new(ServerTickRateManager::new(basic_config.tps));
    let dimensions = vec![Dimension::OVERWORLD];
    let server = Server {
        _session_lock: session_lock,
        basic_config,
        advanced_config,
        telemetry_config,
        data: vanilla_data,
        plugin_manager: Arc::new(PluginManager::new(plugin_loaders)),
        permission_manager,
        container_id: 0.into(),
        recipe_manager: Arc::new(recipe::RecipeManager::new()),
        datapack_manager: Arc::new(crate::data::datapack::DatapackManager::new()),
        enchantment_manager: Arc::new(enchantment::EnchantmentManager::new()),
        map_id: level_info.load().map_id.into(),
        worlds: ArcSwap::from_pointee(vec![]),
        dimensions,
        command_dispatcher,
        block_registry,
        item_registry: crate::item::items::default_registry(),
        key_store: OnceCell::new(),
        bedrock_oidc_keys: Arc::new(OnceCell::new()),
        listing,
        branding: CachedBranding::new(),
        bossbars: std::sync::Mutex::new(CustomBossbars::new()),
        map_manager: MapManager::new(),
        defaultgamemode,
        player_data_storage,
        tick_gate: tokio::sync::Mutex::new(()),
        command_storage: std::sync::Mutex::new(std::collections::HashMap::new()),
        stopwatches: std::sync::Mutex::new(crate::world::stopwatches::Stopwatches::new()),
        random_sequences: Arc::new(std::sync::Mutex::new(
            crate::world::random_sequences::RandomSequences::new(),
        )),
        advancement_manager,
        white_list,
        tick_rate_manager,
        tick_times_nanos: std::sync::Mutex::new([0; 100]),
        aggregated_tick_times_nanos: AtomicI64::new(0),
        tick_count: AtomicI32::new(0),
        debug_profiler: debug_profiler::DebugProfiler::new(),
        tasks: TaskTracker::new(),
        runtime: tokio::runtime::Handle::current(),
        scheduled_functions: Arc::new(crate::server::scheduler::ScheduledFunctionQueue::new()),
        server_guid: rand::random(),
        player_idle_timeout: AtomicI32::new(0),
        mojang_public_keys: ArcSwap::from_pointee(Vec::new()),
        world_info_writer: Arc::new(AnvilLevelInfo),
        level_info,
        management_hub,
    };
    Arc::new(server)
}

pub fn world(server: &Arc<Server>, path: &std::path::Path) -> Arc<World> {
    use pumpkin_config::world::LevelConfig;
    Arc::new(World::load(
        pumpkin_world::level::Level::from_root_folder(
            &LevelConfig::default(),
            path.to_path_buf(),
            0,
            Dimension::OVERWORLD,
        ),
        server.level_info.clone(),
        Dimension::OVERWORLD,
        server.block_registry.clone(),
        Arc::downgrade(server),
    ))
}
