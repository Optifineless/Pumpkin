use pumpkin_data::data_component_impl::{CustomNameImpl, IDSetContent, LockImpl};
use pumpkin_data::tag::Taggable;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use pumpkin_data::effect::StatusEffect;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::boundingbox::BoundingBox;
use pumpkin_util::math::{position::BlockPos, vector2::Vector2};
use pumpkin_util::sync::LockClass;

use crate::block::entities::{BlockEntity, PropertyDelegate};
use crate::world::World;

#[path = "beacon_menu.rs"]
mod menu;

enum BeaconEffects {}

impl LockClass for BeaconEffects {
    const NAME: &'static str = "beacon effects";
    const LEVEL: u8 = 40;
}

enum BeaconMenuMetadata {}

impl LockClass for BeaconMenuMetadata {
    const NAME: &'static str = "beacon menu metadata";
    const LEVEL: u8 = 40;
}

pub struct BeaconBlockEntity {
    pub position: BlockPos,
    pub primary_effect: AtomicI32,
    pub secondary_effect: AtomicI32,
    pub levels: AtomicI32,
    pub dirty: AtomicBool,
    effects_lock: pumpkin_util::sync::Mutex<(), BeaconEffects>,
    beam_active: AtomicBool,
    checking_beam: AtomicBool,

    // Vanilla Parity Fields
    pub custom_name: Mutex<Option<String>>,
    pub lock_key: Mutex<Option<String>>,
    canonical_name: pumpkin_util::sync::Mutex<Option<(String, CustomNameImpl)>, BeaconMenuMetadata>,
    canonical_lock:
        pumpkin_util::sync::Mutex<Option<(Option<String>, LockImpl)>, BeaconMenuMetadata>,
    pub last_check_y: AtomicI32,
}

impl BeaconBlockEntity {
    pub const ID: &'static str = "minecraft:beacon";

    // ContainerData Property Constants
    pub const DATA_LEVELS: usize = 0;
    pub const DATA_PRIMARY: usize = 1;
    pub const DATA_SECONDARY: usize = 2;
    pub const NUM_DATA_VALUES: usize = 3;

    pub(crate) fn has_active_beam(&self) -> bool {
        self.beam_active.load(Ordering::Relaxed)
    }

    fn update_beam(&self, world: &World) {
        // BeaconBlockEntity.tick scans ten blocks per tick and publishes only completed beams.
        let top = world.get_top_block(Vector2::new(self.position.0.x, self.position.0.z)) + 1;
        let mut last = self.last_check_y.load(Ordering::Relaxed);
        if last < self.position.0.y {
            last = self.position.0.y - 1;
            self.checking_beam.store(false, Ordering::Relaxed);
        }
        for y in last + 1..=(last + 10).min(top) {
            let (block, state) =
                world.get_block_and_state(&BlockPos::new(self.position.0.x, y, self.position.0.z));
            // A stale heightmap can end a scan before it reaches the first beam section.
            if y == self.position.0.y && block == &pumpkin_data::Block::BEACON {
                self.checking_beam.store(true, Ordering::Relaxed);
            }
            // Colored beam glass has zero light dampening; bedrock is vanilla's exception.
            if state.opacity >= 15 && block != &pumpkin_data::Block::BEDROCK {
                self.checking_beam.store(false, Ordering::Relaxed);
                last = top;
                break;
            }
            last = y;
        }
        if last >= top {
            self.beam_active.store(
                self.checking_beam.load(Ordering::Relaxed),
                Ordering::Relaxed,
            );
            last = world.dimension.min_y - 1;
        }
        self.last_check_y.store(last, Ordering::Relaxed);
    }

    #[must_use]
    pub const fn new(position: BlockPos) -> Self {
        Self {
            position,
            primary_effect: AtomicI32::new(-1),
            secondary_effect: AtomicI32::new(-1),
            levels: AtomicI32::new(0),
            dirty: AtomicBool::new(false),
            effects_lock: pumpkin_util::sync::Mutex::new(()),
            beam_active: AtomicBool::new(false),
            checking_beam: AtomicBool::new(false),
            custom_name: Mutex::new(None),
            lock_key: Mutex::new(None),
            canonical_name: pumpkin_util::sync::Mutex::new(None),
            canonical_lock: pumpkin_util::sync::Mutex::new(None),
            last_check_y: AtomicI32::new(position.0.y - 1),
        }
    }

