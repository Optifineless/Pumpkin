//! `Horse.randomizeAttributes` and `AbstractHorse`'s individual-stat generators (26.3).

use pumpkin_data::attributes::Attributes;
use rand::RngExt;

use super::horse::HorseEntity;

impl HorseEntity {
    pub(super) fn spawn_attributes(&self) -> Vec<pumpkin_protocol::java::client::play::Property> {
        use pumpkin_protocol::java::client::play::{AttributeModifier, Property};
        let attributes = self
            .mob_entity
            .living_entity
            .attributes
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        [
            Attributes::MAX_HEALTH,
            Attributes::MOVEMENT_SPEED,
            Attributes::JUMP_STRENGTH,
        ]
        .iter()
        .filter_map(|attribute| {
            let instance = attributes.get(&attribute.id)?;
            Some(Property::new(
                i32::from(attribute.id).into(),
                instance.base_value,
                instance
                    .modifiers
                    .iter()
                    .map(|modifier| {
                        AttributeModifier::new(
                            modifier.id.clone(),
                            modifier.amount,
                            modifier.operation as i8,
                        )
                    })
                    .collect(),
            ))
        })
        .collect()
    }

    pub(super) fn randomize_attributes(&self) {
        let mut random = rand::rng();
        let living = &self.mob_entity.living_entity;
        // The Java override draws health, speed, then jump before superclass finalization.
        living.set_attribute_base(
            &Attributes::MAX_HEALTH,
            generate_max_health(|bound| next_int(&mut random, bound)),
        );
        living.set_attribute_base(
            &Attributes::MOVEMENT_SPEED,
            generate_speed(|| next_double(&mut random)),
        );
        living.set_attribute_base(
            &Attributes::JUMP_STRENGTH,
            generate_jump_strength(|| next_double(&mut random)),
        );
        // Pumpkin's base setter has no onAttributeUpdated callback. Clamp after
        // releasing all attribute guards, without healing damaged template data.
        living.set_health(living.health.load());
    }
}

fn generate_max_health(mut integer_by_bound_provider: impl FnMut(u32) -> u32) -> f64 {
    f64::from(15 + integer_by_bound_provider(8) + integer_by_bound_provider(9))
}

fn generate_speed(mut probability_provider: impl FnMut() -> f64) -> f64 {
    (f64::from(0.45f32)
        + probability_provider() * 0.3
        + probability_provider() * 0.3
        + probability_provider() * 0.3)
        * 0.25
}

fn generate_jump_strength(mut probability_provider: impl FnMut() -> f64) -> f64 {
    f64::from(0.4f32)
        + probability_provider() * 0.2
        + probability_provider() * 0.2
        + probability_provider() * 0.2
}

fn next_int(random: &mut impl rand::Rng, bound: u32) -> u32 {
    #[cfg(test)]
    if let Some(value) = super::horse_spawn_stats_test_rng::integer(bound) {
        return value;
    }
    random.random_range(0..bound)
}

fn next_double(random: &mut impl rand::Rng) -> f64 {
    #[cfg(test)]
    if let Some(value) = super::horse_spawn_stats_test_rng::double() {
        return value;
    }
    random.random()
}
