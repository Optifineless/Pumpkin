use super::*;

#[test]
fn verification3_nested_ownership_reuses_scope_and_restores_health_gate() {
    let owner = DamageOwner::default();
    let outer = owner.enter();
    outer.require_health_above(1.0);
    for _ in 0..100 {
        let inner = owner.enter();
        assert!(
            Rc::ptr_eq(&outer.scope, &inner.scope),
            "nested ownership allocated a scope"
        );
        inner.require_health_above(5.0);
        assert!(!owner.admits_health(4.0));
        drop(inner);
        assert!(owner.admits_health(4.0));
        assert!(!owner.admits_health(1.0));
    }
    drop(outer);
    assert!(owner.admits_health(0.0));
    let outer = owner.enter_scope(true);
    owner.changed(2.0);
    let inner = owner.enter_scope(true);
    owner.changed(3.0);
    assert_eq!(inner.health_damage(), 3.0);
    drop(inner);
    assert_eq!(outer.health_damage(), 5.0);
}

#[test]
fn verification4_outer_health_gate_survives_nested_drop() {
    let owner = DamageOwner::default();
    let outer = owner.enter();
    outer.require_health_above(1.0);
    let inner = owner.enter();
    inner.require_health_above(5.0);
    outer.require_health_above(3.0);
    drop(inner);
    assert!(!owner.admits_health(2.0));
    assert!(owner.admits_health(4.0));
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "nested damage token still alive")]
fn verification4_outer_drop_with_live_nested_token_panics() {
    let owner = DamageOwner::default();
    let outer = owner.enter();
    let inner = owner.enter();
    drop(outer);
    drop(inner);
}

#[test]
fn verification4_outer_acquisition_reuses_and_resets_scope() {
    let owner = DamageOwner::default();
    let first = owner.enter();
    let address = Rc::as_ptr(&first.scope);
    first.require_health_above(5.0);
    first.melee();
    owner.changed(2.0);
    assert!(owner.defer_motion(PendingMotion::Mark));
    let allocations = owner.0.scope_allocations.load(Relaxed);
    drop(first);
    owner.reset();
    let next = owner.enter();
    assert_eq!(
        address,
        Rc::as_ptr(&next.scope),
        "outer acquisition allocated a scope"
    );
    assert_eq!(owner.0.scope_allocations.load(Relaxed), allocations);
    assert!(next.is_current_life());
    assert!(owner.admits_health(0.0));
    assert_eq!(next.health_damage(), 0.0);
    assert!(!next.scope.melee.get());
    assert!(next.scope.motion.borrow().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verification4_uncontended_acquisition_skips_runtime_bridge() {
    let owner = DamageOwner::default();
    for _ in 0..100 {
        drop(owner.enter());
    }
    assert_eq!(owner.0.runtime_checks.load(Relaxed), 0);
    assert_eq!(owner.0.waiting.load(Relaxed), 0);
}
