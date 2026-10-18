use crate::{
    entity::{
        Entity, EntityBase, NBTStorage,
        ai::{
            goal::{Goal, active_target::ActiveTargetGoal},
            target_predicate::TargetPredicate,
        },
        mob::{Mob, piglin::PiglinEntity, piglin_ai::PiglinAi, skeleton::skeleton::SkeletonEntity},
    },
    net::java::combat_test_support::TestPlayer,
    server::combat_test_support,
    world::spawn_test_support,
};
use pumpkin_data::{
    Block, biome::Biome, data_component::DataComponent, entity::EntityType, item::Item,
    item_stack::ItemStack,
};
use pumpkin_inventory::Inventory;
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_protocol::{
    VarInt, codec::data_component, java::server::play::SSetHeldItem, ser::NetworkWriteExt,
};
use pumpkin_util::math::vector3::Vector3;

fn custom_item(slot: &str, visibility: Option<f32>) -> ItemStack {
    let mut equippable = NbtCompound::new();
    equippable.put_string("slot", slot.to_owned());
    let mut components = NbtCompound::new();
    components.put_compound("minecraft:equippable", equippable);
    if let Some(visibility) = visibility {
        let mut component = NbtCompound::new();
        component.put_string("targeting_entity_types", "minecraft:skeleton".to_owned());
        component.put_float("visibility", visibility);
        components.put_compound("minecraft:mob_visibility", component);
    }
    let mut item = NbtCompound::new();
    item.put_string("id", "minecraft:stone".to_owned());
    item.put_int("count", 1);
    item.put_compound("components", components);
    ItemStack::read_item_stack(&item).expect("custom item NBT")
}

