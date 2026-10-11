use std::sync::{Arc, Mutex};

/// Serializes merchant creation and deferred cleanup using a unique menu identity.
#[derive(Default)]
pub(super) struct MerchantSession(Mutex<Option<Arc<()>>>);

impl MerchantSession {
    /// Runs state initialization without callbacks, then publishes the session identity.
    pub(super) fn open<T>(&self, create: impl FnOnce() -> T) -> (T, Arc<()>) {
        let mut current = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let session = Arc::new(());
        let menu = create();
        *current = Some(session.clone());
        (menu, session)
    }

    /// Runs state cleanup without callbacks only while this menu still owns the merchant.
    pub(super) fn close(&self, session: &Arc<()>, cleanup: impl FnOnce()) {
        // MerchantMenu.removed is synchronous in vanilla. Deferred cleanup must not
        // clear a replacement menu, even after its window ID wraps or AI stops trading.
        let mut current = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if current
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, session))
        {
            cleanup();
            *current = None;
        }
    }
}
