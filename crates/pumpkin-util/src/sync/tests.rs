use super::*;

#[cfg(debug_assertions)]
fn violation(f: impl FnOnce() + std::panic::UnwindSafe, message: &str) {
    let error = std::panic::catch_unwind(f).unwrap_err();
    let text = error.downcast_ref::<String>().unwrap();
    assert!(text.contains(message), "{text}");
    assert!(text.matches(" at ").count() >= 2, "{text}");
    assert_eq!(text.matches(file!()).count(), 2, "{text}");
    assert_no_guards_held("after unwind");
}

#[test]
#[cfg(debug_assertions)]
fn order_and_recursion_fail_before_blocking() {
    let low = Mutex::<_, ScreenSelection>::new(0);
    let high = Mutex::<_, Cursor>::new(0);
    violation(
        || {
            let _guard = high.lock().unwrap();
            let _ = low.lock();
        },
        "lock order violation",
    );
    violation(
        || {
            let _guard = low.lock().unwrap();
            let _ = low.try_lock();
        },
        "recursive lock acquisition",
    );
    let rw = RwLock::<_, Cursor>::new(0);
    violation(
        || {
            let _guard = rw.read().unwrap();
            let _ = rw.try_write();
        },
        "recursive lock acquisition",
    );
}

#[test]
#[cfg(debug_assertions)]
fn boundary_and_same_class_order() {
    let a = Mutex::<_, Screen>::new(0);
    let b = Mutex::<_, Screen>::new(0);
    let (low, high) = if std::ptr::from_ref(&a) < std::ptr::from_ref(&b) {
        (&a, &b)
    } else {
        (&b, &a)
    };
    let first = low.lock().unwrap();
    let second = high.lock().unwrap();
    drop(first);
    drop(second);
    violation(
        || {
            let _guard = high.lock().unwrap();
            let _ = low.lock();
        },
        "lock order violation",
    );
    violation(
        || {
            let _guard = low.lock().unwrap();
            assert_no_guards_held("test callback");
        },
        "test callback",
    );
}

#[test]
fn poison_and_failed_try_do_not_leak_tracking() {
    let lock = Mutex::<_, Cursor>::new(1);
    let _ = std::panic::catch_unwind(|| {
        let _guard = lock.lock().unwrap();
        panic!("poison");
    });
    let guard = lock.lock().unwrap_err().into_inner();
    assert_eq!(*guard, 1);
    drop(guard);
    lock.clear_poison();
    std::thread::scope(|scope| {
        let _guard = lock.lock().unwrap();
        scope
            .spawn(|| {
                assert!(lock.try_lock().is_err());
                assert_no_guards_held("failed try");
            })
            .join()
            .unwrap();
    });
    assert_no_guards_held("poison cleanup");
}

#[test]
#[cfg(not(debug_assertions))]
fn release_is_std_layout_and_std_guards() {
    assert_eq!(
        size_of::<Mutex<i32, Screen>>(),
        size_of::<std::sync::Mutex<i32>>()
    );
    assert_eq!(
        size_of::<RwLock<i32, Screen>>(),
        size_of::<std::sync::RwLock<i32>>()
    );
    let lock = Mutex::<_, Screen>::new(1);
    let guard: std::sync::MutexGuard<'_, i32> = lock.lock().unwrap();
    assert_no_guards_held("release boundary is empty");
    drop(guard);
    let lock = RwLock::<_, Screen>::new(2);
    let guard: std::sync::RwLockReadGuard<'_, i32> = lock.read().unwrap();
    drop(guard);
    let guard: std::sync::RwLockWriteGuard<'_, i32> = lock.write().unwrap();
    drop(guard);
}

#[test]
#[cfg(debug_assertions)]
fn equal_levels_from_different_classes_are_not_ordered_instances() {
    let screen = Mutex::<_, Screen>::new(0);
    let weather = RwLock::<_, Weather>::new(0);
    violation(
        || {
            let _guard = screen.lock().unwrap();
            drop(weather.try_read());
        },
        "lock order violation",
    );
}

#[test]
#[cfg(debug_assertions)]
fn lock_debug_does_not_invoke_untracked_value_callbacks() {
    struct CallbackDebug<'a>(&'a std::cell::Cell<usize>);
    impl std::fmt::Debug for CallbackDebug<'_> {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            self.0.set(self.0.get() + 1);
            assert_no_guards_held("value formatter callback");
            f.write_str("value")
        }
    }
    let callbacks = std::cell::Cell::new(0);
    let mutex = Mutex::<_, Screen>::new(CallbackDebug(&callbacks));
    let rwlock = RwLock::<_, Screen>::new(CallbackDebug(&callbacks));
    let _ = format!("{mutex:?} {rwlock:?}");
    assert_eq!(callbacks.get(), 0);
    // Explicit guard formatting retains tracking around the value's formatter.
    violation(
        || {
            let guard = mutex.lock().unwrap();
            let _ = format!("{guard:?}");
        },
        "value formatter callback",
    );
}
