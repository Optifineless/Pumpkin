use super::*;
use crate::world::spawn_test_support::Fixture;
use std::sync::Barrier;

#[tokio::test]
async fn concurrent_dye_does_not_restore_sheared_state() {
    let fixture = Fixture::new();
    let sheep = SheepEntity::new(Entity::new(
        fixture.world.clone(),
        Vector3::new(8.0, 64.0, 8.0),
        &EntityType::SHEEP,
    ));
    let barrier = Barrier::new(2);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            barrier.wait();
            for _ in 0..10_000 {
                sheep.set_color(pumpkin_data::dye_color::DyeColor::Yellow as u8);
                sheep.set_color(pumpkin_data::dye_color::DyeColor::Purple as u8);
            }
        });
        scope.spawn(|| {
            barrier.wait();
            assert!(sheep.shear(SoundCategory::Blocks, &ItemStack::new(1, &Item::SHEARS)));
        });
    });
    assert!(sheep.is_sheared());
    assert!(!sheep.shear(SoundCategory::Blocks, &ItemStack::new(1, &Item::SHEARS)));
    fixture.finish().await;
}
