use super::player::Player;
use crate::net::ClientPlatform;
use pumpkin_protocol::java::client::play::CUpdateRecipes;
use pumpkin_util::version::JavaMinecraftVersion;
use std::sync::OnceLock;

pub(super) fn sync(player: &Player) {
    static DATA: OnceLock<Option<Vec<u8>>> = OnceLock::new();
    // PlayerList.placeNewPlayer sends synchronized recipe properties before menus need them.
    let ClientPlatform::Java(client) = &*player.client else {
        return;
    };
    if client.version.load() != JavaMinecraftVersion::V_26_3 {
        return;
    }
    if let Some(data) = DATA.get_or_init(|| {
        match CUpdateRecipes::vanilla_inventory_data(JavaMinecraftVersion::V_26_3) {
            Ok(data) => Some(data),
            Err(error) => {
                tracing::error!(%error, "Failed to encode inventory recipes");
                None
            }
        }
    }) {
        player.try_send_client_packet(&CUpdateRecipes::new(data));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{net::java::combat_test_support::TestPlayer, server::combat_test_support};
    use pumpkin_inventory::{
        screen_handler::ScreenHandler, stonecutter_screen_handler::StonecutterScreenHandler,
        sync_handler::SyncHandler,
    };
    use std::sync::Arc;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stonecutter_sync_sends_recipe_choices_before_contents() {
        let dir = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(dir.path());
        let world = combat_test_support::world(&server, dir.path());
        let mut fixture = TestPlayer::new(&world);
        fixture.take_packets();
        let mut menu = StonecutterScreenHandler::new(1, &fixture.player.inventory);
        let sync = Arc::new(SyncHandler::new());
        sync.store_player(fixture.player.clone());
        menu.update_sync_handler(sync);
        let data = CUpdateRecipes::vanilla_inventory_data(JavaMinecraftVersion::V_26_3).unwrap();
        let expected = fixture
            .client()
            .serialize_packet(&CUpdateRecipes::new(&data))
            .unwrap();
        let packets = fixture.take_packets();
        assert_eq!(packets.first(), Some(&expected));
        assert!(packets.len() >= 2);
        assert!(world.level.shutdown().await.is_ok());
        crate::server::fixture_lifecycle::finish().await;
    }
}
