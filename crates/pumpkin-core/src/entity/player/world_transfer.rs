use super::{
    Arc, CRespawn, CSetSelectedSlot, ClientPlatform, Dimension, EntityBase, Ordering, Player,
    PlayerChangeWorldEvent, PlayerSpawnData, ResourceLocation, VarInt, Vector3, World, biome,
};
use pumpkin_inventory::screen_handler::InventoryPlayer;
use pumpkin_macros::send_cancellable;
use pumpkin_util::math::position::BlockPos;

impl Player {
    pub(super) async fn teleport_world_inner(
        self: &Arc<Self>,
        new_world: Arc<World>,
        position: Vector3<f64>,
        yaw: Option<f32>,
        pitch: Option<f32>,
    ) {
        // PlayerList.respawn publishes the restored life in its resolved level exactly once.
        if self.living_entity.is_respawning() {
            return;
        }
        let current_world = self.living_entity.entity.world.load_full();
        let yaw = yaw.unwrap_or(new_world.level_info.load().spawn_yaw);
        let pitch = pitch.unwrap_or(new_world.level_info.load().spawn_pitch);

        let Some(server) = new_world.server.upgrade() else {
            return;
        };

        send_cancellable! {{
            server;
            PlayerChangeWorldEvent {
                player: self.clone(),
                previous_world: current_world.clone(),
                new_world: new_world.clone(),
                position,
                yaw,
                pitch,
                cancelled: false,
            };

            'after: {
                let Some(transfer) = self.living_entity.begin_world_transfer(&current_world) else {
                    return;
                };
                let position = event.position;
                let yaw = event.yaw;
                let pitch = event.pitch;
                let new_world = event.new_world;

                self.set_client_loaded(false);
                let Some(player) = current_world.remove_player(self, false).await else {
                    return;
                };
                self.unload_watched_chunks(&current_world).await;
                let (lifecycle, last_pos) = {
                    let _owner = self.living_entity.own_damage();
                    if self.living_entity.is_respawning() || self.get_entity().is_removed() || self.client.closed() {
                        return;
                    }
                    let last_pos = self.living_entity.entity.last_pos.load();
                    // ServerPlayer.teleport: setServerLevel/addDuringTeleport form one commit.
                    self.change_world_chunks(&current_world.level, &new_world);
                    self.living_entity.entity.set_world(new_world.clone());
                    player.get_entity().set_pos(position);
                    player.get_entity().set_rotation(yaw, pitch);
                    player.get_entity().last_pos.store(position);
                    new_world.add_arriving_player(&player);
                    new_world.players.rcu(|current_list| {
                        let mut new_list = (**current_list).clone();
                        if !new_list.iter().any(|p| p.entity_id() == player.entity_id()) {
                            new_list.push(player.clone());
                        }
                        new_list
                    });
                    (self.living_entity.damage_lifecycle(), last_pos)
                };
                // Membership is complete before callbacks can start another respawn.
                drop(transfer);

                self.send_world_transfer_packets(&new_world, position, lifecycle, last_pos).await;

                self.finish_world_transfer(&new_world, position, yaw, pitch, lifecycle).await;
                if !self.world_transfer_life_current(&new_world, lifecycle) { return; }
                let mut changed_world_event = crate::plugin::api::events::player::player_changed_world::PlayerChangedWorldEvent {
                    player: player.clone(),
                    from_world: current_world,
                    to_world: new_world,
                    cancelled: false,
                };
                server.plugin_manager.fire(&server, &mut changed_world_event).await;
            }
        }}
    }

    // ServerPlayer.teleport sends the new level's spawn information after committing the move.
    async fn send_world_transfer_packets(
        &self,
        new_world: &Arc<World>,
        position: Vector3<f64>,
        lifecycle: u64,
        last_pos: Vector3<f64>,
    ) {
        if !self.world_transfer_life_current(new_world, lifecycle) {
            return;
        }
        let death_dimension = ResourceLocation::from(self.world().dimension.minecraft_name);
        let death_location = BlockPos(Vector3::new(
            last_pos.x.round() as i32,
            last_pos.y.round() as i32,
            last_pos.z.round() as i32,
        ));
        match self.client.as_ref() {
            ClientPlatform::Java(java) => {
                let packet = CRespawn::new(
                    PlayerSpawnData::new(
                        new_world.dimension.clone(),
                        biome::hash_seed(new_world.level.seed.0), // seed
                        self.gamemode.load() as u8,
                        self.previous_gamemode
                            .load()
                            .unwrap_or(self.gamemode.load()) as i8,
                        false,
                        false,
                        Some((death_dimension, death_location)),
                        VarInt(self.get_entity().portal_cooldown.load(Ordering::Relaxed) as i32),
                        new_world.sea_level.into(),
                    ),
                    CRespawn::KEEP_ALL_DATA,
                );
                if let Ok(data) = java.serialize_packet(&packet) {
                    java.send_packet_now(data).await;
                }
            }
            ClientPlatform::Bedrock(bedrock) => {
                let bedrock_dimension = if new_world.dimension == Dimension::OVERWORLD {
                    0
                } else if new_world.dimension == Dimension::THE_NETHER {
                    1
                } else if new_world.dimension == Dimension::THE_END {
                    2
                } else {
                    0
                };
                let pos_f32 = Vector3::new(position.x as f32, position.y as f32, position.z as f32);
                let change_dim_packet = pumpkin_protocol::bedrock::client::CChangeDimension {
                    dimension_id: bedrock_dimension.into(),
                    position: pos_f32,
                    respawn: false,
                    loading_screen_id: None,
                };
                if let Ok(data) = bedrock.serialize_packet(&change_dim_packet) {
                    bedrock.enqueue_packet(data).await;
                }
                self.bedrock_spawned.store(false, Ordering::Relaxed);
            }
        }
    }

    async fn finish_world_transfer(
        self: &Arc<Self>,
        new_world: &Arc<World>,
        position: Vector3<f64>,
        yaw: f32,
        pitch: f32,
        lifecycle: u64,
    ) {
        if !self.world_transfer_life_current(new_world, lifecycle) {
            return;
        }
        if new_world.dimension == pumpkin_data::dimension::Dimension::THE_NETHER {
            self.trigger_advancement(
                crate::entity::player::advancement::trigger::AdvancementTrigger::EnterDimension {
                    dimension: "the_nether".to_string(),
                },
            );
        } else if new_world.dimension == pumpkin_data::dimension::Dimension::THE_END {
            self.trigger_advancement(
                crate::entity::player::advancement::trigger::AdvancementTrigger::EnterDimension {
                    dimension: "the_end".to_string(),
                },
            );
        }

        if !self.world_transfer_life_current(new_world, lifecycle) {
            return;
        }
        self.send_permission_lvl_update();

        self.send_abilities_update();

        self.enqueue_set_held_item_packet(&CSetSelectedSlot::new(
            self.get_inventory().get_selected_slot() as i8,
        ));

        self.on_screen_handler_opened(&self.player_screen_handler);

        self.send_health();

        new_world.send_world_info(self);
        new_world.send_center_chunk(self).await;

        if !self.world_transfer_life_current(new_world, lifecycle) {
            return;
        }
        // cancellation leaves the current position untouched.
        let _ = self.request_teleport(position, yaw, pitch);
    }
    // PlayerList.respawn replaces ServerPlayer; an old transfer cannot finish a new life.
    fn world_transfer_life_current(&self, world: &World, lifecycle: u64) -> bool {
        let _owner = self.living_entity.own_damage();
        !self.living_entity.is_respawning()
            && !self.get_entity().is_removed()
            && !self.client.closed()
            && self.world().uuid == world.uuid
            && self.living_entity.damage_lifecycle() == lifecycle
    }
}
