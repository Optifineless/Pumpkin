use super::DataComponentImpl;
use crate::{
    item_stack::ItemStack,
    tag::{self, Taggable},
};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};

// BundleContents uses fractions; the existing integer-weight API reports sixty-fourths.
const WEIGHT_SCALE: u32 = 64;
const BEEHIVE_WEIGHT: Weight = Weight {
    numerator: 1,
    denominator: 1,
};
const BUNDLE_IN_BUNDLE_WEIGHT: Weight = Weight {
    numerator: 1,
    denominator: 16,
};

#[derive(Clone)]
pub struct BundleContentsImpl {
    pub items: Vec<crate::item_stack::ItemStack>,
    pub selected_item: i32,
}
impl PartialEq for BundleContentsImpl {
    fn eq(&self, other: &Self) -> bool {
        self.items.len() == other.items.len()
            && self
                .items
                .iter()
                .zip(&other.items)
                .all(|(a, b)| a.item_count == b.item_count && a.are_items_and_components_equal(b))
    }
}
impl Eq for BundleContentsImpl {}
impl std::fmt::Debug for BundleContentsImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "BundleContentsImpl")
    }
}
impl BundleContentsImpl {
    /// Returns how many items fit, including the overhead of nested bundles.
    pub fn max_amount_to_add(&self, stack: &ItemStack) -> u8 {
        if stack.is_empty() || stack.item.has_tag(&tag::Item::MINECRAFT_SHULKER_BOXES) {
            return 0;
        }
        let Some(weight) = self.weight() else {
            return 0;
        };
        let Some(each) = Self::item_weight(stack) else {
            return 0;
        };
        let mut total = weight;
        for count in 0..stack.item_count {
            let Some(next) = total.add(each) else {
                return count;
            };
            if next.numerator > next.denominator {
                return count;
            }
            total = next;
        }
        stack.item_count
    }

    pub fn get_weight(&self) -> u32 {
        self.weight()
            .and_then(|w| Some(w.numerator.checked_mul(u64::from(WEIGHT_SCALE))? / w.denominator))
            .and_then(|w| u32::try_from(w).ok())
            .unwrap_or(WEIGHT_SCALE)
    }

    fn weight(&self) -> Option<Weight> {
        self.items
            .iter()
            .try_fold(Weight::ZERO, |mut total, stack| {
                let weight = Self::item_weight(stack)?;
                for _ in 0..stack.item_count {
                    total = total.add(weight)?;
                }
                Some(total)
            })
    }

    fn item_weight(stack: &ItemStack) -> Option<Weight> {
        // BundleContents.getWeight uses exact fractions and BUNDLE_IN_BUNDLE_WEIGHT = 1/16.
        if let Some(bundle) = stack.get_data_component::<Self>() {
            bundle.weight()?.add(BUNDLE_IN_BUNDLE_WEIGHT)
        } else if stack
            .get_data_component::<super::BeesImpl>()
            .is_some_and(|bees| !bees.bees.is_empty())
        {
            // BundleContents.getWeight charges BEEHIVE_WEIGHT for any nonempty BEES component.
            Some(BEEHIVE_WEIGHT)
        } else {
            let denominator = u64::from(stack.get_max_stack_size());
            if denominator == 0 {
                return None;
            }
            Some(Weight {
                numerator: 1,
                denominator,
            })
        }
    }

    pub fn try_insert(&mut self, stack: &mut ItemStack) -> bool {
        // BundleContents.Mutable.tryInsert moves matching stacks to the front without resetting selection.
        let amount = self.max_amount_to_add(stack);
        if amount == 0 {
            return false;
        }
        let mut inserted = stack.split(amount);
        if inserted.is_stackable()
            && let Some(index) = self
                .items
                .iter()
                .position(|s| s.are_items_and_components_equal(&inserted))
        {
            inserted.increment(self.items.remove(index).item_count);
        }
        self.items.insert(0, inserted);
        true
    }

    pub fn try_extract(&mut self) -> Option<ItemStack> {
        // BundleContents.Mutable.removeOne clears the transient selection after removal.
        if self.items.is_empty() {
            return None;
        }
        let index = usize::try_from(self.selected_item)
            .ok()
            .filter(|i| *i < self.items.len())
            .unwrap_or(0);
        self.selected_item = -1;
        Some(self.items.remove(index))
    }

    pub fn read_data(tag: &NbtTag) -> Option<Self> {
        let mut items = Vec::new();
        if let NbtTag::List(l) = tag {
            for item_tag in l {
                if let NbtTag::Compound(c) = item_tag
                    && let Some(stack) = crate::item_stack::ItemStack::read_item_stack(c)
                {
                    items.push(stack);
                }
            }
        }
        Some(Self {
            items,
            selected_item: -1,
        })
    }
}

#[derive(Clone, Copy)]
struct Weight {
    numerator: u64,
    denominator: u64,
}

impl Weight {
    const ZERO: Self = Self {
        numerator: 0,
        denominator: 1,
    };

