use super::build_from_assets;
use pumpkin_data::{item::Item, villager::VillagerProfession};
use std::{fs, path::Path};

fn write_fixture(assets: &Path, relative: &str, contents: &str) {
    let path = assets.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

#[test]
#[expect(
    clippy::unwrap_used,
    reason = "Temporary generator assets are valid fixtures"
)]
fn b1020d_g15_builtin_omitted_trade_xp_is_one() {
    let assets = std::env::temp_dir().join(format!("pumpkin-villager-xp-{}", std::process::id()));
    let _ = fs::remove_dir_all(&assets);
    write_fixture(
        &assets,
        "villager_data.json",
        r#"{"professions":{"fletcher":{"name":{"translate":"entity.minecraft.villager.fletcher"},"requested_items":[],"trade_sets":{"1":"fletcher/level_1"}}},"types":{"plains":"minecraft:plains"}}"#,
    );
    write_fixture(
        &assets,
        "datapack/data/minecraft/trade_set/fletcher/level_1.json",
        r##"{"trades":"#minecraft:fletcher/level_1"}"##,
    );
    write_fixture(
        &assets,
        "datapack/data/minecraft/tags/villager_trade/fletcher/level_1.json",
        r##"{"values":["minecraft:fletcher/1/explicit_zero","minecraft:fletcher/1/explicit_seven","minecraft:fletcher/1/omitted"]}"##,
    );
    for (name, xp) in [
        ("explicit_zero", Some(r#", "xp": 0"#)),
        ("explicit_seven", Some(r#", "xp": 7"#)),
        ("omitted", None),
    ] {
        let xp = xp.unwrap_or("");
        write_fixture(
            &assets,
            &format!("datapack/data/minecraft/villager_trade/fletcher/1/{name}.json"),
            &format!(
                r#"{{"wants":{{"id":"minecraft:emerald"}},"gives":{{"id":"minecraft:bread"}}{xp}}}"#
            ),
        );
    }

    let generated = build_from_assets(&assets).to_string();
    let xp_values: Vec<i32> = generated
        .split("xp :")
        .skip(1)
        .filter_map(|field| {
            field
                .split_whitespace()
                .next()?
                .trim_end_matches(',')
                .strip_suffix("i32")?
                .parse()
                .ok()
        })
        .collect();
    assert_eq!(
        &xp_values[..2],
        &[0, 7],
        "explicit trade XP must be retained"
    );
    assert_eq!(xp_values[2], 1, "omitted trade XP must match vanilla 26.3");

    let fletcher = VillagerProfession::Fletcher.trade_set(1).unwrap();
    let emerald_arrow = fletcher
        .trades
        .iter()
        .find(|trade| {
            trade.wants.item.id == Item::EMERALD.id && trade.gives.item.id == Item::ARROW.id
        })
        .unwrap();
    assert_eq!(
        emerald_arrow.xp, 1,
        "compiled built-in registry must use vanilla XP"
    );

    let _ = fs::remove_dir_all(&assets);
}
