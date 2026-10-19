use super::EnderDragonEntity;
use crate::{
    enchantment::{EnchantmentHelper, post_attack::AttackEffectContext},
    entity::{EntityBase, mob::creaking::CreakingEntity},
    world::World,
};
use pumpkin_data::damage::DamageType;
use pumpkin_util::math::{boundingbox::BoundingBox, vector3::Vector3};
use std::sync::{Arc, atomic::Ordering::Relaxed};

impl EnderDragonEntity {
    pub(super) fn handle_part_collisions(&self) {
        // EnderDragon.aiStep:270-285, including wasHurtRecently's hurtTime gate.
        if !self.mob_entity.living_entity.is_alive()
            || self.mob_entity.living_entity.hurt_time.load(Relaxed) > 0
        {
            return;
        }
        let world = self.get_entity().world.load_full();
        let bounds = [
            self.parts[6]
                .entity
                .bounding_box
                .load()
                .expand(4.0, 2.0, 4.0)
                .shift(Vector3::new(0.0, -2.0, 0.0)),
            self.parts[7]
                .entity
                .bounding_box
                .load()
                .expand(4.0, 2.0, 4.0)
                .shift(Vector3::new(0.0, -2.0, 0.0)),
            self.parts[0].entity.bounding_box.load().expand_all(1.0),
            self.parts[1].entity.bounding_box.load().expand_all(1.0),
        ];
        let broad_bounds = bounds[1..]
            .iter()
            .fold(bounds[0], |union, part| BoundingBox {
                min: Vector3::new(
                    union.min.x.min(part.min.x),
                    union.min.y.min(part.min.y),
                    union.min.z.min(part.min.z),
                ),
                max: Vector3::new(
                    union.max.x.max(part.max.x),
                    union.max.y.max(part.max.y),
                    union.max.z.max(part.max.z),
                ),
            });
        // EnderDragon.aiStep:270-285 checks each part separately after one broad-phase query.
        let targets = world.get_all_at_box(&broad_bounds);
        for part_bounds in &bounds[..2] {
            self.knock_back(&world, part_bounds, &targets);
        }
        for part_bounds in &bounds[2..] {
            self.hurt_entities(&world, part_bounds, &targets);
        }
    }

    // Level.getEntities / EntitySelector.NO_CREATIVE_OR_SPECTATOR and the LivingEntity
    // checks in EnderDragon.knockBack/hurt. Raw Entity.push has no riding predicate.
    fn contact_target_present(&self, world: &Arc<World>, target: &dyn EntityBase) -> bool {
        let entity = target.get_entity();
        !self.get_entity().is_removed()
            && Arc::ptr_eq(&self.get_entity().world.load_full(), world)
            && entity.entity_id != self.get_entity().entity_id
            && !entity.is_removed()
            && Arc::ptr_eq(&entity.world.load_full(), world)
            && target.get_player().is_none_or(|player| {
                !player.is_creative()
                    && !player.is_spectator()
                    // A disconnect can detach a player before its removal mark is published.
                    // Scan only the player list: an id lookup over every entity per
                    // candidate made dense crowds quadratic each tick.
                    && world
                        .players
                        .load()
                        .iter()
                        .any(|live| std::ptr::eq(live.get_entity(), entity))
            })
    }

    fn contact_target_eligible(
        &self,
        world: &Arc<World>,
        target: &dyn EntityBase,
        bounds: &BoundingBox,
    ) -> bool {
        target
            .get_living_entity()
            .is_some_and(|living| living.is_alive() && !living.dead.load(Relaxed))
            && target.get_entity().bounding_box.load().intersects(bounds)
            && self.contact_target_present(world, target)
    }

    fn do_teams_allow_damage(&self, target: &dyn EntityBase) -> bool {
        // Entity.doTeamsAllowDamage:2558-2565; team packet bit 0 is friendly fire.
        const FRIENDLY_FIRE: i8 = 0x01;
        self.get_team().is_none_or(|team| {
            team.options & FRIENDLY_FIRE != 0
                || target
                    .get_team_name()
                    .is_none_or(|other| other != team.name)
        })
    }

    fn knock_back(
        &self,
        world: &Arc<World>,
        bounds: &BoundingBox,
        targets: &[Arc<dyn EntityBase>],
    ) {
        let body = self.parts[2].entity.bounding_box.load();
        let xm = f64::midpoint(body.min.x, body.max.x);
        let zm = f64::midpoint(body.min.z, body.max.z);
        // Own the candidate Arcs; no registry or phase guard survives an effect.
        for target in targets {
            if !self.contact_target_eligible(world, target.as_ref(), bounds) {
                continue;
            }
            let entity = target.get_entity();
            let pos = entity.pos.load();
            let xd = pos.x - xm;
            let zd = pos.z - zm;
            let dd = (xd * xd + zd * zd).max(0.1);
            // EnderDragon.knockBack:378-394 uses additive Entity.push, not knockback().
            // Creaking.push:197-200 is the living override of raw Entity.push.
            if target
                .cast_any()
                .downcast_ref::<CreakingEntity>()
                .is_none_or(CreakingEntity::can_move)
            {
                entity.push_impulse(Vector3::new(
                    xd / dd * 4.0,
                    f64::from(0.2f32),
                    zd / dd * 4.0,
                ));
            }

            let sitting = self
                .phase
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_sitting();
            let living = target.get_living_entity();
            if !sitting
                && living.is_some_and(|living| {
                    i64::from(living.last_attacked_time.load(Relaxed))
                        < living.combat_ticks.load(Relaxed) - 2
                })
            {
                self.hurt_contact_target(world, target.as_ref(), bounds, 5.0);
            }
        }
    }

    fn hurt_entities(
        &self,
        world: &Arc<World>,
        bounds: &BoundingBox,
        targets: &[Arc<dyn EntityBase>],
    ) {
        // EnderDragon.hurt(List<Entity>):399-405.
        for target in targets {
            self.hurt_contact_target(world, target.as_ref(), bounds, 10.0);
        }
    }

    fn hurt_contact_target(
        &self,
        world: &Arc<World>,
        target: &dyn EntityBase,
        bounds: &BoundingBox,
        damage: f32,
    ) {
        if !self.do_teams_allow_damage(target)
            || !self.contact_target_eligible(world, target, bounds)
        {
            return;
        }
        target.damage_with_context(
            target,
            damage,
            DamageType::MOB_ATTACK,
            None,
            Some(self),
            Some(self),
        );
        // Damage callbacks can move, detach or change the target's mode. A lethal hit
        // still runs vanilla's post-attack effects when the victim remains present.
        if self.contact_target_present(world, target) {
            EnchantmentHelper::on_post_attack(
                target,
                AttackEffectContext::melee(self, DamageType::MOB_ATTACK),
                None,
            );
        }
    }
}