    /// Replicates the Java `ContainerData` used to sync values to the `BeaconMenu`
    pub fn get_data(&self, id: usize) -> i32 {
        match id {
            Self::DATA_LEVELS => self.levels.load(Ordering::Relaxed),
            Self::DATA_PRIMARY => Self::encode_effect(self.effects().0),
            Self::DATA_SECONDARY => Self::encode_effect(self.effects().1),
            _ => 0,
        }
    }

    pub fn set_data(&self, id: usize, value: i32) {
        let _effects = self
            .effects_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match id {
            Self::DATA_LEVELS => self.levels.store(value, Ordering::Relaxed),
            Self::DATA_PRIMARY => self
                .primary_effect
                .store(Self::decode_effect(value), Ordering::Relaxed),
            Self::DATA_SECONDARY => self
                .secondary_effect
                .store(Self::decode_effect(value), Ordering::Relaxed),
            _ => {}
        }
        self.mark_dirty();
    }

    /// Commits both selected effects under one owner, as in `BeaconMenu.updateEffects`.
    pub fn set_effects(&self, primary: Option<i32>, secondary: Option<i32>) {
        let _effects = self
            .effects_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.primary_effect
            .store(primary.unwrap_or(-1), Ordering::Relaxed);
        self.secondary_effect
            .store(secondary.unwrap_or(-1), Ordering::Relaxed);
        self.mark_dirty();
    }

    fn effects(&self) -> (i32, i32) {
        let _effects = self
            .effects_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (
            self.primary_effect.load(Ordering::Relaxed),
            self.secondary_effect.load(Ordering::Relaxed),
        )
    }

    const fn encode_effect(effect_id: i32) -> i32 {
        if effect_id < 0 {
            0
        } else {
            Self::encode_effect_id(Some(effect_id))
        }
    }

    /// Mirrors `BeaconMenu.encodeEffect`: zero means no effect; otherwise use registry ID + 1.
    #[must_use]
    pub const fn encode_effect_id(effect_id: Option<i32>) -> i32 {
        match effect_id {
            Some(effect_id) => effect_id + 1,
            None => 0,
        }
    }

    const fn decode_effect(encoded: i32) -> i32 {
        if encoded <= 0 { -1 } else { encoded - 1 }
    }

