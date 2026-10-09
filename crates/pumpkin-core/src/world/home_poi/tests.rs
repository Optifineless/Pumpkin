use super::*;
use crate::world::spawn_test_support::{Fixture, proto, publish};
use pumpkin_data::biome::Biome;

#[tokio::test]
async fn followup2_home_scan_checks_only_non_bed_palettes() {
    let fixture = Fixture::new();
    let chunk = publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    let section_count = chunk.section.block_sections.read().unwrap().len();
    HOME_STATE_CHECKS.with(|checks| checks.set(0));
    fixture.world.register_chunk_home_pois(Vector2::new(0, 0));
    assert!(HOME_STATE_CHECKS.with(std::cell::Cell::get) <= section_count * 2);
    let mut bed = WhiteBedLikeProperties::default(&Block::RED_BED);
    bed.part = BedPart::Head;
    chunk.set_block_absolute_y(5, 64, 5, bed.to_state_id(&Block::RED_BED));
    fixture.world.unregister_chunk_home_pois(Vector2::new(0, 0));
    HOME_STATE_CHECKS.with(|checks| checks.set(0));
    fixture.world.register_chunk_home_pois(Vector2::new(0, 0));
    assert!(HOME_STATE_CHECKS.with(std::cell::Cell::get) < 4096 + section_count * 4);
    assert_eq!(
        fixture.world.available_homes(BlockPos::new(5, 64, 5)),
        vec![BlockPos::new(5, 64, 5)]
    );
    fixture.finish().await;
}

#[tokio::test]
async fn followup2_unload_prunes_home_caches_but_keeps_saved_tickets() {
    let fixture = Fixture::new();
    let chunk = publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    let pos = BlockPos::new(5, 64, 5);
    let mut bed = WhiteBedLikeProperties::default(&Block::RED_BED);
    bed.part = BedPart::Head;
    chunk.set_block_absolute_y(5, 64, 5, bed.to_state_id(&Block::RED_BED));
    fixture.world.register_chunk_home_pois(pos.chunk_position());
    fixture
        .world
        .portal_poi
        .lock()
        .unwrap()
        .add_with_free_tickets(pos, "minecraft:home", 0);
    fixture
        .world
        .level
        .loaded_chunks
        .remove(&pos.chunk_position());
    fixture
        .world
        .unregister_chunk_home_pois(pos.chunk_position());
    let (homes_empty, indexed_chunks_empty) = {
        let sites = fixture.world.villager_poi.lock().unwrap();
        (sites.homes.is_empty(), sites.indexed_home_chunks.is_empty())
    };
    assert!(homes_empty);
    assert!(indexed_chunks_empty);
    assert_eq!(
        fixture
            .world
            .portal_poi
            .lock()
            .unwrap()
            .free_tickets(&pos, "minecraft:home"),
        Some(0)
    );
    fixture.world.portal_poi.lock().unwrap().remove(&pos);
    fixture.world.release_home(pos, uuid::Uuid::new_v4());
    assert_eq!(
        fixture
            .world
            .portal_poi
            .lock()
            .unwrap()
            .free_tickets(&pos, "minecraft:home"),
        None
    );
    fixture.finish().await;
}

struct CountingOwner {
    entity: crate::entity::Entity,
    checks: std::sync::atomic::AtomicUsize,
}
impl EntityBase for CountingOwner {
    fn get_entity(&self) -> &crate::entity::Entity {
        &self.entity
    }
    fn get_living_entity(&self) -> Option<&crate::entity::living::LivingEntity> {
        self.checks
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        None
    }
    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[tokio::test]
async fn followup2_home_range_filter_precedes_owner_and_ticket_checks() {
    let fixture = Fixture::new();
    publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    let owner = Arc::new(CountingOwner {
        entity: crate::entity::Entity::new(
            fixture.world.clone(),
            pumpkin_util::math::vector3::Vector3::default(),
            &pumpkin_data::entity::EntityType::COW,
        ),
        checks: std::sync::atomic::AtomicUsize::new(0),
    });
    let base: Arc<dyn EntityBase> = owner.clone();
    // Inside the 7x7 chunk square, but outside AcquirePoi's three-dimensional sphere.
    let far = BlockPos::new(5, 200, 5);
    fixture
        .world
        .villager_poi
        .lock()
        .unwrap()
        .homes
        .entry(far.chunk_position())
        .or_default()
        .insert(
            far,
            HomeSite {
                occupied: false,
                owner: Some(Arc::downgrade(&base)),
            },
        );
    fixture
        .world
        .portal_poi
        .lock()
        .unwrap()
        .add_with_free_tickets(far, "minecraft:home", 1);
    // Registration has already run, so the artificial POI is not refreshed away.
    let chunk = fixture
        .world
        .level
        .loaded_chunks
        .get(&far.chunk_position())
        .unwrap()
        .clone();
    fixture
        .world
        .villager_poi
        .lock()
        .unwrap()
        .indexed_home_chunks
        .insert(far.chunk_position(), Arc::downgrade(&chunk));
    assert!(
        fixture
            .world
            .available_homes(BlockPos::new(5, 64, 5))
            .is_empty()
    );
    assert_eq!(owner.checks.load(std::sync::atomic::Ordering::Relaxed), 0);
    fixture.finish().await;
}
