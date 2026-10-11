use pumpkin_data::packet::clientbound::play::MERCHANT_OFFERS;
use pumpkin_macros::java_packet;

use crate::ClientPacket;
use crate::VarInt;
use crate::codec::item_stack_seralizer::ItemStackSerializer;
use crate::ser::NetworkWriteExt;
use pumpkin_util::version::JavaMinecraftVersion;

#[derive(Clone)]
pub struct MerchantOffer {
    pub base_cost_a: ItemStackSerializer<'static>,
    pub output: ItemStackSerializer<'static>,
    pub cost_b: Option<ItemStackSerializer<'static>>,
    pub reward_exp: bool,
    pub uses: i32,
    pub max_uses: i32,
    pub xp: i32,
    pub special_price: i32,
    pub price_multiplier: f32,
    pub demand: i32,
}

impl MerchantOffer {
    #[must_use]
    pub const fn is_out_of_stock(&self) -> bool {
        self.uses >= self.max_uses
    }

    #[must_use]
    pub const fn needs_restock(&self) -> bool {
        self.uses > 0
    }

    pub const fn update_demand(&mut self) {
        self.demand += self.uses - (self.max_uses - self.uses);
    }

    pub const fn reset_uses(&mut self) {
        self.uses = 0;
    }

    pub fn write(
        &self,
        mut write: impl std::io::Write,
        version: &JavaMinecraftVersion,
    ) -> Result<(), crate::ser::WritingError> {
        if *version >= JavaMinecraftVersion::V_1_20_5 {
            self.base_cost_a
                .write_item_cost_with_version(&mut write, version)?;
            self.output.write_with_version(&mut write, version)?;
            write.write_option(&self.cost_b, |w, cost_b| {
                cost_b.write_item_cost_with_version(w, version)
            })?;
        } else {
            self.base_cost_a.write_with_version(&mut write, version)?;
            self.output.write_with_version(&mut write, version)?;
            if let Some(cost_b) = &self.cost_b {
                cost_b.write_with_version(&mut write, version)?;
            } else {
                write.write_bool(false)?;
            }
        }
        // MerchantOffer.writeToStream sends isOutOfStock, not rewardExp.
        write.write_bool(self.is_out_of_stock())?;
        write.write_i32_be(self.uses)?;
        write.write_i32_be(self.max_uses)?;
        write.write_i32_be(self.xp)?;
        write.write_i32_be(self.special_price)?;
        write.write_f32_be(self.price_multiplier)?;
        write.write_i32_be(self.demand)?;
        Ok(())
    }
}

#[java_packet(MERCHANT_OFFERS)]
pub struct CMerchantOffers {
    pub window_id: VarInt,
    pub offers: Vec<MerchantOffer>,
    pub villager_level: VarInt,
    pub experience: VarInt,
    pub is_regular_villager: bool,
    pub can_restock: bool,
}

impl CMerchantOffers {
    #[must_use]
    pub const fn new(
        window_id: VarInt,
        offers: Vec<MerchantOffer>,
        villager_level: VarInt,
        experience: VarInt,
        is_regular_villager: bool,
        can_restock: bool,
    ) -> Self {
        Self {
            window_id,
            offers,
            villager_level,
            experience,
            is_regular_villager,
            can_restock,
        }
    }
}

impl ClientPacket for CMerchantOffers {
    fn write_packet_data(
        &self,
        mut write: impl std::io::Write,
        version: &JavaMinecraftVersion,
    ) -> Result<(), crate::ser::WritingError> {
        write.write_var_int(&self.window_id)?;
        if *version >= JavaMinecraftVersion::V_1_19 {
            write.write_var_int(&VarInt(self.offers.len() as i32))?;
        } else {
            write.write_u8(self.offers.len() as u8)?;
        }
        for offer in &self.offers {
            offer.write(&mut write, version)?;
        }
        write.write_var_int(&self.villager_level)?;
        write.write_var_int(&self.experience)?;
        write.write_bool(self.is_regular_villager)?;
        write.write_bool(self.can_restock)?;
        Ok(())
    }
}

