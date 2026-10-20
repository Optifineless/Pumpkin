//! `Entity.teleportCrossDimension` for boat portal trees, with synchronous publication.

use std::sync::{Arc, PoisonError, atomic::Ordering::Relaxed};

use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_util::math::{pack_degrees, vector3::Vector3};
use pumpkin_world::chunk::ChunkEntityData;
use rustc_hash::FxHashSet;

use super::{World, portal::TeleportTransition};
use crate::entity::{Entity, EntityBase, RemovalReason, player::Player, spawn_mount};

struct Traveler {
    source: Arc<dyn EntityBase>,
    destination: Arc<dyn EntityBase>,
    player: Option<Arc<Player>>,
    parent: Option<usize>,
    position: Vector3<f64>,
    yaw: f32,
    pitch: f32,
}

fn prepare_tree(
    source_world: &World,
    root: &Arc<dyn EntityBase>,
    transition: &TeleportTransition,
) -> Option<Vec<Traveler>> {
    let origin = root.get_entity().pos.load();
    let yaw_delta = transition.yaw.map(|yaw| yaw - root.get_entity().yaw.load());
    let mut pending = vec![(root.clone(), None)];
    let mut travelers = Vec::new();
    let mut seen = FxHashSet::default();
    while let Some((source, parent)) = pending.pop() {
        let base = source.get_entity();
        if !seen.insert(base.entity_uuid) {
            return None;
        }
        let index = travelers.len();
        pending.extend(
            base.passengers
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .rev()
                .cloned()
                .map(|passenger| (passenger, Some(index))),
        );
        let player = if source.get_player().is_some() {
            Some(source_world.get_player_by_uuid(base.entity_uuid)?)
        } else {
            None
        };
        let position = transition.position + (base.pos.load() - origin);
        let yaw = yaw_delta.map_or_else(|| base.yaw.load(), |delta| base.yaw.load() + delta);
        let pitch = transition.pitch.unwrap_or_else(|| base.pitch.load());
        let destination = if parent.is_some() {
            // Retain canonical passenger state and container ownership. Restoring an NBT
            // copy would leave stale native inventory references able to spend old stock.
            source.clone()
        } else {
            let mut nbt = NbtCompound::new();
            source.write_nbt(&mut nbt);
            crate::entity::mob::spawn::load_spawn_entity(&transition.new_world, &nbt, position)?
        };
        travelers.push(Traveler {
            source,
            destination,
            player,
            parent,
            position,
            yaw,
            pitch,
        });
    }
    Some(travelers)
}

fn position_traveler(base: &Entity, traveler: &Traveler) {
    base.set_pos(traveler.position);
    base.set_rotation(traveler.yaw, traveler.pitch);
    base.last_pos.store(traveler.position);
    base.last_sent_pos.store(traveler.position);
    let yaw = pack_degrees(base.yaw.load());
    base.last_sent_yaw.store(yaw, Relaxed);
    base.last_sent_head_yaw.store(yaw, Relaxed);
    base.last_sent_pitch
        .store(pack_degrees(base.pitch.load()), Relaxed);
    base.last_sent_velocity.store(base.velocity.load());
    base.portal_cooldown
        .store(base.default_portal_cooldown(), Relaxed);
}

fn claim_travelers(world: &World, travelers: &[Traveler], bases: &[&Entity]) -> Vec<bool> {
    let mut accepted = Vec::with_capacity(travelers.len());
    for (index, node) in travelers.iter().enumerate() {
        let base = bases[index];
        let linked = node.parent.map_or_else(
            || base.get_vehicle().is_none(),
            |parent| {
                accepted[parent]
                    && base
                        .get_vehicle()
                        .is_some_and(|vehicle| Arc::ptr_eq(&vehicle, &travelers[parent].source))
            },
        );
        let registered = node.player.as_ref().map_or_else(
            || {
                world
                    .entities
                    .load()
                    .iter()
                    .any(|current| Arc::ptr_eq(current, &node.source))
            },
            |player| {
                world
                    .players
                    .load()
                    .iter()
                    .any(|current| Arc::ptr_eq(current, player))
            },
        );
        let current = base.is_alive()
            && std::ptr::eq(base.world.load().as_ref(), world)
            && linked
            && registered;
        let claimed =
            current && (index != 0 || base.mark_removed_only(RemovalReason::ChangedDimension));
        accepted.push(claimed);
        if index == 0 && !claimed {
            break;
        }
    }
    accepted
}

