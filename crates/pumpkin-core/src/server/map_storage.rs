use super::Server;

impl Server {
    /// Persists dirty maps and `MapIndex` on autosave, explicit save, and shutdown.
    pub(crate) async fn save_maps(&self) -> Result<(), String> {
        let path = self.basic_config.get_world_path();
        let data_version = self.level_info.load().data_version;
        self.map_manager
            .save(&path, data_version)
            .await
            .map_err(|error| error.to_string())
    }
}
