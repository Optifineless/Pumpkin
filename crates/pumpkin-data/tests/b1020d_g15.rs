use pumpkin_data::{item::Item, villager::VillagerProfession};

#[test]
#[expect(
    clippy::unwrap_used,
    reason = "Generated villager registry fixture must include the built-in trade"
)]
fn b1020d_g15_generated_registry_uses_vanilla_xp_for_fletcher_arrow_trade() {
    let level_one = VillagerProfession::Fletcher.trade_set(1).unwrap();
    let emerald_arrow = level_one
        .trades
        .iter()
        .find(|trade| {
            trade.wants.item.id == Item::EMERALD.id && trade.gives.item.id == Item::ARROW.id
        })
        .unwrap();

    assert_eq!(emerald_arrow.xp, 1);
}
