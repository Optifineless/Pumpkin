use proc_macro2::{Span, TokenStream};
use quote::quote;
use serde_json::Value;
use syn::{LitFloat, LitInt, LitStr};

pub fn component_tokens(component: &Value) -> TokenStream {
    let uses = match component.get("uses").expect("Brewing fuel must have uses") {
        Value::Number(value) => {
            let value = i32::try_from(value.as_i64().expect("Brewing uses must be an integer"))
                .expect("Brewing uses must fit an integer");
            let value = LitInt::new(&value.to_string(), Span::call_site());
            quote! { BrewingFuelProvider::Constant(#value) }
        }
        value => reference_tokens(value),
    };
    let speed = match component
        .get("speed_multiplier")
        .expect("Brewing fuel must have speed_multiplier")
    {
        Value::Number(value) => {
            let value = value.as_f64().expect("Brewing speed must be a number") as f32;
            let value = LitFloat::new(&format!("{value:?}f32"), Span::call_site());
            quote! { BrewingFuelProvider::Constant(#value) }
        }
        value => reference_tokens(value),
    };
    quote! {
        (BrewingFuel, &BrewingFuelImpl { uses: #uses, speed_multiplier: #speed, }),
    }
}

fn reference_tokens(value: &Value) -> TokenStream {
    let id = match value {
        Value::String(id) => id.as_str(),
        _ => panic!("Brewing fuel default providers must be a number or a registry reference"),
    };
    let id = LitStr::new(id, Span::call_site());
    quote! { BrewingFuelProvider::Id(std::borrow::Cow::Borrowed(#id)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn b1019_g02_item_generator_preserves_brewing_fuel_providers() {
        let components: Value =
            serde_json::from_str(include_str!("../../../assets/items.json")).unwrap();
        let powder = &components["blaze_powder"]["components"]["minecraft:brewing_fuel"];
        let generated = component_tokens(powder).to_string();
        assert!(generated.contains("minecraft:brewing/uses_default"));
        assert!(generated.contains("minecraft:brewing/speed_default"));
        let custom = component_tokens(&serde_json::json!({"uses": 7, "speed_multiplier": 2.5}));
        let expected = quote! {
            (BrewingFuel, &BrewingFuelImpl {
                uses: BrewingFuelProvider::Constant(7),
                speed_multiplier: BrewingFuelProvider::Constant(2.5f32),
            }),
        };
        assert_eq!(custom.to_string(), expected.to_string());
    }
}
