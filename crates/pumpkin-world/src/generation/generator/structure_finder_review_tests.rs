use super::{
    structure_finder::find_nearest_structure_start,
    structure_finder_tests::{cached, generate, owned, pieces, world},
};
use pumpkin_data::{
    dimension::Dimension,
    structures::{StructureKeys, StructureSet},
};
use pumpkin_util::math::position::BlockPos;
use std::sync::Arc;

const NETHER_CHUNK: (i32, i32) = (4, 56);
const NETHER_ORIGIN: BlockPos = BlockPos::new(64, 0, 896);

#[test]
fn b1019_g23_r2_weighted_winner_ignores_requested_key_order() {
    let mut expected = None;
    for generation_first in [false, true] {
        for targets in [
            [StructureKeys::Fortress, StructureKeys::BastionRemnant],
            [StructureKeys::BastionRemnant, StructureKeys::Fortress],
        ] {
            let generator = world(
                0,
                Dimension::THE_NETHER,
                Some("nether_wastes"),
                "nether_complexes",
            );
            let generated = generation_first.then(|| generate(&generator, NETHER_CHUNK));
            let found = find_nearest_structure_start(
                NETHER_ORIGIN,
                &StructureSet::NETHER_COMPLEXES,
                &targets,
                0,
                &generator,
            );
            assert_eq!(found, Some((NETHER_ORIGIN, StructureKeys::BastionRemnant)));
            assert_eq!(
                find_nearest_structure_start(
                    NETHER_ORIGIN,
                    &StructureSet::NETHER_COMPLEXES,
                    &targets,
                    0,
                    &generator,
                ),
                found,
                "warm locate must use the same winner"
            );
            assert!(
                find_nearest_structure_start(
                    NETHER_ORIGIN,
                    &StructureSet::NETHER_COMPLEXES,
                    &[StructureKeys::Fortress],
                    0,
                    &generator,
                )
                .is_none(),
                "a viable fortress is still absent when the bastion succeeds first"
            );
            let located = cached(&generator, StructureKeys::BastionRemnant, NETHER_CHUNK).unwrap();
            let generated = generated.unwrap_or_else(|| generate(&generator, NETHER_CHUNK));
            assert!(!generated.has_structure(StructureKeys::Fortress));
            let actual = owned(&generated, StructureKeys::BastionRemnant);
            assert!(Arc::ptr_eq(&located.collector, &actual.collector));
            assert_eq!(pieces(&located), pieces(actual));
            let actual = pieces(actual);
            if let Some(expected) = &expected {
                assert_eq!(
                    &actual, expected,
                    "command order must preserve actual generated pieces"
                );
            } else {
                expected = Some(actual);
            }
        }
    }
}

#[test]
fn b1019_g23_r2_failed_weighted_entry_falls_back_to_fortress() {
    let mut expected = None;
    for generation_first in [false, true] {
        let generator = world(
            0,
            Dimension::THE_NETHER,
            Some("basalt_deltas"),
            "nether_complexes",
        );
        let generated = generation_first.then(|| generate(&generator, NETHER_CHUNK));
        assert_eq!(
            find_nearest_structure_start(
                NETHER_ORIGIN,
                &StructureSet::NETHER_COMPLEXES,
                &[StructureKeys::BastionRemnant, StructureKeys::Fortress],
                0,
                &generator,
            ),
            Some((NETHER_ORIGIN, StructureKeys::Fortress)),
            "bastions fail in basalt deltas, so the next weighted entry must be tried"
        );
        let located = cached(&generator, StructureKeys::Fortress, NETHER_CHUNK).unwrap();
        let generated = generated.unwrap_or_else(|| generate(&generator, NETHER_CHUNK));
        assert!(!generated.has_structure(StructureKeys::BastionRemnant));
        let actual = owned(&generated, StructureKeys::Fortress);
        assert!(Arc::ptr_eq(&located.collector, &actual.collector));
        assert_eq!(pieces(&located), pieces(actual));
        let actual = pieces(actual);
        if let Some(expected) = &expected {
            assert_eq!(&actual, expected);
        } else {
            expected = Some(actual);
        }
    }
}
