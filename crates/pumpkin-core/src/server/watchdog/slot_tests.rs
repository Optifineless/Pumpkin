use super::*;

#[test]
fn formats_active_subjects_with_elapsed_time() {
    let phase = Record::new();
    phase.enter(Phase::EntityTicks, 100);
    assert_eq!(
        phase
            .describe(2100)
            .map(|description| description.to_string())
            .as_deref(),
        Some("entity ticks / player collisions, entered 2000 ms ago")
    );
    let slots = Arc::new(Slots::new(Instant::now()));
    let marker = slots.scope(2, None);
    let entity = marker.entity(-42, EntityType::MINECART.id, Phase::EntityTicks);
    let mut report = String::new();
    slots.append(&mut report, |_| None);
    assert!(report.contains("World #2: entity ticks / player collisions, entity -42 (minecart)"));
    drop(entity);
    let hopper = BLOCK_ENTITY_TYPES.iter().position(|name| *name == "hopper");
    assert!(hopper.is_some());
    let block = marker.block_entity(
        BlockPos::new(-30_000_000, -64, 30_000_000),
        hopper.unwrap_or_default() as u16,
    );
    report.clear();
    slots.append(&mut report, |_| None);
    assert!(report.contains("block entity minecraft:hopper at (-30000000, -64, 30000000)"));
    drop(block);
    report.clear();
    slots.append(&mut report, |_| None);
    assert!(report.is_empty());
    phase.finish();
    assert!(phase.describe(2100).is_none());
}

#[test]
fn nested_packet_phase_restores_parent_and_retires_subject() {
    let slots = Arc::new(Slots::new(Instant::now()));
    let parent_phase = Arc::new(Record::new());
    let parent = slots.scope(2, Some(parent_phase.clone()));
    Phase::WorldTick.enter();
    // Nested Rayon work can run on a waiting world/player worker. Its scope
    // must restore both the outer subject and its world/phase context.
    let outer = parent.entity(7, EntityType::MINECART.id, Phase::EntityTicks);
    let child = slots.scope(3, None);
    let subject = child.entity(42, EntityType::PLAYER.id, Phase::PlayerTicks);
    Phase::Packets.enter();
    let mut report = String::new();
    slots.append(&mut report, |_| None);
    assert!(report.contains("World #3: packet processing, player 42 (player)"));
    drop(subject);
    drop(child);
    let mut report = String::new();
    slots.append(&mut report, |_| None);
    assert!(report.contains("World #2: entity ticks / player collisions, entity 7 (minecart)"));
    drop(outer);
    let hopper = BLOCK_ENTITY_TYPES
        .iter()
        .position(|name| *name == "hopper")
        .unwrap() as u16;
    let outer_block = parent.block_entity(BlockPos::new(12, 64, -3), hopper);
    let child = slots.scope(3, None);
    let inner_block = child.block_entity(BlockPos::new(-7, 90, 28), hopper);
    report.clear();
    slots.append(&mut report, |_| None);
    assert!(report.contains("World #3: block entity ticks / comparator updates, block entity minecraft:hopper at (-7, 90, 28)"));
    drop(inner_block);
    drop(child);
    report.clear();
    slots.append(&mut report, |_| None);
    assert!(report.contains("World #2: block entity ticks / comparator updates, block entity minecraft:hopper at (12, 64, -3)"));
    drop(outer_block);
    Phase::BlockEvents.enter();
    assert!(
        parent_phase
            .describe(0)
            .is_some_and(|s| s.to_string().starts_with("block events"))
    );
    assert_eq!(current_world(), 2);
    drop(parent);
    report.clear();
    slots.append(&mut report, |_| None);
    assert!(report.is_empty());
    assert!(parent_phase.describe(0).is_none());
    assert_eq!(current_world(), 0);
}
