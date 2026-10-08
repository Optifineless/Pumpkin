#![expect(
    clippy::rc_buffer,
    reason = "ArcSwap requires a sized allocation for the tick snapshot"
)]

use super::World;
use crate::entity::{EntityBase, experience_orb::ExperienceOrbEntity};
use std::sync::Arc;

fn orb_list(entities: &[Arc<dyn EntityBase>]) -> Arc<Vec<Arc<dyn EntityBase>>> {
    Arc::new(
        entities
            .iter()
            .filter(|entity| entity.cast_any().is::<ExperienceOrbEntity>())
            .cloned()
            .collect(),
    )
}

impl World {
    // ExperienceOrb.scanForMerges and Player.aiStep share the tick's type-filtered entity snapshot.
    pub(super) fn begin_experience_orb_tick(
        &self,
        entities: &[Arc<dyn EntityBase>],
    ) -> Arc<Vec<Arc<dyn EntityBase>>> {
        let orbs = orb_list(entities);
        self.ticking_experience_orbs.store(Some(orbs.clone()));
        orbs
    }

    pub(crate) fn experience_orbs(&self) -> Arc<Vec<Arc<dyn EntityBase>>> {
        self.ticking_experience_orbs
            .load_full()
            .unwrap_or_else(|| orb_list(&self.entities.load()))
    }
}
