use super::Explosion;
use crate::entity::EntityBase;
use pumpkin_data::damage::DamageType;
use std::sync::Arc;

impl Explosion {
    /// Carries `ExplodeEffect`'s attribution, damage identity and fire setting.
    #[must_use]
    pub fn with_enchantment_settings(
        self,
        source: Option<Arc<dyn EntityBase>>,
        damage_type: Option<DamageType>,
        create_fire: bool,
    ) -> Self {
        let mut explosion = self.with_source(source).with_fire(create_fire);
        explosion.damage_type = damage_type;
        explosion
    }
}
