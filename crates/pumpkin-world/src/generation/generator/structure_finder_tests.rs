use super::{VanillaGenerator, WorldGenerator, structure_finder::find_nearest_structure_start};
use crate::{
    ProtoChunk,
    generation::structure::{
        height_sampler::NoiseHeightSampler,
        piece::StructurePieceType,
        structures::{HeightSampler, StructureInstance, StructurePosition},
    },
    world_info::BiomeSource,
};
use pumpkin_data::{
    dimension::Dimension,
    structures::{StructureKeys, StructureSet},
};
use pumpkin_util::{
    math::{block_box::BlockBox, position::BlockPos},
    world_seed::Seed,
};
use std::{
    sync::{Arc, mpsc},
    thread,
    time::Duration,
};

const WAIT: Duration = Duration::from_secs(20);
const END_CHUNK: (i32, i32) = (25, 123);

pub(super) fn world(
    seed: u64,
    dimension: Dimension,
    biome: Option<&str>,
    set: &str,
) -> WorldGenerator {
    let source = biome.map(|biome| BiomeSource::Fixed {
        biome: format!("minecraft:{biome}"),
        biome_type: "minecraft:fixed".to_owned(),
    });
    WorldGenerator::Noise(Box::new(VanillaGenerator::new_with_all_settings(
        Seed(seed),
        dimension,
        None,
        source.as_ref(),
        Some(&[set.to_owned()]),
    )))
}

pub(super) fn noise(world: &WorldGenerator) -> &VanillaGenerator {
    let WorldGenerator::Noise(generator) = world else {
        unreachable!()
    };
    generator
}

pub(super) fn generate(world: &WorldGenerator, (x, z): (i32, i32)) -> ProtoChunk {
    let mut chunk = ProtoChunk::new(x, z, world);
    chunk.step_to_biomes(noise(world));
    chunk.set_structure_starts(noise(world));
    chunk
}

fn locate(
    world: &WorldGenerator,
    set: &StructureSet,
    key: StructureKeys,
    (x, z): (i32, i32),
) -> Option<(BlockPos, StructureKeys)> {
    find_nearest_structure_start(BlockPos::new(x * 16, 0, z * 16), set, &[key], 0, world)
}

pub(super) fn cached(
    world: &WorldGenerator,
    key: StructureKeys,
    (x, z): (i32, i32),
) -> Option<StructurePosition> {
    noise(world)
        .global_structure_cache
        .get_or_compute_structure_start(key, x, z, || {
            panic!("expected a cached start, including None")
        })
}

