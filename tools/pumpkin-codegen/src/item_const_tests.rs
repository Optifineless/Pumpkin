use super::*;

#[test]
fn generated_inline_sounds_and_hidden_effects_compile_as_constants() {
    let components: ItemComponents = serde_json::from_value(serde_json::json!({
        "minecraft:item_name": {"translate": "item.test.fixture"},
        "minecraft:max_stack_size": 1,
        "minecraft:use_effects": {"interact_vibrations": false},
        "minecraft:blocks_attacks": {
            "block_sound": {"sound_id": "test:block", "range": 12.5},
            "disabled_sound": {"sound_id": "test:disable"}
        },
        "minecraft:death_protection": {"death_effects": [
            {"type": "minecraft:play_sound", "sound": {"sound_id": "test:resurrect"}},
            {"type": "minecraft:apply_effects", "effects": [
                {"id": "minecraft:regeneration", "duration": 20,
                 "hidden_effect": {"amplifier": 1, "duration": 100}}
            ]}
        ]}
    }))
    .unwrap();
    let generated = components.to_token_stream();
    let fixture = quote! {
        use std::borrow::Cow;
        use pumpkin_data::{data_component::DataComponent, data_component::DataComponent::*, data_component_impl::*};
        const COMPONENTS: &[(DataComponent, &dyn DataComponentImpl)] = &[#generated];
        fn main() { assert!(!COMPONENTS.is_empty()); }
    };
    // Compile against the real component types, rather than duplicated fixture models.
    let deps = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_owned();
    let dir = std::env::temp_dir().join(format!("pumpkin-item-const-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let source = dir.join("fixture.rs");
    fs::write(&source, fixture.to_string()).unwrap();
    let rustc = std::env::var_os("RUSTC")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO")).with_file_name(if cfg!(windows) {
                "rustc.exe"
            } else {
                "rustc"
            })
        });
    // Resolve the exact dependency fingerprint linked into this test executable.
    let exe = std::env::current_exe().unwrap();
    let stem = exe.file_stem().unwrap().to_str().unwrap();
    let hash = stem.rsplit_once('-').unwrap().1;
    let fingerprints = deps.parent().unwrap().join(".fingerprint");
    let metadata: serde_json::Value = serde_json::from_slice(
        &fs::read(fingerprints.join(format!(
            "pumpkin-codegen-{hash}/test-bin-pumpkin-codegen.json"
        )))
        .unwrap(),
    )
    .unwrap();
    let expected = metadata["deps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|dep| dep[1] == "pumpkin_data")
        .unwrap()[3]
        .as_u64()
        .unwrap();
    let fingerprint = expected
        .to_le_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let artifacts: Vec<_> = fs::read_dir(&fingerprints)
        .unwrap()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let hash = name.strip_prefix("pumpkin-data-")?;
            if fs::read_to_string(entry.path().join("lib-pumpkin_data"))
                .ok()?
                .trim()
                != fingerprint
            {
                return None;
            }
            [
                format!("libpumpkin_data-{hash}.rlib"),
                format!("pumpkin_data-{hash}.rlib"),
            ]
            .into_iter()
            .map(|name| deps.join(name))
            .find(|path| path.is_file())
        })
        .collect();
    assert_eq!(
        artifacts.len(),
        1,
        "current build must identify exactly one pumpkin-data artifact"
    );
    let result = std::process::Command::new(&rustc)
        .arg("--edition=2024")
        .arg("--emit=metadata")
        .arg(&source)
        .arg("-L")
        .arg(format!("dependency={}", deps.display()))
        .arg("--extern")
        .arg(format!("pumpkin_data={}", artifacts[0].display()))
        .arg("-o")
        .arg(dir.join("fixture.rmeta"))
        .output()
        .unwrap();
    fs::remove_dir_all(&dir).unwrap();
    assert!(
        result.status.success(),
        "Generated constants failed to compile: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}
