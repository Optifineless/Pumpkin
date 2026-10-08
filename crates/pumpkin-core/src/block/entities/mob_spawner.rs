use std::sync::{
    Arc,
    atomic::{AtomicI32, Ordering},
};

use crossbeam::atomic::AtomicCell;
use pumpkin_data::{entity::EntityType, world::WorldEvent};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use rand::seq::IndexedRandom;

use crate::{
    block::entities::BlockEntity,
    entity::EntityBase,
    entity::mob::spawn::{SpawnReason, finalize_spawn_with_reason, load_spawn_entity},
    world::World,
};

pub struct MobSpawnerBlockEntity {
    pub position: BlockPos,
    pub delay: AtomicI32,
    pub max_delay: i32,
    pub min_delay: i32,
    pub spawn_count: i32,
    pub spawn_range: i32,
    pub max_nearby_entities: i32,
    pub required_player_range: i32,
    pub entity_type: AtomicCell<Option<&'static EntityType>>,
    spawn_data: std::sync::Mutex<Option<NbtCompound>>,
    spawn_potentials: Vec<NbtCompound>,
}

impl MobSpawnerBlockEntity {
    pub const ID: &'static str = "minecraft:mob_spawner";
    pub const DEFAULT_DELAY: i32 = 20;
    pub const DEFAULT_MAX_SPAWN_DELAY: i32 = 800;
    pub const DEFAULT_MIN_SPAWN_DELAY: i32 = 200;
    pub const DEFAULT_SPAWN_COUNT: i32 = 4;
    pub const DEFAULT_SPAWN_RANGE: i32 = 4;
    pub const DEFAULT_MAX_NEARBY_ENTITIES: i32 = 6;
    pub const DEFAULT_REQUIRED_PLAYER_RANGE: i32 = 16;

    #[must_use]
    pub const fn new(position: BlockPos, entity_type: Option<&'static EntityType>) -> Self {
        Self {
            position,
            delay: AtomicI32::new(Self::DEFAULT_DELAY),
            max_delay: Self::DEFAULT_MAX_SPAWN_DELAY,
            min_delay: Self::DEFAULT_MIN_SPAWN_DELAY,
            spawn_count: Self::DEFAULT_SPAWN_COUNT,
            spawn_range: Self::DEFAULT_SPAWN_RANGE,
            max_nearby_entities: Self::DEFAULT_MAX_NEARBY_ENTITIES,
            required_player_range: Self::DEFAULT_REQUIRED_PLAYER_RANGE,
            entity_type: AtomicCell::new(entity_type),
            spawn_data: std::sync::Mutex::new(None),
            spawn_potentials: Vec::new(),
        }
    }

    pub fn write_spawner_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put_short("Delay", self.delay.load(Ordering::Relaxed) as i16);
        nbt.put_short("MinSpawnDelay", self.min_delay as i16);
        nbt.put_short("MaxSpawnDelay", self.max_delay as i16);
        nbt.put_short("SpawnCount", self.spawn_count as i16);
        nbt.put_short("SpawnRange", self.spawn_range as i16);
        nbt.put_short("MaxNearbyEntities", self.max_nearby_entities as i16);
        nbt.put_short("RequiredPlayerRange", self.required_player_range as i16);

        // BaseSpawner.save retains SpawnData and the complete weighted SpawnPotentials list.
        if let Some(data) = self
            .spawn_data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
        {
            nbt.put_compound("SpawnData", data);
        } else if let Some(entity_type) = self.entity_type.load() {
            let mut spawn_entry = NbtCompound::new();

            let mut entity_nbt = NbtCompound::new();
            entity_nbt.put_string("id", format!("minecraft:{}", entity_type.resource_name));

            spawn_entry.put_compound("entity", entity_nbt);

            nbt.put_compound("SpawnData", spawn_entry);
        }
        nbt.put(
            "SpawnPotentials",
            NbtTag::List(
                self.spawn_potentials
                    .iter()
                    .cloned()
                    .map(NbtTag::Compound)
                    .collect(),
            ),
        );
    }
}

impl MobSpawnerBlockEntity {
    fn update_spawns(&self, world: &Arc<World>) {
        let min_delay = self.min_delay;
        let max_delay = self.max_delay;

        self.delay.store(
            if max_delay <= min_delay {
                min_delay
            } else {
                min_delay + rand::random_range(0..max_delay - min_delay)
            },
            Ordering::Relaxed,
        );
        world.add_synced_block_event(self.position, 1, 0);
        if let Some(data) = self.select_potential() {
            self.set_spawn_data(data);
        }
    }

