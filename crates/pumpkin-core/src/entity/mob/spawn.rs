//! Vanilla `Mob.finalizeSpawn`: the step run on freshly created mobs before they enter the world.

use std::sync::Arc;

pub use super::spawn_variants::inherit_breeding_variant;

use pumpkin_data::attributes::Attributes;
use pumpkin_data::effect::StatusEffect;
use rand::RngExt;

use crate::entity::EntityBase;
use crate::entity::attributes::{Modifier, ModifierOperation};
use crate::entity::mob::{Mob, MobEntity};
use crate::world::World;
use pumpkin_data::{
    Block, BlockDirection,
    entity::EntityType,
    tag::{self, Taggable},
};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_util::math::vector3::Vector3;

const RANDOM_SPAWN_BONUS_ID: &str = "minecraft:random_spawn_bonus";

/// EntityGetter.isUnobstructed for spawning, excluding spectators and the mob's riding tree.
pub fn is_unobstructed_for_spawn(world: &World, entity: &crate::entity::Entity) -> bool {
    use pumpkin_data::{entity::EntityType, tag::Taggable};
    fn root_vehicle(entity: &crate::entity::Entity) -> i32 {
        let mut id = entity.entity_id;
        let mut vehicle = entity.get_vehicle();
        while let Some(current) = vehicle {
            id = current.get_entity().entity_id;
            vehicle = current.get_entity().get_vehicle();
        }
        id
    }
    let root = root_vehicle(entity);
    world
        .get_all_at_box(&entity.bounding_box.load())
        .iter()
        .all(|other| {
            let base = other.get_entity();
            let ty = base.entity_type;
            // blocksBuilding is set by LivingEntity and these non-living constructors in vanilla.
            let blocks_building = other.get_living_entity().is_some()
                || ty.has_tag(&pumpkin_data::tag::EntityType::MINECRAFT_BOAT)
                || [
                    EntityType::MINECART.id,
                    EntityType::CHEST_MINECART.id,
                    EntityType::FURNACE_MINECART.id,
                    EntityType::HOPPER_MINECART.id,
                    EntityType::TNT_MINECART.id,
                    EntityType::SPAWNER_MINECART.id,
                    EntityType::COMMAND_BLOCK_MINECART.id,
                    EntityType::TNT.id,
                    EntityType::FALLING_BLOCK.id,
                    EntityType::END_CRYSTAL.id,
                ]
                .contains(&ty.id);
            base.entity_id == entity.entity_id
                || other.is_spectator()
                || base.removed.load(std::sync::atomic::Ordering::Relaxed)
                || !blocks_building
                || root_vehicle(base) == root
        })
}

/// Local brightness for Monster.isDarkEnoughToSpawn after its separate raw-sky/block tests.
#[must_use]
pub const fn monster_spawn_brightness(
    sky_light: u8,
    block_light: u8,
    sky_darken: u8,
    is_thundering: bool,
) -> u8 {
    let darkened_sky = sky_light.saturating_sub(if is_thundering { 10 } else { sky_darken });
    if darkened_sky > block_light {
        darkened_sky
    } else {
        block_light
    }
}

/// State shared by every mob of one spawn group (vanilla `SpawnGroupData`).
pub enum SpawnGroupData {
    /// The effect the first spider of a group rolled on hard difficulty.
    SpiderEffects(Option<&'static StatusEffect>),
    Ageable(AgeableGroupData),
    Wolf {
        variant: &'static str,
        ageable: AgeableGroupData,
    },
    Zombie {
        is_baby: bool,
        can_spawn_jockey: bool,
    },
}

/// Baby selection state passed from one ageable mob to the next in a spawn group.
pub struct AgeableGroupData {
    pub group_size: u32,
    pub adult_count: u32,
    pub should_spawn_baby: bool,
    pub baby_spawn_chance: f32,
}

impl AgeableGroupData {
    #[must_use]
    pub const fn new(should_spawn_baby: bool, baby_spawn_chance: f32) -> Self {
        Self {
            group_size: 0,
            adult_count: 1,
            should_spawn_baby,
            baby_spawn_chance,
        }
    }

