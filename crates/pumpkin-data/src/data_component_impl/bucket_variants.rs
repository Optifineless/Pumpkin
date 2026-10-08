// enum ids from Axolotl.Variant, Salmon.Variant and TropicalFish.Pattern.
use super::{AxolotlVariantImpl, SalmonSizeImpl, TropicalFishPatternImpl};

macro_rules! ordinal_variant {
    ($ty:ty, $names:expr) => {
        impl $ty {
            pub const NAMES: &'static [&'static str] = $names;

            #[must_use]
            pub fn variant_id(&self) -> Option<i32> {
                Self::NAMES
                    .iter()
                    .position(|name| *name == self.value)
                    .map(|id| id as i32)
            }

            #[must_use]
            pub fn from_variant_id(id: i32) -> Option<Self> {
                Some(Self {
                    value: (*Self::NAMES.get(usize::try_from(id).ok()?)?).into(),
                })
            }
        }
    };
}

ordinal_variant!(
    AxolotlVariantImpl,
    &["lucy", "wild", "gold", "cyan", "blue"]
);
ordinal_variant!(SalmonSizeImpl, &["small", "medium", "large"]);

impl TropicalFishPatternImpl {
    // TropicalFish.Pattern has six pattern indices for each of its two Base values.
    const PATTERNS_PER_BASE: usize = 6;
    pub const NAMES: &'static [&'static str] = &[
        "kob",
        "sunstreak",
        "snooper",
        "dasher",
        "brinely",
        "spotty",
        "flopper",
        "stripey",
        "glitter",
        "blockfish",
        "betty",
        "clayfish",
    ];

    #[must_use]
    pub fn variant_id(&self) -> Option<i32> {
        let ordinal = Self::NAMES.iter().position(|name| *name == self.value)?;
        Some(
            (ordinal / Self::PATTERNS_PER_BASE) as i32
                | ((ordinal % Self::PATTERNS_PER_BASE) as i32) << 8,
        )
    }

    #[must_use]
    pub fn from_variant_id(id: i32) -> Option<Self> {
        let base = usize::try_from(id & 0xFF).ok()?;
        let pattern = usize::try_from(id >> 8).ok()?;
        if base > 1 || pattern >= Self::PATTERNS_PER_BASE {
            return None;
        }
        Some(Self {
            value: Self::NAMES[base * Self::PATTERNS_PER_BASE + pattern].into(),
        })
    }
}