    pub fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::Relaxed);
    }

    #[must_use]
    pub const fn is_valid_primary_effect(effect_id: i32, levels: i32) -> bool {
        match effect_id {
            // Speed (0), Haste (2)
            0 | 2 => levels >= 1,
            // Resistance (10), Jump Boost (7)
            10 | 7 => levels >= 2,
            // Strength (4)
            4 => levels >= 3,
            _ => false,
        }
    }

    #[must_use]
    pub const fn is_valid_secondary_effect(
        primary_id: Option<i32>,
        secondary_id: Option<i32>,
        levels: i32,
    ) -> bool {
        match secondary_id {
            None => true,
            Some(secondary_id) => {
                let matches_primary = match primary_id {
                    Some(primary_id) => secondary_id == primary_id,
                    None => false,
                };
                levels >= 4 && (secondary_id == 9 || matches_primary)
            }
        }
    }

    #[must_use]
    pub const fn validate_effects(
        primary: Option<i32>,
        secondary: Option<i32>,
        levels: i32,
    ) -> bool {
        if let Some(primary_id) = primary
            && !Self::is_valid_primary_effect(primary_id, levels)
        {
            return false;
        }

        if !Self::is_valid_secondary_effect(primary, secondary, levels) {
            return false;
        }

        true
    }

    pub fn update_base(&self, world: &Arc<World>) -> i32 {
        let x = self.position.0.x;
        let y = self.position.0.y;
        let z = self.position.0.z;

        let mut current_level = 0;

        for level in 1..=4 {
            let layer_y = y - level;
            if layer_y < world.dimension.min_y {
                break;
            }

            let mut layer_valid = true;
            for dx in -level..=level {
                for dz in -level..=level {
                    let block_pos = BlockPos::new(x + dx, layer_y, z + dz);
                    let state = world.get_block_state(&block_pos);
                    let block = world.get_block(&block_pos);

                    if !block.has_tag(&pumpkin_data::tag::Block::MINECRAFT_BEACON_BASE_BLOCKS) {
                        layer_valid = false;
                        break;
                    }

                    // Optional: Stricter block type validations can happen here
                    let _ = state;
                }
                if !layer_valid {
                    break;
                }
            }

            if layer_valid {
                current_level = level;
            } else {
                break;
            }
        }

        current_level
    }

    pub fn apply_effects(&self, world: &Arc<World>, levels: i32) {
        let (primary_id, secondary_id) = self.effects();

        if primary_id < 0 {
            return;
        }

        let primary_effect = StatusEffect::from_id(primary_id as u16);
        let secondary_effect = StatusEffect::from_id(secondary_id as u16);

        // Vanilla duration: (9 + levels * 2) * 20 ticks
        let duration_ticks = (9 + levels * 2) * 20;

        // Base amplifier: primary gets amp 1 (Level II) if secondary matches primary
        let base_amp = i32::from(levels >= 4 && primary_id == secondary_id);

        // Vanilla Range is level * 10 + 10 blocks in each horizontal direction
        let range = f64::from(levels * 10 + 10);
        let pos = self.position.0.to_f64();
        let box_min = [pos.x - range, pos.y - range, pos.z - range];
        let box_max = [
            pos.x + range + 1.0,
            pos.y + range + 1.0 + 384.0,
            pos.z + range + 1.0,
        ];
        let bounds = BoundingBox::new_array(box_min, box_max);

        // Apply effect to all players in range
        let players = world.players.load();
        for player in players.iter() {
            if !bounds.intersects(&player.living_entity.entity.bounding_box.load()) {
                continue;
            }

            if let Some(effect) = primary_effect {
                player.add_effect(pumpkin_data::potion::Effect {
                    effect_type: effect,
                    duration: duration_ticks,
                    amplifier: base_amp as u8,
                    ambient: true,
                    show_particles: true,
                    show_icon: true,
                    blend: false,
                });
            }

            if levels >= 4
                && primary_id != secondary_id
                && let Some(effect) = secondary_effect
            {
                player.add_effect(pumpkin_data::potion::Effect {
                    effect_type: effect,
                    duration: duration_ticks,
                    amplifier: 0,
                    ambient: true,
                    show_particles: true,
                    show_icon: true,
                    blend: false,
                });
            }
        }
    }
}

impl PropertyDelegate for BeaconBlockEntity {
    fn get_property(&self, index: i32) -> i32 {
        if (0..Self::NUM_DATA_VALUES as i32).contains(&index) {
            self.get_data(index as usize)
        } else {
            0
        }
    }

    fn set_property(&self, index: i32, value: i32) {
        if (0..Self::NUM_DATA_VALUES as i32).contains(&index) {
            self.set_data(index as usize, value);
        }
    }

    fn get_properties_size(&self) -> i32 {
        Self::NUM_DATA_VALUES as i32
    }
}

impl BlockEntity for BeaconBlockEntity {
    fn container_lock(&self) -> Option<LockImpl> {
        self.menu_lock(false)
    }

    fn collect_item_components(&self, stack: &mut pumpkin_data::item_stack::ItemStack) {
        self.collect_menu_components(stack);
    }

    fn apply_item_components(&self, stack: &pumpkin_data::item_stack::ItemStack) {
        self.apply_menu_components(stack);
    }

