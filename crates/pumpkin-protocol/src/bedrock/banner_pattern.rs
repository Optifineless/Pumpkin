//! Banner pattern identifiers shared by Bedrock requests and item NBT.

// Geyser BannerPattern edition mapping.
const PATTERNS: &[(&str, &str)] = &[
    ("b", "base"),
    ("bl", "square_bottom_left"),
    ("br", "square_bottom_right"),
    ("tl", "square_top_left"),
    ("tr", "square_top_right"),
    ("bs", "stripe_bottom"),
    ("ts", "stripe_top"),
    ("ls", "stripe_left"),
    ("rs", "stripe_right"),
    ("cs", "stripe_center"),
    ("ms", "stripe_middle"),
    ("drs", "stripe_downright"),
    ("dls", "stripe_downleft"),
    ("ss", "small_stripes"),
    ("cr", "cross"),
    ("sc", "straight_cross"),
    ("bt", "triangle_bottom"),
    ("tt", "triangle_top"),
    ("bts", "triangles_bottom"),
    ("tts", "triangles_top"),
    ("ld", "diagonal_left"),
    ("rd", "diagonal_up_right"),
    ("lud", "diagonal_up_left"),
    ("rud", "diagonal_right"),
    ("mc", "circle"),
    ("mr", "rhombus"),
    ("vh", "half_vertical"),
    ("hh", "half_horizontal"),
    ("vhr", "half_vertical_right"),
    ("hhb", "half_horizontal_bottom"),
    ("bo", "border"),
    ("cbo", "curly_border"),
    ("gra", "gradient"),
    ("gru", "gradient_up"),
    ("bri", "bricks"),
    ("glb", "globe"),
    ("cre", "creeper"),
    ("sku", "skull"),
    ("flo", "flower"),
    ("moj", "mojang"),
    ("pig", "piglin"),
    ("flw", "flow"),
    ("gus", "guster"),
];

/// Resolves a Bedrock pattern identifier to its Java registry path.
#[must_use]
pub fn java_pattern(bedrock: &str) -> Option<&'static str> {
    PATTERNS
        .iter()
        .find_map(|&(id, path)| (id == bedrock).then_some(path))
}

/// Resolves a vanilla Java registry key to its Bedrock pattern identifier.
#[must_use]
pub fn bedrock_pattern(java: &str) -> Option<&'static str> {
    let path = java.strip_prefix("minecraft:")?;
    PATTERNS
        .iter()
        .find_map(|&(id, key)| (key == path).then_some(id))
}
