use std::cell::Cell;

thread_local! {
    static HEIGHT_SAMPLER_BUDGET: Cell<Option<(usize, usize)>> = const { Cell::new(None) };
}

struct ResetBudget(Option<(usize, usize)>);

impl Drop for ResetBudget {
    fn drop(&mut self) {
        HEIGHT_SAMPLER_BUDGET.set(self.0);
    }
}

/// Counts sampler constructions on this thread and aborts before exceeding the budget.
pub fn with_height_sampler_budget<R>(budget: usize, run: impl FnOnce() -> R) -> (R, usize) {
    let reset = ResetBudget(HEIGHT_SAMPLER_BUDGET.replace(Some((budget, 0))));
    let result = run();
    let count = HEIGHT_SAMPLER_BUDGET.get().map_or(0, |(_, count)| count);
    drop(reset);
    (result, count)
}

pub(super) fn record_height_sampler() {
    if let Some((budget, count)) = HEIGHT_SAMPLER_BUDGET.get() {
        let count = count + 1;
        HEIGHT_SAMPLER_BUDGET.set(Some((budget, count)));
        assert!(
            count <= budget,
            "height sampler construction budget exceeded: {count} > {budget}"
        );
    }
}
