use super::*;
use pumpkin_util::Hand;

fn use_saddle(
    scenario: &Scenario,
    session: &Session,
    entity: &Arc<dyn EntityBase>,
    hand: Hand,
) -> ItemStack {
    let mut saddle = ItemStack::new(1, &Item::SADDLE);
    saddle.set_custom_name("Keep this saddle".to_owned());
    session
        .player
        .inventory()
        .set_stack_in_hand(hand, saddle.clone());
    session.confirm_teleport(&scenario.server);
    session.send(
        &scenario.server,
        &SInteract {
            entity_id: VarInt(entity.get_entity().entity_id),
            r#type: VarInt(0),
            target_position: None,
            hand: Some(VarInt(i32::from(hand == Hand::Left))),
            sneaking: false,
        },
    );
    assert_retained(session, hand, &saddle);
    assert!(
        !horse(entity).is_saddled(),
        "refused saddle changed horse flags"
    );
    saddle
}

fn assert_retained(session: &Session, hand: Hand, expected: &ItemStack) {
    let actual = session.player.inventory().get_stack_in_hand(hand);
    assert!(
        actual.are_equal(expected),
        "horse saddle use lost inventory stock"
    );
    assert_eq!(actual.uid, expected.uid, "refused use replaced the stack");
}

#[tokio::test]
async fn feat_horses_s2_saddle_payment_refused_both_java_hands() {
    run(async {
        let scenario = Scenario::new().await;
        let owner_id = Uuid::new_v4();
        let owner = scenario.join(owner_id, "HorseSaddle").await;
        let world = owner.player.world();
        let entity = spawn(&world);
        let horse_id = entity.get_entity().entity_uuid;
        interact(&scenario, &owner, &entity);
        script(&entity, 0, 0);
        opportunity(&scenario, &entity).await;
        remount(&scenario, &owner, &entity).await;
        script(&entity, 0, 4);
        opportunity(&scenario, &entity).await;
        assert!(horse(&entity).is_tame());
        entity
            .get_entity()
            .remove_passenger(owner.player.entity_id());
        let main = use_saddle(&scenario, &owner, &entity, Hand::Right);
        let off = use_saddle(&scenario, &owner, &entity, Hand::Left);
        scenario.prepare_area(&world, Vector2::new(10, 0)).await;
        scenario.command("tp HorseSaddle 163.5 64 3.5");
        owner.loaded(&scenario.server);
        scenario
            .until("saddle refusal horse unload", || {
                scenario.server.tasks.is_empty() && owner.is_idle()
            })
            .await;
        scenario.ticks(1).await;
        assert!(entity.get_entity().is_removed());
        assert!(world.get_entity_by_uuid(horse_id).is_none());
        scenario.command("tp HorseSaddle 3.5 64 3.5");
        owner.loaded(&scenario.server);
        scenario
            .until("saddle refusal horse activation", || {
                scenario.server.tasks.is_empty() && owner.is_idle()
            })
            .await;
        scenario.ticks(1).await;
        let loaded = world.get_entity_by_uuid(horse_id).unwrap();
        assert!(!Arc::ptr_eq(&loaded, &entity));
        assert!(horse(&loaded).is_tame());
        assert!(!horse(&loaded).is_saddled());
        assert_retained(&owner, Hand::Right, &main);
        assert_retained(&owner, Hand::Left, &off);
        let scenario = scenario.restart_with_horse_taming(false).await;
        let owner = scenario.join(owner_id, "HorseSaddle").await;
        let world = owner.player.world();
        let loaded = world.get_entity_by_uuid(horse_id).unwrap();
        assert_eq!(horse(&loaded).get_owner(), Some(owner_id));
        assert!(horse(&loaded).is_tame());
        assert!(!horse(&loaded).is_saddled());
        assert!(
            owner
                .player
                .inventory()
                .get_stack_in_hand(Hand::Right)
                .are_equal(&main)
        );
        assert!(
            owner
                .player
                .inventory()
                .get_stack_in_hand(Hand::Left)
                .are_equal(&off)
        );
        let main = use_saddle(&scenario, &owner, &loaded, Hand::Right);
        let off = use_saddle(&scenario, &owner, &loaded, Hand::Left);
        assert_eq!(scenario.command(&format!("kill {horse_id}")), 1);
        assert!(!loaded.get_living_entity().unwrap().is_alive());
        scenario.ticks(21).await;
        assert_retained(&owner, Hand::Right, &main);
        assert_retained(&owner, Hand::Left, &off);
        assert!(
            !world.entities.load().iter().any(|entity| {
                entity
                    .cast_any()
                    .downcast_ref::<crate::entity::item::ItemEntity>()
                    .is_some_and(|item| item.get_item_stack().lock().unwrap().item == &Item::SADDLE)
            }),
            "refused payment generated a duplicate saddle drop"
        );
        scenario.stop().await;
    })
    .await;
}