    fn add(self, other: Self) -> Option<Self> {
        let divisor = gcd(self.denominator, other.denominator);
        let left = other.denominator / divisor;
        let right = self.denominator / divisor;
        let numerator = self
            .numerator
            .checked_mul(left)?
            .checked_add(other.numerator.checked_mul(right)?)?;
        let denominator = self.denominator.checked_mul(left)?;
        let divisor = gcd(numerator, denominator);
        Some(Self {
            numerator: numerator / divisor,
            denominator: denominator / divisor,
        })
    }
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{data_component_impl::MaxStackSizeImpl, item::Item};

    #[test]
    fn occupied_hives_fill_bundles_and_preserve_bees_through_nbt() {
        let mut bee = NbtCompound::new();
        bee.put_string("id", "minecraft:bee".to_owned());
        bee.put_bool("HasNectar", true);
        let mut occupant = NbtCompound::new();
        occupant.put_compound("entity_data", bee);
        occupant.put_int("ticks_in_hive", 12);
        occupant.put_int("min_ticks_in_hive", 2400);
        let tag = NbtTag::List(vec![NbtTag::Compound(occupant)]);
        let bees = super::super::BeesImpl::read_data(&tag).unwrap();
        assert_eq!(bees.write_data(), tag);
        for item in [&Item::BEEHIVE, &Item::BEE_NEST] {
            let mut hive = ItemStack::new(2, item);
            hive.set_data_component(bees.clone());
            let mut contents = BundleContentsImpl {
                items: Vec::new(),
                selected_item: -1,
            };
            assert!(contents.try_insert(&mut hive));
            assert_eq!(hive.item_count, 1);
            assert_eq!(contents.get_weight(), 64);
            assert_eq!(
                contents.items[0]
                    .get_data_component::<super::super::BeesImpl>()
                    .unwrap()
                    .write_data(),
                tag
            );
            let mut nested = ItemStack::new(1, &Item::BUNDLE);
            nested.set_data_component(contents);
            let outer = BundleContentsImpl {
                items: Vec::new(),
                selected_item: -1,
            };
            assert_eq!(outer.max_amount_to_add(&nested), 0);
            assert_eq!(outer.max_amount_to_add(&ItemStack::new(64, item)), 64);
        }
    }

    #[test]
    fn bundles_reject_shulkers_and_charge_nested_weight() {
        let mut contents = BundleContentsImpl {
            items: Vec::new(),
            selected_item: -1,
        };
        let mut shulker = ItemStack::new(1, &Item::SHULKER_BOX);
        assert!(!contents.try_insert(&mut shulker));
        let mut nested = ItemStack::new(1, &Item::BUNDLE);
        assert!(contents.try_insert(&mut nested));
        let mut stone = ItemStack::new(64, &Item::STONE);
        assert!(contents.try_insert(&mut stone));
        assert_eq!(stone.item_count, 4);
    }

    #[test]
    fn custom_stack_sizes_use_fractional_weights_without_rounding() {
        let mut contents = BundleContentsImpl {
            items: Vec::new(),
            selected_item: -1,
        };
        let mut stack = ItemStack::new(99, &Item::STONE);
        stack.set_data_component(MaxStackSizeImpl { size: 99 });
        assert!(contents.try_insert(&mut stack));
        assert!(stack.is_empty());
        assert_eq!(contents.items[0].item_count, 99);
    }

    #[test]
    fn extracting_selected_contents_removes_the_real_stack_once() {
        let mut contents = BundleContentsImpl {
            items: vec![
                ItemStack::new(3, &Item::STONE),
                ItemStack::new(2, &Item::DIAMOND),
            ],
            selected_item: 1,
        };
        assert_eq!(contents.try_extract().unwrap().item, &Item::DIAMOND);
        assert_eq!(contents.try_extract().unwrap().item, &Item::STONE);
        assert!(contents.try_extract().is_none());
    }

    #[test]
    fn inserting_preserves_selection_while_merging_to_front() {
        let mut contents = BundleContentsImpl {
            items: vec![
                ItemStack::new(3, &Item::STONE),
                ItemStack::new(2, &Item::DIAMOND),
            ],
            selected_item: 1,
        };
        let mut stone = ItemStack::new(4, &Item::STONE);
        assert!(contents.try_insert(&mut stone));
        assert!(stone.is_empty());
        assert_eq!(contents.items[0].item_count, 7);
        assert_eq!(contents.try_extract().unwrap().item, &Item::DIAMOND);
        assert_eq!(contents.selected_item, -1);
    }
}
impl DataComponentImpl for BundleContentsImpl {
    fn write_data(&self) -> NbtTag {
        let mut list = Vec::new();
        for stack in &self.items {
            let mut item_compound = NbtCompound::new();
            stack.write_item_stack(&mut item_compound);
            list.push(NbtTag::Compound(item_compound));
        }
        NbtTag::List(list)
    }
    default_impl!(BundleContents);
}