    /// Records a group member and reports whether it should be a baby.
    pub fn next_is_baby(&mut self, roll: f32) -> bool {
        // AgeableMob.finalizeSpawn never makes the first group member a baby.
        let baby = self.should_spawn_baby
            && self.group_size >= self.adult_count
            && roll <= self.baby_spawn_chance;
        self.group_size += 1;
        baby
    }
}

/// Creates the species' initial `AgeableMob` group state; subtype finalizers may supply their own.
#[must_use]
pub fn ageable_group_data(entity_type: &pumpkin_data::entity::EntityType) -> AgeableGroupData {
    // The chances/disabled babies are Java constants in these species' finalizeSpawn overrides.
    match entity_type.resource_name {
        "ocelot" | "polar_bear" | "rabbit" => AgeableGroupData::new(true, 1.0),
        "panda" | "donkey" | "mule" | "skeleton_horse" | "zombie_horse" => {
            AgeableGroupData::new(true, 0.2)
        }
        "dolphin" => AgeableGroupData::new(true, 0.1),
        "axolotl" => AgeableGroupData {
            adult_count: 2,
            ..AgeableGroupData::new(true, 1.0)
        },
        "strider" => AgeableGroupData::new(true, 0.5),
        "parrot" | "villager" | "wandering_trader" | "trader_llama" | "fox" | "wolf" => {
            AgeableGroupData::new(false, 0.05)
        }
        _ => AgeableGroupData::new(true, 0.05),
    }
}

/// Vanilla `EntitySpawnReason`, supplied only when finalizing a newly created mob.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpawnReason {
    Natural,
    Event,
    Triggered,
    MobSummoned,
    SpawnBucket,
    ChunkGeneration,
    Spawner,
    TrialSpawner,
    Structure,
    Breeding,
    Jockey,
    SpawnItemUse,
    Command,
    Conversion,
    Reinforcement,
    Load,
    DimensionTravel,
}

impl SpawnReason {
    #[must_use]
    pub const fn is_spawner(self) -> bool {
        matches!(self, Self::Spawner | Self::TrialSpawner)
    }

    #[must_use]
    pub const fn ignores_light_requirements(self) -> bool {
        matches!(self, Self::TrialSpawner)
    }
}

impl MobEntity {
    /// Vanilla `Mob.finalizeSpawn`: random follow range bonus and left-handedness.
    pub fn finalize_spawn_base(&self) {
        let mut rng = rand::rng();
        {
            let mut attributes = self
                .living_entity
                .attributes
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(follow_range) = attributes.get_mut(&Attributes::FOLLOW_RANGE.id)
                && !follow_range
                    .modifiers
                    .iter()
                    .any(|modifier| modifier.id == RANDOM_SPAWN_BONUS_ID)
            {
                follow_range.add_or_replace_modifier(Modifier {
                    id: RANDOM_SPAWN_BONUS_ID.to_string(),
                    // Mob.finalizeSpawn uses RandomSource.triangle(double, double).
                    amount: 0.114_850_000_000_000_01 * (rng.random::<f64>() - rng.random::<f64>()),
                    operation: ModifierOperation::MultiplyBase,
                    // Vanilla `Mob.finalizeSpawn` adds this as a permanent modifier.
                    permanent: true,
                });
            }
        }
        self.set_left_handed(rng.random::<f32>() < 0.05);
    }

    /// Queues a mob to be added and mounted onto this one when it enters the world.
    pub fn add_pending_rider(&self, rider: Arc<dyn EntityBase>) {
        self.pending_riders
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(rider);
    }

    pub(crate) fn take_pending_riders(&self) -> Vec<Arc<dyn EntityBase>> {
        std::mem::take(
            &mut *self
                .pending_riders
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }
}

/// Finalizes `entity` if it is a mob, and returns the group data for the next mob of the group.
pub fn finalize_spawn(
    entity: &Arc<dyn EntityBase>,
    world: &Arc<World>,
    group_data: Option<SpawnGroupData>,
) -> Option<SpawnGroupData> {
    finalize_spawn_with_reason(entity, world, SpawnReason::SpawnItemUse, group_data)
}

/// Finalizes fresh mobs with regional difficulty and reason; loading never calls this helper.
pub fn finalize_spawn_with_reason(
    entity: &Arc<dyn EntityBase>,
    world: &Arc<World>,
    reason: SpawnReason,
    group_data: Option<SpawnGroupData>,
) -> Option<SpawnGroupData> {
    finalize_spawn_in_view(
        entity,
        world,
        &crate::world::spawn_view::SpawnView::live(world),
        reason,
        group_data,
    )
}

/// Finalizes a fresh entity using the same environmental accessor as placement and obstruction.
pub fn finalize_spawn_in_view(
    entity: &Arc<dyn EntityBase>,
    _world: &Arc<World>,
    view: &crate::world::spawn_view::SpawnView<'_>,
    reason: SpawnReason,
    group_data: Option<SpawnGroupData>,
) -> Option<SpawnGroupData> {
    let difficulty = view.difficulty_at(entity.get_entity().pos.load());
    // Cat.finalizeSpawn selects variants after its superclass; the other SpawnContext users do so before.
    let variants_after_super = entity.get_entity().entity_type == &EntityType::CAT;
    let group_data = if variants_after_super {
        group_data
    } else {
        super::spawn_variants::finalize_variants(entity, view, group_data)
    };
    let group_data = match entity.get_mob() {
        Some(mob) => mob.finalize_spawn_with_context(entity, view, &difficulty, reason, group_data),
        None => group_data,
    };
    if variants_after_super {
        super::spawn_variants::finalize_variants(entity, view, group_data)
    } else {
        group_data
    }
}

/// Attempts Zombie.finalizeSpawn's existing-chicken or fresh-chicken mount for a baby group member.
#[expect(
    clippy::same_functions_in_if_condition,
    reason = "Zombie.finalizeSpawn makes two independent rolls in this else-if."
)]
pub fn try_chicken_jockey(
    zombie: &Arc<dyn EntityBase>,
    world: &Arc<World>,
    view: &crate::world::spawn_view::SpawnView<'_>,
) {
    use pumpkin_data::entity::EntityType;
    let bounds = zombie
        .get_entity()
        .bounding_box
        .load()
        .expand(5.0, 3.0, 5.0);
    if rand::random::<f32>() < 0.05 {
        if let Some(chicken) = (if view.cache.is_some() {
            Vec::new()
        } else {
            world.get_entities_at_box(&bounds)
        })
        .into_iter()
        .find(crate::entity::spawn_mount::available_chicken)
        {
            crate::entity::spawn_mount::queue_existing_chicken(zombie, chicken);
        }
    } else if rand::random::<f32>() < 0.05 {
        let chicken = crate::entity::r#type::from_type(
            &EntityType::CHICKEN,
            zombie.get_entity().pos.load(),
            world,
            uuid::Uuid::new_v4(),
        );
        chicken
            .get_entity()
            .set_rotation(zombie.get_entity().yaw.load(), 0.0);
        finalize_spawn_in_view(&chicken, world, view, SpawnReason::Jockey, None);
        if let Some(mob) = chicken.get_mob() {
            mob.set_chicken_jockey(true);
        }
        crate::entity::spawn_mount::queue_spawn_mount(zombie, chicken);
    }
}

