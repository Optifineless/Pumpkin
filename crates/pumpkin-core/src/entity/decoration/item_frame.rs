use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::entity::player::Player;
use crate::entity::{Entity, EntityBase, living::LivingEntity};
use crossbeam::atomic::AtomicCell;
use pumpkin_data::BlockDirection;
use pumpkin_data::damage::DamageType;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::Sound;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::codec::item_stack_seralizer::ItemStackSerializer;
use pumpkin_protocol::java::client::play::{CSetEntityMetadata, Metadata};
use pumpkin_util::math::boundingbox::BoundingBox;
use pumpkin_util::math::vector3::Vector3;

use super::hanging;
use crate::world::map::FrameAttachment;
use crate::{block::blocks::redstone::is_diode, server::Server};
use pumpkin_data::data_component_impl::MapIdImpl;

/// An item frame or glow item frame.
///
/// Holds the displayed item and its rotation so that comparators can read the
/// frame's analog output and so frames from vanilla worlds keep their data
/// across save cycles.
pub struct ItemFrameEntity {
    entity: Entity,
    item_stack: Mutex<Arc<ItemStack>>,
    item_revision: AtomicU64,
    map_attachment: AtomicCell<Option<FrameAttachment>>,
    /// Rotation of the displayed item, always in `0..8`.
    rotation: AtomicU8,
    /// The direction the frame faces, i.e. the axis pointing away from the
    /// block it hangs on. Stored as the vanilla 3D direction index
    /// (0 = down, 1 = up, 2 = north, 3 = south, 4 = west, 5 = east).
    facing: AtomicU8,
    item_drop_chance: AtomicCell<f32>,
    invisible: AtomicBool,
    fixed: AtomicBool,
    survival_check_ticks: AtomicU32,
}

impl ItemFrameEntity {
    /// Facing used when a frame is created without NBT, matching vanilla.
    const DEFAULT_FACING: BlockDirection = BlockDirection::South;

    pub fn new(entity: Entity) -> Self {
        let facing = Self::DEFAULT_FACING.to_index();
        // The spawn packet reads the direction from the entity data field, so
        // it has to agree with `facing` or the frame spawns facing elsewhere.
        entity.data.store(i32::from(facing), Ordering::Relaxed);
        let frame = Self {
            entity,
            item_stack: Mutex::new(Arc::new(ItemStack::EMPTY.clone())),
            item_revision: AtomicU64::new(0),
            map_attachment: AtomicCell::new(None),
            rotation: AtomicU8::new(0),
            facing: AtomicU8::new(facing),
            item_drop_chance: AtomicCell::new(1.0),
            invisible: AtomicBool::new(false),
            fixed: AtomicBool::new(false),
            survival_check_ticks: AtomicU32::new(0),
        };
        frame.recalculate_bounding_box();
        frame
    }

    // ItemFrame.createBoundingBox: saved Pumpkin positions remain the attachment cell's center.
    fn create_bounding_box(&self, has_map: bool) -> BoundingBox {
        let step = self.get_facing().to_offset();
        let pos = self.entity.block_pos.load().0.to_f64() + Vector3::new(0.5, 0.5, 0.5)
            - step.to_f64() * hanging::WALL_OFFSET;
        let face_size = if has_map { 1.0 } else { 0.75 };
        hanging::box_of_size(
            pos,
            Vector3::new(
                if step.x == 0 {
                    face_size
                } else {
                    hanging::DEPTH
                },
                if step.y == 0 {
                    face_size
                } else {
                    hanging::DEPTH
                },
                if step.z == 0 {
                    face_size
                } else {
                    hanging::DEPTH
                },
            ),
        )
    }

    pub(super) fn hanging_bounding_box(&self) -> BoundingBox {
        self.create_bounding_box(
            self.snapshot_item()
                .get_data_component::<pumpkin_data::data_component_impl::MapIdImpl>()
                .is_some(),
        )
    }

    fn recalculate_bounding_box(&self) {
        let item = self.snapshot_item();
        let has_map = item
            .get_data_component::<pumpkin_data::data_component_impl::MapIdImpl>()
            .is_some();
        let current = self
            .item_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if Arc::ptr_eq(&current, &item) {
            self.entity
                .bounding_box
                .store(self.create_bounding_box(has_map));
        }
    }