fn selects_player(mob: &dyn Mob, player: &TestPlayer) -> bool {
    let mut goal = ActiveTargetGoal::new(
        mob.get_mob_entity(),
        &EntityType::PLAYER,
        0,
        true,
        false,
        None::<fn(&crate::entity::living::LivingEntity, &crate::world::World) -> bool>,
    );
    mob.get_mob_entity().set_target(None);
    if !goal.can_start(mob) {
        return false;
    }
    goal.start(mob);
    assert_eq!(
        mob.get_mob_entity()
            .get_target()
            .unwrap()
            .get_entity()
            .entity_id,
        player.player.get_entity().entity_id
    );
    true
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn visibility_target_goal_scales_sneaking_invisibility_and_armor() {
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    spawn_test_support::publish(
        &world,
        spawn_test_support::proto(&Biome::PLAINS, &Block::STONE),
    );
    let player = TestPlayer::new(&world);
    let skeleton = SkeletonEntity::new(Entity::new(
        world.clone(),
        Vector3::new(0.5, 64.0, 0.5),
        &EntityType::SKELETON,
    ));
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(14.0, 64.0, 0.5));
    assert!(selects_player(skeleton.as_ref(), &player));
    player.player.get_entity().set_sneaking(true);
    assert!(!selects_player(skeleton.as_ref(), &player));
    assert!(
        TargetPredicate::create_attackable()
            .set_base_max_distance(16.0)
            .ignore_distance_scaling_factor()
            .test(&world, Some(skeleton.as_ref()), player.player.as_ref())
    );
    player.player.get_entity().set_sneaking(false);
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(3.5, 64.0, 0.5));
    player.player.get_entity().set_invisible(true);
    assert!(!selects_player(skeleton.as_ref(), &player));
    player
        .player
        .inventory()
        .set_stack(38, ItemStack::new(1, &Item::IRON_CHESTPLATE));
    assert!(!selects_player(skeleton.as_ref(), &player));
    player
        .player
        .inventory()
        .set_stack(36, ItemStack::new(1, &Item::IRON_BOOTS));
    assert!(selects_player(skeleton.as_ref(), &player));
    assert!(world.level.shutdown().await.is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn visibility_hotbar_component_follows_real_carried_item_selection() {
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    spawn_test_support::publish(
        &world,
        spawn_test_support::proto(&Biome::PLAINS, &Block::STONE),
    );
    let player = TestPlayer::new(&world);
    let skeleton = SkeletonEntity::new(Entity::new(
        world.clone(),
        Vector3::new(0.5, 64.0, 0.5),
        &EntityType::SKELETON,
    ));
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(6.5, 64.0, 0.5));
    // A saved equipment entry is stale derived state: selected hotbar stock owns MAIN_HAND.
    let mut cached_hand = NbtCompound::new();
    ItemStack::new(1, &Item::DIAMOND_SWORD).write_item_stack(&mut cached_hand);
    let mut equipment = NbtCompound::new();
    equipment.put_compound("mainhand", cached_hand);
    let mut saved = NbtCompound::new();
    saved.put_compound("equipment", equipment);
    player.player.living_entity.read_living_nbt_non_mut(&saved);
    player
        .player
        .inventory()
        .set_stack(0, custom_item("mainhand", Some(0.25)));
    player
        .player
        .inventory()
        .set_stack(1, ItemStack::new(1, &Item::STONE));
    assert!(!selects_player(skeleton.as_ref(), &player));
    player
        .client()
        .handle_set_held_item(&server, &player.player, &SSetHeldItem { slot: 1 });
    assert_eq!(player.player.inventory().get_selected_slot(), 1);
    assert!(selects_player(skeleton.as_ref(), &player));
    player
        .client()
        .handle_set_held_item(&server, &player.player, &SSetHeldItem { slot: 0 });
    assert!(!selects_player(skeleton.as_ref(), &player));
    player
        .player
        .inventory()
        .update_slot(0, &mut |stack| stack.item_count = 0);
    assert!(selects_player(skeleton.as_ref(), &player));
    // A component intended for a different equipment slot must not apply from the hand.
    player
        .player
        .inventory()
        .set_stack(0, custom_item("head", Some(0.0)));
    assert!(selects_player(skeleton.as_ref(), &player));
    assert!(world.level.shutdown().await.is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn visibility_loaded_legacy_and_vanilla_skulls_keep_item_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    spawn_test_support::publish(
        &world,
        spawn_test_support::proto(&Biome::PLAINS, &Block::STONE),
    );
    let player = TestPlayer::new(&world);
    let skeleton = SkeletonEntity::new(Entity::new(
        world.clone(),
        Vector3::new(0.5, 64.0, 0.5),
        &EntityType::SKELETON,
    ));
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(10.5, 64.0, 0.5));
    for legacy in [true, false] {
        let mut skull = NbtCompound::new();
        skull.put_string("id", "minecraft:skeleton_skull".to_owned());
        skull.put_int("count", 1);
        let mut saved = NbtCompound::new();
        if legacy {
            skull.put_byte("Slot", 103);
            saved.put_list("Inventory", vec![NbtTag::Compound(skull)]);
        } else {
            let mut equipment = NbtCompound::new();
            equipment.put_compound("head", skull);
            saved.put_compound("equipment", equipment);
        }
        player.player.inventory().read_nbt_non_mut(&saved);
        assert!(!selects_player(skeleton.as_ref(), &player));
        // Removing the default component explicitly must also survive the normal save codec.
        let mut removed = NbtCompound::new();
        ItemStack::new(1, &Item::SKELETON_SKULL).write_item_stack(&mut removed);
        let mut components = NbtCompound::new();
        components.put_compound("!minecraft:mob_visibility", NbtCompound::new());
        removed.put_compound("components", components);
        let removed = ItemStack::read_item_stack(&removed).unwrap();
        player.player.inventory().set_stack(39, removed);
        assert!(selects_player(skeleton.as_ref(), &player));
    }
    assert!(world.level.shutdown().await.is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn visibility_piglin_targetability_preserves_stream_nan_range() {
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    for chunk_x in [0, 1] {
        let mut chunk = spawn_test_support::proto(&Biome::PLAINS, &Block::STONE);
        chunk.x = chunk_x;
        spawn_test_support::publish(&world, chunk);
    }
    let player = TestPlayer::new(&world);
    let piglin = PiglinEntity::new(Entity::new(
        world.clone(),
        Vector3::new(0.5, 64.0, 0.5),
        &EntityType::PIGLIN,
    ));
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(20.5, 64.0, 0.5));
    assert!(!PiglinAi::is_entity_targetable(&piglin, &player.player));
    // MobVisibility.STREAM_CODEC accepts NaN. Its direct holder set contains this piglin type.
    let mut payload = vec![2];
    payload
        .write_var_int(&VarInt(i32::from(EntityType::PIGLIN.id)))
        .unwrap();
    payload.extend(f32::NAN.to_be_bytes());
    let component =
        data_component::deserialize(DataComponent::MobVisibility, &mut payload.as_slice()).unwrap();
    let mut item = custom_item("mainhand", None);
    item.patch
        .push((DataComponent::MobVisibility, Some(component)));
    player.player.inventory().set_stack(0, item);
    assert!(PiglinAi::is_entity_targetable(&piglin, &player.player));
    assert!(
        TargetPredicate::create_attackable()
            .set_base_max_distance(16.0)
            .test(&world, Some(piglin.as_ref()), player.player.as_ref())
    );
    assert!(world.level.shutdown().await.is_ok());
}