fn unlink_travelers(travelers: &[Traveler], bases: &[&Entity], accepted: &[bool]) {
    for (index, node) in travelers.iter().enumerate() {
        let Some(parent) = node.parent else {
            continue;
        };
        if !(accepted.get(index).copied().unwrap_or(false)
            || accepted.get(parent).copied().unwrap_or(false) && travelers[parent].player.is_none())
        {
            continue;
        }
        {
            let mut vehicle = bases[index]
                .vehicle
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if vehicle
                .as_ref()
                .is_some_and(|vehicle| Arc::ptr_eq(vehicle, &travelers[parent].source))
            {
                *vehicle = None;
            }
        }
        bases[parent]
            .passengers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|passenger| !Arc::ptr_eq(passenger, &node.source));
    }
}

impl World {
    pub(crate) fn transfer_boat_portal(
        self: &Arc<Self>,
        root: &Arc<dyn EntityBase>,
        transition: &TeleportTransition,
        storage: &ChunkEntityData,
    ) -> bool {
        let destination = &transition.new_world;
        if Arc::ptr_eq(self, destination) {
            return false;
        }
        let Some(travelers) = prepare_tree(self, root, transition) else {
            return false;
        };
        let candidates: Vec<_> = travelers
            .iter()
            .filter(|node| node.player.is_none())
            .map(|node| node.destination.clone())
            .collect();
        if !storage.live.load(Relaxed) {
            let ids: FxHashSet<_> = candidates
                .iter()
                .map(|entity| entity.get_entity().entity_uuid)
                .collect();
            let records = storage.data.lock().unwrap_or_else(PoisonError::into_inner);
            if records
                .iter()
                .any(|record| saved_uuid_conflict(record, &ids))
            {
                return false;
            }
        }
        let Some(_reservation) = destination.reserve_spawn_uuids(&candidates) else {
            return false;
        };
        let accepted = self.commit_boat_portal(destination, &travelers);
        if !accepted.first().copied().unwrap_or(false) {
            return false;
        }
        self.publish_boat_portal(destination, travelers, accepted);
        true
    }

