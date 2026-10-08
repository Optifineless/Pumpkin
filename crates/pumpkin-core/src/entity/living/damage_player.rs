use crate::entity::{EntityBase, player::Player};
use pumpkin_data::{
    damage::{DamageScaling, DamageType},
    tag::{self, Taggable},
};
use pumpkin_util::Difficulty;

impl Player {
    // ServerPlayer.hurtServer:996 and Player.hurtServer:673 each dispatch isInvulnerableTo
    // before delegating. These are admission rolls, never callback revalidation rolls.
    pub(crate) fn admit_incoming_damage(
        &self,
        damage: f32,
        damage_type: DamageType,
        cause: Option<&dyn EntityBase>,
        source: Option<&dyn EntityBase>,
    ) -> Option<f32> {
        if self.player_entry_invulnerable(damage_type, source, cause)
            || !self.accepts_damage_source(cause)
            || self.player_entry_invulnerable(damage_type, source, cause)
        {
            return None;
        }
        let amount = self.prepare_incoming_damage(damage, damage_type, cause, source)?;
        (self.living_entity.health.load() > 0.0).then_some(amount)
    }

    fn player_entry_invulnerable(
        &self,
        damage_type: DamageType,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) -> bool {
        // Player.isInvulnerableTo:657 / ServerPlayer.isInvulnerableTo:1333.
        self.living_entity
            .entity
            .is_invulnerable_to(&damage_type, cause)
            || self
                .living_entity
                .is_immune_to_enchantment_damage(self, damage_type, source, cause)
            || self.player_damage_type_disabled(damage_type)
    }

    fn player_damage_type_disabled(&self, damage_type: DamageType) -> bool {
        let world = self.world();
        let rules = &world.level_info.load().game_rules;
        !self.has_client_loaded()
            || (damage_type.has_tag(&tag::DamageType::MINECRAFT_IS_DROWNING)
                && !rules.drowning_damage)
            || (damage_type.has_tag(&tag::DamageType::MINECRAFT_IS_FALL) && !rules.fall_damage)
            || (damage_type.has_tag(&tag::DamageType::MINECRAFT_IS_FIRE) && !rules.fire_damage)
            || (damage_type.has_tag(&tag::DamageType::MINECRAFT_IS_FREEZING)
                && !rules.freeze_damage)
    }

    fn accepts_damage_source(&self, cause: Option<&dyn EntityBase>) -> bool {
        let arrow_owner = cause
            .filter(|source| {
                let kind = source.get_entity().entity_type;
                kind.has_tag(&tag::EntityType::MINECRAFT_ARROWS)
                    || kind == &pumpkin_data::entity::EntityType::TRIDENT
            })
            .and_then(EntityBase::get_owner_id)
            .and_then(|id| self.world().get_entity_by_id(id));
        cause
            .and_then(EntityBase::get_player)
            .is_none_or(|attacker| self.can_harm_player(attacker))
            && arrow_owner
                .as_deref()
                .and_then(EntityBase::get_player)
                .is_none_or(|attacker| self.can_harm_player(attacker))
    }
    // ServerPlayer.hurtServer (995-1006), Player.isInvulnerableTo/hurtServer (657-701).
    pub(crate) fn prepare_incoming_damage(
        &self,
        damage: f32,
        damage_type: DamageType,
        cause: Option<&dyn EntityBase>,
        _source: Option<&dyn EntityBase>,
    ) -> Option<f32> {
        let world = self.world();
        let bypasses = damage_type.has_tag(&tag::DamageType::MINECRAFT_BYPASSES_INVULNERABILITY);
        if self.player_damage_type_disabled(damage_type)
            || (!bypasses
                && self
                    .abilities
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .invulnerable)
            || !self.accepts_damage_source(cause)
        {
            return None;
        }
        self.living_entity
            .no_action_time
            .store(0, std::sync::atomic::Ordering::Relaxed); // Player.hurtServer:681.
        let scales = match damage_type.scaling {
            DamageScaling::Never => false,
            DamageScaling::Always => true,
            DamageScaling::WhenCausedByLivingNonPlayer => cause.is_some_and(|entity| {
                entity.get_living_entity().is_some() && entity.get_player().is_none()
            }),
        };
        let damage = if scales {
            scale_damage_for_difficulty(damage, world.level_info.load().difficulty)
        } else {
            damage
        };
        (damage != 0.0).then_some(damage)
    }

    /// Checks incoming player-caused damage, including self-owned projectiles, using victim team rules.
    pub fn can_harm_player(&self, attacker: &Self) -> bool {
        // ServerPlayer.canHarmPlayer / Entity.doTeamsAllowDamage; team packet bit 0 is friendly fire.
        const FRIENDLY_FIRE: i8 = 0x01;
        self.world()
            .server
            .upgrade()
            .is_none_or(|server| server.advanced_config.pvp.enabled)
            && self.get_team().is_none_or(|team| {
                team.options & FRIENDLY_FIRE != 0
                    || attacker.get_team_name().as_deref() != Some(team.name.as_str())
            })
    }
}

// Player.hurtServer: difficulty is applied to incoming damage, before blocking and cooldown.
pub(super) fn scale_damage_for_difficulty(damage: f32, difficulty: Difficulty) -> f32 {
    match difficulty {
        Difficulty::Peaceful => 0.0,
        Difficulty::Easy => (damage / 2.0 + 1.0).min(damage),
        Difficulty::Normal => damage,
        Difficulty::Hard => damage * 3.0 / 2.0,
    }
}
