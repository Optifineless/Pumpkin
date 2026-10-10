use std::{collections::BTreeMap, sync::LazyLock};

use pumpkin_data::data_component::DataComponent;
use pumpkin_util::version::JavaMinecraftVersion;

// Extractor registry snapshots from the upstream ports 42c903493, 0635b048b and 23e7992b2.
// TypedDataComponent.STREAM_CODEC uses the connection's registry to select the value codec.
type Registry = Result<BTreeMap<u8, DataComponent>, String>;
static V_1_21_11: LazyLock<Registry> = LazyLock::new(|| {
    registry(include_str!(
        "../../../../assets/data_component_versions/1.21.11.json"
    ))
});
static V_26_1: LazyLock<Registry> = LazyLock::new(|| {
    registry(include_str!(
        "../../../../assets/data_component_versions/26.1.json"
    ))
});
static V_26_2: LazyLock<Registry> = LazyLock::new(|| {
    registry(include_str!(
        "../../../../assets/data_component_versions/26.2.json"
    ))
});

fn registry(data: &str) -> Registry {
    let ids: BTreeMap<String, u8> = serde_json::from_str(data).map_err(|err| err.to_string())?;
    Ok(ids
        .into_iter()
        .filter_map(|(name, id)| {
            DataComponent::try_from_name(&name).map_or_else(
                || {
                    // Each version's LazyLock reports unavailable historical codecs only once.
                    tracing::warn!(name, id, "Unknown item cost component in registry snapshot");
                    None
                },
                |component| Some((id, component)),
            )
        })
        .collect())
}

fn registry_for(
    version: JavaMinecraftVersion,
) -> Result<&'static BTreeMap<u8, DataComponent>, String> {
    let registry = if version >= JavaMinecraftVersion::V_26_2 {
        &*V_26_2
    } else if version >= JavaMinecraftVersion::V_26_1 {
        &*V_26_1
    } else if version >= JavaMinecraftVersion::V_1_21_11 {
        &*V_1_21_11
    } else {
        return Err("Item cost component registry is unavailable for this version".into());
    };
    registry.as_ref().map_err(Clone::clone)
}

pub(super) fn from_id(id: u8, version: JavaMinecraftVersion) -> Result<DataComponent, String> {
    let component = if version >= JavaMinecraftVersion::V_26_3 {
        DataComponent::try_from_id(id)
    } else {
        registry_for(version)?.get(&id).copied()
    };
    component.ok_or_else(|| format!("Unknown item cost component ID: {id}"))
}

pub(super) fn to_id(component: DataComponent, version: JavaMinecraftVersion) -> Result<u8, String> {
    if version >= JavaMinecraftVersion::V_26_3 {
        return Ok(component.to_id());
    }
    registry_for(version)?
        .iter()
        .find_map(|(&id, &value)| (value == component).then_some(id))
        .ok_or_else(|| "Item cost component is unavailable for this version".into())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex, PoisonError};

    use tracing::{Event, Metadata, Subscriber, field::Visit, span};

    use super::*;

    #[derive(Clone, Default)]
    struct RegistryWarnings(Arc<Mutex<Vec<String>>>);

    impl Visit for RegistryWarnings {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            if field.name() == "name" {
                self.0
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(format!("{value:?}"));
            }
        }
    }

    impl Subscriber for RegistryWarnings {
        fn enabled(&self, metadata: &Metadata<'_>) -> bool {
            *metadata.level() == tracing::Level::WARN
        }

        fn new_span(&self, _span: &span::Attributes<'_>) -> span::Id {
            span::Id::from_u64(1)
        }

        fn record(&self, _span: &span::Id, _values: &span::Record<'_>) {}

        fn record_follows_from(&self, _span: &span::Id, _follows: &span::Id) {}

        fn event(&self, event: &Event<'_>) {
            event.record(&mut self.clone());
        }

        fn enter(&self, _span: &span::Id) {}

        fn exit(&self, _span: &span::Id) {}
    }

    #[test]
    fn regression_snapshot_components_are_resolved_or_reported()
    -> Result<(), Box<dyn std::error::Error>> {
        for snapshot in [
            include_str!("../../../../assets/data_component_versions/1.21.11.json"),
            include_str!("../../../../assets/data_component_versions/26.1.json"),
            include_str!("../../../../assets/data_component_versions/26.2.json"),
        ] {
            let warnings = RegistryWarnings::default();
            let resolved =
                tracing::subscriber::with_default(warnings.clone(), || registry(snapshot))?;
            let names: BTreeMap<String, u8> = serde_json::from_str(snapshot)?;
            let reported = warnings.0.lock().unwrap_or_else(PoisonError::into_inner);
            let mut missing = 0;
            for (name, id) in names {
                if let Some(component) = DataComponent::try_from_name(&name) {
                    assert!(resolved.get(&id) == Some(&component), "{name}");
                } else {
                    missing += 1;
                    assert!(reported.contains(&format!("{name:?}")), "{name}");
                }
            }
            assert_eq!(reported.len(), missing);
        }
        Ok(())
    }
}
