use serde_json::Value;
use std::{fs, path::Path};

/// Resolves a block-state provider holder from the vanilla datapack registry.
pub(super) fn resolve(value: &Value) -> Option<Value> {
    // BlockStateProvider.CODEC accepts named registry holders (upstream PR #3903).
    let id = value.as_str()?;
    let (namespace, name) = id.split_once(':').unwrap_or(("minecraft", id));
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/datapack/data")
        .join(namespace)
        .join("worldgen/block_state_provider")
        .join(format!("{name}.json"));
    let content = fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("Failed to read block state provider {id}: {err}"));
    Some(
        serde_json::from_str(&content)
            .unwrap_or_else(|err| panic!("Failed to parse block state provider {id}: {err}")),
    )
}

#[cfg(test)]
mod tests {
    use super::super::value_to_block_state_provider;
    use serde_json::Value;
    use std::{fs, path::Path};

    // Upstream PR #3903; compare references with the actual vanilla provider definition.
    #[test]
    fn resolves_tree_soil_provider_references() {
        for name in ["soil_beneath_tree", "podzol_beneath_tree"] {
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../assets/datapack/data/minecraft/worldgen/block_state_provider")
                .join(format!("{name}.json"));
            let inline: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
            let expected = value_to_block_state_provider(&inline).to_string();
            assert!(expected.contains("BlockStateProvider :: Rule"));
            for reference in [format!("minecraft:{name}"), name.to_string()] {
                assert_eq!(
                    value_to_block_state_provider(&Value::String(reference)).to_string(),
                    expected
                );
            }
        }
    }
}
