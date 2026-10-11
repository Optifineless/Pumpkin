use std::cell::RefCell;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Point {
    MotionReady,
    Damping,
    ProjectileFollowup,
    PlayerMotionFlush,
    CommandDispatch,
}

type Hook = Box<dyn Fn(Point)>;

thread_local! {
    static HOOK: RefCell<Option<Hook>> = RefCell::default();
}

pub fn install(hook: impl Fn(Point) + 'static) {
    HOOK.with(|current| *current.borrow_mut() = Some(Box::new(hook)));
}

pub fn reach(point: Point) {
    HOOK.with(|current| {
        if let Some(hook) = current.borrow().as_ref() {
            hook(point);
        }
    });
}
