use super::*;
use pumpkin_data::{Block, advancement::Advancement, item::Item, item_stack::ItemStack};
use pumpkin_protocol::{codec::var_int::VarInt, java::server::play::SUseItemOn};
use pumpkin_util::{GameMode, math::vector3::Vector3};

fn sign_fixture(
    path: &std::path::Path,
) -> (
    Arc<crate::server::Server>,
    Arc<World>,
    TestPlayer,
    BlockPos,
    Arc<dyn BlockEntity>,
) {
    let (server, world) = test_world(path);
    let pos = BlockPos::new(0, 64, 1);
    let mut chunk = spawn_test_support::proto(&Biome::PLAINS, &Block::STONE);
    let state = Block::OAK_SIGN
        .from_properties(&[("rotation", "0")])
        .to_state_id(&Block::OAK_SIGN);
    chunk.set_block_state(0, 64, 1, state.to_state());
    spawn_test_support::publish(&world, chunk);

    let entity: Arc<dyn BlockEntity> = Arc::new(SignBlockEntity::empty(pos));
    world.add_block_entity(entity.clone());
    let fixture = TestPlayer::new(&world);
    fixture
        .player
        .advancements
        .try_lock()
        .unwrap()
        .set_player(&fixture.player);
    fixture
        .player
        .get_entity()
        .set_pos(Vector3::new(0.5, 64.0, 3.5));
    (server, world, fixture, pos, entity)
}

fn use_item_on_sign(fixture: &TestPlayer, server: &Arc<crate::server::Server>, pos: BlockPos) {
    fixture
        .client()
        .handle_use_item_on(
            &fixture.player,
            &SUseItemOn {
                hand: VarInt(0),
                position: pos,
                face: VarInt(3),
                cursor_pos: Vector3::new(0.5, 0.5, 1.0),
                inside_block: false,
                is_against_world_border: false,
                sequence: VarInt(1),
            },
            server,
        )
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn glow_ink_applies_only_to_a_nonblank_selected_sign_face() {
    let dir = tempfile::tempdir().unwrap();
    let (server, world, fixture, pos, entity) = sign_fixture(dir.path());
    let sign = SignEntityRef::from_block_entity(&*entity).unwrap();
    let front = sign.get_text(true);
    let back = sign.get_text(false);
    let player = &fixture.player;
    player.gamemode.store(GameMode::Survival);
    player
        .inventory()
        .set_stack(0, ItemStack::new(2, &Item::GLOW_INK_SAC));

    assert!(is_facing_front_text(
        &world,
        &pos,
        world.get_block(&pos),
        player
    ));
    let glow_advancement = Advancement::HUSBANDRY_MAKE_A_SIGN_GLOW;
    assert!(!player.has_advancement(glow_advancement));
    use_item_on_sign(&fixture, &server, pos);
    assert!(!front.has_glowing_text());
    assert_eq!(player.inventory().get_slot(0).item_count, 2);
    assert!(!player.has_advancement(glow_advancement));

    front.set_message(0, "front text".into(), None);
    use_item_on_sign(&fixture, &server, pos);
    assert!(front.has_glowing_text());
    assert_eq!(player.inventory().get_slot(0).item_count, 1);
    assert!(player.has_advancement(glow_advancement));

    player
        .inventory()
        .set_stack(0, ItemStack::new(2, &Item::GLOW_INK_SAC));
    use_item_on_sign(&fixture, &server, pos);
    assert_eq!(player.inventory().get_slot(0).item_count, 2);

    front.set_has_glowing_text(false);
    front.set_message(0, "".into(), None);
    back.set_message(0, "opposite face text".into(), None);
    use_item_on_sign(&fixture, &server, pos);
    assert!(!front.has_glowing_text());
    assert!(back.has_message(false));
    assert_eq!(player.inventory().get_slot(0).item_count, 2);

    let mut config = (**player.config.load()).clone();
    config.text_filtering = true;
    player.config.store(Arc::new(config));
    front.set_message(0, "unfiltered text".into(), Some("".into()));
    use_item_on_sign(&fixture, &server, pos);
    assert!(!front.has_glowing_text());
    assert_eq!(player.inventory().get_slot(0).item_count, 2);
    world.level.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn honeycomb_still_waxes_a_blank_sign_face() {
    let dir = tempfile::tempdir().unwrap();
    let (server, world, fixture, pos, entity) = sign_fixture(dir.path());
    let sign = SignEntityRef::from_block_entity(&*entity).unwrap();
    let player = &fixture.player;
    player.gamemode.store(GameMode::Survival);
    player
        .inventory()
        .set_stack(0, ItemStack::new(2, &Item::HONEYCOMB));

    use_item_on_sign(&fixture, &server, pos);

    assert!(!sign.front_text().has_message(false));
    assert!(!sign.back_text().has_message(false));
    assert!(sign.is_waxed());
    assert_eq!(player.inventory().get_slot(0).item_count, 1);
    world.level.shutdown().await.unwrap();
}
