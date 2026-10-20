use super::*;
use crate::plugin::api::events::block::block_cook::BlockCookEvent;

struct CookHandler {
    calls: Arc<AtomicUsize>,
    campfire: Arc<CampfireBlockEntity>,
    cancelled_items: Vec<&'static Item>,
}

impl EventHandler<BlockCookEvent> for CookHandler {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut BlockCookEvent,
    ) -> BoxFuture<'a, ()> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        if event.block_pos == self.campfire.position {
            let transaction = self.campfire.slot_transaction(0);
            assert!(
                transaction.try_lock().is_some(),
                "cook callback must not hold the campfire transaction"
            );
            if self.cancelled_items.contains(&event.source.item) {
                event.cancelled = true;
            }
        }
        Box::pin(async {})
    }
}

fn register_cook_handler(
    fixture: &DeathTestWorld,
    campfire: &Arc<CampfireBlockEntity>,
    cancelled_items: Vec<&'static Item>,
) -> Arc<AtomicUsize> {
    let calls = Arc::new(AtomicUsize::new(0));
    fixture.server.plugin_manager.register::<BlockCookEvent, _>(
        Arc::new(CookHandler {
            calls: calls.clone(),
            campfire: campfire.clone(),
            cancelled_items,
        }),
        EventPriority::Normal,
        true,
    );
    calls
}

fn saved_campfire(items: [ItemStack; 4], progress: [i32; 4], totals: [i32; 4]) -> NbtCompound {
    let mut nbt = NbtCompound::new();
    let mut saved_items = Vec::new();
    for (slot, stack) in items.into_iter().enumerate() {
        if stack.is_empty() {
            continue;
        }
        let mut item_nbt = NbtCompound::new();
        item_nbt.put_byte("Slot", slot as i8);
        stack.write_item_stack(&mut item_nbt);
        saved_items.push(NbtTag::Compound(item_nbt));
    }
    nbt.put_list("Items", saved_items);
    nbt.put("CookingTimes", NbtTag::IntArray(progress.to_vec()));
    nbt.put("CookingTotalTimes", NbtTag::IntArray(totals.to_vec()));
    nbt
}

fn saved_stack(nbt: &NbtCompound, slot: i8) -> ItemStack {
    nbt.get_list("Items")
        .unwrap()
        .iter()
        .filter_map(|tag| tag.extract_compound())
        .find(|item| item.get_byte("Slot") == Some(slot))
        .and_then(ItemStack::read_item_stack)
        .unwrap_or_else(|| ItemStack::EMPTY.clone())
}

fn install_campfire(
    world: &Arc<World>,
    position: BlockPos,
    lit: bool,
    nbt: &NbtCompound,
) -> Arc<CampfireBlockEntity> {
    let mut properties = CampfireLikeProperties::default(&Block::CAMPFIRE);
    properties.lit = lit;
    world.set_block_state(
        &position,
        properties.to_state_id(&Block::CAMPFIRE),
        BlockFlags::NOTIFY_ALL,
    );
    let campfire = Arc::new(CampfireBlockEntity::from_nbt(nbt, position));
    world.add_block_entity(campfire.clone());
    campfire
}

fn use_item_on(
    fixture: &DeathTestWorld,
    client: &TestPlayer,
    position: BlockPos,
    hand: i32,
    face: i32,
    sequence: i32,
) {
    client
        .client()
        .handle_use_item_on(
            &client.player,
            &SUseItemOn {
                hand: VarInt(hand),
                position,
                face: VarInt(face),
                cursor_pos: Vector3::new(0.5, 0.5, 0.5),
                inside_block: false,
                is_against_world_border: false,
                sequence: VarInt(sequence),
            },
            &fixture.server,
        )
        .unwrap();
}

