use super::MAX_LOOT_STACKS;
use crate::entity::EntityBase;
use pumpkin_data::{BlockState, damage::DamageType, entity::EntityType, item_stack::ItemStack};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashSet},
    sync::atomic::Ordering::Relaxed,
};
const MAX_ENTITY_RELATION_DEPTH: usize = 16;

#[derive(Default, Clone)]
pub struct LootContextParameters {
    pub world: Option<std::sync::Arc<crate::world::World>>,
    pub registry: Option<std::sync::Arc<crate::data::datapack::DatapackManager>>,
    pub explosion_radius: Option<f32>,
    pub block_state: Option<&'static BlockState>,
    /// Death/XP credit supplied by the caller; loot conditions use `last_damage_player_state`.
    pub killed_by_player: Option<bool>,
    pub luck: f32,
    pub this_entity: Option<&'static EntityType>,
    pub killer_entity: Option<&'static EntityType>,
    pub direct_killer_entity: Option<&'static EntityType>,
    pub position: Option<pumpkin_util::math::vector3::Vector3<f64>>,
    pub world_time: u64,
    pub damage_type: Option<DamageType>,
    /// Block loot treats `None` as an empty tool; other contexts may omit `TOOL`.
    pub tool: Option<ItemStack>,
    pub is_raining: Option<bool>,
    pub is_thundering: Option<bool>,
    pub is_on_fire: Option<bool>,
    pub this_entity_state: Option<EntityLootState>,
    pub attacking_entity_state: Option<EntityLootState>,
    pub direct_attacking_entity_state: Option<EntityLootState>,
    pub last_damage_player_state: Option<EntityLootState>,
    /// Components collected from `BLOCK_ENTITY` by the block-drop caller.
    pub block_entity_components: std::collections::HashMap<
        pumpkin_data::data_component::DataComponent,
        Box<dyn pumpkin_data::data_component_impl::DataComponentImpl>,
    >,
}

/// A death-time snapshot, so loot evaluation never holds entity or equipment locks.
#[derive(Default, Clone)]
pub struct EntityLootState {
    /// Identity used to resolve vehicle/passenger back references in the snapshot.
    pub entity_id: Option<i32>,
    pub entity_type: Option<&'static EntityType>,
    /// Whether vanilla's `LivingEntity` flag restrictions apply.
    pub is_living: Option<bool>,
    pub components: BTreeMap<String, Value>,
    pub flags: BTreeMap<String, bool>,
    pub sheared: Option<bool>,
    /// `FishingHookPredicate` reads the hook's latched open-water state.
    pub fishing_open_water: Option<bool>,
    pub equipment: BTreeMap<String, ItemStack>,
    pub vehicle: Option<Box<Self>>,
    pub passengers: Vec<Self>,
}

/// Build the live entity-death loot parameters.
///
/// The caller supplies kill credit and tool in `base`; this preserves those values and
/// all drop eligibility/XP decisions. `last_hurt_by_player` is the credited player, if known.
/// Pass seed zero to generation for unseeded deaths to use the named sequence.
#[must_use]
pub fn build_entity_death_loot_context(
    entity: &dyn EntityBase,
    killer: Option<&dyn EntityBase>,
    direct_killer: Option<&dyn EntityBase>,
    last_hurt_by_player: Option<&dyn EntityBase>,
    base: &LootContextParameters,
) -> LootContextParameters {
    // LivingEntity.dropFromLootTable supplies the live entity, rather than its type.
    let snapshot = |entity| snapshot_entity(entity, &mut HashSet::new(), 0);
    // Credit is resolved server-wide by the death caller; never look it up by local entity ID.
    let last_player = last_hurt_by_player.filter(|_| base.killed_by_player == Some(true));
    LootContextParameters {
        world: Some(entity.get_entity().world.load_full()),
        registry: entity
            .get_entity()
            .world
            .load()
            .server
            .upgrade()
            .map(|server| server.datapack_manager.clone()),
        this_entity: Some(entity.get_entity().entity_type),
        killer_entity: killer.map(|killer| killer.get_entity().entity_type),
        direct_killer_entity: direct_killer.map(|killer| killer.get_entity().entity_type),
        this_entity_state: snapshot(entity),
        attacking_entity_state: killer.and_then(snapshot),
        direct_attacking_entity_state: direct_killer.and_then(snapshot),
        last_damage_player_state: last_player.and_then(snapshot),
        luck: last_player
            .and_then(EntityBase::get_living_entity)
            .map_or(base.luck, |player| {
                player.get_attribute_value(&pumpkin_data::attributes::Attributes::LUCK) as f32
            }),
        ..base.clone()
    }
}

