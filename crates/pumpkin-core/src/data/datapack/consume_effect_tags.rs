use super::{DatapackManager, LoadedDatapack};
use pumpkin_data::{Block, effect::StatusEffect, tag::BLOCK_TAG_DEFINITIONS};
use serde::Deserialize;
use std::{collections::HashMap, fs, path::Path};

const MAX_TAG_BYTES: u64 = 1 << 20;
const MAX_TAG_DEPTH: usize = 64;

#[derive(Default)]
pub(super) struct ConsumeEffectTags {
    effects: HashMap<String, Vec<&'static StatusEffect>>,
    blocks: HashMap<String, Vec<String>>,
}

#[derive(Deserialize)]
struct TagFile {
    #[serde(default)]
    replace: bool,
    values: Vec<TagEntry>,
}

#[derive(Clone, Deserialize)]
#[serde(untagged)]
enum TagEntry {
    Name(String),
    Entry {
        id: String,
        #[serde(default = "required")]
        required: bool,
    },
}

const fn required() -> bool {
    true
}

impl TagEntry {
    fn details(&self) -> (&str, bool) {
        match self {
            Self::Name(id) => (id, true),
            Self::Entry { id, required } => (id, *required),
        }
    }
}

fn location(id: &str) -> String {
    if id.contains(':') {
        id.to_owned()
    } else {
        format!("minecraft:{id}")
    }
}

fn read_tags(
    namespace: &str,
    base: &Path,
    dir: &Path,
    depth: usize,
    tags: &mut HashMap<String, Vec<TagEntry>>,
) {
    if depth > MAX_TAG_DEPTH {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            read_tags(namespace, base, &path, depth + 1, tags);
        } else if path.extension().is_some_and(|ext| ext == "json")
            && entry
                .metadata()
                .is_ok_and(|meta| meta.len() <= MAX_TAG_BYTES)
            && let Ok(relative) = path.strip_prefix(base)
            && let Ok(text) = fs::read_to_string(&path)
        {
            let Ok(file) = serde_json::from_str::<TagFile>(&text) else {
                tracing::warn!("Invalid consume-effect tag {}", path.display());
                continue;
            };
            let id = format!(
                "{namespace}:{}",
                relative
                    .with_extension("")
                    .to_string_lossy()
                    .replace('\\', "/")
            );
            // TagLoader merges lower-priority vanilla entries unless replace is requested.
            let values = tags.entry(id).or_default();
            if file.replace {
                values.clear();
            }
            values.extend(file.values);
        }
    }
}

// TagLoader.build: resolve nested tags, honor optional entries and reject missing required holders.
fn resolve(
    id: &str,
    tags: &HashMap<String, Vec<TagEntry>>,
    path: &mut Vec<String>,
    block: bool,
) -> Option<Vec<String>> {
    let id = location(id);
    if path.len() >= MAX_TAG_DEPTH || path.contains(&id) {
        return None;
    }
    let entries = tags.get(&id)?;
    path.push(id);
    let result = (|| {
        let mut holders = Vec::new();
        for entry in entries {
            let (name, required) = entry.details();
            let resolved = name.strip_prefix('#').map_or_else(
                || {
                    let name = location(name);
                    let valid = if block {
                        Block::from_name(&name).is_some()
                    } else {
                        StatusEffect::from_minecraft_name(&name).is_some()
                    };
                    valid.then_some(vec![name])
                },
                |tag| resolve(tag, tags, path, block),
            );
            match resolved {
                Some(entries) => {
                    for entry in entries {
                        if !holders.contains(&entry) {
                            holders.push(entry);
                        }
                    }
                }
                None if required => return None,
                None => {}
            }
        }
        Some(holders)
    })();
    path.pop();
    result
}

fn vanilla_block_tags() -> HashMap<String, Vec<TagEntry>> {
    BLOCK_TAG_DEFINITIONS
        .iter()
        .filter_map(|(id, json)| {
            serde_json::from_str::<TagFile>(json)
                .ok()
                .map(|tag| ((*id).to_owned(), tag.values))
        })
        .collect()
}

impl DatapackManager {
    pub(super) fn load_consume_effect_tags(&self, packs: &[LoadedDatapack], enabled: &[String]) {
        let mut effects = HashMap::new();
        let mut blocks = vanilla_block_tags();
        let mut packs: Vec<_> = packs.iter().collect();
        packs.sort_by_key(|pack| {
            enabled
                .iter()
                .position(|id| id == &pack.id || id == &pack.name)
        });
        for pack in packs {
            let Ok(namespaces) = fs::read_dir(pack.root_path.join("data")) else {
                continue;
            };
            for ns in namespaces.flatten() {
                let namespace = ns.file_name().to_string_lossy().into_owned();
                for (category, tags) in [("mob_effect", &mut effects), ("block", &mut blocks)] {
                    let dir = ns.path().join("tags").join(category);
                    read_tags(&namespace, &dir, &dir, 0, tags);
                }
            }
        }
        let mut resolved = ConsumeEffectTags::default();
        for id in effects.keys() {
            if let Some(names) = resolve(id, &effects, &mut Vec::new(), false) {
                resolved.effects.insert(
                    id.clone(),
                    names
                        .iter()
                        .filter_map(|name| StatusEffect::from_minecraft_name(name))
                        .collect(),
                );
            } else {
                tracing::warn!("Invalid mob-effect tag {id}");
            }
        }
        for id in blocks.keys() {
            if let Some(names) = resolve(id, &blocks, &mut Vec::new(), true) {
                resolved.blocks.insert(id.clone(), names);
            } else {
                resolved.blocks.insert(id.clone(), Vec::new());
                tracing::warn!("Invalid block tag {id}");
            }
        }
        *self
            .consume_effect_tags
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = resolved;
    }

