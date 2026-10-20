//! Thread-local, test-only literal draws. Current-thread tests install scopes
//! only around synchronous production spawn calls, never across an await.
use std::{cell::RefCell, collections::VecDeque};

use rand::{Rng, SeedableRng};

#[derive(Debug)]
enum Draw {
    Integer(u32, u32),
    Double(f64),
}

thread_local! {
    static DRAWS: RefCell<Option<VecDeque<Draw>>> = const { RefCell::new(None) };
    static NATURAL: RefCell<Option<rand::rngs::StdRng>> = const { RefCell::new(None) };
}

pub struct Script;
impl Script {
    pub fn new(health: [u32; 2], doubles: [f64; 6], count: usize) -> Self {
        let draws = (0..count)
            .flat_map(|_| {
                [Draw::Integer(8, health[0]), Draw::Integer(9, health[1])]
                    .into_iter()
                    .chain(doubles.map(Draw::Double))
            })
            .collect();
        DRAWS.with(|slot| assert!(slot.replace(Some(draws)).is_none()));
        Self
    }

    pub fn remaining() -> usize {
        DRAWS.with(|slot| slot.borrow().as_ref().unwrap().len())
    }

    pub fn consumed(self) {
        assert_eq!(Self::remaining(), 0, "missing horse-stat draws");
        drop(self);
    }
}

impl Drop for Script {
    fn drop(&mut self) {
        DRAWS.with(|slot| slot.replace(None));
    }
}

pub fn integer(bound: u32) -> Option<u32> {
    DRAWS.with(|slot| {
        let mut slot = slot.borrow_mut();
        let draws = slot.as_mut()?;
        let Draw::Integer(expected, value) = draws.pop_front().expect("unexpected horse reroll")
        else {
            panic!("horse draw order changed")
        };
        assert_eq!(bound, expected);
        assert!(value < bound);
        Some(value)
    })
}

pub fn double() -> Option<f64> {
    DRAWS.with(|slot| {
        let mut slot = slot.borrow_mut();
        let draws = slot.as_mut()?;
        let Draw::Double(value) = draws.pop_front().expect("unexpected horse reroll") else {
            panic!("horse draw order changed")
        };
        assert!((0.0..1.0).contains(&value));
        Some(value)
    })
}

pub struct NaturalScope;
impl NaturalScope {
    pub fn new(seed: u64) -> Self {
        NATURAL.with(|slot| {
            assert!(
                slot.replace(Some(rand::rngs::StdRng::seed_from_u64(seed)))
                    .is_none()
            );
        });
        Self
    }
}
impl Drop for NaturalScope {
    fn drop(&mut self) {
        NATURAL.with(|slot| slot.replace(None));
    }
}

pub struct NaturalRandom;
pub fn natural_rng() -> NaturalRandom {
    NaturalRandom
}

impl rand::TryRng for NaturalRandom {
    type Error = std::convert::Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        Ok(NATURAL.with(|slot| {
            slot.borrow_mut()
                .as_mut()
                .map_or_else(|| rand::rng().next_u32(), Rng::next_u32)
        }))
    }

    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        Ok(NATURAL.with(|slot| {
            slot.borrow_mut()
                .as_mut()
                .map_or_else(|| rand::rng().next_u64(), Rng::next_u64)
        }))
    }

    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Self::Error> {
        NATURAL.with(|slot| {
            if let Some(random) = slot.borrow_mut().as_mut() {
                random.fill_bytes(dst);
            } else {
                rand::rng().fill_bytes(dst);
            }
        });
        Ok(())
    }
}
