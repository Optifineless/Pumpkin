use super::*;
use pumpkin_data::damage::DamageType;
use std::sync::Mutex;

#[derive(Debug, PartialEq)]
pub(super) struct Hit {
    pub amount: f32,
    pub kind: u8,
    pub direct: Option<i32>,
    pub cause: Option<i32>,
    pub raw_position: Option<Vector3<f64>>,
}
pub(super) struct Receiver {
    pub living: LivingEntity,
    pub hits: Mutex<Vec<Hit>>,
    pub accepted: bool,
}
impl EntityBase for Receiver {
    fn can_hit(&self) -> bool {
        true
    }
    fn get_entity(&self) -> &Entity {
        &self.living.entity
    }
    fn get_living_entity(&self) -> Option<&LivingEntity> {
        Some(&self.living)
    }
    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }
    fn damage_with_context(
        &self,
        _caller: &dyn EntityBase,
        amount: f32,
        kind: DamageType,
        raw_position: Option<Vector3<f64>>,
        direct: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) -> bool {
        self.hits.lock().unwrap().push(Hit {
            amount,
            kind: kind.id,
            direct: direct.map(|e| e.get_entity().entity_id),
            cause: cause.map(|e| e.get_entity().entity_id),
            raw_position,
        });
        self.accepted
    }
}
