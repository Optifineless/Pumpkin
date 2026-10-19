use super::*;

fn compass_bound_to(count: u8, fixture: &Fixture, position: BlockPos) -> ItemStack {
    let mut stack = component_rich_compass(count);
    stack.set_data_component(LodestoneTrackerImpl {
        target: Some(LodestoneTarget {
            dimension: fixture.world.dimension.minecraft_name.to_string(),
            x: position.0.x,
            y: position.0.y,
            z: position.0.z,
        }),
        tracked: true,
    });
    stack
}

fn total_compasses(fixture: &Fixture) -> u16 {
    let inventory_count = fixture.actor.player.inventory().count(&Item::COMPASS);
    let dropped_count = compass_item_entities(&fixture.world)
        .iter()
        .map(|stack| u16::from(stack.item_count))
        .sum::<u16>();
    u16::from(inventory_count) + dropped_count
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020_g02_same_lodestone_stacked_rebind_conserves_compass_count() {
    for hand in [Hand::Right, Hand::Left] {
        for allow_output in [true, false] {
            let mut fixture = Fixture::new();
            fixture.put_lodestone(LODESTONE_POSITION);
            let source = compass_bound_to(2, &fixture, LODESTONE_POSITION);
            let source_uid = source.uid;
            let sword = fill_inventory(&fixture, hand, source, allow_output);
            fixture.finish_equipment_setup();

            assert_eq!(total_compasses(&fixture), 2);
            for sequence in 1..=3 {
                fixture.use_on(hand, LODESTONE_POSITION, sequence);
                let held = fixture.actor.player.inventory().get_stack_in_hand(hand);
                assert_eq!(held.uid, source_uid);
                assert_eq!(held.item_count, 2);
                preserved_component_patch(&held);
                lodestone_tracker(&held, &fixture, LODESTONE_POSITION);
                assert!(
                    fixture
                        .actor
                        .player
                        .inventory()
                        .get_stack_in_hand(other_hand(hand))
                        .are_equal(&sword)
                );
                assert_eq!(
                    total_compasses(&fixture),
                    2,
                    "same-lodestone use must conserve the total in {hand:?}, allow_output={allow_output}, sequence={sequence}"
                );
            }
            assert!(compass_item_entities(&fixture.world).is_empty());
            fixture.finish().await;
        }
    }
}