    /// Resolves the mob-effect `HolderSet` used by `RemoveStatusEffectsConsumeEffect`.
    pub fn get_consume_effect_tag(&self, id: &str) -> Vec<&'static StatusEffect> {
        self.consume_effect_tags
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .effects
            .get(&location(id))
            .cloned()
            .unwrap_or_default()
    }

    /// Tests the vanilla block-tag graph after enabled datapack overrides have been resolved.
    pub fn consume_effect_block_has_tag(&self, block: &Block, id: &str) -> bool {
        let tags = self
            .consume_effect_tags
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        tags.blocks.get(&location(id)).is_some_and(|names| {
            names
                .iter()
                .any(|name| Block::from_name(name).is_some_and(|entry| entry == block))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datapack_teleport_tags_append_to_vanilla_unless_replaced() {
        let dir =
            std::env::temp_dir().join(format!("pumpkin-teleport-tags-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("entities_can_teleport_to.json");
        fs::write(&file, r#"{"values":["minecraft:air"]}"#).unwrap();
        let mut tags = vanilla_block_tags();
        read_tags("minecraft", &dir, &dir, 0, &mut tags);
        let resolved = resolve(
            "minecraft:entities_can_teleport_to",
            &tags,
            &mut Vec::new(),
            true,
        )
        .unwrap();
        assert!(resolved.iter().any(|id| id == "minecraft:stone"));
        assert!(resolved.iter().any(|id| id == "minecraft:air"));
        fs::write(&file, r#"{"replace":true,"values":["minecraft:air"]}"#).unwrap();
        read_tags("minecraft", &dir, &dir, 0, &mut tags);
        assert_eq!(
            resolve(
                "minecraft:entities_can_teleport_to",
                &tags,
                &mut Vec::new(),
                true
            ),
            Some(vec!["minecraft:air".into()])
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn nested_vanilla_tags_use_pack_overrides_before_expansion() {
        let dir = tempfile::tempdir().unwrap();
        let tags_dir = dir.path().join("data/minecraft/tags/block");
        fs::create_dir_all(&tags_dir).unwrap();
        let packs = [LoadedDatapack {
            id: "test".into(),
            name: "test".into(),
            description: String::new(),
            pack_format: 0,
            root_path: dir.path().to_path_buf(),
            recipe_count: 0,
            function_count: 0,
            known_packs: Vec::new(),
        }];
        let manager = DatapackManager::new();
        let tag = "minecraft:entities_can_teleport_to";
        fs::write(
            tags_dir.join("blocks_motion.json"),
            r#"{"replace":true,"values":["minecraft:air"]}"#,
        )
        .unwrap();
        manager.load_consume_effect_tags(&packs, &["test".into()]);
        assert!(manager.consume_effect_block_has_tag(&Block::AIR, tag));
        assert!(!manager.consume_effect_block_has_tag(&Block::STONE, tag));
        fs::write(
            tags_dir.join("blocks_motion.json"),
            r#"{"replace":true,"values":["test:missing"]}"#,
        )
        .unwrap();
        manager.load_consume_effect_tags(&packs, &["test".into()]);
        assert!(!manager.consume_effect_block_has_tag(&Block::STONE, tag));
        assert!(!manager.consume_effect_block_has_tag(&Block::AIR, tag));
    }

    #[test]
    fn effect_tags_resolve_nested_holders_and_reject_required_missing_entries() {
        let tags = HashMap::from([
            (
                "test:inner".into(),
                vec![TagEntry::Name("regeneration".into())],
            ),
            (
                "test:outer".into(),
                vec![
                    TagEntry::Name("#test:inner".into()),
                    TagEntry::Entry {
                        id: "test:missing".into(),
                        required: false,
                    },
                ],
            ),
            (
                "test:invalid".into(),
                vec![TagEntry::Name("test:missing".into())],
            ),
            (
                "test:cycle".into(),
                vec![TagEntry::Name("#test:cycle".into())],
            ),
        ]);
        assert_eq!(
            resolve("test:outer", &tags, &mut Vec::new(), false),
            Some(vec!["minecraft:regeneration".into()])
        );
        assert!(resolve("test:invalid", &tags, &mut Vec::new(), false).is_none());
        assert!(resolve("test:cycle", &tags, &mut Vec::new(), false).is_none());
    }
}