    fn resource_location(&self) -> &'static str {
        Self::ID
    }

    fn get_position(&self) -> BlockPos {
        self.position
    }

    fn from_nbt(nbt: &pumpkin_nbt::compound::NbtCompound, position: BlockPos) -> Self
    where
        Self: Sized,
    {
        let primary = nbt
            .get_string("primary_effect")
            .and_then(|s| {
                StatusEffect::from_minecraft_name(s)
                    .or_else(|| StatusEffect::from_name(s))
                    .map(|e| e.id as i32)
            })
            .or_else(|| nbt.get_int("primary_effect"))
            .unwrap_or(-1);
        let secondary = nbt
            .get_string("secondary_effect")
            .and_then(|s| {
                StatusEffect::from_minecraft_name(s)
                    .or_else(|| StatusEffect::from_name(s))
                    .map(|e| e.id as i32)
            })
            .or_else(|| nbt.get_int("secondary_effect"))
            .unwrap_or(-1);
        let levels = nbt.get_int("Levels").unwrap_or(0);
        let (custom_name, canonical_name) = menu::read_custom_name(nbt);
        let lock_key = nbt.get_string("Lock").map(std::string::ToString::to_string);
        let canonical_lock = super::container_lock::ContainerLock::from_nbt(nbt)
            .get()
            .map(|lock| (lock_key.clone(), lock));

        Self {
            position,
            primary_effect: AtomicI32::new(primary),
            secondary_effect: AtomicI32::new(secondary),
            levels: AtomicI32::new(levels),
            dirty: AtomicBool::new(false),
            effects_lock: pumpkin_util::sync::Mutex::new(()),
            beam_active: AtomicBool::new(false),
            checking_beam: AtomicBool::new(false),
            custom_name: Mutex::new(custom_name),
            lock_key: Mutex::new(lock_key),
            canonical_name: pumpkin_util::sync::Mutex::new(canonical_name),
            canonical_lock: pumpkin_util::sync::Mutex::new(canonical_lock),
            last_check_y: AtomicI32::new(position.0.y - 1),
        }
    }

    fn write_nbt(&self, nbt: &mut NbtCompound) {
        let (primary, secondary) = self.effects();
        if primary >= 0 {
            if let Some(eff) = <StatusEffect as IDSetContent>::from_id(primary as u16) {
                nbt.put_string("primary_effect", eff.minecraft_name.to_string());
            } else {
                nbt.put_int("primary_effect", primary);
            }
        }
        if secondary >= 0 {
            if let Some(eff) = <StatusEffect as IDSetContent>::from_id(secondary as u16) {
                nbt.put_string("secondary_effect", eff.minecraft_name.to_string());
            } else {
                nbt.put_int("secondary_effect", secondary);
            }
        }
        nbt.put_int("Levels", self.levels.load(Ordering::Relaxed));

        self.write_menu_nbt(nbt, false);
    }

    fn tick(&self, world: &Arc<World>) {
        self.update_beam(world);
        // Check properties every 80 ticks matching Java
        if world.get_time_of_day() % 80 == 0 {
            let levels = self.update_base(world);
            self.levels.store(levels, Ordering::Relaxed);

            if levels > 0 {
                self.apply_effects(world, levels);
            }
        }
    }

    fn chunk_data_nbt(&self) -> Option<NbtCompound> {
        let mut nbt = NbtCompound::new();
        let (primary, secondary) = self.effects();
        if primary >= 0 {
            if let Some(eff) = <StatusEffect as IDSetContent>::from_id(primary as u16) {
                nbt.put_string("primary_effect", eff.minecraft_name.to_string());
            } else {
                nbt.put_int("primary_effect", primary);
            }
        }
        if secondary >= 0 {
            if let Some(eff) = <StatusEffect as IDSetContent>::from_id(secondary as u16) {
                nbt.put_string("secondary_effect", eff.minecraft_name.to_string());
            } else {
                nbt.put_int("secondary_effect", secondary);
            }
        }
        nbt.put_int("Levels", self.levels.load(Ordering::Relaxed));
        self.write_menu_nbt(&mut nbt, true);
        Some(nbt)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
