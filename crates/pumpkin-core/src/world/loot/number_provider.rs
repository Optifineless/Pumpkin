use super::{LootRandom, MAX_LOOT_ROLLS};
use serde_json::Value;
pub(super) fn number_int(value: &Value, rng: &mut LootRandom<'_>, depth: usize) -> Option<i32> {
    // ContextIntProviders: integer uniform bounds are inclusive, binomial is not uniform.
    if !pumpkin_util::loot_table::number_provider_supported(value, depth) {
        return None;
    }
    if let Some(number) = value.as_f64() {
        return Some(number.floor() as i32);
    }
    let field = |name| value.get(name).unwrap_or(&Value::Null);
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("minecraft:uniform")
        .trim_start_matches("minecraft:");
    Some(match kind {
        "constant" => number_int(field("value"), rng, depth + 1)?,
        "uniform" => {
            let min = number_int(field("min"), rng, depth + 1)?;
            let max = number_int(field("max"), rng, depth + 1)?;
            let range = max.saturating_sub(min).saturating_add(1);
            if min >= max {
                min
            } else {
                min.saturating_add(rng.next_bounded_i32(range))
            }
        }
        "binomial" => {
            let n = number_int(field("n"), rng, depth + 1)?.clamp(0, MAX_LOOT_ROLLS);
            let p = number_float(field("p"), rng, depth + 1)?;
            (0..n).filter(|_| rng.next_f32() < p).count() as i32
        }
        _ => return None,
    })
}
pub(super) fn number_float(value: &Value, rng: &mut LootRandom<'_>, depth: usize) -> Option<f32> {
    if !pumpkin_util::loot_table::number_provider_supported(value, depth) {
        return None;
    }
    if let Some(number) = value.as_f64() {
        return Some(number as f32);
    }
    let field = |name| value.get(name).unwrap_or(&Value::Null);
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("minecraft:uniform")
        .trim_start_matches("minecraft:");
    Some(match kind {
        "constant" => number_float(field("value"), rng, depth + 1)?,
        "uniform" => {
            let min = number_float(field("min"), rng, depth + 1)?;
            let max = number_float(field("max"), rng, depth + 1)?;
            if min >= max {
                min
            } else {
                min + rng.next_f32() * (max - min)
            }
        }
        "binomial" => number_int(value, rng, depth + 1)? as f32,
        _ => return None,
    })
}