/// Restores a configured entity and its passengers without running spawn finalization.
/// Rejected trees must be owned by an `UnpublishedRidingTree` until insertion or serialization.
pub fn load_spawn_entity(
    world: &Arc<World>,
    nbt: &NbtCompound,
    position: Vector3<f64>,
) -> Option<Arc<dyn EntityBase>> {
    fn load(
        world: &Arc<World>,
        nbt: &NbtCompound,
        position: Vector3<f64>,
        depth: usize,
    ) -> Option<Arc<dyn EntityBase>> {
        if depth >= pumpkin_nbt::MAX_NBT_DEPTH {
            return None;
        }
        let ty = pumpkin_data::entity::EntityType::from_name(nbt.get_string("id")?)?;
        let entity = crate::entity::r#type::from_type(
            ty,
            position,
            world,
            nbt.get_uuid("UUID").unwrap_or_else(uuid::Uuid::new_v4),
        );
        let mut riding_tree = crate::entity::spawn_mount::UnpublishedRidingTree::new(&entity);
        entity.read_nbt_non_mut(nbt);
        entity.get_entity().set_pos(position);
        if let Some(passengers) = nbt.get_list("Passengers") {
            for data in passengers.iter().filter_map(NbtTag::extract_compound) {
                let Some(passenger) = load(
                    world,
                    data,
                    configured_spawn_position(data).unwrap_or(position),
                    depth + 1,
                ) else {
                    // EntityType.loadPassengersRecursive skips unreadable passengers.
                    continue;
                };
                crate::entity::spawn_mount::attach_unpublished(&entity, passenger);
            }
        }
        riding_tree.keep_links();
        Some(entity)
    }
    load(world, nbt, position, 0)
}

/// Reads the finite three-number `Pos` list used by entity and spawner codecs.
#[must_use]
pub fn configured_spawn_position(nbt: &NbtCompound) -> Option<Vector3<f64>> {
    let list = nbt.get_list("Pos").filter(|list| list.len() == 3)?;
    let pos = Vector3::new(
        list[0].as_numeric_double()?,
        list[1].as_numeric_double()?,
        list[2].as_numeric_double()?,
    );
    (pos.x.is_finite() && pos.y.is_finite() && pos.z.is_finite()).then_some(pos)
}

impl MobEntity {
    /// Baby state kept only as a negative age and the shared baby flag (hoglins, zoglins and
    /// zombified piglins, which are not `AgeableMob`s here).
    pub fn set_baby_by_age(&self) {
        let entity = &self.living_entity.entity;
        entity
            .age
            .store(-24000, std::sync::atomic::Ordering::Relaxed);
        entity.set_synced_data(pumpkin_data::tracked_data::ageable_mob::DATA_BABY_ID, true);
    }