    pub fn survives(&self) -> bool {
        if self.is_fixed() {
            return true;
        }
        let world = self.entity.world.load_full();
        let facing = self.get_facing();
        let support_pos = self
            .entity
            .block_pos
            .load()
            .offset(facing.opposite().to_offset());
        let bounds = self.create_bounding_box(false);
        if !hanging::terrain_available(&world, bounds.expand_all(1.0))
            || (world.is_in_build_limit(support_pos) && !world.is_loaded(&support_pos))
        {
            return true;
        }
        let (block, state) = world.get_block_and_state(&support_pos);
        !hanging::has_level_collision(self, bounds)
            && (state.is_solid() || (facing.is_horizontal() && is_diode(block)))
            && hanging::can_coexist(&self.entity, bounds, facing, true)
    }

    fn break_and_drop(
        &self,
        cause: Option<&dyn EntityBase>,
        damage_type: Option<DamageType>,
    ) -> bool {
        if !hanging::break_allowed(&self.entity, cause) {
            return false;
        }
        let facing = self.get_facing();
        if damage_type.is_none() && self.survives() {
            return false;
        }
        let world = self.entity.world.load_full();
        let can_hurt_fixed = damage_type.is_some_and(|damage| {
            damage == DamageType::OUT_OF_WORLD
                || damage == DamageType::GENERIC_KILL
                || cause.is_some_and(|cause| {
                    cause
                        .cast_any()
                        .downcast_ref::<Player>()
                        .is_some_and(Player::is_creative)
                })
        });
        let item_guard = self
            .item_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if (self.is_fixed() && !can_hurt_fixed)
            || (damage_type.is_none() && facing != self.get_facing())
            || !world.try_mark_entity_removed(&self.entity)
        {
            return false;
        }
        let cleanup = self.change_map_attachment(None, true);
        drop(item_guard);
        self.finish_map_cleanup(cleanup);
        world.finish_entity_removal(self);
        self.entity.play_sound(self.get_break_sound());
        self.drop_item(cause, true);
        true
    }

    pub const fn is_glow(&self) -> bool {
        self.entity.entity_type.id == EntityType::GLOW_ITEM_FRAME.id
    }

    pub const fn get_add_item_sound(&self) -> Sound {
        if self.is_glow() {
            Sound::EntityGlowItemFrameAddItem
        } else {
            Sound::EntityItemFrameAddItem
        }
    }

    pub const fn get_remove_item_sound(&self) -> Sound {
        if self.is_glow() {
            Sound::EntityGlowItemFrameRemoveItem
        } else {
            Sound::EntityItemFrameRemoveItem
        }
    }

    pub const fn get_rotate_item_sound(&self) -> Sound {
        if self.is_glow() {
            Sound::EntityGlowItemFrameRotateItem
        } else {
            Sound::EntityItemFrameRotateItem
        }
    }

    pub const fn get_break_sound(&self) -> Sound {
        if self.is_glow() {
            Sound::EntityGlowItemFrameBreak
        } else {
            Sound::EntityItemFrameBreak
        }
    }

    pub const fn get_place_sound(&self) -> Sound {
        if self.is_glow() {
            Sound::EntityGlowItemFramePlace
        } else {
            Sound::EntityItemFramePlace
        }
    }

    pub fn get_facing(&self) -> BlockDirection {
        BlockDirection::from_index(self.facing.load(Ordering::Relaxed))
            .unwrap_or(Self::DEFAULT_FACING)
    }

    pub fn set_facing(&self, facing: BlockDirection) {
        let item_guard = self
            .item_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.entity.is_removed() {
            return;
        }
        let index = facing.to_index();
        self.facing.store(index, Ordering::Relaxed);
        self.entity.data.store(i32::from(index), Ordering::Relaxed);
        drop(item_guard);
        self.recalculate_bounding_box();
    }

    pub fn get_item(&self) -> ItemStack {
        self.snapshot_item().as_ref().clone()
    }

