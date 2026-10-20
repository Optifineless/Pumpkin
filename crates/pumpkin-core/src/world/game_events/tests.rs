use super::test_support::{DEPARTURE, Scenario};
use pumpkin_data::game_event::GameEvent;
use std::sync::atomic::Ordering::Relaxed;

fn cast_and_reel(scenario: &Scenario, component: Option<bool>, enabled: bool) {
    scenario.capture(None);
    scenario.rod(component);
    scenario.use_item();
    let hook_id = scenario.owner.player.fishing_bobber.load(Relaxed);
    let hook = scenario
        .world
        .get_entity_by_id(hook_id)
        .expect("cast admits the hook");
    hook.get_entity().on_ground.store(true, Relaxed);
    scenario.use_item();
    assert!(hook.get_entity().is_removed(), "reel removes admitted hook");
    assert!(scenario.world.get_entity_by_id(hook_id).is_none());
    assert_eq!(scenario.owner.player.fishing_bobber.load(Relaxed), -1);
    let hand = scenario.owner.player.inventory().held_item();
    assert_eq!(hand.item_count, 1);
    assert_eq!(
        hand.get_damage(),
        2,
        "reel commits durability before vibration"
    );
    let observations = scenario.sink.lock().unwrap();
    if component != Some(true) {
        assert!(observations.plugins.is_empty());
        assert!(observations.forwarded.is_empty());
        return;
    }
    assert_eq!(observations.plugins.len(), 2);
    assert_eq!(observations.plugins[0].key, "item_interact_start");
    assert_eq!(observations.plugins[1].key, "item_interact_finish");
    assert_eq!(observations.plugins[0].damage, 0);
    assert_eq!(observations.plugins[1].damage, 2);
    assert!(observations.plugins.iter().all(|p| p.position == DEPARTURE));
    if !enabled {
        assert!(
            observations.forwarded.is_empty(),
            "off switch must suppress typed forwarding"
        );
        assert_eq!(observations.order, ["plugin", "plugin"]);
        return;
    }
    assert_eq!(
        observations.order,
        ["plugin", "dispatch", "plugin", "dispatch"]
    );
    assert_eq!(observations.forwarded.len(), 2);
    assert_eq!(
        observations.forwarded[0].event,
        GameEvent::ItemInteractStart
    );
    assert_eq!(
        observations.forwarded[1].event,
        GameEvent::ItemInteractFinish
    );
    for payload in &observations.forwarded {
        assert_eq!(payload.position, DEPARTURE);
        assert!(payload.affected_state.is_none());
        scenario.assert_source(
            payload
                .source
                .as_ref()
                .expect("fishing must preserve its source"),
            DEPARTURE,
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s1_fishing_preserves_source_and_order() {
    for component in [None, Some(false), Some(true)] {
        cast_and_reel(&Scenario::new(true), component, true);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s1_config_defaults_and_off_preserves_plugins() {
    for enabled in [true, false] {
        cast_and_reel(&Scenario::new(enabled), Some(true), enabled);
    }
}
