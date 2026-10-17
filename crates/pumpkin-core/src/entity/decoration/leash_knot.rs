use crate::entity::player::Player;
use crate::entity::{Entity, EntityBase, living::LivingEntity};
use crate::world::World;
use pumpkin_data::entity::EntityType;
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::Taggable;
use pumpkin_util::math::boundingbox::{BoundingBox, EntityDimensions};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

const LEASH_SEARCH_SIZE: f32 = 32.0;

pub struct LeashKnotEntity {
    entity: Entity,
    pos: BlockPos,
    remove_when_empty: AtomicBool,
}

impl LeashKnotEntity {
    pub const OFFSET_Y: f64 = 0.375;

    pub const fn new(entity: Entity, pos: BlockPos) -> Self {
        Self {
            entity,
            pos,
            remove_when_empty: AtomicBool::new(false),
        }
    }

    pub const fn block_pos(&self) -> BlockPos {
        self.pos
    }

    pub fn get_or_create(world: &Arc<World>, pos: BlockPos) -> Arc<dyn EntityBase> {
        if let Some(existing) = Self::get_knot(world, pos) {
            return existing;
        }
        Self::create_knot(world, pos) as Arc<dyn EntityBase>
    }

    pub fn get_knot(world: &Arc<World>, pos: BlockPos) -> Option<Arc<dyn EntityBase>> {
        let center = Vector3::new(
            f64::from(pos.0.x) + 0.5,
            f64::from(pos.0.y) + Self::OFFSET_Y,
            f64::from(pos.0.z) + 0.5,
        );

        let search_dim = EntityDimensions {
            width: 2.0,
            height: 2.0,
            eye_height: 1.0,
        };

        let search_box = BoundingBox::new_from_pos(center.x, center.y, center.z, &search_dim);
        let entities = world.get_entities_at_box(&search_box);

        for entity_base in entities {
            if entity_base.get_entity().entity_type == &EntityType::LEASH_KNOT
                // LeashFenceKnotEntity.getKnot checks the entity class, then returns that entity.
                && let Some(knot) = entity_base.cast_any().downcast_ref::<Self>()
                && knot.pos == pos
            {
                return Some(entity_base);
            }
        }
        None
    }

    pub fn create_knot(world: &Arc<World>, pos: BlockPos) -> Arc<Self> {
        let raw_pos = Vector3::new(
            f64::from(pos.0.x) + 0.5,
            f64::from(pos.0.y) + Self::OFFSET_Y,
            f64::from(pos.0.z) + 0.5,
        );

        let entity = Entity::new(world.clone(), raw_pos, &EntityType::LEASH_KNOT);
        let knot = Arc::new(Self::new(entity, pos));
        world.spawn_entity(knot.clone() as Arc<dyn EntityBase>);

        world.play_sound(Sound::ItemLeadTied, SoundCategory::Neutral, &raw_pos);

        knot
    }

    pub fn play_placement_sound(&self, world: &World) {
        let pos = self.entity.pos.load();
        world.play_sound(Sound::ItemLeadTied, SoundCategory::Neutral, &pos);
    }

    fn has_live_leashed_mobs(&self, world: &World) -> bool {
        // Leashable.leashableLeashedTo searches a 32-block box centered on the holder bounds.
        let bounds = self.entity.bounding_box.load();
        let center = (bounds.min + bounds.max) * 0.5;
        let half_size = f64::from(LEASH_SEARCH_SIZE) / 2.0;
        let radius = Vector3::new(half_size, half_size, half_size);
        let search_box = BoundingBox::new(center - radius, center + radius);
        let knot_id = self.entity.entity_id;
        world
            .get_entities_at_box(&search_box)
            .iter()
            .map(|entity_base| entity_base.get_entity())
            .any(|entity| !entity.is_removed() && leash_holder_id(entity) == Some(knot_id))
    }
}

use crate::server::Server;

impl EntityBase for LeashKnotEntity {
    fn get_entity(&self) -> &Entity {
        &self.entity
    }

    fn get_living_entity(&self) -> Option<&LivingEntity> {
        None
    }

    fn can_hit(&self) -> bool {
        !self.entity.is_removed()
    }

