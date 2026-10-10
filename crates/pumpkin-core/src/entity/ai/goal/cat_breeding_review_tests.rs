use super::BreedGoal;
use crate::entity::{
    EntityBase,
    ai::goal::Goal,
    passive::{animal::review_test_support::Fixture, cat::CatEntity, tamable::TamableAnimal},
};
use pumpkin_data::{cat_variant::CatVariant, dye_color::DyeColor, entity::EntityType};
use pumpkin_nbt::compound::NbtCompound;
use std::sync::Arc;

fn pair(fixture: &Fixture) -> (Arc<dyn EntityBase>, Arc<dyn EntityBase>) {
    let parent = fixture.spawn(&EntityType::CAT, 0);
    let partner = fixture.spawn(&EntityType::CAT, 0);
    for animal in [&parent, &partner] {
        animal
            .get_mob()
            .unwrap()
            .get_mob_entity()
            .set_love_ticks(600, Some(fixture.player.player.gameprofile.id));
    }
    (parent, partner)
}

fn breed(fixture: &Fixture, parent: &dyn EntityBase) -> Arc<dyn EntityBase> {
    let mob = parent.get_mob().unwrap();
    let mut goal = BreedGoal::new(0.8);
    assert!(goal.can_start(mob));
    goal.start(mob);
    for _ in 0..60 {
        if !goal.should_continue(mob) {
            break;
        }
        goal.tick(mob);
    }
    let children: Vec<_> = fixture
        .world
        .entities
        .load()
        .iter()
        .filter(|entity| {
            entity.get_entity().entity_type == &EntityType::CAT
                && entity
                    .get_entity()
                    .age
                    .load(std::sync::atomic::Ordering::Relaxed)
                    < 0
        })
        .cloned()
        .collect();
    assert_eq!(
        children.len(),
        1,
        "real breeding goal must produce one kitten"
    );
    children[0].clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review116_cat_breeding_inherits_variant_owner_and_tame_state() {
    let fixture = Fixture::new();
    let (parent, partner) = pair(&fixture);
    let mother = parent.cast_any().downcast_ref::<CatEntity>().unwrap();
    let father = partner.cast_any().downcast_ref::<CatEntity>().unwrap();
    mother.set_variant(CatVariant::Siamese.id());
    father.set_variant(CatVariant::Ragdoll.id());
    father.set_owner(Some(uuid::Uuid::new_v4()));
    let child = breed(&fixture, parent.as_ref());
    let kitten = child.cast_any().downcast_ref::<CatEntity>().unwrap();
    let variant = kitten.variant.load(std::sync::atomic::Ordering::Relaxed);
    assert!([CatVariant::Siamese.id(), CatVariant::Ragdoll.id()].contains(&variant));
    assert!(kitten.is_tame());
    assert_eq!(kitten.get_owner(), mother.get_owner());
    assert!(!kitten.is_sitting());
    let mut saved = NbtCompound::new();
    child.write_custom_nbt(&mut saved);
    assert_eq!(saved.get_uuid("Owner"), mother.get_owner());
    assert_eq!(saved.get_int("Age"), Some(-24000));
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review116_cat_breeding_mixes_collar_colors_using_a_recipe() {
    let fixture = Fixture::new();
    let (parent, partner) = pair(&fixture);
    parent
        .cast_any()
        .downcast_ref::<CatEntity>()
        .unwrap()
        .set_collar_color(DyeColor::White.id());
    partner
        .cast_any()
        .downcast_ref::<CatEntity>()
        .unwrap()
        .set_collar_color(DyeColor::Red.id());
    let child = breed(&fixture, parent.as_ref());
    assert_eq!(
        child
            .cast_any()
            .downcast_ref::<CatEntity>()
            .unwrap()
            .get_collar_color(),
        DyeColor::Pink.id()
    );
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review116_cat_breeding_uses_parent_collar_when_no_mix_recipe_exists() {
    let fixture = Fixture::new();
    let (parent, partner) = pair(&fixture);
    parent
        .cast_any()
        .downcast_ref::<CatEntity>()
        .unwrap()
        .set_collar_color(DyeColor::Green.id());
    partner
        .cast_any()
        .downcast_ref::<CatEntity>()
        .unwrap()
        .set_collar_color(DyeColor::Brown.id());
    let child = breed(&fixture, parent.as_ref());
    let color = child
        .cast_any()
        .downcast_ref::<CatEntity>()
        .unwrap()
        .get_collar_color();
    assert!([DyeColor::Green.id(), DyeColor::Brown.id()].contains(&color));
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review116_cat_breeding_requires_both_parents_to_be_tame() {
    let fixture = Fixture::new();
    let (parent, partner) = pair(&fixture);
    partner
        .cast_any()
        .downcast_ref::<CatEntity>()
        .unwrap()
        .set_tame(false, None);
    let mut goal = BreedGoal::new(0.8);
    assert!(!goal.can_start(parent.get_mob().unwrap()));
    fixture.finish().await;
}
