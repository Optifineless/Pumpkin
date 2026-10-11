use std::{num::NonZero, sync::Arc};

use pumpkin_inventory::screen_handler::InventoryPlayer;
use pumpkin_util::math::{position::BlockPos, vector2::Vector2, vector3::Vector3};

use super::{
    CRespawn, CSetSelectedSlot, CUnloadChunk, ClientPlatform, Cylindrical, Dimension, Player,
    PlayerChangeWorldEvent, PlayerSpawnData, ResourceLocation, VarInt, biome,
};
use crate::{
    entity::{
        Entity, EntityBase,
        portal_travel::{PortalTravel, passengers::PortalPassengers},
    },
    world::{World, portal::TeleportTransition, transfer_admission::WorldAdmission},
};

impl Player {
    pub(super) async fn perform_world_transfer(
        self: &Arc<Self>,
        new_world: Arc<World>,
        position: Vector3<f64>,
        yaw: Option<f32>,
        pitch: Option<f32>,
        portal: Option<&PortalTravel>,
    ) -> Option<TeleportTransition> {
        if self.client.closed() || portal.is_some_and(|trip| !trip.is_valid()) {
            return None;
        }
        let current_world = self.world();
        let source_admission = current_world.admit_transfer()?;
        let server = new_world.server.upgrade()?;
        let mut event = PlayerChangeWorldEvent {
            player: self.clone(),
            previous_world: current_world.clone(),
            yaw: yaw.unwrap_or(new_world.level_info.load().spawn_yaw),
            pitch: pitch.unwrap_or(new_world.level_info.load().spawn_pitch),
            new_world,
            position,
            cancelled: false,
        };
        server.plugin_manager.fire(&server, &mut event).await;
        if event.cancelled {
            return None;
        }
        let admission = event.new_world.admit_transfer()?;
        #[cfg(test)]
        if portal.is_some() {
            self.pause_world_transfer_for_test().await;
        }
        let transition = TeleportTransition {
            new_world: event.new_world,
            position: event.position,
            yaw: Some(event.yaw),
            pitch: Some(event.pitch),
        };
        let last_pos = self.get_entity().last_pos.load();
        if !self.commit_world_transfer(&current_world, &transition, portal, source_admission) {
            return None;
        }
        drop(admission);
        // Publication committed already. An outgoing watcher callback may supersede only
        // the arrival position; the client still needs this committed dimension change.
        if self.client.closed() || !Arc::ptr_eq(&self.world(), &transition.new_world) {
            return None;
        }
        if !Arc::ptr_eq(&current_world, &transition.new_world) {
            self.send_dimension_change(&transition, last_pos);
        }
        if portal.is_some_and(|trip| !trip.is_current_in(&transition.new_world)) {
            return None;
        }
        self.send_permission_lvl_update();
        self.send_transfer_state(&transition.new_world);
        transition.new_world.send_world_info(self);
        transition.new_world.send_center_chunk(self).await;
        // ServerPlayer.teleport is synchronous in vanilla. ChunkSend callbacks can supersede
        // our yielded arrival, so validate the initiating life and trip after every await.
        if self.client.closed()
            || portal.is_some_and(|trip| !trip.is_current_in(&transition.new_world))
        {
            return None;
        }
        let position = self.accept_teleport_destination(transition.position)?;
        if portal.is_some_and(|trip| !trip.is_current_in(&transition.new_world)) {
            return None;
        }
        self.send_teleport(position, event.yaw, event.pitch);
        let mut changed =
            crate::plugin::api::events::player::player_changed_world::PlayerChangedWorldEvent {
                player: self.clone(),
                from_world: current_world,
                to_world: transition.new_world.clone(),
                cancelled: false,
            };
        server.plugin_manager.fire(&server, &mut changed).await;
        if portal.is_some_and(|trip| !trip.is_current_in(&transition.new_world)) {
            return None;
        }
        Some(TeleportTransition {
            position,
            ..transition
        })
    }

    #[cfg(test)]
    async fn pause_world_transfer_for_test(&self) {
        let gate = self
            .get_entity()
            .portal_travel
            .transfer_gate
            .lock()
            .unwrap()
            .take();
        if let Some((ready, resume)) = gate {
            let _ = ready.send(());
            let _ = resume.await;
        }
    }

    fn commit_world_transfer(
        self: &Arc<Self>,
        current_world: &Arc<World>,
        transition: &TeleportTransition,
        portal: Option<&PortalTravel>,
        source_admission: WorldAdmission,
    ) -> bool {
        // Reject before watcher teardown or client dimension publication. There is no await
        // from source removal through destination registration (ServerPlayer.teleport).
        if self.client.closed()
            || !Arc::ptr_eq(&self.world(), current_world)
            || portal.is_some_and(|trip| !trip.is_valid())
        {
            return false;
        }
        // Entity.teleportSameDimension retains membership and watches after event redirection.
        let cross_world = !Arc::ptr_eq(current_world, &transition.new_world);
        let passengers = cross_world.then(|| PortalPassengers::new(self.clone()));
        if self.client.closed()
            || portal.is_some_and(|trip| !trip.is_current())
            || passengers.as_ref().is_some_and(|riders| {
                !riders.is_current_in(current_world) || riders.has_new_riders()
            })
        {
            return false;
        }
        if cross_world {
            if current_world.detach_transferring_player(self).is_none() {
                return false;
            }
            if self.client.closed()
                || portal.is_some_and(|trip| !trip.is_current())
                || passengers.as_ref().is_some_and(|riders| {
                    !riders.is_current_in(current_world) || riders.has_new_riders()
                })
            {
                self.restore_rejected_source(current_world);
                return false;
            }
            self.set_client_loaded(false);
        }
        let entity = self.get_entity();
        let previous_yaw = entity.yaw.load();
        let previous_pitch = entity.pitch.load();
        if cross_world {
            self.change_world_chunks(&current_world.level, &transition.new_world);
            entity.set_portal_world(transition.new_world.clone());
        }
        entity.set_pos(transition.position);
        entity.set_rotation(
            transition.yaw.unwrap_or(previous_yaw),
            transition.pitch.unwrap_or(entity.pitch.load()),
        );
        entity.last_pos.store(transition.position);
        if cross_world {
            transition.new_world.players.rcu(|players| {
                let mut players = (**players).clone();
                players.push(self.clone());
                players
            });
        }
        // Entity.teleport moves the riding tree with the applied transition, before yielding.
        if let Some(passengers) = passengers {
            passengers.teleport(
                transition.position,
                transition.yaw.map(|yaw| yaw - previous_yaw),
                transition.pitch.map(|pitch| pitch - previous_pitch),
                &transition.new_world,
            );
        } else {
            Entity::teleport_passengers_recursive(
                entity,
                transition.position,
                transition.yaw.map(|yaw| yaw - previous_yaw),
                transition.pitch.map(|pitch| pitch - previous_pitch),
                &transition.new_world,
            );
        }
        if cross_world {
            self.retire_source_watchers(current_world, source_admission);
            transition.new_world.add_arriving_player(self);
        }
        true
    }