    /// Baby state kept as a mob's own flag and synced key (vanilla zombies and piglins), so it
    /// never ages up.
    pub fn set_baby_flag(
        &self,
        flag: &std::sync::atomic::AtomicBool,
        tracked: pumpkin_data::tracked_data::TrackedData,
        baby: bool,
    ) {
        flag.store(baby, std::sync::atomic::Ordering::Relaxed);
        let entity = &self.living_entity.entity;
        entity.set_synced_data(tracked, baby);
        // Pumpkin's negative age is what reports the Bedrock baby flag.
        entity.age.store(
            if baby { -24000 } else { 0 },
            std::sync::atomic::Ordering::Relaxed,
        );
    }
}

// Mob.checkSpawnObstruction with the WaterAnimal/Ravager/IronGolem/Ocelot overrides.
pub(super) fn check_spawn_obstruction<M: Mob + ?Sized>(mob: &M, world: &World) -> bool {
    check_spawn_obstruction_in_view(mob, &crate::world::spawn_view::SpawnView::live(world))
}

pub(crate) fn check_spawn_obstruction_in_view<M: Mob + ?Sized>(
    mob: &M,
    world: &crate::world::spawn_view::SpawnView<'_>,
) -> bool {
    let entity = mob.get_entity();
    let bounds = entity.bounding_box.load();
    let water_mob = matches!(
        entity.entity_type.spawn_restriction.location,
        pumpkin_data::entity::SpawnLocation::InWater
    ) || entity.entity_type == &EntityType::STRIDER
        || entity.entity_type == &EntityType::IRON_GOLEM;
    // Mob.checkSpawnObstruction; WaterAnimal.checkSpawnObstruction permits water.
    if !water_mob && world.contains_any_liquid(bounds) {
        return false;
    }
    // Ravager.checkSpawnObstruction only tests liquid.
    if entity.entity_type == &EntityType::RAVAGER {
        return true;
    }
    // WorldGenRegion.getEntities returns an empty list during generation.
    let unobstructed = world.cache.is_some() || is_unobstructed_for_spawn(world, entity);
    if !unobstructed {
        return false;
    }
    if entity.entity_type == &EntityType::IRON_GOLEM {
        // IronGolem.checkSpawnObstruction checks support/two head blocks, ignoring fluid at its feet.
        let pos = entity.block_pos.load();
        return world
            .get_block_state(&pos.down())
            .is_side_solid(BlockDirection::Up)
            && (1..3).all(|dy| {
                crate::world::natural_spawner::is_valid_empty_spawn_block(
                    world,
                    world.get_block_state(&pos.add(0, dy, 0)),
                    entity.entity_type,
                )
            })
            && crate::world::natural_spawner::is_valid_empty_spawn_block_with_fluid(
                world,
                world.get_block_state(&pos),
                entity.entity_type,
                false,
            );
    }
    if entity.entity_type == &EntityType::OCELOT {
        // Ocelot.checkSpawnObstruction requires grass/leaves at or above sea level.
        let pos = entity.block_pos.load();
        let below = world.get_block(&pos.down());
        return pos.0.y >= world.sea_level
            && (below == &Block::GRASS_BLOCK || below.has_tag(&tag::Block::MINECRAFT_LEAVES));
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_spawn_position_rejects_malformed_and_nonfinite_lists() {
        use pumpkin_nbt::tag::NbtTag;
        let read = |tags| {
            let mut nbt = NbtCompound::new();
            nbt.put("Pos", NbtTag::List(tags));
            configured_spawn_position(&nbt)
        };
        assert_eq!(
            read(vec![
                NbtTag::Double(14.5),
                NbtTag::Double(67.0),
                NbtTag::Double(-22.25)
            ]),
            Some(Vector3::new(14.5, 67.0, -22.25))
        );
        assert_eq!(
            read(vec![NbtTag::Int(1), NbtTag::Int(64), NbtTag::Int(-2)]),
            Some(Vector3::new(1.0, 64.0, -2.0))
        );
        assert!(read(vec![NbtTag::Double(0.0); 2]).is_none());
        assert!(read(vec![NbtTag::Double(0.0); 4]).is_none());
        assert!(read(vec![NbtTag::String("bad".into()); 3]).is_none());
        assert!(read(vec![NbtTag::Double(f64::NAN); 3]).is_none());
        assert!(read(vec![NbtTag::Double(f64::INFINITY); 3]).is_none());
    }

    #[test]
    fn ageable_groups_start_with_an_adult_and_use_the_species_chance() {
        let mut data = AgeableGroupData::new(true, 0.05);
        assert!(!data.next_is_baby(0.0));
        assert!(data.next_is_baby(0.04));
        assert!(!data.next_is_baby(0.06));
        let mut ocelots = AgeableGroupData::new(true, 1.0);
        assert!(!ocelots.next_is_baby(0.0));
        assert!(ocelots.next_is_baby(0.99));
        let mut parrots = AgeableGroupData::new(false, 0.05);
        assert!(!parrots.next_is_baby(0.0));
        assert!(!parrots.next_is_baby(0.0));
    }
}
