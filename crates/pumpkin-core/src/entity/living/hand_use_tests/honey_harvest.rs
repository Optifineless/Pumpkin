use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hand_use_honey_releases_angry_bee_without_smoke() {
    use pumpkin_data::block_properties::BeeNestLikeProperties;
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    let pos = BlockPos::new(0, 64, 1);
    let mut props = BeeNestLikeProperties::default(&Block::BEEHIVE);
    props.honey_level = 5;
    let mut chunk = spawn_test_support::proto(&Biome::PLAINS, &Block::STONE);
    chunk.set_block_state(0, 64, 1, props.to_state_id(&Block::BEEHIVE).to_state());
    spawn_test_support::publish(&world, chunk);
    let hive = Arc::new(crate::block::entities::beehive::BeehiveBlockEntity::new(
        pos,
    ));
    let mut data = pumpkin_nbt::compound::NbtCompound::new();
    data.put_string("id", "minecraft:bee".to_owned());
    let mut occupant = pumpkin_nbt::compound::NbtCompound::new();
    occupant.put_compound("entity_data", data);
    *hive.bees.lock().unwrap() = Some(vec![pumpkin_nbt::tag::NbtTag::Compound(occupant)]);
    world.add_block_entity(hive.clone());
    let fixture = test_player(&world);
    let inventory = fixture.player.inventory();
    inventory.set_stack(0, ItemStack::new(1, &Item::GLASS_BOTTLE));
    use_block(&fixture, &server, pos, Hand::Right);
    assert_eq!(
        fixture.player.inventory().held_item().item,
        &Item::HONEY_BOTTLE
    );
    assert_eq!(
        BeeNestLikeProperties::from_state_id(world.get_block_state_id(&pos)).honey_level,
        0
    );
    assert!(hive.bees.lock().unwrap().as_ref().unwrap().is_empty());
    assert!(
        !fixture
            .player
            .has_advancement(pumpkin_data::Advancement::HUSBANDRY_SAFELY_HARVEST_HONEY)
    );
    let bees = world.get_entities_at_box(
        &pumpkin_util::math::boundingbox::BoundingBox::from_block(&pos).expand_all(4.0),
    );
    let bee = bees
        .iter()
        .find(|entity| entity.get_entity().entity_type == &pumpkin_data::entity::EntityType::BEE)
        .unwrap();
    assert_eq!(
        bee.get_mob()
            .unwrap()
            .get_mob_entity()
            .get_target()
            .unwrap()
            .get_entity()
            .entity_uuid,
        fixture.player.get_entity().entity_uuid
    );
    world.set_block_state(
        &pos,
        props.to_state_id(&Block::BEEHIVE),
        pumpkin_world::world::BlockFlags::NOTIFY_ALL,
    );
    world.set_block_state(
        &pos.down(),
        Block::CAMPFIRE.default_state.id,
        pumpkin_world::world::BlockFlags::NOTIFY_ALL,
    );
    let mut data = pumpkin_nbt::compound::NbtCompound::new();
    data.put_string("id", "minecraft:bee".to_owned());
    let mut occupant = pumpkin_nbt::compound::NbtCompound::new();
    occupant.put_compound("entity_data", data);
    *hive.bees.lock().unwrap() = Some(vec![pumpkin_nbt::tag::NbtTag::Compound(occupant)]);
    inventory.set_stack(0, ItemStack::new(1, &Item::GLASS_BOTTLE));
    use_block(&fixture, &server, pos, Hand::Right);
    assert_eq!(hive.bees.lock().unwrap().as_ref().unwrap().len(), 1);
    assert!(
        fixture
            .player
            .has_advancement(pumpkin_data::Advancement::HUSBANDRY_SAFELY_HARVEST_HONEY)
    );
    assert_eq!(
        fixture.player.stats.lock().unwrap().get(
            pumpkin_data::statistic::StatisticCategory::Used,
            i32::from(Item::GLASS_BOTTLE.id)
        ),
        2
    );
    assert!(world.level.shutdown().await.is_ok());
    crate::server::fixture_lifecycle::finish().await;
}