    fn restore_rejected_source(self: &Arc<Self>, world: &Arc<World>) {
        if self.client.closed()
            || self.get_entity().is_removed()
            || !Arc::ptr_eq(&self.world(), world)
        {
            return;
        }
        world.players.rcu(|players| {
            let mut players = (**players).clone();
            if !players.iter().any(|player| Arc::ptr_eq(player, self)) {
                players.push(self.clone());
            }
            players
        });
        world.add_arriving_player(self);
    }

    fn retire_source_watchers(&self, world: &Arc<World>, admission: WorldAdmission) {
        let chunks: Vec<_> = self.watched_section.load().all_chunks_within().collect();
        self.watched_section.store(Cylindrical::new(
            Vector2::new(0, 0),
            NonZero::new(1).unwrap_or(NonZero::<u8>::MIN),
        ));
        for chunk in &chunks {
            if let ClientPlatform::Java(client) = self.client.as_ref()
                && let Ok(data) = client.serialize_packet(&CUnloadChunk::new(chunk.x, chunk.y))
            {
                client.try_enqueue_packet(data);
            }
        }
        let world = world.clone();
        if let Some(server) = world.server.upgrade() {
            // Once publication commits, source cleanup belongs to the world, not the
            // cancellable connection arrival. Capture the old watches, never player state.
            server.spawn_task(async move {
                let cleaned = world.level.mark_chunks_as_not_watched(chunks).await;
                world.remove_entities_in_chunks(&cleaned).await;
                world.level.clean_entity_chunks(&cleaned);
                drop(admission);
            });
        }
    }

    fn send_dimension_change(&self, transition: &TeleportTransition, last_pos: Vector3<f64>) {
        let world = &transition.new_world;
        match self.client.as_ref() {
            ClientPlatform::Java(java) => {
                let packet = CRespawn::new(
                    PlayerSpawnData::new(
                        world.dimension.clone(),
                        biome::hash_seed(world.level.seed.0),
                        self.gamemode.load() as u8,
                        self.previous_gamemode
                            .load()
                            .unwrap_or(self.gamemode.load()) as i8,
                        false,
                        false,
                        Some((
                            ResourceLocation::from(world.dimension.minecraft_name),
                            BlockPos(Vector3::new(
                                last_pos.x.round() as i32,
                                last_pos.y.round() as i32,
                                last_pos.z.round() as i32,
                            )),
                        )),
                        VarInt(
                            self.get_entity()
                                .portal_cooldown
                                .load(std::sync::atomic::Ordering::Relaxed)
                                as i32,
                        ),
                        world.sea_level.into(),
                    ),
                    CRespawn::KEEP_ALL_DATA,
                );
                // FIFO enqueue is synchronous: no rejected transition is exposed to the client.
                if let Ok(data) = java.serialize_packet(&packet) {
                    java.try_enqueue_packet(data);
                }
            }
            ClientPlatform::Bedrock(bedrock) => {
                let dimension = if world.dimension == Dimension::THE_NETHER {
                    1
                } else if world.dimension == Dimension::THE_END {
                    2
                } else {
                    0
                };
                let pos = transition.position;
                let packet = pumpkin_protocol::bedrock::client::CChangeDimension {
                    dimension_id: dimension.into(),
                    position: Vector3::new(pos.x as f32, pos.y as f32, pos.z as f32),
                    respawn: false,
                    loading_screen_id: None,
                };
                if let Ok(data) = bedrock.serialize_packet(&packet) {
                    bedrock.try_enqueue_packet(data);
                }
                self.bedrock_spawned
                    .store(false, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }

    fn send_transfer_state(&self, world: &World) {
        if world.dimension == Dimension::THE_NETHER || world.dimension == Dimension::THE_END {
            self.trigger_advancement(
                super::advancement::trigger::AdvancementTrigger::EnterDimension {
                    dimension: if world.dimension == Dimension::THE_NETHER {
                        "the_nether"
                    } else {
                        "the_end"
                    }
                    .to_owned(),
                },
            );
        }
        self.send_abilities_update();
        self.enqueue_set_held_item_packet(&CSetSelectedSlot::new(
            self.get_inventory().get_selected_slot() as i8,
        ));
        self.on_screen_handler_opened(&self.player_screen_handler);
        self.send_health();
    }
}
