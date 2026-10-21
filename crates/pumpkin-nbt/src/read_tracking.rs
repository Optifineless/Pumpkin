//! Read ownership for codecs which understand only part of a compound.

use std::{cell::RefCell, collections::HashMap};

use crate::{NbtCompound, tag::NbtTag};

type ReadKeys = HashMap<Box<str>, bool>;
type ReadMap = HashMap<usize, ReadKeys>;

thread_local! {
    static READS: RefCell<Vec<ReadMap>> = const { RefCell::new(Vec::new()) };
}

pub(crate) fn record(compound: &NbtCompound, name: &str, whole: bool) {
    READS.with_borrow_mut(|scopes| {
        if let Some(reads) = scopes.last_mut() {
            let keys = reads
                .entry(std::ptr::from_ref(compound).addr())
                .or_default();
            *keys.entry(name.into()).or_default() |= whole;
        }
    });
}

pub(crate) fn descend(compound: &NbtCompound) {
    READS.with_borrow_mut(|scopes| {
        if let Some(reads) = scopes.last_mut() {
            reads
                .entry(std::ptr::from_ref(compound).addr())
                .or_default();
        }
    });
}

/// Keys attempted by a synchronous NBT reader, including absent and mistyped keys.
#[derive(Default)]
pub struct NbtRead(ReadMap);

// Restores the thread-local recorder on unwinding. It owns no borrow or synchronization guard.
struct ReadScope;

impl Drop for ReadScope {
    fn drop(&mut self) {
        READS.with_borrow_mut(|scopes| {
            scopes.pop();
        });
    }
}

/// Unknown tags, with separate descendants for partially consumed compounds.
#[derive(Clone, Debug, Default)]
pub struct NbtRemainder(HashMap<Box<str>, RetainedTag>);

#[derive(Clone, Debug)]
enum RetainedTag {
    Unknown(NbtTag),
    Compound(NbtRemainder),
}

impl NbtRead {
    /// Records reads on this thread for the duration of `read`.
    ///
    /// `get` and list/scalar getters consume the whole value. `get_compound`
    /// descends into its returned compound, retaining unread children even when none
    /// are recognized. Use `get` for opaque copies. Direct `child_tags` reads are not recorded; dynamic
    /// readers must use `get` for recognized entries. Nested captures are isolated.
    pub fn capture(read: impl FnOnce()) -> Self {
        READS.with_borrow_mut(|scopes| scopes.push(ReadMap::new()));
        let scope = ReadScope;
        read();
        let reads = READS.with_borrow_mut(|scopes| scopes.last_mut().map(std::mem::take));
        drop(scope);
        Self(reads.unwrap_or_default())
    }

    /// Applies a partial load to prior retained data without restoring consumed keys.
    #[must_use]
    pub fn retain(&self, source: &NbtCompound, previous: &NbtRemainder) -> NbtRemainder {
        let mut retained = previous.clone();
        let keys = self.0.get(&std::ptr::from_ref(source).addr());
        for (name, tag) in &source.child_tags {
            if keys.is_none_or(|keys| !keys.contains_key(name)) {
                retained
                    .0
                    .insert(name.clone(), RetainedTag::Unknown(tag.clone()));
            }
        }
        if let Some(keys) = keys {
            for (name, whole) in keys {
                let previous = retained.0.remove(name);
                if !whole && !source.child_tags.contains_key(name) {
                    // A partial edit can omit an owned compound without deleting its unknown children.
                    if let Some(previous @ RetainedTag::Compound(_)) = previous {
                        retained.0.insert(name.clone(), previous);
                    }
                    continue;
                }
                if !whole
                    && let Some(NbtTag::Compound(compound)) = source.child_tags.get(name)
                    && self.0.contains_key(&std::ptr::from_ref(compound).addr())
                {
                    let previous = match previous {
                        Some(RetainedTag::Compound(previous)) => previous,
                        Some(RetainedTag::Unknown(NbtTag::Compound(previous))) => NbtRemainder(
                            previous
                                .into_iter()
                                .map(|(key, tag)| (key, RetainedTag::Unknown(tag)))
                                .collect(),
                        ),
                        _ => NbtRemainder::default(),
                    };
                    let remainder = self.retain(compound, &previous);
                    if !remainder.0.is_empty() {
                        retained
                            .0
                            .insert(name.clone(), RetainedTag::Compound(remainder));
                    }
                }
            }
        }
        retained
    }
}