    fn commit_boat_portal(&self, destination: &Arc<Self>, travelers: &[Traveler]) -> Vec<bool> {
        let bases: Vec<_> = travelers
            .iter()
            .map(|node| node.source.get_entity())
            .collect();
        let targets: Vec<_> = travelers
            .iter()
            .map(|node| node.destination.get_entity())
            .collect();
        let Some(source_mutation) = self.reserve_block_entity_mutation(bases[0].block_pos.load())
        else {
            return Vec::new();
        };
        let Some(destination_mutation) =
            destination.reserve_block_entity_mutation(targets[0].block_pos.load())
        else {
            return Vec::new();
        };
        let source_activation = self.begin_entity_activation();
        let destination_activation = destination.begin_entity_activation();
        let accepted = claim_travelers(self, travelers, &bases);
        if accepted.first().copied().unwrap_or(false) {
            unlink_travelers(travelers, &bases, &accepted);
            for (index, node) in travelers.iter().enumerate() {
                if accepted[index] && node.player.is_none() {
                    targets[index].set_world(destination.clone());
                    position_traveler(targets[index], node);
                    if let Some(parent) = node.parent
                        && travelers[parent].player.is_none()
                    {
                        spawn_mount::attach_unpublished(
                            &travelers[parent].destination,
                            node.destination.clone(),
                        );
                    }
                }
            }
            let moved: Vec<_> = travelers
                .iter()
                .zip(&accepted)
                .filter(|(node, accepted)| **accepted && node.player.is_none())
                .collect();
            destination.entities.rcu(|current| {
                let mut next = (**current).clone();
                next.extend(moved.iter().map(|(node, _)| node.destination.clone()));
                next
            });
            self.entities.rcu(|current| {
                current
                    .iter()
                    .filter(|entity| {
                        !moved
                            .iter()
                            .any(|(node, _)| Arc::ptr_eq(entity, &node.source))
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            });
        }
        drop(destination_activation);
        drop(source_activation);
        drop(destination_mutation);
        drop(source_mutation);
        accepted
    }

    fn publish_boat_portal(
        &self,
        destination: &Arc<Self>,
        travelers: Vec<Traveler>,
        accepted: Vec<bool>,
    ) {
        for (index, (node, accepted)) in travelers.iter().zip(&accepted).enumerate() {
            if !*accepted || node.player.is_some() {
                continue;
            }
            if index == 0 {
                self.finish_entity_removal(node.source.as_ref());
            } else {
                self.spawn_state
                    .load()
                    .remove_entity(self, node.source.as_ref());
                self.entity_tracker
                    .remove_entity(node.source.as_ref(), self);
            }
            if same_destination(&node.destination, destination) {
                node.destination.init_data_tracker();
            }
            if same_destination(&node.destination, destination) {
                destination
                    .spawn_state
                    .load()
                    .add_entity(destination, node.destination.as_ref());
                destination
                    .entity_tracker
                    .add_entity(&node.destination, destination);
                if same_destination(&node.destination, destination) {
                    spawn_mount::publish_passengers(&node.destination);
                } else {
                    destination
                        .entity_tracker
                        .remove_entity(node.destination.as_ref(), destination);
                }
            }
        }
        let Some(server) = destination.server.upgrade() else {
            return;
        };
        let source = travelers[0].source.get_entity().world.load_full();
        let destination = destination.clone();
        // Player world-change hooks can cancel or redirect independently. Old links
        // were detached at the leaf; remount only a current, unmounted arrival.
        server.spawn_task(async move {
            for (node, accepted) in travelers.iter().zip(&accepted) {
                if *accepted
                    && let Some(player) = &node.player
                    && Arc::ptr_eq(&player.world(), &source)
                    && !player.get_entity().has_vehicle()
                {
                    let base = player.get_entity();
                    base.portal_cooldown
                        .store(base.default_portal_cooldown(), Relaxed);
                    player
                        .teleport_boat_passenger(
                            destination.clone(),
                            node.position,
                            node.yaw,
                            node.pitch,
                        )
                        .await;
                    let base = player.get_entity();
                    if let Some(vehicle) = base.get_vehicle()
                        && !Arc::ptr_eq(&vehicle.get_entity().world.load_full(), &player.world())
                    {
                        vehicle
                            .get_entity()
                            .remove_passenger_on_disconnect(base.entity_id);
                    }
                }
            }
            for (node, accepted) in travelers.iter().zip(&accepted) {
                if *accepted
                    && let Some(parent) = node.parent
                    && (node.player.is_some() || travelers[parent].player.is_some())
                    && !node.destination.get_entity().has_vehicle()
                    && same_destination(&node.destination, &destination)
                    && same_destination(&travelers[parent].destination, &destination)
                {
                    spawn_mount::attach_unpublished(
                        &travelers[parent].destination,
                        node.destination.clone(),
                    );
                    spawn_mount::publish_passengers(&travelers[parent].destination);
                }
            }
        });
    }
}

fn saved_uuid_conflict(nbt: &NbtCompound, ids: &FxHashSet<uuid::Uuid>) -> bool {
    nbt.get_uuid("UUID").is_some_and(|uuid| ids.contains(&uuid))
        || nbt.get_list("Passengers").is_some_and(|passengers| {
            passengers
                .iter()
                .filter_map(NbtTag::extract_compound)
                .any(|child| saved_uuid_conflict(child, ids))
        })
}

fn same_destination(entity: &Arc<dyn EntityBase>, world: &Arc<World>) -> bool {
    let base = entity.get_entity();
    base.is_alive()
        && !world.level.cancel_token.is_cancelled()
        && Arc::ptr_eq(&base.world.load_full(), world)
        && if entity.get_player().is_some() {
            world
                .get_player_by_uuid(base.entity_uuid)
                .is_some_and(|current| std::ptr::eq(current.get_entity(), base))
        } else {
            world
                .get_entity_by_uuid(base.entity_uuid)
                .is_some_and(|current| Arc::ptr_eq(&current, entity))
        }
}
