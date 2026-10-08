use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote};
use serde::Deserialize;

use super::recipes::{RecipeIngredientTypes, RecipeResultStruct};

#[derive(Deserialize)]
#[serde(untagged)]
pub enum MaterialCount {
    Exact(u8),
    Range { min: Option<u8>, max: Option<u8> },
}

impl Default for MaterialCount {
    fn default() -> Self {
        Self::Exact(1) // TransmuteRecipe.DEFAULT_MATERIAL_COUNT.
    }
}

impl MaterialCount {
    pub fn bounds(&self) -> (u8, u8) {
        match *self {
            Self::Exact(n) => (n, n),
            Self::Range { min, max } => (min.unwrap_or(1), max.unwrap_or(8)),
        }
    }
}

#[derive(Deserialize)]
pub struct SpecialRecipeStruct {
    category: Option<super::recipes::RecipeCategoryTypes>,
    group: Option<String>,
    source: Option<RecipeIngredientTypes>,
    material: Option<RecipeIngredientTypes>,
    shell: Option<RecipeIngredientTypes>,
    fuel: Option<RecipeIngredientTypes>,
    star: Option<RecipeIngredientTypes>,
    target: Option<RecipeIngredientTypes>,
    dye: Option<RecipeIngredientTypes>,
    banner: Option<RecipeIngredientTypes>,
    allowed_generations: Option<MaterialCount>,
    result: RecipeResultStruct,
}

impl SpecialRecipeStruct {
    pub fn tokens(&self, name: &str) -> TokenStream {
        let variant = format_ident!("{name}");
        let ingredients: Vec<_> = [
            &self.source,
            &self.material,
            &self.shell,
            &self.fuel,
            &self.star,
            &self.target,
            &self.dye,
            &self.banner,
        ]
        .into_iter()
        .flatten()
        .map(ToTokens::to_token_stream)
        .collect();
        let result = self.result.to_token_stream();
        let metadata = if name == "Dye" {
            let category = self.category.as_ref().map_or_else(
                || super::recipes::RecipeCategoryTypes::Misc.to_token_stream(),
                ToTokens::to_token_stream,
            );
            let group = self
                .group
                .as_ref()
                .map_or_else(|| quote! { None }, |group| quote! { Some(#group) });
            quote! { category: #category, group: #group, }
        } else {
            TokenStream::new()
        };
        let generations = if name == "BookCloning" {
            // BookCloningRecipe.DEFAULT_BOOK_GENERATION_RANGES.
            let (min, max) = self
                .allowed_generations
                .as_ref()
                .map_or((0, 1), MaterialCount::bounds);
            quote! { allowed_generations: (#min, #max), }
        } else {
            TokenStream::new()
        };
        quote! { CraftingRecipeTypes::#variant { ingredients: &[#(#ingredients),*], #metadata #generations result: #result } }
    }
}
