//! MinecartItem.useOn's initial AbstractMinecart.createMinecart rail adjustment.

use super::MinecartEntity;
use crate::{
    entity::{Entity, EntityBase, player::Player},
    item::{item_utils, items::minecart::MinecartItem},
    net::java::play::hand_use_result::hand_slot,
    server::Server,
    world::World,
};
use pumpkin_data::{
    Block, BlockStateId,
    block_properties::{
        BlockProperties, EnumVariants, PoweredRailLikeProperties, RailLikeProperties, RailShape,
    },
    item_stack::ItemStack,
    tag::{self, Taggable},
};
use pumpkin_util::{
    GameMode, Hand,
    math::{boundingbox::BoundingBox, position::BlockPos, vector3::Vector3},
};
use std::sync::{Arc, Mutex, Weak};

impl MinecartEntity {
    pub(crate) fn place_from_item(
        item: &ItemStack,
        player: &Player,
        location: BlockPos,
        block: &Block,
        server: &Server,
        hand: Hand,
        source: usize,
    ) -> bool {
        let world = player.world();
        let state = world.get_block_state_id(&location);
        if Block::from_state_id(state) != block || !block.has_tag(&tag::Block::MINECRAFT_RAILS) {
            return false;
        }
        let Some(terrain) = world
            .level
            .loaded_chunks
            .get(&location.chunk_position())
            .map(|c| c.clone())
        else {
            return false;
        };
        let shape = rail_shape(state);
        let center = location.to_f64();
        let position = Vector3::new(
            center.x,
            center.y + super::RAIL_HEIGHT_OFFSET + if shape.is_ascending() { 0.5 } else { 0.0 },
            center.z,
        );
        let cart = Self::new(Entity::new(
            world.clone(),
            position,
            MinecartItem::item_to_entity(item.item),
        ));
        let experimental = Self::uses_new_behavior(server);
        let (aligned_pos, aligned_state) = if experimental {
            cart.adjust_initial_rails(&world)
        } else {
            (location, state)
        };
        let cart: Arc<dyn EntityBase> = Arc::new(cart);
        let _reservation = if experimental {
            let Some(reservation) = CreationReservation::reserve(&world, cart.get_entity()) else {
                return false;
            };
            Some(reservation)
        } else {
            None
        };
        let clear = || {
            !experimental
                || !world
                    .get_entities_at_box(&cart.get_entity().bounding_box.load())
                    .iter()
                    .any(|other| other.cast_any().is::<Self>())
        };
        if !clear() {
            return false;
        }
        let mode = player.gamemode.load();
        let life = player.living_entity.damage_lifecycle();
        let mut receipt = None;
        // Spawn callbacks run without hand/terrain guards. Charge only the revalidated source.
        let spawned = world.admit_spawn_tree(&cart, true, || {
            if !Arc::ptr_eq(&world, &player.world())
                || !world
                    .players
                    .load()
                    .iter()
                    .any(|live| std::ptr::eq(player, live.as_ref()))
                || player.client.closed()
                || !player.has_client_loaded()
                || !player.living_entity.is_alive()
                || player.living_entity.damage_lifecycle() != life
                || player.get_entity().is_removed()
                || player.gamemode.load() != mode
                || mode == GameMode::Spectator
                || hand_slot(player, hand) != source
                || !player.can_interact_with_block_at(&location, 1.0)
                || Self::uses_new_behavior(server) != experimental
                || !world
                    .level
                    .loaded_chunks
                    .get(&location.chunk_position())
                    .is_some_and(|live| Arc::ptr_eq(&terrain, &live))
                || world.get_block_state_id(&location) != state
                || world.get_block_state_id(&aligned_pos) != aligned_state
                || !clear()
            {
                return false;
            }
            receipt = claim_item_source(player, source, item, mode);
            receipt.is_some()
        });
        if !spawned && let Some(receipt) = receipt {
            item_utils::give_or_drop(player, receipt);
        }
        spawned
    }

    // AbstractMinecart.getCurrentBlockPosOrRailBelow during experimental creation.
    fn adjust_initial_rails(&self, world: &World) -> (BlockPos, BlockStateId) {
        let position = self.vehicle.entity.pos.load();
        let mut rail = BlockPos::new(
            position.x.floor() as i32,
            position.y.floor() as i32,
            position.z.floor() as i32,
        );
        let below = BlockPos::new(
            rail.0.x,
            (position.y - 0.1 - f64::from(1.0e-5f32)).floor() as i32,
            rail.0.z,
        );
        if world
            .get_block(&below)
            .has_tag(&tag::Block::MINECRAFT_RAILS)
        {
            rail = below;
        }
        let state = world.get_block_state_id(&rail);
        if Block::from_state_id(state).has_tag(&tag::Block::MINECRAFT_RAILS) {
            self.adjust_to_rails(rail, rail_shape(state));
        }
        (rail, state)
    }