    fn snapshot_item(&self) -> Arc<ItemStack> {
        self.item_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn sync_item(&self) {
        let (item, revision) = {
            let item = self
                .item_stack
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (item.clone(), self.item_revision.load(Ordering::Acquire))
        };
        // Component cloning/serialization may replace the item again. Publish only
        // this revision, with the check inside the metadata publication lock.
        self.entity.synched_data.set_if_current_revision(
            pumpkin_data::tracked_data::item_frame::ITEM,
            ItemStackSerializer::from(item.as_ref().clone()),
            &self.item_revision,
            revision,
        );
    }

    pub fn set_item(&self, item_stack: ItemStack, update_neighbours: bool) {
        self.try_set_item(item_stack, update_neighbours, false);
    }

    fn try_set_item(
        &self,
        mut item_stack: ItemStack,
        update_neighbours: bool,
        require_empty: bool,
    ) -> bool {
        if !item_stack.is_empty() {
            item_stack.item_count = 1;
        }

        let play_sound = !item_stack.is_empty();
        let map_id = item_stack
            .get_data_component::<MapIdImpl>()
            .map(|map| map.id);
        let has_map = map_id.is_some();
        let mut current = self
            .item_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.entity.is_removed() || (require_empty && (!current.is_empty() || self.is_fixed())) {
            return false;
        }
        let retired = std::mem::replace(&mut *current, Arc::new(item_stack));
        self.item_revision.fetch_add(1, Ordering::Release);
        self.entity
            .bounding_box
            .store(self.create_bounding_box(has_map));
        let cleanup_old = self.map_attachment.load().is_some_and(|old| {
            Some(old.map_id) != map_id || old.pos != self.entity.block_pos.load()
        });
        let cleanup = self.change_map_attachment(map_id, cleanup_old);
        drop(current);
        self.finish_map_cleanup(cleanup);
        drop(retired);
        self.sync_item();

        if play_sound {
            self.entity.play_sound(self.get_add_item_sound());
        }

        if update_neighbours {
            let world = self.entity.world.load_full();
            let pos = self.entity.block_pos.load();
            world.update_neighbors(&pos, None);
        }
        true
    }

    // Called only while the item transaction is held. Keep the old attachment
    // position even when Entity.read_nbt has already installed a new Pos.
    fn change_map_attachment(&self, map_id: Option<i32>, cleanup: bool) -> Option<i32> {
        let old = self.map_attachment.load();
        let new = map_id.map(|map_id| FrameAttachment {
            map_id,
            pos: self.entity.block_pos.load(),
            frame: self.entity.entity_uuid,
            runtime_id: self.entity.entity_id,
            revision: self.item_revision.load(Ordering::Acquire),
        });
        if let Some(server) = self.entity.world.load().server.upgrade() {
            server
                .map_manager
                .change_frame_attachment(old, new, cleanup);
        }
        self.map_attachment.store(new);
        old.filter(|_| cleanup).map(|old| old.map_id)
    }

    fn finish_map_cleanup(&self, id: Option<i32>) {
        if let Some(id) = id
            && let Some(server) = self.entity.world.load().server.upgrade()
        {
            server.map_manager.finish_frame_cleanup(id);
        }
    }

    fn remove_framed_map(&self, attachment: Option<FrameAttachment>, cleanup: bool) {
        if let Some(old) = attachment
            && let Some(server) = self.entity.world.load().server.upgrade()
        {
            server
                .map_manager
                .change_frame_attachment(Some(old), None, cleanup);
            if cleanup {
                server.map_manager.finish_frame_cleanup(old.map_id);
            }
        }
    }

    pub fn get_rotation(&self) -> u8 {
        self.rotation.load(Ordering::Relaxed) % 8
    }

    pub fn set_rotation(&self, rotation: u8, update_neighbours: bool) {
        let rot = rotation % 8;
        let item_guard = self
            .item_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.entity.is_removed() {
            return;
        }
        self.rotation.store(rot, Ordering::Relaxed);
        drop(item_guard);

        self.entity
            .set_synced_data(pumpkin_data::tracked_data::item_frame::ROTATION, rot as i32);

        if update_neighbours {
            let world = self.entity.world.load_full();
            let pos = self.entity.block_pos.load();
            world.update_neighbors(&pos, None);
        }
    }

    pub fn get_drop_chance(&self) -> f32 {
        self.item_drop_chance.load()
    }

    pub fn set_drop_chance(&self, chance: f32) {
        let _item = self
            .item_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.entity.is_removed() {
            return;
        }
        self.item_drop_chance.store(chance);
    }

    pub fn is_fixed(&self) -> bool {
        self.fixed.load(Ordering::Relaxed)
    }

    pub fn set_fixed(&self, fixed: bool) {
        let _item = self
            .item_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.entity.is_removed() {
            return;
        }
        self.fixed.store(fixed, Ordering::Relaxed);
    }

    pub fn is_invisible(&self) -> bool {
        self.invisible.load(Ordering::Relaxed)
    }

    pub fn set_invisible(&self, invisible: bool) {
        let _item = self
            .item_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.entity.is_removed() {
            return;
        }
        self.invisible.store(invisible, Ordering::Relaxed);
    }

    pub fn get_frame_item_stack(&self) -> ItemStack {
        if self.is_glow() {
            ItemStack::new(1, &Item::GLOW_ITEM_FRAME)
        } else {
            ItemStack::new(1, &Item::ITEM_FRAME)
        }
    }

    pub fn get_frame_item_stack_with_data(&self) -> ItemStack {
        let mut stack = self.get_frame_item_stack();
        if let Some(custom_name) = self.entity.custom_name.load().as_ref().clone() {
            stack.set_data_component(pumpkin_data::data_component_impl::CustomNameImpl {
                name: custom_name,
            });
        }
        stack
    }

    pub fn get_pick_result(&self) -> ItemStack {
        let framed_stack = self.get_item();
        if framed_stack.is_empty() {
            self.get_frame_item_stack_with_data()
        } else {
            framed_stack
        }
    }

    /// The comparator signal this frame produces.
    ///
    /// Vanilla: `getItem().isEmpty() ? 0 : getRotation() % 8 + 1`.
    pub fn get_analog_output(&self) -> u8 {
        if self
            .item_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
        {
            0
        } else {
            self.rotation.load(Ordering::Relaxed) % 8 + 1
        }
    }

    pub fn drop_item(&self, caused_by: Option<&dyn EntityBase>, with_frame: bool) {
        self.drop_item_observed(caused_by, with_frame, None);
    }

    fn drop_item_observed(
        &self,
        caused_by: Option<&dyn EntityBase>,
        with_frame: bool,
        expected: Option<&Arc<ItemStack>>,
    ) -> bool {
        if self.is_fixed() {
            return false;
        }

        let empty = Arc::new(ItemStack::EMPTY.clone());
        let (item_stack, attachment) = {
            let mut item = self
                .item_stack
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if self.is_fixed()
                || (!with_frame && self.entity.is_removed())
                || expected.is_some_and(|observed| !Arc::ptr_eq(&item, observed))
            {
                return false;
            }
            let retired = std::mem::replace(&mut *item, empty);
            self.item_revision.fetch_add(1, Ordering::Release);
            self.entity
                .bounding_box
                .store(self.create_bounding_box(false));
            (retired, self.map_attachment.swap(None))
        };
        self.sync_item();
        let item_stack = Arc::unwrap_or_clone(item_stack);

        let is_creative_player = caused_by.is_some_and(|s| {
            s.cast_any()
                .downcast_ref::<Player>()
                .is_some_and(Player::is_creative)
        });
        let world = self.entity.world.load_full();
        let entity_drops = world.level_info.load().game_rules.entity_drops;
        // ItemFrame.dropItem reads policy after setItem/copy callbacks. The owner
        // rejects this retired token if a callback installed a newer attachment.
        self.remove_framed_map(attachment, entity_drops || caused_by.is_none());
        if !entity_drops || is_creative_player {
            return true;
        }
        let pos = self.entity.block_pos.load();

        if with_frame {
            world.drop_stack(&pos, self.get_frame_item_stack_with_data());
        }

        if !item_stack.is_empty() {
            let drop_chance = self.item_drop_chance.load();
            if rand::random::<f32>() < drop_chance {
                world.drop_stack(&pos, item_stack);
            }
        }
        true
    }
}

impl Drop for ItemFrameEntity {
    fn drop(&mut self) {
        // Chunk unload retires only runtime ownership; its saved marker remains.
        if let Some(server) = self.entity.world.load().server.upgrade() {
            server
                .map_manager
                .change_frame_attachment(self.map_attachment.load(), None, false);
        }
    }
}

impl EntityBase for ItemFrameEntity {
    fn tick(&self, caller: &dyn EntityBase, _server: &Server) {
        if self.entity.is_removed() {
            return;
        }
        // BlockAttachedEntity.tick deliberately does not call Entity.tick.
        self.entity.check_out_of_world(caller);
        if hanging::at_check_interval(&self.survival_check_ticks)
            && !self.entity.is_removed()
            && !self.survives()
        {
            self.break_and_drop(None, None);
        }
    }

