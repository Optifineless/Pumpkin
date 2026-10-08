use super::DataComponentImpl;
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};

#[derive(Clone, Debug, PartialEq)]
pub struct BeeOccupant {
    pub entity_data: NbtCompound,
    pub ticks_in_hive: i32,
    pub min_ticks_in_hive: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BeesImpl {
    pub bees: Vec<BeeOccupant>,
}

impl BeesImpl {
    pub const EMPTY: Self = Self { bees: Vec::new() };

    pub fn read_data(data: &NbtTag) -> Option<Self> {
        // Bees.CODEC -> BeehiveBlockEntity.Occupant.CODEC -> TypedEntityData.codec.
        let bees = data
            .extract_list()?
            .iter()
            .map(|tag| {
                let occupant = tag.extract_compound()?;
                let entity_data = occupant.get_compound("entity_data")?.clone();
                entity_data.get_string("id")?;
                Some(BeeOccupant {
                    entity_data,
                    ticks_in_hive: occupant.get_int("ticks_in_hive")?,
                    min_ticks_in_hive: occupant.get_int("min_ticks_in_hive")?,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Self { bees })
    }
}

impl DataComponentImpl for BeesImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::List(
            self.bees
                .iter()
                .map(|bee| {
                    let mut occupant = NbtCompound::new();
                    occupant.put_compound("entity_data", bee.entity_data.clone());
                    occupant.put_int("ticks_in_hive", bee.ticks_in_hive);
                    occupant.put_int("min_ticks_in_hive", bee.min_ticks_in_hive);
                    NbtTag::Compound(occupant)
                })
                .collect(),
        )
    }
    default_impl!(Bees);
}