    pub fn set_entity_type(&self, entity_type: &'static EntityType) {
        self.entity_type.store(Some(entity_type));
        let mut data = self.current_spawn_data();
        let mut entity = data.get_compound("entity").cloned().unwrap_or_default();
        entity.put_string("id", format!("minecraft:{}", entity_type.resource_name));
        data.put_compound("entity", entity);
        self.set_spawn_data(data);
    }

    fn set_spawn_data(&self, data: NbtCompound) {
        self.entity_type.store(
            data.get_compound("entity")
                .and_then(|entity| entity.get_string("id"))
                .and_then(EntityType::from_name),
        );
        *self
            .spawn_data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(data);
    }

    fn select_potential(&self) -> Option<NbtCompound> {
        self.spawn_potentials
            .choose_weighted(&mut rand::rng(), |entry| {
                entry.get_int("weight").unwrap_or(1).max(0)
            })
            .ok()
            .and_then(|entry| entry.get_compound("data"))
            .cloned()
    }

    fn current_spawn_data(&self) -> NbtCompound {
        if let Some(data) = self
            .spawn_data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
        {
            return data;
        }
        if let Some(data) = self.select_potential() {
            self.set_spawn_data(data.clone());
            return data;
        }
        let mut data = NbtCompound::new();
        if let Some(ty) = self.entity_type.load() {
            let mut entity = NbtCompound::new();
            entity.put_string("id", format!("minecraft:{}", ty.resource_name));
            data.put_compound("entity", entity);
        }
        data
    }

    fn has_no_configuration(entity: &NbtCompound) -> bool {
        entity.child_tags.len() == 1 && entity.get_string("id").is_some()
    }

    pub(super) fn custom_rules_pass(rules: &NbtCompound, block: u8, sky: u8) -> bool {
        // SpawnData.CustomSpawnRules uses block light and effective sky brightness.
        [("block_light_limit", block), ("sky_light_limit", sky)]
            .iter()
            .all(|(name, light)| {
                rules.get_compound(name).is_none_or(|range| {
                    let min = range.get_int("min_inclusive").unwrap_or(0);
                    let max = range.get_int("max_inclusive").unwrap_or(15);
                    (0..=15).contains(&min)
                        && (min..=15).contains(&max)
                        && (min..=max).contains(&i32::from(*light))
                })
            })
    }
}