    fn write_custom_nbt(&self, nbt: &mut NbtCompound) {
        let (item, drop_chance, rotation, facing, invisible, fixed) = {
            let item = self
                .item_stack
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (
                item.clone(),
                self.item_drop_chance.load(),
                self.rotation.load(Ordering::Relaxed),
                self.facing.load(Ordering::Relaxed),
                self.invisible.load(Ordering::Relaxed),
                self.fixed.load(Ordering::Relaxed),
            )
        };
        if !item.is_empty() {
            let mut item_compound = NbtCompound::new();
            item.write_item_stack(&mut item_compound);
            nbt.put_compound("Item", item_compound);
        }
        nbt.put_float("ItemDropChance", drop_chance);
        nbt.put_byte("ItemRotation", rotation as i8);
        nbt.put_byte("Facing", facing as i8);
        nbt.put_bool("Invisible", invisible);
        nbt.put_bool("Fixed", fixed);
    }

    fn read_custom_nbt(&self, nbt: &NbtCompound) {
        let observed = self.snapshot_item();
        let stack = nbt
            .get_compound("Item")
            .and_then(ItemStack::read_item_stack)
            .unwrap_or_else(|| ItemStack::EMPTY.clone());
        // ItemFrame.readAdditionalSaveData uses ItemStack.matches, including components.
        let cleanup_old = !observed.is_empty() && !stack.are_equal(observed.as_ref());
        let map_id = stack.get_data_component::<MapIdImpl>().map(|map| map.id);
        let stack = Arc::new(stack);
        let rotation = (nbt.get_byte("ItemRotation").unwrap_or(0) as u8) % 8;
        let facing = nbt.get_byte("Facing").unwrap_or(0) as u8 % 6;
        let drop_chance = nbt.get_float("ItemDropChance").unwrap_or(1.0);
        let invisible = nbt.get_bool("Invisible").unwrap_or(false);
        let fixed = nbt.get_bool("Fixed").unwrap_or(false);
        let mut item = self
            .item_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.entity.is_removed() || !Arc::ptr_eq(&item, &observed) {
            return;
        }
        let retired = std::mem::replace(&mut *item, stack);
        self.item_revision.fetch_add(1, Ordering::Release);
        self.rotation.store(rotation, Ordering::Relaxed);
        self.facing.store(facing, Ordering::Relaxed);
        // The spawn packet's data field carries the frame's direction.
        self.entity.data.store(i32::from(facing), Ordering::Relaxed);
        self.item_drop_chance.store(drop_chance);
        self.invisible.store(invisible, Ordering::Relaxed);
        self.fixed.store(fixed, Ordering::Relaxed);
        let cleanup = self.change_map_attachment(map_id, cleanup_old);
        drop(item);
        self.finish_map_cleanup(cleanup);
        drop(retired);
        self.recalculate_bounding_box();
        self.sync_item();
    }