    // NewMinecartBehavior.adjustToRails, for a fresh, unflipped cart with zero delta movement.
    fn adjust_to_rails(&self, position: BlockPos, shape: RailShape) {
        let (exit0, exit1) = creation_exits(shape);
        let exit0 = exit0 * 0.5;
        let exit1 = exit1 * 0.5;
        let mut out_direction = Vector3::new(exit0.x, 0.0, exit0.z);
        let mut in_direction = Vector3::new(exit1.x, 0.0, exit1.z);
        if is_descending(in_direction, shape) {
            std::mem::swap(&mut out_direction, &mut in_direction);
        }
        let entity = &self.vehicle.entity;
        let previous = entity.pos.load();
        let base = position.to_f64();
        let mut yaw = 180.0 - out_direction.z.atan2(out_direction.x).to_degrees() as f32;
        let mut target = if exit0.x != exit1.x && exit0.z != exit1.z {
            let between = exit1 - exit0;
            let from_exit = previous - base - exit0;
            let travel = between * (between.dot(&from_exit) / between.dot(&between));
            yaw = 180.0 - travel.z.atan2(travel.x).to_degrees() as f32;
            base + exit0 + travel
        } else {
            let z_snap = exit0.x != exit1.x;
            let x_snap = exit0.z != exit1.z;
            Vector3::new(
                if x_snap { base.x } else { previous.x },
                base.y,
                if z_snap { base.z } else { previous.z },
            )
        };
        let in_hill = exit0.y != exit1.y;
        let pitch = if in_hill {
            target.y += ((base + in_direction) - target).length() + 0.1;
            -45.0
        } else {
            target.y += 0.1;
            0.0
        };
        entity.set_pos(target);
        entity.set_rotation(yaw, pitch);
    }
}

// UseOnContext retains the originating source cell even if another packet selects a new slot.
fn claim_item_source(
    player: &Player,
    source: usize,
    expected: &ItemStack,
    mode: GameMode,
) -> Option<ItemStack> {
    if expected.is_empty() {
        return None;
    }
    let mut replacement = expected.clone();
    replacement.decrement_unless_creative(mode, 1);
    player
        .inventory()
        .replace_stack_if_unchanged(source, expected, replacement)
        .then(|| {
            if mode == GameMode::Creative {
                ItemStack::EMPTY.clone()
            } else {
                expected.copy_with_count(1)
            }
        })
}

// AbstractMinecart.EXITS. Legacy movement's exits use a different slope origin/order.
const fn creation_exits(shape: RailShape) -> (Vector3<f64>, Vector3<f64>) {
    use RailShape::{
        AscendingEast, AscendingNorth, AscendingSouth, AscendingWest, EastWest, NorthEast,
        NorthSouth, NorthWest, SouthEast, SouthWest,
    };
    match shape {
        NorthSouth => (Vector3::new(0.0, 0.0, -1.0), Vector3::new(0.0, 0.0, 1.0)),
        EastWest => (Vector3::new(-1.0, 0.0, 0.0), Vector3::new(1.0, 0.0, 0.0)),
        AscendingEast => (Vector3::new(-1.0, -1.0, 0.0), Vector3::new(1.0, 0.0, 0.0)),
        AscendingWest => (Vector3::new(-1.0, 0.0, 0.0), Vector3::new(1.0, -1.0, 0.0)),
        AscendingNorth => (Vector3::new(0.0, 0.0, -1.0), Vector3::new(0.0, -1.0, 1.0)),
        AscendingSouth => (Vector3::new(0.0, -1.0, -1.0), Vector3::new(0.0, 0.0, 1.0)),
        SouthEast => (Vector3::new(0.0, 0.0, 1.0), Vector3::new(1.0, 0.0, 0.0)),
        SouthWest => (Vector3::new(0.0, 0.0, 1.0), Vector3::new(-1.0, 0.0, 0.0)),
        NorthWest => (Vector3::new(0.0, 0.0, -1.0), Vector3::new(-1.0, 0.0, 0.0)),
        NorthEast => (Vector3::new(0.0, 0.0, -1.0), Vector3::new(1.0, 0.0, 0.0)),
    }
}

// NewMinecartBehavior.isDecending (spelling in vanilla).
const fn is_descending(movement: Vector3<f64>, shape: RailShape) -> bool {
    match shape {
        RailShape::AscendingEast => movement.x < 0.0,
        RailShape::AscendingWest => movement.x > 0.0,
        RailShape::AscendingNorth => movement.z > 0.0,
        RailShape::AscendingSouth => movement.z < 0.0,
        _ => false,
    }
}

fn rail_shape(state: BlockStateId) -> RailShape {
    if PoweredRailLikeProperties::handles_block_id(Block::from_state_id(state).id) {
        RailShape::from_value(
            PoweredRailLikeProperties::from_state_id(state)
                .shape
                .to_value(),
        )
    } else {
        RailLikeProperties::from_state_id(state).shape
    }
}

struct ReservedCart {
    world: Weak<World>,
    uuid: uuid::Uuid,
    bounds: BoundingBox,
}

// In-flight item placements reserve geometry, like spawn UUIDs, without owning a guard.
static RESERVED_CARTS: Mutex<Vec<ReservedCart>> = Mutex::new(Vec::new());
struct CreationReservation(uuid::Uuid);

impl CreationReservation {
    fn reserve(world: &Arc<World>, cart: &Entity) -> Option<Self> {
        let mut reserved = RESERVED_CARTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let world = Arc::downgrade(world);
        let bounds = cart.bounding_box.load();
        if reserved
            .iter()
            .any(|other| Weak::ptr_eq(&world, &other.world) && bounds.intersects(&other.bounds))
        {
            return None;
        }
        reserved.push(ReservedCart {
            world,
            uuid: cart.entity_uuid,
            bounds,
        });
        Some(Self(cart.entity_uuid))
    }
}
impl Drop for CreationReservation {
    fn drop(&mut self) {
        RESERVED_CARTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|other| other.uuid != self.0);
    }
}