pub(super) fn snapshot_entity(
    entity: &dyn EntityBase,
    seen: &mut HashSet<i32>,
    depth: usize,
) -> Option<EntityLootState> {
    let raw = entity.get_entity();
    if depth >= MAX_ENTITY_RELATION_DEPTH {
        return None;
    }
    let first_visit = seen.insert(raw.entity_id);
    let mut state = EntityLootState {
        entity_id: Some(raw.entity_id),
        entity_type: Some(raw.entity_type),
        is_living: Some(entity.get_living_entity().is_some()),
        ..Default::default()
    };
    // EntityFlagsPredicate.matches and SheepPredicate.matches.
    // MobEntity.set_baby_flag also mirrors non-ageable baby flags into negative age.
    let baby = entity.get_mob().is_some() && raw.age.load(Relaxed) < 0;
    state.flags.extend([
        ("is_on_fire".to_owned(), raw.is_on_fire()),
        ("is_on_ground".to_owned(), raw.on_ground.load(Relaxed)),
        ("is_sneaking".to_owned(), raw.is_sneaking()),
        ("is_sprinting".to_owned(), raw.is_sprinting()),
        ("is_swimming".to_owned(), raw.is_swimming()),
        ("is_in_water".to_owned(), raw.is_in_water()),
        ("is_fall_flying".to_owned(), raw.is_fall_flying()),
        (
            "is_flying".to_owned(),
            entity.get_living_entity().is_some()
                && (raw.is_fall_flying()
                    || entity
                        .get_player()
                        .is_some_and(crate::entity::player::Player::is_flying)),
        ),
        ("is_baby".to_owned(), baby),
    ]);
    snapshot_entity_components(entity, &mut state);
    if let Some(hook) = entity
        .cast_any()
        .downcast_ref::<crate::entity::projectile::fishing_bobber::FishingBobberEntity>(
    ) {
        state.fishing_open_water = Some(hook.is_open_water_fishing());
    }
    if let Some(living) = entity.get_living_entity() {
        state
            .equipment
            .insert("mainhand".to_owned(), living.held_item(entity));
        state
            .equipment
            .insert("offhand".to_owned(), living.off_hand_item(entity));
        let equipment = living
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (slot, stack) in &equipment.equipment {
            if slot.to_name().as_ref() == "mainhand" || slot.to_name().as_ref() == "offhand" {
                continue;
            }
            state
                .equipment
                .insert(slot.to_name().to_string(), stack.clone());
        }
    }
    // VehiclePredicate/PassengerPredicate follow a live graph, including rider back references.
    if !first_visit {
        return Some(state);
    }
    let vehicle = raw
        .vehicle
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let passengers = raw
        .passengers
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    state.vehicle = vehicle
        .and_then(|v| snapshot_entity(v.as_ref(), seen, depth + 1))
        .map(Box::new);
    state.passengers = passengers
        .iter()
        .take(MAX_LOOT_STACKS)
        .filter_map(|p| snapshot_entity(p.as_ref(), seen, depth + 1))
        .collect();
    seen.remove(&raw.entity_id);
    Some(state)
}

fn snapshot_entity_components(entity: &dyn EntityBase, state: &mut EntityLootState) {
    // Sheep.get, Chicken.get and MushroomCow.get expose their live component values.
    if let Some(sheep) = entity
        .cast_any()
        .downcast_ref::<crate::entity::passive::sheep::SheepEntity>()
    {
        let color = pumpkin_data::dye_color::DyeColor::by_id(sheep.get_color());
        if let Some(color) = color {
            state.components.insert(
                "minecraft:sheep/color".to_owned(),
                Value::from(color.name()),
            );
        }
        state.sheared = Some(sheep.is_sheared());
    }
    if let Some(chicken) = entity
        .cast_any()
        .downcast_ref::<crate::entity::passive::chicken::ChickenEntity>()
        && let Some(variant) =
            pumpkin_data::chicken_variant::ChickenVariant::from_id(chicken.variant.load(Relaxed))
    {
        state.components.insert(
            "minecraft:chicken/variant".to_owned(),
            Value::from(format!("minecraft:{}", variant.to_name())),
        );
    }
    if let Some(mooshroom) = entity
        .cast_any()
        .downcast_ref::<crate::entity::passive::mooshroom::MooshroomEntity>()
    {
        state.components.insert(
            "minecraft:mooshroom/variant".to_owned(),
            Value::from(mooshroom.get_variant().as_str()),
        );
    }
}

/// Build `/loot kill` parameters using the command source, without remembered death credit.
#[must_use]
pub fn build_command_kill_loot_context(
    entity: &dyn EntityBase,
    source: Option<&dyn EntityBase>,
    base: &LootContextParameters,
) -> LootContextParameters {
    // LootCommand.dropKillLoot supplies magic damage and the executing player as LAST_DAMAGE_PLAYER.
    let snapshot = |entity| snapshot_entity(entity, &mut HashSet::new(), 0);
    LootContextParameters {
        this_entity: Some(entity.get_entity().entity_type),
        this_entity_state: snapshot(entity),
        attacking_entity_state: source.and_then(snapshot),
        direct_attacking_entity_state: source.and_then(snapshot),
        last_damage_player_state: source
            .filter(|source| source.get_player().is_some())
            .and_then(snapshot),
        damage_type: Some(DamageType::MAGIC),
        ..base.clone()
    }
}

/// Build the live container/archaeology context with the opening player, if supplied.
#[must_use]
pub fn build_container_loot_context(
    world: &std::sync::Arc<crate::world::World>,
    position: pumpkin_util::math::vector3::Vector3<f64>,
    player: Option<&crate::entity::player::Player>,
) -> LootContextParameters {
    // RandomizableContainer.unpackLootTable / ContainerEntity.unpackChestVehicleLootTable.
    LootContextParameters {
        world: Some(world.clone()),
        registry: world
            .server
            .upgrade()
            .map(|server| server.datapack_manager.clone()),
        position: Some(position),
        luck: player.map_or(0.0, |player| {
            player
                .living_entity
                .get_attribute_value(&pumpkin_data::attributes::Attributes::LUCK) as f32
        }),
        this_entity_state: player
            .and_then(|player| snapshot_entity(player, &mut HashSet::new(), 0)),
        this_entity: player.map(|player| player.get_entity().entity_type),
        ..Default::default()
    }
}