    fn tick(&self, _caller: &dyn EntityBase, _server: &Server) {
        let world = self.entity.world.load();
        let block = world.get_block(&self.pos);
        if !block.has_tag(&pumpkin_data::tag::Block::MINECRAFT_FENCES) {
            let knot_id = self.entity.entity_id;
            let search_dim = EntityDimensions {
                width: 32.0,
                height: 32.0,
                eye_height: 16.0,
            };
            let pos = self.entity.pos.load();
            let search_box = BoundingBox::new_from_pos(pos.x, pos.y, pos.z, &search_dim);
            let entities = world.get_entities_at_box(&search_box);

            for entity_base in entities {
                let ent = entity_base.get_entity();
                let is_attached_to_knot = ent
                    .leashed_to
                    .try_lock()
                    .ok()
                    .and_then(|guard| {
                        guard
                            .as_ref()
                            .map(|holder| holder.get_entity().entity_id == knot_id)
                    })
                    .unwrap_or(false);

                if is_attached_to_knot {
                    ent.unleash();
                    let lead_item = pumpkin_data::item_stack::ItemStack::new(
                        1,
                        &pumpkin_data::item::Item::LEAD,
                    );
                    world.drop_stack(&ent.block_pos.load(), lead_item);
                }
            }

            world.play_sound(Sound::ItemLeadUntied, SoundCategory::Neutral, &pos);
            self.entity.remove();
        } else if self.remove_when_empty.swap(false, Ordering::Relaxed)
            && !self.has_live_leashed_mobs(&world)
        {
            // LeashFenceKnotEntity.notifyLeasheeRemoved discards an empty knot. Run this after
            // parallel player packet handling so another packet cannot attach to a removed knot.
            self.entity.remove();
        }
    }

    // LeashFenceKnotEntity.interact transfers eligible leashes, then removes an empty knot.
    fn interact(&self, player: &Arc<Player>, _item_stack: &mut ItemStack) -> bool {
        let world = player.world();
        let knot_id = self.entity.entity_id;
        let player_id = player.entity_id();

        let search_dim = EntityDimensions {
            width: LEASH_SEARCH_SIZE,
            height: LEASH_SEARCH_SIZE,
            eye_height: LEASH_SEARCH_SIZE / 2.0,
        };
        let pos = self.entity.pos.load();
        let search_box = BoundingBox::new_from_pos(pos.x, pos.y, pos.z, &search_dim);
        let entities = world.get_entities_at_box(&search_box);

        // Leashable.leashableLeashedTo plus canHaveALeashAttachedTo.
        let player_leashed_mobs: Vec<_> = entities
            .iter()
            .map(|entity_base| entity_base.get_entity())
            .filter(|ent| {
                leash_holder_id(ent) == Some(player_id)
                    && can_have_a_leash_attached_to(ent, &self.entity)
            })
            .collect();

        let mut attached_mob = false;

        if let Some(self_knot) = Self::get_knot(&world, self.pos) {
            for mob in player_leashed_mobs {
                mob.leash_to(self_knot.clone());
                attached_mob |= leash_holder_id(mob) == Some(self_knot.get_entity().entity_id);
            }
        }

        let mut any_dropped = false;
        if !attached_mob && !player.get_entity().is_sneaking() {
            let knot_leashed_mobs: Vec<_> = entities
                .iter()
                .map(|entity_base| entity_base.get_entity())
                .filter(|ent| {
                    leash_holder_id(ent) == Some(knot_id)
                        && can_have_a_leash_attached_to(ent, player.get_entity())
                })
                .collect();

            for mob in knot_leashed_mobs {
                mob.leash_to(player.clone() as Arc<dyn EntityBase>);
                any_dropped |= leash_holder_id(mob) == Some(player_id);
            }
        }

        if attached_mob || any_dropped {
            world.emit_game_event(GameEvent::BlockAttach.name(), pos);
            self.play_placement_sound(&world);
            if any_dropped {
                self.remove_when_empty.store(true, Ordering::Relaxed);
            }
            true
        } else {
            false
        }
    }

    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }
}

fn leash_holder_id(entity: &Entity) -> Option<i32> {
    // Snapshot before leash changes can fire an event or send packets.
    // Block on contention so the vanilla transfer cannot silently skip this leash.
    let holder = entity
        .leashed_to
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    holder.map(|holder| holder.get_entity().entity_id)
}

fn can_have_a_leash_attached_to(entity: &Entity, holder: &Entity) -> bool {
    if entity.entity_id == holder.entity_id {
        return false;
    }

    let entity_bounds = entity.bounding_box.load();
    let holder_bounds = holder.bounding_box.load();
    let difference = (entity_bounds.min + entity_bounds.max) * 0.5
        - (holder_bounds.min + holder_bounds.max) * 0.5;
    difference.length_squared() <= Entity::LEASH_SNAP_DISTANCE * Entity::LEASH_SNAP_DISTANCE
}

#[cfg(test)]
#[path = "leash_knot_tests.rs"]
mod tests;
