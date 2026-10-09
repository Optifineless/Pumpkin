pub mod bit_set;
pub mod bitset;
mod bucket_variants;
pub mod data_component;
pub mod item_stack_seralizer;
mod item_stack_validation;
// incoming-stack tests also cover item 7 templates.
#[cfg(test)]
mod incoming_item_tests;
pub mod little_endian;
pub mod lp_vector_3d;
pub mod optional_int;
pub mod particle_options;
pub mod recipe;
mod u24_type;
pub mod uuid;
pub mod var_int;
pub mod var_long;
pub mod var_uint;
pub mod var_ulong;

pub use u24_type::u24;

#[cfg(test)]
mod item_decode_tests;

mod item_cost_component_ids;