    fn tick_in_void(&self, caller: &dyn EntityBase) {
        self.kill(caller);
    }

    fn kill(&self, _caller: &dyn EntityBase) {
        let world = self.entity.world.load_full();
        let item = self
            .item_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !world.try_mark_entity_removed(&self.entity) {
            return;
        }
        // ItemFrame.onKilled is independent of dropItem's Fixed/no-drop branches.
        let cleanup = self.change_map_attachment(None, true);
        drop(item);
        self.finish_map_cleanup(cleanup);
        world.finish_entity_removal(self);
    }

    fn get_entity(&self) -> &Entity {
        &self.entity
    }

    fn get_living_entity(&self) -> Option<&LivingEntity> {
        None
    }

    fn can_hit(&self) -> bool {
        !self.entity.is_removed()
    }

    fn init_data_tracker(&self) {
        self.recalculate_bounding_box();
        let rotation = self.get_rotation() as i32;
        self.sync_item();
        self.entity
            .set_synced_data(pumpkin_data::tracked_data::item_frame::ROTATION, rotation);
    }

    fn send_java_spawn_packet(&self, client: &crate::net::java::JavaClient) {
        let spawn_packet = self.entity.create_spawn_packet();
        if let Ok(data) = client.serialize_packet(&spawn_packet) {
            client.try_enqueue_packet(data);
        }

        let ver = pumpkin_data::packet::CURRENT_MC_VERSION;
        let item_serializer = ItemStackSerializer::from(self.get_item());
        let rotation = self.get_rotation() as i32;

        let mut data = Vec::new();
        let meta_item = Metadata::new(
            pumpkin_data::tracked_data::item_frame::ITEM,
            item_serializer,
        );
        let meta_rot = Metadata::new(pumpkin_data::tracked_data::item_frame::ROTATION, rotation);

        if meta_item.write(&mut data, &ver).is_ok() && meta_rot.write(&mut data, &ver).is_ok() {
            data.push(255);
            let meta_packet = CSetEntityMetadata::new(self.entity.entity_id.into(), data.into());
            if let Ok(meta_data) = client.serialize_packet(&meta_packet) {
                client.try_enqueue_packet(meta_data);
            }
        }
    }