pub(super) fn owned(chunk: &ProtoChunk, key: StructureKeys) -> &StructurePosition {
    let Some(StructureInstance::Start(position)) = chunk.structure_starts.get(&key) else {
        panic!("generated chunk must own {key:?}")
    };
    position
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Pieces {
    start: BlockPos,
    pieces: Vec<(StructurePieceType, BlockBox)>,
}

pub(super) fn pieces(position: &StructurePosition) -> Pieces {
    let collector = position.collector.lock().unwrap();
    assert!(
        !collector.pieces.is_empty(),
        "a valid start must contain actual pieces"
    );
    Pieces {
        start: position.start_pos,
        pieces: collector
            .pieces
            .iter()
            .map(|piece| {
                let base = piece.get_structure_piece();
                (base.r#type, base.bounding_box)
            })
            .collect(),
    }
}

fn assert_order(
    seed: u64,
    dimension: Dimension,
    biome: Option<&str>,
    set_name: &str,
    set: &StructureSet,
    key: StructureKeys,
    chunk: (i32, i32),
) {
    let generation_first = world(seed, dimension.clone(), biome, set_name);
    let expected_chunk = generate(&generation_first, chunk);
    let expected = pieces(owned(&expected_chunk, key));
    assert_eq!(
        locate(&generation_first, set, key, chunk),
        Some((BlockPos::new(chunk.0 * 16, 0, chunk.1 * 16), key))
    );

    let locate_first = world(seed, dimension, biome, set_name);
    assert_eq!(
        locate(&locate_first, set, key, chunk),
        Some((BlockPos::new(chunk.0 * 16, 0, chunk.1 * 16), key)),
        "cold locate must accept the configured biome and generation height"
    );
    let located = cached(&locate_first, key, chunk).unwrap();
    assert_eq!(
        pieces(&located),
        expected,
        "locate must cache the same pieces and heights as generation"
    );
    let actual_chunk = generate(&locate_first, chunk);
    let actual = owned(&actual_chunk, key);
    assert_eq!(
        pieces(actual),
        expected,
        "locate-first must preserve actual generated pieces"
    );
    assert!(Arc::ptr_eq(&located.collector, &actual.collector));

    // A cold reference computation is the third writer of this same cache key.
    let references_first = world(seed, generation_first.dimension().clone(), biome, set_name);
    let neighbor = (chunk.0 + 1, chunk.1);
    let mut reference_chunk = generate(&references_first, neighbor);
    reference_chunk.set_structure_references(noise(&references_first));
    let reference_start = cached(&references_first, key, chunk).unwrap();
    assert_eq!(
        pieces(&reference_start),
        expected,
        "reference-first must compute the same start"
    );
    if reference_start.get_bounding_box().intersects_raw_xz(
        neighbor.0 * 16,
        neighbor.1 * 16,
        neighbor.0 * 16 + 15,
        neighbor.1 * 16 + 15,
    ) {
        let Some(StructureInstance::Reference(reference)) =
            reference_chunk.structure_starts.get(&key)
        else {
            panic!("neighbor must retain the intersecting {key:?} pieces")
        };
        assert!(Arc::ptr_eq(reference, &reference_start.collector));
    }
}

#[test]
fn b1019_g23_end_order_and_real_pieces() {
    assert_order(
        12_345,
        Dimension::THE_END,
        None,
        "end_cities",
        &StructureSet::END_CITIES,
        StructureKeys::EndCity,
        END_CHUNK,
    );
}

#[test]
fn b1019_g23_configured_fixed_swamp_order() {
    assert_order(
        0,
        Dimension::OVERWORLD,
        Some("swamp"),
        "swamp_huts",
        &StructureSet::SWAMP_HUTS,
        StructureKeys::SwampHut,
        (0, 18),
    );
}

#[test]
fn b1019_g23_nether_order_and_real_pieces() {
    assert_order(
        0,
        Dimension::THE_NETHER,
        None,
        "nether_complexes",
        &StructureSet::NETHER_COMPLEXES,
        StructureKeys::Fortress,
        (15, 2),
    );
}

#[test]
fn b1019_g23_generation_height_matches_locate() {
    // Upstream #4029's height-sensitive coastal village, independently regenerated here.
    assert_order(
        1_790_191_031_698_462_450,
        Dimension::OVERWORLD,
        None,
        "villages",
        &StructureSet::VILLAGES,
        StructureKeys::VillageSnowy,
        (222, 187),
    );
}

#[test]
fn b1019_g23_generation_first_controls() {
    for (seed, dimension, biome, set_name, set, key, chunk) in [
        (
            12_345,
            Dimension::THE_END,
            None,
            "end_cities",
            &StructureSet::END_CITIES,
            StructureKeys::EndCity,
            END_CHUNK,
        ),
        (
            0,
            Dimension::OVERWORLD,
            Some("swamp"),
            "swamp_huts",
            &StructureSet::SWAMP_HUTS,
            StructureKeys::SwampHut,
            (0, 18),
        ),
        (
            0,
            Dimension::THE_NETHER,
            None,
            "nether_complexes",
            &StructureSet::NETHER_COMPLEXES,
            StructureKeys::Fortress,
            (15, 2),
        ),
    ] {
        let world = world(seed, dimension, biome, set_name);
        let generated = generate(&world, chunk);
        let expected = pieces(owned(&generated, key));
        assert!(locate(&world, set, key, chunk).is_some());
        assert_eq!(pieces(&cached(&world, key, chunk).unwrap()), expected);
    }
}

#[test]
fn b1019_g23_invalid_biome_stays_absent() {
    let world = world(12_345, Dimension::THE_END, Some("plains"), "end_cities");
    assert!(
        locate(
            &world,
            &StructureSet::END_CITIES,
            StructureKeys::EndCity,
            END_CHUNK
        )
        .is_none()
    );
    assert!(
        !generate(&world, END_CHUNK)
            .structure_starts
            .contains_key(&StructureKeys::EndCity)
    );
    assert!(cached(&world, StructureKeys::EndCity, END_CHUNK).is_none());
}

#[test]
fn b1019_g23_invalid_height_stays_absent() {
    // This placement candidate is in the void. A valid fixed biome cannot admit it.
    let world = world(
        12_345,
        Dimension::THE_END,
        Some("end_highlands"),
        "end_cities",
    );
    let chunk = (4, 22);
    let mut heights = NoiseHeightSampler::new(noise(&world));
    assert_eq!(
        heights.estimate_height(chunk.0 * 16 + 7, chunk.1 * 16 + 7),
        0
    );
    assert!(
        locate(
            &world,
            &StructureSet::END_CITIES,
            StructureKeys::EndCity,
            chunk
        )
        .is_none()
    );
    assert!(
        !generate(&world, chunk)
            .structure_starts
            .contains_key(&StructureKeys::EndCity)
    );
}

#[test]
fn b1019_g23_overworld_refuses_end_city() {
    let world = world(12_345, Dimension::OVERWORLD, None, "end_cities");
    assert!(
        locate(
            &world,
            &StructureSet::END_CITIES,
            StructureKeys::EndCity,
            END_CHUNK
        )
        .is_none()
    );
    assert!(
        !generate(&world, END_CHUNK)
            .structure_starts
            .contains_key(&StructureKeys::EndCity)
    );
}

#[test]
fn b1019_g23_raw_generation_context_before_column_clipping() {
    // WorldGenerationContext.<init> uses min(heightAccessor.height, generator.genDepth),
    // not the overlap trimmed for NoiseBasedChunkGenerator's individual columns.
    let mut dimension = Dimension::THE_NETHER;
    dimension.min_y = 16;
    dimension.height = 128;
    assert_order(
        0,
        dimension.clone(),
        Some("soul_sand_valley"),
        "nether_fossils",
        &StructureSet::NETHER_FOSSILS,
        StructureKeys::NetherFossil,
        (0, 2),
    );
    for locate_first in [false, true] {
        let world = world(
            0,
            dimension.clone(),
            Some("soul_sand_valley"),
            "nether_fossils",
        );
        if locate_first {
            assert!(
                locate(
                    &world,
                    &StructureSet::NETHER_FOSSILS,
                    StructureKeys::NetherFossil,
                    (0, 0)
                )
                .is_none()
            );
        }
        let mut chunk = generate(&world, (0, 0));
        assert_eq!(chunk.generation_height(), 112);
        assert!(
            !chunk
                .structure_starts
                .contains_key(&StructureKeys::NetherFossil),
            "clipped height must not change the fossil's generation context"
        );
        assert!(
            locate(
                &world,
                &StructureSet::NETHER_FOSSILS,
                StructureKeys::NetherFossil,
                (0, 0)
            )
            .is_none()
        );
        chunk.set_structure_references(noise(&world));
        assert!(cached(&world, StructureKeys::NetherFossil, (0, 0)).is_none());
    }
    let world = world(0, dimension, Some("soul_sand_valley"), "nether_fossils");
    let mut neighbor = generate(&world, (1, 0));
    neighbor.set_structure_references(noise(&world));
    assert!(
        cached(&world, StructureKeys::NetherFossil, (0, 0)).is_none(),
        "cold references must use the raw context too"
    );
}

#[test]
fn b1019_g23_concurrent_real_locate_and_generation() {
    enum Completed {
        Locate(Option<(BlockPos, StructureKeys)>),
        Generation(Box<ProtoChunk>),
    }
    let world = Arc::new(world(12_345, Dimension::THE_END, None, "end_cities"));
    let (done_tx, done_rx) = mpsc::channel();
    let mut starts = Vec::new();
    let mut jobs = Vec::new();
    for generation in [false, true] {
        let world = world.clone();
        let done = done_tx.clone();
        let (start_tx, start_rx) = mpsc::channel();
        starts.push(start_tx);
        jobs.push(thread::spawn(move || {
            start_rx.recv_timeout(WAIT).unwrap();
            let result = if generation {
                Completed::Generation(Box::new(generate(&world, END_CHUNK)))
            } else {
                Completed::Locate(locate(
                    &world,
                    &StructureSet::END_CITIES,
                    StructureKeys::EndCity,
                    END_CHUNK,
                ))
            };
            done.send(result).unwrap();
        }));
    }
    for start in starts {
        start.send(()).unwrap();
    }
    let mut generated = None;
    let mut located = None;
    for _ in 0..2 {
        match done_rx.recv_timeout(WAIT).unwrap() {
            Completed::Locate(found) => located = found,
            Completed::Generation(chunk) => generated = Some(chunk),
        }
    }
    for job in jobs {
        job.join().unwrap();
    }
    assert!(located.is_some(), "concurrent cold End locate must succeed");
    let generated = generated.unwrap();
    let canonical = cached(&world, StructureKeys::EndCity, END_CHUNK).unwrap();
    assert_eq!(
        pieces(owned(&generated, StructureKeys::EndCity)),
        pieces(&canonical)
    );
    assert!(Arc::ptr_eq(
        &owned(&generated, StructureKeys::EndCity).collector,
        &canonical.collector
    ));
}

fn delayed_publication(late: Option<StructurePosition>, absent_winner: bool) {
    let world = Arc::new(world(
        12_345,
        Dimension::THE_END,
        absent_winner.then_some("plains"),
        "end_cities",
    ));
    let (entered_tx, entered_rx) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let worker_world = world.clone();
    let job = thread::spawn(move || {
        let result = noise(&worker_world)
            .global_structure_cache
            .get_or_compute_structure_start(
                StructureKeys::EndCity,
                END_CHUNK.0,
                END_CHUNK.1,
                || {
                    entered_tx.send(()).unwrap();
                    resume_rx.recv_timeout(WAIT).unwrap();
                    late
                },
            );
        done_tx.send(result).unwrap();
    });
    entered_rx.recv_timeout(WAIT).unwrap();
    let generated = generate(&world, END_CHUNK);
    let winner = cached(&world, StructureKeys::EndCity, END_CHUNK);
    assert_eq!(winner.is_none(), absent_winner);
    if let Some(winner) = &winner {
        assert!(Arc::ptr_eq(
            &winner.collector,
            &owned(&generated, StructureKeys::EndCity).collector
        ));
    }
    resume_tx.send(()).unwrap();
    let returned = done_rx.recv_timeout(WAIT).unwrap();
    job.join().unwrap();
    let retained = cached(&world, StructureKeys::EndCity, END_CHUNK);
    if let Some(winner) = winner {
        assert!(
            Arc::ptr_eq(
                &returned
                    .expect("late miss must reuse published Some")
                    .collector,
                &winner.collector
            ),
            "late miss must return the canonical mutable collector"
        );
        assert!(Arc::ptr_eq(&retained.unwrap().collector, &winner.collector));
    } else {
        assert!(returned.is_none(), "late miss must reuse published None");
        assert!(
            retained.is_none(),
            "negative cache entry must remain initialized"
        );
    }
}

#[test]
fn b1019_g23_first_published_collector_survives_late_miss() {
    let independent = world(12_345, Dimension::THE_END, None, "end_cities");
    let generated = generate(&independent, END_CHUNK);
    delayed_publication(
        Some(owned(&generated, StructureKeys::EndCity).clone()),
        false,
    );
}

#[test]
fn b1019_g23_published_start_survives_late_negative() {
    let independent = world(12_345, Dimension::THE_END, Some("plains"), "end_cities");
    let _ = generate(&independent, END_CHUNK);
    delayed_publication(
        cached(&independent, StructureKeys::EndCity, END_CHUNK),
        false,
    );
}

#[test]
fn b1019_g23_published_absence_survives_late_positive() {
    let independent = world(12_345, Dimension::THE_END, None, "end_cities");
    let generated = generate(&independent, END_CHUNK);
    delayed_publication(
        Some(owned(&generated, StructureKeys::EndCity).clone()),
        true,
    );
}