impl NbtRemainder {
    /// Removes retained fields absent from a complete edited NBT image.
    pub fn retain_present(&mut self, source: &NbtCompound) {
        self.0.retain(|name, retained| {
            let Some(tag) = source.child_tags.get(name) else {
                return false;
            };
            if let RetainedTag::Compound(remainder) = retained {
                if let NbtTag::Compound(compound) = tag {
                    remainder.retain_present(compound);
                } else {
                    return false;
                }
            }
            true
        });
    }

    /// Fills unknown fields after the live writer, which always takes precedence.
    ///
    /// Partially consumed compounds are extended only while the writer still emits
    /// them. Removing a known optional compound cannot resurrect its old contents.
    pub fn write_to(&self, target: &mut NbtCompound) {
        for (name, tag) in &self.0 {
            match tag {
                RetainedTag::Unknown(tag) => {
                    target
                        .child_tags
                        .entry(name.clone())
                        .or_insert_with(|| tag.clone());
                }
                RetainedTag::Compound(remainder) => {
                    if let Some(NbtTag::Compound(compound)) = target.child_tags.get_mut(name) {
                        remainder.write_to(compound);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_unknowns_and_partial_writes_follow_reader_ownership() {
        let mut child = NbtCompound::new();
        child.put_int("known", 1);
        child.put_long("future", 17);
        let mut input = NbtCompound::new();
        input.put_compound("type", child);
        input.put_byte("optional", 1);
        input.put_double("future_root", -1.25);
        let reads = NbtRead::capture(|| {
            let child = input.get_compound("type").unwrap();
            assert_eq!(child.get_int("known"), Some(1));
            let _ = input.get_bool("optional");
        });
        let retained = reads.retain(&input, &NbtRemainder::default());
        let mut saved = NbtCompound::new();
        let mut live_child = NbtCompound::new();
        live_child.put_int("known", 9);
        saved.put_compound("type", live_child);
        retained.write_to(&mut saved);
        assert_eq!(saved.get_double("future_root"), Some(-1.25));
        assert!(!saved.has("optional"));
        let child = saved.get_compound("type").unwrap();
        assert_eq!(child.get_int("known"), Some(9));
        assert_eq!(child.get_long("future"), Some(17));

        let mut partial = NbtCompound::new();
        partial.put_double("another_future", 0.5);
        let reads = NbtRead::capture(|| {
            // A newer reader owns this key even when absent or malformed.
            let _ = partial.get_double("future_root");
            let _ = partial.get_compound("type");
        });
        let retained = reads.retain(&partial, &retained);
        let mut saved = NbtCompound::new();
        retained.write_to(&mut saved);
        assert_eq!(saved.get_double("another_future"), Some(0.5));
        assert!(!saved.has("future_root"));
        assert!(!saved.has("type"), "removed owned compounds cannot return");
        let mut live_child = NbtCompound::new();
        live_child.put_int("known", 10);
        saved.put_compound("type", live_child);
        retained.write_to(&mut saved);
        let child = saved.get_compound("type").unwrap();
        assert_eq!(child.get_int("known"), Some(10));
        assert_eq!(child.get_long("future"), Some(17));
    }

    #[test]
    fn opaque_reads_and_live_writes_take_whole_value_ownership() {
        let mut input = NbtCompound::new();
        let mut child = NbtCompound::new();
        child.put_int("old", 1);
        input.put_compound("opaque", child);
        input.put_int("writer_only", 2);
        let reads = NbtRead::capture(|| {
            let _ = input.get("opaque");
        });
        let retained = reads.retain(&input, &NbtRemainder::default());
        let mut saved = NbtCompound::new();
        saved.put_int("writer_only", 3);
        retained.write_to(&mut saved);
        assert!(!saved.has("opaque"));
        assert_eq!(saved.get_int("writer_only"), Some(3));
    }

    #[test]
    fn captures_are_isolated_and_restore_after_unwinding() {
        let mut outer = NbtCompound::new();
        outer.put_int("future", 2);
        let reads = NbtRead::capture(|| {
            let result = std::panic::catch_unwind(|| {
                NbtRead::capture(|| {
                    let _ = outer.get_int("future");
                    panic!("reader unwinds");
                });
            });
            assert!(result.is_err());
        });
        let retained = reads.retain(&outer, &NbtRemainder::default());
        let mut saved = NbtCompound::new();
        retained.write_to(&mut saved);
        assert_eq!(saved.get_int("future"), Some(2));
        READS.with_borrow(|scopes| assert!(scopes.is_empty()));
    }
}
