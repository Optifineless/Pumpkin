use crate::{
    block::BlockIsReplacing,
    entity::death_test_world::DeathTestWorld,
    world::spawn_test_support::{proto, publish},
};
use pumpkin_data::{
    Block, BlockDirection, BlockStateId,
    biome::Biome,
    block_properties::{NoteBlockLikeProperties, NoteblockInstrument},
};
use pumpkin_protocol::{VarInt, java::server::play::SUseItemOn};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use pumpkin_world::world::BlockFlags;
use std::sync::Arc;

const NOTE_POS: BlockPos = BlockPos::new(8, 64, 8);

fn put(world: &Arc<crate::world::World>, position: BlockPos, state: BlockStateId) {
    world.set_block_state(
        &position,
        state,
        BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
    );
}

fn place_note(
    fixture: &DeathTestWorld,
    world: &Arc<crate::world::World>,
    player: &crate::entity::player::Player,
    position: &BlockPos,
) -> BlockStateId {
    let use_item_on = SUseItemOn {
        hand: VarInt(0),
        position: *position,
        face: VarInt(1),
        cursor_pos: Vector3::new(0.5, 1.0, 0.5),
        inside_block: false,
        is_against_world_border: false,
        sequence: VarInt(1),
    };
    fixture.server.block_registry.on_place(
        &fixture.server,
        world,
        player,
        &Block::NOTE_BLOCK,
        position,
        BlockDirection::Up,
        BlockIsReplacing::None,
        &use_item_on,
    )
}

fn note_instrument(state: BlockStateId) -> NoteblockInstrument {
    NoteBlockLikeProperties::from_state_id(state).instrument
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn note_block_placement_selects_supported_base_instruments() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let player = fixture.player("note-placer");

    let cases = [
        ("stone", &Block::STONE, NoteblockInstrument::Basedrum),
        ("diamond", &Block::DIAMOND_BLOCK, NoteblockInstrument::Harp),
        ("glowstone", &Block::GLOWSTONE, NoteblockInstrument::Pling),
        ("copper", &Block::COPPER_BLOCK, NoteblockInstrument::Trumpet),
        (
            "exposed copper",
            &Block::EXPOSED_COPPER,
            NoteblockInstrument::TrumpetExposed,
        ),
        (
            "oxidized copper",
            &Block::OXIDIZED_COPPER,
            NoteblockInstrument::TrumpetOxidized,
        ),
        (
            "weathered copper",
            &Block::WEATHERED_COPPER,
            NoteblockInstrument::TrumpetWeathered,
        ),
    ];
    let mut mismatches = Vec::new();
    for (name, support, expected) in cases {
        put(&world, NOTE_POS.down(), support.default_state.id);
        put(&world, NOTE_POS.up(), Block::AIR.default_state.id);
        let actual = note_instrument(place_note(&fixture, &world, &player, &NOTE_POS));
        if actual != expected {
            mismatches.push((name, expected, actual));
        }
    }
    assert!(
        mismatches.is_empty(),
        "wrong placement instruments: {mismatches:?}"
    );
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn note_block_placement_preserves_mob_head_behavior() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let player = fixture.player("note-placer");

    put(&world, NOTE_POS.up(), Block::AIR.default_state.id);
    put(&world, NOTE_POS.down(), Block::ZOMBIE_HEAD.default_state.id);
    assert_eq!(
        note_instrument(place_note(&fixture, &world, &player, &NOTE_POS)),
        NoteblockInstrument::Harp,
        "a mob head below the note block should retain the Harp fallback"
    );

    put(&world, NOTE_POS.down(), Block::STONE.default_state.id);
    put(&world, NOTE_POS.up(), Block::ZOMBIE_HEAD.default_state.id);
    assert_eq!(
        note_instrument(place_note(&fixture, &world, &player, &NOTE_POS)),
        NoteblockInstrument::Zombie,
        "a mob head above the note block should retain its instrument override"
    );
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn note_block_vertical_neighbor_update_reclassifies_support_instruments() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let player = fixture.player("note-placer");
    put(&world, NOTE_POS.down(), Block::STONE.default_state.id);
    put(&world, NOTE_POS.up(), Block::AIR.default_state.id);
    put(
        &world,
        NOTE_POS,
        place_note(&fixture, &world, &player, &NOTE_POS),
    );

    let cases = [
        (&Block::DIAMOND_BLOCK, NoteblockInstrument::Harp),
        (&Block::GLOWSTONE, NoteblockInstrument::Pling),
        (&Block::COPPER_BLOCK, NoteblockInstrument::Trumpet),
        (&Block::EXPOSED_COPPER, NoteblockInstrument::TrumpetExposed),
        (
            &Block::OXIDIZED_COPPER,
            NoteblockInstrument::TrumpetOxidized,
        ),
        (
            &Block::WEATHERED_COPPER,
            NoteblockInstrument::TrumpetWeathered,
        ),
    ];
    let mut mismatches = Vec::new();
    for (support, expected) in cases {
        let support_position = NOTE_POS.down();
        let support_state = support.default_state.id;
        put(&world, support_position, support_state);
        let note_state = world.get_block_state_id(&NOTE_POS);
        let updated = fixture.server.block_registry.get_state_for_neighbor_update(
            &world,
            &Block::NOTE_BLOCK,
            note_state,
            &NOTE_POS,
            BlockDirection::Down,
            &support_position,
            support_state,
        );
        let actual = note_instrument(updated);
        if actual != expected {
            mismatches.push((support.name, expected, actual));
        }
        put(&world, NOTE_POS, updated);
    }
    assert!(
        mismatches.is_empty(),
        "wrong updated instruments: {mismatches:?}"
    );
    fixture.server.shutdown().await;
}
