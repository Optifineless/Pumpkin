use crate::command::CommandSource;
use crate::command::errors::{
    command_syntax_error::CommandSyntaxError, error_types::CommandErrorType,
};
use crate::entity::living::LivingEntity;
use crate::entity::living::waypoint_icon::LocatorBarIcon;
use crate::entity::player::Player;
use crate::entity::{Entity, EntityBase};
use pumpkin_data::attributes::Attributes;
use std::sync::Arc;

#[path = "waypoint_icon_packet.rs"]
mod packet;
use packet::IconRefresh;

const INVALID_WAYPOINT: CommandErrorType<0> =
    CommandErrorType::new("argument.waypoint.invalid", "argument.waypoint.invalid");

pub(super) fn mutate_icon(
    source: &CommandSource,
    waypoint: &Arc<dyn EntityBase>,
    change: impl FnOnce(&mut LocatorBarIcon),
) -> Result<(), CommandSyntaxError> {
    // WaypointCommand.mutateIcon untracks the old icon, mutates it, then tracks it again.
    let living = waypoint
        .get_living_entity()
        .ok_or(INVALID_WAYPOINT.create_without_context())?;
    let entity = waypoint.get_entity();
    let world = entity.world.load();
    // Until ServerWaypointManager exists, retain the original command's sender-only audience.
    let viewer = source.as_player().filter(|viewer| {
        !viewer.client.closed()
            && world
                .players
                .load()
                .iter()
                .any(|player| Arc::ptr_eq(player, viewer))
    });
    if let Some(viewer) = &viewer {
        viewer.try_send_client_packet(&IconRefresh::remove(entity.entity_uuid));
    }
    let icon = living.waypoint_icon.mutate(change);
    if let Some(viewer) = viewer
        && entity.entity_uuid != viewer.get_entity().entity_uuid
        && world.level_info.load().game_rules.locator_bar
        && !does_source_ignore_receiver(waypoint.as_ref(), living, &viewer)
    {
        let icon = icon.clone_and_assign_style(waypoint.as_ref());
        viewer.try_send_client_packet(&IconRefresh::add(
            entity.entity_uuid,
            &icon,
            entity.block_pos.load(),
        ));
    }
    Ok(())
}

fn does_source_ignore_receiver(
    waypoint: &dyn EntityBase,
    living: &LivingEntity,
    viewer: &Player,
) -> bool {
    // WaypointTransmitter.doesSourceIgnoreReceiver exempts spectator receivers.
    if viewer.is_spectator() {
        return false;
    }
    if waypoint.is_spectator() || has_indirect_passenger(waypoint.get_entity(), viewer) {
        return true;
    }
    let broadcast_range = living
        .get_attribute_value(&Attributes::WAYPOINT_TRANSMIT_RANGE)
        .min(
            viewer
                .living_entity
                .get_attribute_value(&Attributes::WAYPOINT_RECEIVE_RANGE),
        );
    f64::from(distance_to(&living.entity, viewer.get_entity())) >= broadcast_range
}

fn distance_to(entity: &Entity, other: &Entity) -> f32 {
    // Entity.distanceTo casts each delta to float; Mth.sqrt then casts the double square root.
    let delta = (entity.pos.load() - other.pos.load()).to_f32_lossy();
    f64::from(delta.length_squared()).sqrt() as f32
}

fn has_indirect_passenger(entity: &Entity, viewer: &Player) -> bool {
    // Entity.hasIndirectPassenger follows the passenger's vehicle chain.
    let mut vehicle = viewer.get_entity().get_vehicle();
    while let Some(ridden) = vehicle {
        if ridden.get_entity().entity_uuid == entity.entity_uuid {
            return true;
        }
        vehicle = ridden.get_entity().get_vehicle();
    }
    false
}
