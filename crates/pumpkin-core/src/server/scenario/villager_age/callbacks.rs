//! Native navigation callbacks exercise reentrant age changes and canonical removal.
use super::*;
use crate::entity::ai::brain::memory::types;

#[tokio::test]
async fn villager_s6_native_navigation_callbacks_revalidate_age_effects() {
    bounded(async {
        let scenario = Scenario::new().await;
        let pose_id = Uuid::new_v4();
        let retire_id = Uuid::new_v4();
        seed(
            &scenario,
            vec![fixture(pose_id, -3, true), fixture(retire_id, -1, false)],
        )
        .await;
        let (java, _) = activate(&scenario, "AgeCallbacks").await;
        let world = java.player.world();
        let pose_entity = world.get_entity_by_uuid(pose_id).unwrap();
        let reentrant = Arc::downgrade(&pose_entity);
        let dimensions = with_villager(&scenario, pose_id, |villager| {
            navigation::watch_actions(
                villager,
                Some(Box::new(move || {
                    let entity = reentrant.upgrade().unwrap();
                    assert!(
                        entity
                            .cast_any()
                            .downcast_ref::<VillagerEntity>()
                            .unwrap()
                            .set_biological_age(0)
                    );
                })),
                None,
            )
        });
        // This invokes the existing virtual navigator while changing pose. Its setter
        // must queue behind the publisher rather than recursively lock navigation.
        pose_entity.get_entity().set_pose(EntityPose::Sleeping);
        assert_eq!(age(&scenario, pose_id), 0, "reentrant pose age committed");
        pose_entity.get_entity().set_pose(EntityPose::Standing);
        assert_eq!(
            dimensions.load(),
            <(f32, f32)>::from(EntityType::VILLAGER.dimension)
        );
        assert_eq!(
            pose_entity.get_entity().get_eye_height(),
            f64::from(EntityType::VILLAGER.eye_height)
        );

        let retired = world.get_entity_by_uuid(retire_id).unwrap();
        let remove = Arc::downgrade(&retired);
        let partner = Arc::downgrade(&pose_entity);
        with_villager(&scenario, retire_id, |villager| {
            navigation::watch_actions(
                villager,
                None,
                Some(Box::new(move || {
                    let entity = remove.upgrade().unwrap();
                    // /data reloads persisted Brain first; establish this transient value
                    // inside the callback so only post-callback work could erase it.
                    entity
                        .cast_any()
                        .downcast_ref::<VillagerEntity>()
                        .unwrap()
                        .mob_entity
                        .brain
                        .lock()
                        .unwrap()
                        .set(types::INTERACTION_TARGET, partner.upgrade().unwrap());
                    entity.get_entity().remove();
                })),
            );
        });
        scenario.command(&format!("data merge entity {retire_id} {{Age:0}}"));
        assert!(
            retired.get_entity().is_removed(),
            "native stop used canonical removal"
        );
        let villager = retired.cast_any().downcast_ref::<VillagerEntity>().unwrap();
        assert!(
            villager
                .mob_entity
                .brain
                .lock()
                .unwrap()
                .has_memory_value(types::INTERACTION_TARGET.id()),
            "retired callback received no later social mutation"
        );
        assert!(
            !villager.set_biological_age(-1),
            "retired callback owner cannot write"
        );
        scenario.stop().await;
    })
    .await;
}