impl<'a> crate::ServerPacket<'a> for CMerchantOffers {
    fn read(
        bytebuf: &mut &'a [u8],
        version: &JavaMinecraftVersion,
    ) -> Result<Self, crate::ser::ReadingError> {
        use crate::ser::NetworkReadExt;
        let _scope = crate::ser::decode_budget::DecodeScope::packet();
        let window_id = bytebuf.get_var_int()?;
        let offers_count: usize = if *version >= JavaMinecraftVersion::V_1_19 {
            bytebuf
                .get_var_int()?
                .0
                .try_into()
                .map_err(|_| crate::ser::ReadingError::Message("Negative offer count".into()))?
        } else {
            bytebuf.get_u8()? as usize
        };

        let mut offers = Vec::with_capacity(crate::ser::collection_capacity(offers_count)?);
        for _ in 0..offers_count {
            let (base_cost_a, output, cost_b) = if *version >= JavaMinecraftVersion::V_1_20_5 {
                let base_cost_a = ItemStackSerializer::read_item_cost(bytebuf, version)?;
                let output = ItemStackSerializer::read_with_version(bytebuf, version)?;
                let cost_b = if bytebuf.get_bool()? {
                    Some(ItemStackSerializer::read_item_cost(bytebuf, version)?)
                } else {
                    None
                };
                (base_cost_a, output, cost_b)
            } else {
                let base_cost_a = ItemStackSerializer::read_with_version(bytebuf, version)?;
                let output = ItemStackSerializer::read_with_version(bytebuf, version)?;
                let cost_b_stack = ItemStackSerializer::read_with_version(bytebuf, version)?;
                let cost_b = if cost_b_stack.0.is_empty() {
                    None
                } else {
                    Some(cost_b_stack)
                };
                (base_cost_a, output, cost_b)
            };

            let is_exhausted = bytebuf.get_bool()?;
            let uses = bytebuf.get_i32_be()?;
            let max_uses = bytebuf.get_i32_be()?;
            let xp = bytebuf.get_i32_be()?;
            let special_price = bytebuf.get_i32_be()?;
            let price_multiplier = bytebuf.get_f32_be()?;
            let demand = bytebuf.get_i32_be()?;

            offers.push(MerchantOffer {
                base_cost_a,
                output,
                cost_b,
                reward_exp: true,
                uses: if is_exhausted { max_uses } else { uses },
                max_uses,
                xp,
                special_price,
                price_multiplier,
                demand,
            });
        }

        let villager_level = bytebuf.get_var_int()?;
        let experience = bytebuf.get_var_int()?;
        let is_regular_villager = bytebuf.get_bool()?;
        let can_restock = bytebuf.get_bool()?;

        Ok(Self {
            window_id,
            offers,
            villager_level,
            experience,
            is_regular_villager,
            can_restock,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{borrow::Cow, io::Cursor};

    use pumpkin_data::{
        data_component::DataComponent,
        data_component_impl::{
            DataComponentImpl, DyedColorImpl, ItemNameImpl, MapIdImpl, SuspiciousStewEffect,
            SuspiciousStewEffectsImpl,
        },
        item::Item,
        item_stack::ItemStack,
    };
    use pumpkin_util::version::JavaMinecraftVersion;

    use crate::ser::NetworkReadExt;

    use super::*;

    fn offer() -> MerchantOffer {
        MerchantOffer {
            base_cost_a: ItemStackSerializer(Cow::Owned(ItemStack::new(12, &Item::EMERALD))),
            output: ItemStackSerializer(Cow::Owned(ItemStack::new(1, &Item::BOOK))),
            cost_b: None,
            reward_exp: true,
            uses: 0,
            max_uses: 12,
            xp: 1,
            special_price: 0,
            price_multiplier: 0.05,
            demand: 0,
        }
    }

    #[test]
    fn merchant_inputs_use_item_cost_encoding() {
        let version = JavaMinecraftVersion::V_26_2;
        let packet =
            CMerchantOffers::new(VarInt(1), vec![offer()], VarInt(1), VarInt(0), true, true);
        let mut bytes = Vec::new();
        packet.write_packet_data(&mut bytes, &version).unwrap();
        let mut cursor = Cursor::new(&bytes);

        assert_eq!(cursor.get_var_int().unwrap(), VarInt(1));
        assert_eq!(cursor.get_var_int().unwrap(), VarInt(1));
        assert_eq!(
            cursor.get_var_int().unwrap(),
            VarInt::from(Item::EMERALD.id)
        );
        assert_eq!(cursor.get_var_int().unwrap(), VarInt(12));
        assert_eq!(cursor.get_var_int().unwrap(), VarInt(0));
    }

    #[test]
    fn merchant_offer_is_out_of_stock_at_max_uses() {
        let mut offer = offer();
        offer.uses = offer.max_uses - 1;
        assert!(!offer.is_out_of_stock());
        offer.uses += 1;
        assert!(offer.is_out_of_stock());
    }

    #[test]
    fn novice_fletcher_offers_match_vanilla_wire_bytes() {
        use pumpkin_data::villager::TRADES_FLETCHER_LEVEL_1;
        let offers: Vec<_> = TRADES_FLETCHER_LEVEL_1
            .iter()
            .map(|trade| MerchantOffer {
                base_cost_a: ItemStackSerializer(Cow::Owned(ItemStack::new(
                    trade.wants.count as u8,
                    trade.wants.item,
                ))),
                output: ItemStackSerializer(Cow::Owned(ItemStack::new(
                    trade.gives.count as u8,
                    trade.gives.item,
                ))),
                cost_b: trade.wants_b.as_ref().map(|cost| {
                    ItemStackSerializer(Cow::Owned(ItemStack::new(cost.count as u8, cost.item)))
                }),
                reward_exp: true,
                uses: 0,
                max_uses: trade.max_uses,
                xp: trade.xp,
                special_price: 0,
                price_multiplier: trade.price_multiplier,
                demand: 0,
            })
            .collect();
        let packet = CMerchantOffers::new(VarInt(1), offers, VarInt(1), VarInt(0), true, true);
        let mut actual = Vec::new();
        packet
            .write_packet_data(&mut actual, &JavaMinecraftVersion::V_26_3)
            .unwrap();
        // MerchantOffer.writeToStream: item costs use id/count/predicate;
        // results use count/id/patch, followed by exhausted (not rewardExp).
        let mut expected = vec![1, 3];
        for (input, count, output, result_count, second_cost, max_uses, xp) in [
            (&Item::STICK, 32, &Item::EMERALD, 1, None, 16i32, 2i32),
            (&Item::EMERALD, 1, &Item::ARROW, 16, None, 12, 1),
            (
                &Item::GRAVEL,
                10,
                &Item::FLINT,
                10,
                Some((&Item::EMERALD, 1)),
                12,
                1,
            ),
        ] {
            expected.write_var_int(&VarInt::from(input.id)).unwrap();
            expected.extend([count, 0, result_count]);
            expected.write_var_int(&VarInt::from(output.id)).unwrap();
            expected.extend([0, 0, u8::from(second_cost.is_some())]);
            if let Some((item, count)) = second_cost {
                expected.write_var_int(&VarInt::from(item.id)).unwrap();
                expected.extend([count, 0]);
            }
            expected.push(0);
            expected.extend(0i32.to_be_bytes());
            expected.extend(max_uses.to_be_bytes());
            expected.extend(xp.to_be_bytes());
            expected.extend(0i32.to_be_bytes());
            expected.extend(0.05f32.to_be_bytes());
            expected.extend(0i32.to_be_bytes());
        }
        expected.extend([1, 0, 1, 1]);
        assert_eq!(actual, expected);
        let decoded = <CMerchantOffers as crate::ServerPacket>::read(
            &mut expected.as_slice(),
            &JavaMinecraftVersion::V_26_3,
        )
        .unwrap();
        assert!(
            decoded
                .offers
                .iter()
                .all(|offer| offer.reward_exp && !offer.is_out_of_stock())
        );
    }

    #[test]
    fn restock_updates_demand_before_resetting_uses() {
        let mut offer = offer();
        offer.uses = 8;

        assert!(offer.needs_restock());
        offer.update_demand();
        offer.reset_uses();

        assert_eq!(offer.demand, 4);
        assert_eq!(offer.uses, 0);
        assert!(!offer.needs_restock());
    }

    #[test]
    fn dynamic_villager_results_have_network_codecs() {
        let mut dyed = ItemStack::new(1, &Item::LEATHER_CHESTPLATE);
        dyed.patch.push((
            DataComponent::DyedColor,
            Some(DyedColorImpl { rgb: 0x12_34_56 }.to_dyn()),
        ));
        let mut stew = ItemStack::new(1, &Item::SUSPICIOUS_STEW);
        stew.patch.push((
            DataComponent::SuspiciousStewEffects,
            Some(
                SuspiciousStewEffectsImpl {
                    effects: Cow::Owned(vec![SuspiciousStewEffect {
                        effect: Cow::Borrowed("minecraft:night_vision"),
                        duration: 100,
                    }]),
                }
                .to_dyn(),
            ),
        ));
        let mut map = ItemStack::new(1, &Item::FILLED_MAP);
        map.patch
            .push((DataComponent::MapId, Some(MapIdImpl { id: 1 }.to_dyn())));
        map.patch.push((
            DataComponent::ItemName,
            Some(
                ItemNameImpl {
                    name: Cow::Borrowed("filled_map.mansion"),
                }
                .to_dyn(),
            ),
        ));

        for output in [dyed, stew, map] {
            let mut dynamic_offer = offer();
            dynamic_offer.output = ItemStackSerializer(Cow::Owned(output));
            let packet = CMerchantOffers::new(
                VarInt(1),
                vec![dynamic_offer],
                VarInt(1),
                VarInt(0),
                true,
                true,
            );
            packet
                .write_packet_data(&mut Vec::new(), &JavaMinecraftVersion::V_26_2)
                .unwrap();
        }
    }
    #[test]
    fn merchant_reader_retains_typed_predicates_in_both_costs() {
        use pumpkin_data::data_component_impl::{DamageImpl, RepairCostImpl};
        // ItemCost.STREAM_CODEC / TypedDataComponent.STREAM_CODEC: id, count, list(id,value).
        let mut bytes = vec![1, 1];
        bytes
            .write_var_int(&VarInt::from(Item::EMERALD.id))
            .unwrap();
        bytes.extend([12, 1, DataComponent::RepairCost.to_id(), 7]);
        bytes.push(1);
        bytes.write_var_int(&VarInt::from(Item::BOOK.id)).unwrap();
        bytes.extend([0, 0, 1]);
        bytes
            .write_var_int(&VarInt::from(Item::DIAMOND_SWORD.id))
            .unwrap();
        bytes.extend([1, 1, DataComponent::Damage.to_id(), 3, 0]);
        for value in [0i32, 12, 1, 0] {
            bytes.extend(value.to_be_bytes());
        }
        bytes.extend(0.05f32.to_be_bytes());
        bytes.extend(0i32.to_be_bytes());
        bytes.extend([1, 0, 1, 1]);
        let mut input = bytes.as_slice();
        let decoded = <CMerchantOffers as crate::ServerPacket>::read(
            &mut input,
            &JavaMinecraftVersion::V_26_3,
        )
        .unwrap();
        assert!(input.is_empty());
        let offer = &decoded.offers[0];
        assert_eq!(
            offer
                .base_cost_a
                .0
                .get_data_component::<RepairCostImpl>()
                .unwrap()
                .cost,
            7
        );
        assert_eq!(
            offer
                .cost_b
                .as_ref()
                .unwrap()
                .0
                .get_data_component::<DamageImpl>()
                .unwrap()
                .damage,
            3
        );
        let mut rewritten = Vec::new();
        decoded
            .write_packet_data(&mut rewritten, &JavaMinecraftVersion::V_26_3)
            .unwrap();
        assert_eq!(rewritten, bytes);
    }

    #[test]
    fn item_cost_predicate_uses_the_connections_component_registry() {
        // Extracted MapId registry IDs: 26.2=46 (23e7992b2), 26.3=48 (df88eeaa3).
        for (version, map_id) in [
            (JavaMinecraftVersion::V_26_2, 46),
            (JavaMinecraftVersion::V_26_3, 48),
        ] {
            let mut bytes = Vec::new();
            bytes
                .write_var_int(&VarInt::from(Item::EMERALD.id))
                .unwrap();
            bytes.extend([1, 1, map_id, 42]);
            let mut input = bytes.as_slice();
            let cost = ItemStackSerializer::read_item_cost(&mut input, &version).unwrap();
            assert!(input.is_empty());
            assert_eq!(cost.0.get_data_component::<MapIdImpl>().unwrap().id, 42);
            let mut rewritten = Vec::new();
            cost.write_item_cost_with_version(&mut rewritten, &version)
                .unwrap();
            assert_eq!(rewritten, bytes);
        }
    }

    #[test]
    fn older_merchant_packet_omits_unmappable_predicate_and_keeps_other_components() {
        use pumpkin_data::data_component_impl::WaxedImpl;
        let version = JavaMinecraftVersion::V_26_2;
        let mut offer = offer();
        offer.base_cost_a = ItemStack::new_with_component(
            12,
            &Item::EMERALD,
            vec![
                (DataComponent::Waxed, Some(WaxedImpl.to_dyn())),
                (DataComponent::MapId, Some(MapIdImpl { id: 42 }.to_dyn())),
            ],
        )
        .into();
        offer.cost_b = Some(offer.base_cost_a.clone());
        let packet = CMerchantOffers::new(VarInt(1), vec![offer], VarInt(1), VarInt(0), true, true);
        let mut bytes = Vec::new();
        packet.write_packet_data(&mut bytes, &version).unwrap();
        let mut input = bytes.as_slice();
        let decoded = <CMerchantOffers as crate::ServerPacket>::read(&mut input, &version).unwrap();
        assert!(input.is_empty());
        for cost in [
            &decoded.offers[0].base_cost_a,
            decoded.offers[0].cost_b.as_ref().unwrap(),
        ] {
            assert_eq!(cost.0.patch.len(), 1);
            assert_eq!(cost.0.get_data_component::<MapIdImpl>().unwrap().id, 42);
            assert_eq!(cost.0.item_count, 12);
        }
    }

    #[test]
    fn merchant_reader_rejects_invalid_predicate_lengths_and_item_ids() {
        for fields in [
            [-1, 1, 0],
            [i32::from(Item::STICK.id), 256, 0],
            [i32::from(Item::STICK.id), 1, -1],
            [i32::from(Item::STICK.id), 1, 257],
        ] {
            let mut bytes = vec![1, 1];
            for value in fields {
                bytes.write_var_int(&VarInt(value)).unwrap();
            }
            assert!(
                <CMerchantOffers as crate::ServerPacket>::read(
                    &mut bytes.as_slice(),
                    &JavaMinecraftVersion::V_26_3
                )
                .is_err()
            );
        }
    }
}
