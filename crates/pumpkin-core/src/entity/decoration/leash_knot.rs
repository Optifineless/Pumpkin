use crate::entity::player::Player;
use crate::entity::{
    Entity, EntityBase,
    leashable::{LeashToResult, can_have_a_leash_attached_to, leash_search_box},
    living::LivingEntity,
};
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
#[cfg(test)]
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

const KNOT_EMPTY_CLEANUP: u64 = 1 << 63;
const KNOT_REMOVED: u64 = 1 << 61;
const KNOT_ATTACHMENT_COUNT_MASK: u64 = (1 << 16) - 1;
const KNOT_GENERATION_INCREMENT: u64 = 1 << 16;
const KNOT_GENERATION_MASK: u64 = !(KNOT_EMPTY_CLEANUP | KNOT_REMOVED | KNOT_ATTACHMENT_COUNT_MASK);

/// Reserves the knot link update against concurrent empty-knot removal.
pub(crate) struct KnotAttachment<'a>(&'a AtomicU64);

impl Drop for KnotAttachment<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Release);
    }
}

pub struct LeashKnotEntity {
    entity: Entity,
    pos: BlockPos,
    remove_when_empty: AtomicBool,
    attachment_state: AtomicU64,
    #[cfg(test)]
    test_pause: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    #[cfg(test)]
    attachment_check_pause: AtomicBool,
}

impl LeashKnotEntity {
    pub const OFFSET_Y: f64 = 0.375;

    pub const fn new(entity: Entity, pos: BlockPos) -> Self {
        Self {
            entity,
            pos,
            remove_when_empty: AtomicBool::new(false),
            attachment_state: AtomicU64::new(0),
            #[cfg(test)]
            test_pause: Mutex::new(None),
            #[cfg(test)]
            attachment_check_pause: AtomicBool::new(false),
        }
    }

    pub const fn block_pos(&self) -> BlockPos {
        self.pos
    }

    pub fn get_or_create(world: &Arc<World>, pos: BlockPos) -> Arc<dyn EntityBase> {
        if let Some(existing) = Self::get_knot(world, pos) {
            return existing;
        }
        let created = Self::create_knot(world, pos);
        Self::get_knot(world, pos).unwrap_or(created as Arc<dyn EntityBase>)
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
                && !knot.is_removed_or_removing()
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
        if !world.spawn_entity(knot.clone() as Arc<dyn EntityBase>) {
            // A concurrent getOrCreateKnot may have published the knot first.
            knot.attachment_state.store(KNOT_REMOVED, Ordering::Release);
        }

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
        let knot_id = self.entity.entity_id;
        world
            .get_entities_at_box(&leash_search_box(center))
            .iter()
            .map(|entity_base| entity_base.get_entity())
            .any(|entity| !entity.is_removed() && leash_holder_id(entity) == Some(knot_id))
    }

