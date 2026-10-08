//! Constructor maluses from vanilla 26.3 species and their superclasses.
use super::{Navigator, node::PathType};

pub(super) fn configure(navigation: &mut Navigator, name: &str) {
    if matches!(name, "goat" | "wolf") {
        goat_and_wolf(navigation);
    }
    if matches!(
        name,
        "mule"
            | "pig"
            | "cat"
            | "wandering_trader"
            | "polar_bear"
            | "llama"
            | "armadillo"
            | "skeleton_horse"
            | "happy_ghast"
            | "panda"
            | "camel"
            // CamelHusk constructor inherits Camel -> Animal fire penalties.
            | "camel_husk"
            | "sheep"
            | "cow"
            | "horse"
            | "ocelot"
            | "mooshroom"
            | "rabbit"
            | "trader_llama"
            | "donkey"
            | "zombie_horse"
            | "villager"
    ) {
        animal(navigation);
    }
    if matches!(
        name,
        "squid"
            | "cod"
            | "salmon"
            | "tropical_fish"
            | "dolphin"
            | "glow_squid"
            | "tadpole"
            | "pufferfish"
    ) {
        water_animal(navigation);
    }
    if matches!(name, "strider") {
        strider(navigation);
    }
    if matches!(name, "turtle") {
        turtle(navigation);
    }
    if matches!(name, "parrot") {
        parrot(navigation);
    }
    if matches!(name, "chicken" | "axolotl") {
        amphibious_animal(navigation);
    }
    if matches!(name, "copper_golem") {
        copper_golem(navigation);
    }
    if matches!(name, "fox") {
        fox(navigation);
    }
    if matches!(name, "sniffer") {
        sniffer(navigation);
    }
    if matches!(name, "bee") {
        bee(navigation);
    }
    if matches!(name, "frog") {
        frog(navigation);
    }
}

// Goat, Wolf constructors.
fn goat_and_wolf(navigation: &mut Navigator) {
    navigation.set_pathfinding_malus(PathType::DamageFire, -1.0);
    navigation.set_pathfinding_malus(PathType::DangerFire, 16.0);
    navigation.set_pathfinding_malus(PathType::DangerPowderSnow, -1.0);
    navigation.set_pathfinding_malus(PathType::PowderSnow, -1.0);
}

// Mule, Pig, Cat, WanderingTrader, PolarBear, Llama, Armadillo, SkeletonHorse, HappyGhast, Panda, Camel, Sheep, Cow, Horse, Ocelot, MushroomCow, Rabbit, TraderLlama, Donkey, ZombieHorse, Villager constructors.
fn animal(navigation: &mut Navigator) {
    navigation.set_pathfinding_malus(PathType::DamageFire, -1.0);
    navigation.set_pathfinding_malus(PathType::DangerFire, 16.0);
}

// Squid, Cod, Salmon, TropicalFish, Dolphin, GlowSquid, Tadpole, Pufferfish constructors.
fn water_animal(navigation: &mut Navigator) {
    navigation.set_pathfinding_malus(PathType::Water, 0.0);
}

// Strider constructors.
fn strider(navigation: &mut Navigator) {
    navigation.set_pathfinding_malus(PathType::DamageFire, 0.0);
    navigation.set_pathfinding_malus(PathType::DangerFire, 0.0);
    navigation.set_pathfinding_malus(PathType::Lava, 0.0);
    navigation.set_pathfinding_malus(PathType::Water, -1.0);
}

// Turtle constructors.
fn turtle(navigation: &mut Navigator) {
    navigation.set_pathfinding_malus(PathType::DoorIronClosed, -1.0);
    navigation.set_pathfinding_malus(PathType::DoorOpen, -1.0);
    navigation.set_pathfinding_malus(PathType::DoorWoodClosed, -1.0);
    navigation.set_pathfinding_malus(PathType::DamageFire, -1.0);
    navigation.set_pathfinding_malus(PathType::DangerFire, 16.0);
    navigation.set_pathfinding_malus(PathType::Water, 0.0);
}

// Parrot constructors.
fn parrot(navigation: &mut Navigator) {
    navigation.set_pathfinding_malus(PathType::Cocoa, -1.0);
    navigation.set_pathfinding_malus(PathType::DamageFire, -1.0);
    navigation.set_pathfinding_malus(PathType::DangerFire, -1.0);
}

// Chicken, Axolotl constructors; AbstractNautilus sets these in its Rust constructor.
fn amphibious_animal(navigation: &mut Navigator) {
    navigation.set_pathfinding_malus(PathType::DamageFire, -1.0);
    navigation.set_pathfinding_malus(PathType::DangerFire, 16.0);
    navigation.set_pathfinding_malus(PathType::Water, 0.0);
}

// CopperGolem constructors.
fn copper_golem(navigation: &mut Navigator) {
    navigation.set_pathfinding_malus(PathType::DangerOther, 16.0);
    navigation.set_pathfinding_malus(PathType::DamageFire, -1.0);
    navigation.set_pathfinding_malus(PathType::DangerFire, 16.0);
}

// Fox constructors.
fn fox(navigation: &mut Navigator) {
    navigation.set_pathfinding_malus(PathType::DamageOther, 0.0);
    navigation.set_pathfinding_malus(PathType::DangerOther, 0.0);
    navigation.set_pathfinding_malus(PathType::DamageFire, -1.0);
    navigation.set_pathfinding_malus(PathType::DangerFire, 16.0);
}

// Sniffer constructors.
fn sniffer(navigation: &mut Navigator) {
    navigation.set_pathfinding_malus(PathType::DamageCautious, -1.0);
    navigation.set_pathfinding_malus(PathType::DamageFire, -1.0);
    navigation.set_pathfinding_malus(PathType::DangerFire, 16.0);
    navigation.set_pathfinding_malus(PathType::DangerPowderSnow, -1.0);
    navigation.set_pathfinding_malus(PathType::Water, -1.0);
}

// Bee constructors.
fn bee(navigation: &mut Navigator) {
    navigation.set_pathfinding_malus(PathType::Cocoa, -1.0);
    navigation.set_pathfinding_malus(PathType::Fence, -1.0);
    navigation.set_pathfinding_malus(PathType::DamageFire, -1.0);
    navigation.set_pathfinding_malus(PathType::DangerFire, 16.0);
    navigation.set_pathfinding_malus(PathType::Water, -1.0);
    navigation.set_pathfinding_malus(PathType::WaterBorder, 16.0);
}

// Frog constructors.
fn frog(navigation: &mut Navigator) {
    navigation.set_pathfinding_malus(PathType::DamageFire, -1.0);
    navigation.set_pathfinding_malus(PathType::DangerFire, 16.0);
    navigation.set_pathfinding_malus(PathType::Trapdoor, -1.0);
    navigation.set_pathfinding_malus(PathType::Water, 4.0);
}
