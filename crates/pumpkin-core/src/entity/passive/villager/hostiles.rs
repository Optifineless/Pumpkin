use pumpkin_data::entity::EntityType;

// VillagerHostilesSensor.ACCEPTABLE_DISTANCE_FROM_HOSTILES. Keep Zombie first
// so WanderingTrader.registerGoals retains its Java class-goal insertion order.
pub const HOSTILE_DISTANCES: &[(&EntityType, f64)] = &[
    (&EntityType::ZOMBIE, 8.0),
    (&EntityType::EVOKER, 12.0),
    (&EntityType::VINDICATOR, 10.0),
    (&EntityType::VEX, 8.0),
    (&EntityType::PILLAGER, 15.0),
    (&EntityType::ILLUSIONER, 12.0),
    (&EntityType::ZOGLIN, 10.0),
    (&EntityType::DROWNED, 8.0),
    (&EntityType::HUSK, 8.0),
    (&EntityType::ZOMBIE_VILLAGER, 8.0),
    (&EntityType::RAVAGER, 12.0),
];

pub fn hostile_distance(kind: &EntityType) -> Option<f64> {
    HOSTILE_DISTANCES
        .iter()
        .find_map(|&(hostile, distance)| (kind == hostile).then_some(distance))
}