fn assert_saved_item(nbt: &NbtCompound, slot: i8, item: &'static Item, count: u8) {
    let stack = saved_stack(nbt, slot);
    assert_eq!(stack.item, item);
    assert_eq!(stack.item_count, count);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g09_campfire_cancel_then_extinguish_clamps() {
    bounded(async {
        let (fixture, client, campfire) = setup(&Block::CAMPFIRE, true).await;
        let world = fixture.world();
        client
            .player
            .inventory()
            .set_stack(40, ItemStack::new(2, &Item::BEEF));
        client
            .player
            .inventory()
            .set_stack(0, ItemStack::new(1, &Item::IRON_SHOVEL));
        register(&fixture, &client, &campfire, 40, Action::Duration(100));
        use_food(&fixture, &client, 1);

        assert_eq!(campfire.get_stack(0).item_count, 1);
        assert_eq!(client.player.inventory().get_stack(40).item_count, 1);
        assert_eq!(*campfire.cooking_total_times[0].lock().unwrap(), 100);

        let cook_calls = register_cook_handler(&fixture, &campfire, vec![&Item::BEEF]);
        for _ in 0..110 {
            campfire.tick(&world);
        }
        assert_eq!(*campfire.cooking_times[0].lock().unwrap(), 110);
        assert_eq!(cook_calls.load(Ordering::Relaxed), 11);
        assert_eq!(dropped(&world, &Item::COOKED_BEEF), 0);

        use_item_on(&fixture, &client, POS, 0, 1, 2);
        assert!(!CampfireLikeProperties::from_state_id(world.get_block_state_id(&POS)).lit);
        assert_eq!(client.player.inventory().get_stack(40).item_count, 1);

        campfire.tick(&world);
        let mut cooled_once = NbtCompound::new();
        campfire.write_internal(&mut cooled_once);
        assert_eq!(cooled_once.get_int_array("CookingTimes").unwrap()[0], 100);
        assert_eq!(
            cooled_once.get_int_array("CookingTotalTimes").unwrap()[0],
            100
        );
        assert_saved_item(&cooled_once, 0, &Item::BEEF, 1);

        campfire.tick(&world);
        let mut cooled_twice = NbtCompound::new();
        campfire.write_internal(&mut cooled_twice);
        assert_eq!(cooled_twice.get_int_array("CookingTimes").unwrap()[0], 98);
        assert_saved_item(&cooled_twice, 0, &Item::BEEF, 1);
        assert_eq!(campfire.get_stack(0).item_count, 1);
        assert_eq!(client.player.inventory().get_stack(40).item_count, 1);
        assert_eq!(dropped(&world, &Item::COOKED_BEEF), 0);
        assert_eq!(dropped(&world, &Item::BEEF), 0);
        fixture.server.shutdown().await;
    })
    .await;
}

fn check_boundary_controls(world: &Arc<World>) {
    let position = POS.east();
    let nbt = saved_campfire(
        [
            ItemStack::new(1, &Item::BEEF),
            ItemStack::new(1, &Item::PORKCHOP),
            ItemStack::new(1, &Item::CHICKEN),
            ItemStack::new(1, &Item::KELP),
        ],
        [1, 12, 0, -3],
        [40, 30, 20, 10],
    );
    let boundary = install_campfire(world, position, false, &nbt);
    boundary.tick(world);
    let mut saved = NbtCompound::new();
    boundary.write_internal(&mut saved);
    assert_eq!(
        saved.get_int_array("CookingTimes").unwrap(),
        &[0, 10, 0, -3]
    );
    for (slot, item) in [&Item::BEEF, &Item::PORKCHOP, &Item::CHICKEN, &Item::KELP]
        .into_iter()
        .enumerate()
    {
        assert_saved_item(&saved, slot as i8, item, 1);
    }
}

fn check_overshoot_relight_controls(
    fixture: &DeathTestWorld,
    client: &TestPlayer,
    world: &Arc<World>,
) -> NbtCompound {
    let position = POS.north();
    let nbt = saved_campfire(
        [
            ItemStack::new(1, &Item::BEEF),
            ItemStack::new(1, &Item::PORKCHOP),
            ItemStack::new(1, &Item::CHICKEN),
            ItemStack::new(1, &Item::KELP),
        ],
        [110, 25, 35, 45],
        [100, 20, 30, 40],
    );
    let overshoot = install_campfire(world, position, false, &nbt);
    overshoot.tick(world);
    let mut saved = NbtCompound::new();
    overshoot.write_internal(&mut saved);
    assert_eq!(
        saved.get_int_array("CookingTotalTimes").unwrap(),
        &[100, 20, 30, 40]
    );
    for (slot, item) in [&Item::BEEF, &Item::PORKCHOP, &Item::CHICKEN, &Item::KELP]
        .into_iter()
        .enumerate()
    {
        assert_saved_item(&saved, slot as i8, item, 1);
    }
    assert_eq!(dropped(world, &Item::DRIED_KELP), 0);

    let cancelled = register_cook_handler(
        fixture,
        &overshoot,
        vec![&Item::BEEF, &Item::PORKCHOP, &Item::CHICKEN],
    );
    client
        .player
        .inventory()
        .set_stack(0, ItemStack::new(1, &Item::FLINT_AND_STEEL));
    use_item_on(fixture, client, position, 0, 1, 3);
    assert!(CampfireLikeProperties::from_state_id(world.get_block_state_id(&position)).lit);
    overshoot.tick(world);
    overshoot.tick(world);
    assert_eq!(cancelled.load(Ordering::Relaxed), 7);
    assert_eq!(dropped(world, &Item::DRIED_KELP), 1);
    assert_eq!(overshoot.get_stack(0).item, &Item::BEEF);
    assert_eq!(overshoot.get_stack(1).item, &Item::PORKCHOP);
    assert_eq!(overshoot.get_stack(2).item, &Item::CHICKEN);
    assert!(overshoot.get_stack(3).is_empty());
    saved
}

fn check_ordinary_cooking(world: &Arc<World>) {
    let position = POS.south();
    let nbt = saved_campfire(
        [
            ItemStack::new(1, &Item::PORKCHOP),
            ItemStack::EMPTY.clone(),
            ItemStack::EMPTY.clone(),
            ItemStack::EMPTY.clone(),
        ],
        [19, 0, 0, 0],
        [20, 0, 0, 0],
    );
    let ordinary = install_campfire(world, position, true, &nbt);
    ordinary.tick(world);
    assert!(ordinary.is_empty());
    assert_eq!(dropped(world, &Item::COOKED_PORKCHOP), 1);
    ordinary.tick(world);
    assert_eq!(dropped(world, &Item::COOKED_PORKCHOP), 1);
}

fn check_negative_plugin_duration(
    fixture: &DeathTestWorld,
    client: &TestPlayer,
    world: &Arc<World>,
    campfire: &Arc<CampfireBlockEntity>,
) {
    client
        .player
        .inventory()
        .set_stack(40, ItemStack::new(2, &Item::BEEF));
    let calls = register(fixture, client, campfire, 40, Action::Duration(-1));
    use_food(fixture, client, 1);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    let mut accepted = NbtCompound::new();
    campfire.write_internal(&mut accepted);
    assert_eq!(accepted.get_int_array("CookingTotalTimes").unwrap()[0], -1);
    accepted.put("CookingTimes", NbtTag::IntArray(vec![1, 0, 0, 0]));
    let negative = install_campfire(world, POS.west(), false, &accepted);
    negative.tick(world);
    assert_eq!(*negative.cooking_times[0].lock().unwrap(), -1);
    negative.tick(world);
    let mut saved = NbtCompound::new();
    negative.write_internal(&mut saved);
    assert_eq!(saved.get_int_array("CookingTimes").unwrap()[0], -1);
    assert_eq!(saved.get_int_array("CookingTotalTimes").unwrap()[0], -1);
    assert_saved_item(&saved, 0, &Item::BEEF, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g09_campfire_cooldown_slot_and_boundary_controls() {
    bounded(async {
        let (fixture, client, admitted_campfire) = setup(&Block::CAMPFIRE, false).await;
        let world = fixture.world();
        check_boundary_controls(&world);
        let overshoot_saved = check_overshoot_relight_controls(&fixture, &client, &world);
        check_ordinary_cooking(&world);
        check_negative_plugin_duration(&fixture, &client, &world, &admitted_campfire);
        assert_eq!(
            overshoot_saved.get_int_array("CookingTimes").unwrap(),
            &[100, 20, 30, 40],
            "each slot must clamp to its paired cooking duration"
        );
        fixture.server.shutdown().await;
    })
    .await;
}
