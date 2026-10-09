use super::ItemComponents;
use quote::ToTokens;

#[test]
fn avoidance_followup_mob_visibility_survives_item_generation() {
    // MobVisibility.CODEC accepts a tagged holder set as well as the mob-head item lists.
    let components: ItemComponents = serde_json::from_str(
        r##"{
        "minecraft:item_name": {"translate": "item.minecraft.piglin_head"},
        "minecraft:max_stack_size": 64,
        "minecraft:mob_visibility": {
            "targeting_entity_types": "#minecraft:skeletons",
            "visibility": 0.25
        }
    }"##,
    )
    .unwrap();
    let output = components.into_token_stream().to_string();
    assert!(output.contains("MobVisibilityImpl"));
    assert!(output.contains("minecraft:skeletons"));
    assert!(output.contains("0.25"));
}