impl BlockEntity for MobSpawnerBlockEntity {
    fn resource_location(&self) -> &'static str {
        Self::ID
    }

    fn get_position(&self) -> BlockPos {
        self.position
    }

    #[expect(
        clippy::too_many_lines,
        reason = "Keeps BaseSpawner.serverTick attempt ordering together for vanilla review."
    )]
    fn tick(&self, world: &Arc<World>) {
        // BaseSpawner.serverTick: activation, placement, per-attempt cap, conditional finalization.
        let center = self.position.to_centered_f64();
        let range = f64::from(self.required_player_range);
        if !world.level_info.load().game_rules.spawner_blocks_work
            || !world.players.load().iter().any(|p| {
                !p.is_spectator()
                    && !p.living_entity.dead.load(Ordering::Relaxed)
                    && p.get_entity().pos.load().squared_distance_to_vec(&center) < range * range
            })
        {
            return;
        }
        if self.delay.load(Ordering::Relaxed) == -1 {
            self.update_spawns(world);
        }
        if self.delay.load(Ordering::Relaxed) > 0 {
            self.delay.fetch_sub(1, Ordering::Relaxed);
            return;
        }
        let data = self.current_spawn_data();
        let Some(nbt) = data.get_compound("entity") else {
            return;
        };
        let Some(ty) = nbt.get_string("id").and_then(EntityType::from_name) else {
            self.update_spawns(world);
            return;
        };
        let rules = data.get_compound("custom_spawn_rules");
        let min = self.position.0.to_f64();
        let bounds = pumpkin_util::math::boundingbox::BoundingBox {
            min,
            max: min.add_raw(1.0, 1.0, 1.0),
        }
        .expand(
            f64::from(self.spawn_range),
            f64::from(self.spawn_range),
            f64::from(self.spawn_range),
        );
        let mut completed = false;
        for _ in 0..self.spawn_count {
            let configured_pos = crate::entity::mob::spawn::configured_spawn_position(nbt);
            let spawn_pos = configured_pos.unwrap_or_else(|| {
                Vector3::new(
                    center.x
                        + (rand::random::<f64>() - rand::random::<f64>())
                            * f64::from(self.spawn_range),
                    f64::from(self.position.0.y + rand::random_range(0..3) - 1),
                    center.z
                        + (rand::random::<f64>() - rand::random::<f64>())
                            * f64::from(self.spawn_range),
                )
            });
            if !spawn_pos.x.is_finite() || !spawn_pos.y.is_finite() || !spawn_pos.z.is_finite() {
                continue;
            }
            if !world.is_space_empty(ty.get_spawn_bounding_box(
                spawn_pos.x,
                spawn_pos.y,
                spawn_pos.z,
            )) {
                continue;
            }
            let pos = BlockPos::floored_v(spawn_pos);
            if let Some(rules) = rules {
                if !ty.category.is_friendly
                    && world.level_info.load().difficulty == pumpkin_util::Difficulty::Peaceful
                {
                    continue;
                }
                if !Self::custom_rules_pass(
                    rules,
                    world.get_block_light_level(&pos).unwrap_or(0),
                    world
                        .get_sky_light_level(&pos)
                        .saturating_sub(world.get_sky_darken() as u8),
                ) {
                    continue;
                }
            } else if !crate::entity::r#type::check_spawn_rules_with_reason(
                ty,
                world,
                &pos,
                world.is_thundering(),
                SpawnReason::Spawner,
            ) {
                continue;
            }
            let Some(entity) = load_spawn_entity(world, nbt, spawn_pos) else {
                self.update_spawns(world);
                return;
            };
            let mut riding_tree = crate::entity::spawn_mount::UnpublishedRidingTree::new(&entity);
            // Recount after every successful insertion, using the inflated block AABB.
            let count = world
                .get_entities_at_box(&bounds)
                .iter()
                .filter(|e| e.get_entity().entity_type == ty)
                .count();
            if count as i32 >= self.max_nearby_entities {
                self.update_spawns(world);
                return;
            }
            entity
                .get_entity()
                .set_rotation(rand::random::<f32>() * 360.0, 0.0);
            if let Some(mob) = entity.get_mob()
                && ((rules.is_none() && !mob.check_spawn_rules(world, SpawnReason::Spawner))
                    || !mob.check_spawn_obstruction(world))
            {
                continue;
            }
            let mut event =
                crate::plugin::api::events::entity::spawner_spawn::SpawnerSpawnEvent::new(
                    entity.get_entity().entity_id,
                    self.position,
                );
            if let Some(server) = world.server.upgrade() {
                server.plugin_manager.fire_blocking(&server, &mut event);
            }
            if event.cancelled {
                completed = true;
                continue;
            }
            if let Some(mob) = entity.get_mob() {
                if Self::has_no_configuration(nbt) {
                    finalize_spawn_with_reason(&entity, world, SpawnReason::Spawner, None);
                }
                if let Some(equipment) = data.get_compound("equipment") {
                    crate::entity::mob::equipment::equip_from_spawn_data(mob, world, equipment);
                }
            }
            if world.spawn_entity_with_passengers(&entity) {
                riding_tree.keep_links();
                world.sync_world_event(WorldEvent::ParticlesMobblockSpawn, self.position, 0);
                completed = true;
            } else {
                self.update_spawns(world);
                return;
            }
        }
        if completed {
            self.update_spawns(world);
        }
    }

    fn from_nbt(nbt: &pumpkin_nbt::compound::NbtCompound, position: BlockPos) -> Self
    where
        Self: Sized,
    {
        let get_num = |name: &str| {
            nbt.get_short(name)
                .map(i32::from)
                .or_else(|| nbt.get_int(name))
                .or_else(|| nbt.get_byte(name).map(i32::from))
        };

        let delay = get_num("Delay").unwrap_or(Self::DEFAULT_DELAY);
        let min_delay = get_num("MinSpawnDelay").unwrap_or(Self::DEFAULT_MIN_SPAWN_DELAY);
        let max_delay = get_num("MaxSpawnDelay").unwrap_or(Self::DEFAULT_MAX_SPAWN_DELAY);
        let spawn_count = get_num("SpawnCount").unwrap_or(Self::DEFAULT_SPAWN_COUNT);
        let spawn_range = get_num("SpawnRange").unwrap_or(Self::DEFAULT_SPAWN_RANGE);
        let max_nearby_entities =
            get_num("MaxNearbyEntities").unwrap_or(Self::DEFAULT_MAX_NEARBY_ENTITIES);
        let required_player_range =
            get_num("RequiredPlayerRange").unwrap_or(Self::DEFAULT_REQUIRED_PLAYER_RANGE);

        let entity_type = nbt
            .get_compound("SpawnData")
            .and_then(|data| {
                data.get_compound("entity")
                    .and_then(|entity| entity.get_string("id"))
                    .or_else(|| data.get_string("id"))
            })
            .or_else(|| {
                nbt.get_list("SpawnPotentials")
                    .and_then(|list| list.first())
                    .and_then(|tag| tag.extract_compound())
                    .and_then(|entry| {
                        entry
                            .get_compound("data")
                            .and_then(|data| {
                                data.get_compound("entity")
                                    .and_then(|entity| entity.get_string("id"))
                                    .or_else(|| data.get_string("id"))
                            })
                            .or_else(|| {
                                entry
                                    .get_compound("entity")
                                    .and_then(|entity| entity.get_string("id"))
                            })
                    })
            })
            .or_else(|| nbt.get_string("EntityId"))
            .and_then(EntityType::from_name);

        let spawn_data = nbt.get_compound("SpawnData").cloned();
        let spawn_potentials = nbt.get_list("SpawnPotentials").map_or_else(
            || {
                spawn_data
                    .iter()
                    .map(|data| {
                        let mut entry = NbtCompound::new();
                        entry.put_compound("data", data.clone());
                        entry.put_int("weight", 1);
                        entry
                    })
                    .collect()
            },
            |list| {
                list.iter()
                    .filter_map(NbtTag::extract_compound)
                    .cloned()
                    .collect()
            },
        );

        Self {
            position,
            delay: AtomicI32::new(delay),
            max_delay,
            min_delay,
            spawn_count,
            spawn_range,
            max_nearby_entities,
            required_player_range,
            entity_type: AtomicCell::new(entity_type),
            spawn_data: std::sync::Mutex::new(spawn_data),
            spawn_potentials,
        }
    }

    fn write_nbt(&self, nbt: &mut NbtCompound) {
        self.write_spawner_nbt(nbt);
    }

    fn chunk_data_nbt(&self) -> Option<NbtCompound> {
        let mut nbt = NbtCompound::new();
        self.write_spawner_nbt(&mut nbt);
        Some(nbt)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_spawn_data_and_weighted_potentials_survive_reload() {
        let mut entity = NbtCompound::new();
        entity.put_string("id", "minecraft:zombie".to_string());
        entity.put_bool("IsBaby", true);
        let mut passenger = NbtCompound::new();
        passenger.put_string("id", "minecraft:chicken".to_string());
        entity.put(
            "Passengers",
            NbtTag::List(vec![NbtTag::Compound(passenger)]),
        );
        let mut rules = NbtCompound::new();
        let mut light = NbtCompound::new();
        light.put_int("min_inclusive", 0);
        light.put_int("max_inclusive", 7);
        rules.put_compound("block_light_limit", light);
        let mut data = NbtCompound::new();
        data.put_compound("entity", entity);
        data.put_compound("custom_spawn_rules", rules);
        let mut equipment = NbtCompound::new();
        equipment.put_string(
            "loot_table",
            "minecraft:equipment/trial_chamber".to_string(),
        );
        equipment.put_float("slot_drop_chances", 2.0);
        data.put_compound("equipment", equipment);
        let mut potential = NbtCompound::new();
        potential.put_int("weight", 7);
        potential.put_compound("data", data.clone());
        let mut input = NbtCompound::new();
        input.put_compound("SpawnData", data.clone());
        input.put(
            "SpawnPotentials",
            NbtTag::List(vec![NbtTag::Compound(potential)]),
        );
        let spawner = MobSpawnerBlockEntity::from_nbt(&input, BlockPos::new(0, 64, 0));
        let mut output = NbtCompound::new();
        spawner.write_spawner_nbt(&mut output);
        assert_eq!(output.get_compound("SpawnData"), Some(&data));
        assert_eq!(
            output.get_list("SpawnPotentials"),
            input.get_list("SpawnPotentials")
        );
        assert!(!MobSpawnerBlockEntity::has_no_configuration(
            data.get_compound("entity").unwrap_or(&NbtCompound::new())
        ));
        assert_eq!(spawner.select_potential(), Some(data));
    }

    #[test]
    fn custom_light_ranges_include_both_boundaries() {
        let mut rules = NbtCompound::new();
        let mut range = NbtCompound::new();
        range.put_int("min_inclusive", 3);
        range.put_int("max_inclusive", 7);
        rules.put_compound("sky_light_limit", range);
        assert!(MobSpawnerBlockEntity::custom_rules_pass(&rules, 0, 3));
        assert!(MobSpawnerBlockEntity::custom_rules_pass(&rules, 15, 7));
        assert!(!MobSpawnerBlockEntity::custom_rules_pass(&rules, 0, 2));
        assert!(!MobSpawnerBlockEntity::custom_rules_pass(&rules, 0, 8));
    }
}