    /// Reserves only the leash pointer mutation; callers release it before callbacks or packets.
    pub(crate) fn try_begin_attachment(&self) -> Option<KnotAttachment<'_>> {
        let mut state = self.attachment_state.load(Ordering::Acquire);
        loop {
            let attachment_count = state & KNOT_ATTACHMENT_COUNT_MASK;
            if self.entity.is_removed()
                || state & KNOT_REMOVED != 0
                || attachment_count == KNOT_ATTACHMENT_COUNT_MASK
            {
                return None;
            }
            #[cfg(test)]
            self.wait_at_attachment_check_pause();
            let flags = state & (KNOT_EMPTY_CLEANUP | KNOT_REMOVED);
            let generation = (state & KNOT_GENERATION_MASK).wrapping_add(KNOT_GENERATION_INCREMENT)
                & KNOT_GENERATION_MASK;
            let next = flags | generation | (attachment_count + 1);
            match self.attachment_state.compare_exchange_weak(
                state,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(KnotAttachment(&self.attachment_state)),
                Err(actual) => state = actual,
            }
        }
    }

    fn try_begin_empty_cleanup(&self) -> Option<u64> {
        let mut state = self.attachment_state.load(Ordering::Acquire);
        loop {
            if state & (KNOT_EMPTY_CLEANUP | KNOT_REMOVED) != 0
                || state & KNOT_ATTACHMENT_COUNT_MASK != 0
            {
                return None;
            }
            let cleanup_state = state | KNOT_EMPTY_CLEANUP;
            match self.attachment_state.compare_exchange_weak(
                state,
                cleanup_state,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(cleanup_state),
                Err(actual) => state = actual,
            }
        }
    }

    fn try_mark_invalid_removed(&self) -> bool {
        let mut state = self.attachment_state.load(Ordering::Acquire);
        loop {
            if state & (KNOT_EMPTY_CLEANUP | KNOT_REMOVED) != 0
                || state & KNOT_ATTACHMENT_COUNT_MASK != 0
            {
                return false;
            }
            let removal_state = state | KNOT_REMOVED;
            match self.attachment_state.compare_exchange_weak(
                state,
                removal_state,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return true,
                Err(actual) => state = actual,
            }
        }
    }

    pub(crate) fn is_removed_or_removing(&self) -> bool {
        self.entity.is_removed()
            || self.attachment_state.load(Ordering::Acquire) & KNOT_REMOVED != 0
    }

    fn reopen_after_empty_check(&self) {
        self.attachment_state
            .fetch_and(!KNOT_EMPTY_CLEANUP, Ordering::AcqRel);
    }

    #[cfg(test)]
    pub(crate) fn set_test_pause(&self, pause: Arc<dyn Fn() + Send + Sync>) {
        self.attachment_check_pause.store(false, Ordering::Relaxed);
        *self
            .test_pause
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(pause);
    }

    #[cfg(test)]
    pub(crate) fn set_attachment_check_pause(&self, pause: Arc<dyn Fn() + Send + Sync>) {
        self.attachment_check_pause.store(true, Ordering::Relaxed);
        *self
            .test_pause
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(pause);
    }

    #[cfg(test)]
    fn wait_at_test_pause(&self) {
        if self.attachment_check_pause.load(Ordering::Relaxed) {
            return;
        }
        let pause = self
            .test_pause
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(pause) = pause {
            pause();
        }
    }

    #[cfg(test)]
    fn wait_at_attachment_check_pause(&self) {
        if !self.attachment_check_pause.load(Ordering::Relaxed) {
            return;
        }
        let pause = self
            .test_pause
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(pause) = pause {
            pause();
        }
    }

    pub(crate) fn notify_leashee_removed(&self) {
        // Leashable.dropLeash and setLeashedTo notify their former holder after changing the link.
        self.remove_when_empty.store(true, Ordering::Release);
    }

    /// Notifies a knot when a leash link changes away from its holder.
    pub(crate) fn notify_leashee_removed_from(holder: &dyn EntityBase) {
        if let Some(knot) = holder.cast_any().downcast_ref::<Self>() {
            knot.notify_leashee_removed();
        }
    }

    /// Removes an empty knot after excluding concurrent leash pointer updates.
    pub(crate) fn remove_if_empty(&self, world: &World) {
        let Some(cleanup_state) = self.try_begin_empty_cleanup() else {
            self.remove_when_empty.store(true, Ordering::Release);
            return;
        };
        if self.has_live_leashed_mobs(world) {
            self.reopen_after_empty_check();
            return;
        }

        #[cfg(test)]
        self.wait_at_test_pause();

        if self
            .attachment_state
            .compare_exchange(
                cleanup_state,
                cleanup_state | KNOT_REMOVED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
        {
            self.entity.remove();
        } else {
            // A link commit advanced the generation after the empty scan; leave its holder alive.
            self.reopen_after_empty_check();
            self.remove_when_empty.store(true, Ordering::Release);
        }
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
        let world = self.entity.world.load_full();
        let block = world.get_block(&self.pos);
        if !block.has_tag(&pumpkin_data::tag::Block::MINECRAFT_FENCES) {
            if !self.try_mark_invalid_removed() {
                return;
            }
            #[cfg(test)]
            self.wait_at_test_pause();
            let knot_id = self.entity.entity_id;
            let bounds = self.entity.bounding_box.load();
            let center = (bounds.min + bounds.max) * 0.5;
            let search_box = leash_search_box(center);
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

                if is_attached_to_knot
                    && ent.unleash_if_leashed()
                    && world.level_info.load().game_rules.entity_drops
                {
                    let lead_item = pumpkin_data::item_stack::ItemStack::new(
                        1,
                        &pumpkin_data::item::Item::LEAD,
                    );
                    world.drop_stack(&ent.block_pos.load(), lead_item);
                }
            }

            let pos = self.entity.pos.load();
            world.play_sound(Sound::ItemLeadUntied, SoundCategory::Neutral, &pos);
            self.entity.remove();
        } else if self.remove_when_empty.swap(false, Ordering::AcqRel) {
            // LeashFenceKnotEntity.notifyLeasheeRemoved discards an empty knot. Run this after
            // parallel player packet handling so another packet cannot attach to a removed knot.
            self.remove_if_empty(&world);
        }
    }

    // LeashFenceKnotEntity.interact transfers eligible leashes, then removes an empty knot.
    fn interact(&self, player: &Arc<Player>, item_stack: &mut ItemStack) -> bool {
        if crate::entity::leash_shearing::shear_leashes_by_player(&self.entity, player, item_stack)
        {
            return true;
        }

        let world = player.world();
        let knot_id = self.entity.entity_id;
        let player_id = player.entity_id();

        let bounds = self.entity.bounding_box.load();
        let center = (bounds.min + bounds.max) * 0.5;
        let search_box = leash_search_box(center);
        let entities = world.get_entities_at_box(&search_box);

        // Leashable.leashableLeashedTo plus canHaveALeashAttachedTo.
        let player_leashed_mobs: Vec<_> = entities
            .iter()
            .map(|entity_base| entity_base.get_entity())
            .filter(|ent| leash_holder_id(ent) == Some(player_id))
            .collect();

        let mut attached_mob = false;

        if !player_leashed_mobs.is_empty() {
            let mut self_knot = Self::get_or_create(&world, self.pos);
            for mob in player_leashed_mobs {
                attached_mob |= Self::attach_to_knot(&world, self.pos, mob, &mut self_knot);
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
                mob.leash_to_with_result(player.clone() as Arc<dyn EntityBase>);
                any_dropped |= leash_holder_id(mob) == Some(player_id);
            }
        }

        if attached_mob || any_dropped {
            let pos = self.entity.pos.load();
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

impl LeashKnotEntity {
    /// Attaches a mob to a live knot, retrying once if empty-knot cleanup won before the link commit.
    pub(crate) fn attach_to_knot(
        world: &Arc<World>,
        pos: BlockPos,
        mob: &Entity,
        holder: &mut Arc<dyn EntityBase>,
    ) -> bool {
        for attempt in 0..=1 {
            if !can_have_a_leash_attached_to(mob, holder.get_entity()) {
                return false;
            }
            match mob.leash_to_with_result(holder.clone()) {
                LeashToResult::Attached => return true,
                LeashToResult::HolderUnavailable if attempt == 0 => {
                    *holder = Self::get_or_create(world, pos);
                }
                LeashToResult::Cancelled | LeashToResult::HolderUnavailable => return false,
            }
        }
        false
    }
}

/// Returns a snapshot of the entity's current leash holder ID.
pub(crate) fn leash_holder_id(entity: &Entity) -> Option<i32> {
    // Snapshot before leash changes can fire an event or send packets.
    // Block on contention so the vanilla transfer cannot silently skip this leash.
    let holder = entity
        .leashed_to
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    holder.map(|holder| holder.get_entity().entity_id)
}

#[cfg(test)]
#[path = "leash_knot_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "leash_knot_review_tests.rs"]
mod review_tests;
