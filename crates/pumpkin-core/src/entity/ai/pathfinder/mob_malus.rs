//! Constructor maluses from vanilla 26.3 species and their superclasses.
use super::{Navigator, node::PathType};

pub(super) fn configure(navigation: &mut Navigator, name: &str) {
    if matches!(name, "zombified_piglin" | "wither_skeleton") {
        undead_nether_mobs(navigation);
    }
    if matches!(name, "blaze") {
        blaze(navigation);
    }
    if matches!(name, "warden") {
        warden(navigation);
    }
    if matches!(name, "hoglin" | "piglin_brute" | "piglin") {
        piglins_and_hoglin(navigation);
    }
    if matches!(name, "ravager") {
        ravager(navigation);
    }
    if matches!(name, "enderman") {
        enderman(navigation);
    }
    if matches!(name, "breeze") {
        breeze(navigation);
    }
    if matches!(name, "elder_guardian" | "guardian" | "drowned") {
        aquatic_monster(navigation);
    }
}

// ZombifiedPiglin, WitherSkeleton constructors.
fn undead_nether_mobs(navigation: &mut Navigator) {
    navigation.set_pathfinding_malus(PathType::Lava, 8.0);
}

// Blaze constructors.
fn blaze(navigation: &mut Navigator) {
    navigation.set_pathfinding_malus(PathType::DamageFire, 0.0);
    navigation.set_pathfinding_malus(PathType::DangerFire, 0.0);
    navigation.set_pathfinding_malus(PathType::Lava, 8.0);
    navigation.set_pathfinding_malus(PathType::Water, -1.0);
}

// Warden constructors.
fn warden(navigation: &mut Navigator) {
    navigation.set_pathfinding_malus(PathType::DamageOther, 8.0);
    navigation.set_pathfinding_malus(PathType::DamageFire, 0.0);
    navigation.set_pathfinding_malus(PathType::DangerFire, 0.0);
    navigation.set_pathfinding_malus(PathType::Lava, 8.0);
    navigation.set_pathfinding_malus(PathType::PowderSnow, 8.0);
    navigation.set_pathfinding_malus(PathType::UnpassableRail, 0.0);
}

// Hoglin, PiglinBrute, Piglin constructors.
fn piglins_and_hoglin(navigation: &mut Navigator) {
    navigation.set_pathfinding_malus(PathType::DamageFire, -1.0);
    navigation.set_pathfinding_malus(PathType::DangerFire, 16.0);
}

// Ravager constructors.
fn ravager(navigation: &mut Navigator) {
    navigation.set_pathfinding_malus(PathType::Leaves, 0.0);
}

// Enderman constructors.
fn enderman(navigation: &mut Navigator) {
    navigation.set_pathfinding_malus(PathType::Water, -1.0);
}

// Breeze constructors.
fn breeze(navigation: &mut Navigator) {
    navigation.set_pathfinding_malus(PathType::DamageFire, -1.0);
    navigation.set_pathfinding_malus(PathType::DangerTrapdoor, -1.0);
}

// ElderGuardian, Guardian, Drowned constructors.
fn aquatic_monster(navigation: &mut Navigator) {
    navigation.set_pathfinding_malus(PathType::Water, 0.0);
}
