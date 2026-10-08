use std::cell::Cell;

use super::ReadingError;

// Fork hardening around ItemStack.OPTIONAL_STREAM_CODEC / DataComponentPatch.STREAM_CODEC.
// Vanilla caps initial collections at 65,536 but has no aggregate recursion budget.
const MAX_ITEM_DEPTH: usize = 32;
const MAX_PACKET_WORK: usize = 65_536;

#[derive(Clone, Copy, Default)]
struct Budget {
    scopes: usize,
    depth: usize,
    work: usize,
}

thread_local! {
    static BUDGET: Cell<Budget> = const { Cell::new(Budget { scopes: 0, depth: 0, work: 0 }) };
}

// Decoding is synchronous. Guards must never survive an await or move between threads.
pub struct DecodeScope {
    node: bool,
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl DecodeScope {
    pub(crate) fn packet() -> Self {
        BUDGET.with(|cell| {
            let mut budget = cell.get();
            if budget.scopes == 0 {
                budget = Budget::default();
            }
            budget.scopes += 1;
            cell.set(budget);
        });
        Self {
            node: false,
            _thread: std::marker::PhantomData,
        }
    }

    pub(crate) fn component() -> Result<Self, ReadingError> {
        let mut scope = Self::packet();
        charge_work(1)?;
        BUDGET.with(|cell| {
            let mut budget = cell.get();
            if budget.depth >= MAX_ITEM_DEPTH {
                return Err(ReadingError::TooLarge("Item component nesting".into()));
            }
            budget.depth += 1;
            cell.set(budget);
            scope.node = true;
            Ok(scope)
        })
    }
}

impl Drop for DecodeScope {
    fn drop(&mut self) {
        BUDGET.with(|cell| {
            let mut budget = cell.get();
            budget.scopes -= 1;
            if self.node {
                budget.depth -= 1;
            }
            cell.set(budget);
        });
    }
}

pub fn charge_work(count: usize) -> Result<(), ReadingError> {
    BUDGET.with(|cell| {
        let mut budget = cell.get();
        if budget.scopes != 0 {
            budget.work = budget
                .work
                .checked_add(count)
                .filter(|work| *work <= MAX_PACKET_WORK)
                .ok_or_else(|| ReadingError::TooLarge("Packet decode work".into()))?;
            cell.set(budget);
        }
        Ok(())
    })
}

pub fn charge_collection_work(count: usize) -> Result<(), ReadingError> {
    // Only item-component collections share this work budget. Other packet lists retain
    // ByteBufCodecs.collection's initial-capacity rule rather than a total element limit.
    if BUDGET.with(|cell| cell.get().depth > 0) {
        charge_work(count)?;
    }
    Ok(())
}
