use proc_macro2::TokenStream;
use quote::quote;
use std::{collections::BTreeMap, fs, path::Path};

pub(super) fn definitions() -> Vec<TokenStream> {
    fn visit(namespace: &str, base: &Path, dir: &Path, tags: &mut BTreeMap<String, String>) {
        for entry in fs::read_dir(dir).unwrap().map(Result::unwrap) {
            let path = entry.path();
            if path.is_dir() {
                visit(namespace, base, &path, tags);
            } else if path.extension().is_some_and(|ext| ext == "json") {
                let id = format!(
                    "{}:{}",
                    namespace,
                    path.strip_prefix(base)
                        .unwrap()
                        .with_extension("")
                        .to_string_lossy()
                        .replace('\\', "/")
                );
                let json: serde_json::Value =
                    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
                tags.insert(id, serde_json::to_string(&json).unwrap());
            }
        }
    }
    let mut tags = BTreeMap::new();
    for namespace in fs::read_dir("../../assets/datapack/data")
        .unwrap()
        .map(Result::unwrap)
    {
        let dir = namespace.path().join("tags/block");
        if dir.is_dir() {
            visit(
                &namespace.file_name().to_string_lossy(),
                &dir,
                &dir,
                &mut tags,
            );
        }
    }
    tags.into_iter()
        .map(|(id, json)| quote! { (#id, #json) })
        .collect()
}