    fn interact(&self, player: &Arc<Player>, item_stack: &mut ItemStack) -> bool {
        if self.is_fixed() {
            return false;
        }

        let frame_has_item = !self.get_item().is_empty();
        let has_held_item = !item_stack.is_empty();

        if frame_has_item {
            let new_rot = self.get_rotation() + 1;
            self.set_rotation(new_rot, true);
            self.entity.play_sound(self.get_rotate_item_sound());
            true
        } else if has_held_item && !self.entity.removed.load(Ordering::Relaxed) {
            let mut new_stack = item_stack.clone();
            new_stack.item_count = 1;
            if !self.try_set_item(new_stack, true, true) {
                return false;
            }

            if !player.is_creative() {
                item_stack.decrement(1);
            }
            true
        } else {
            false
        }
    }

    fn damage_with_context(
        &self,
        _caller: &dyn EntityBase,
        _amount: f32,
        damage_type: DamageType,
        _position: Option<Vector3<f64>>,
        _source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) -> bool {
        let fixed = self.is_fixed();
        if self.entity.is_removed() {
            return false;
        }
        let is_creative_player = cause.is_some_and(|s| {
            s.cast_any()
                .downcast_ref::<Player>()
                .is_some_and(Player::is_creative)
        });
        let bypasses_invuln =
            damage_type == DamageType::OUT_OF_WORLD || damage_type == DamageType::GENERIC_KILL;

        if fixed {
            if !bypasses_invuln && !is_creative_player {
                return false;
            }
            return self.break_and_drop(cause, Some(damage_type));
        }

        let observed = self.snapshot_item();
        let has_item = !observed.as_ref().clone().is_empty();
        let is_explosion =
            damage_type == DamageType::EXPLOSION || damage_type == DamageType::PLAYER_EXPLOSION;

        if !is_explosion && has_item {
            if !self.drop_item_observed(cause, false, Some(&observed)) {
                return false;
            }
            self.entity.play_sound(self.get_remove_item_sound());
        } else {
            return self.break_and_drop(cause, Some(damage_type));
        }
        true
    }

    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }
}
