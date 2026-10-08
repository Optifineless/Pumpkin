use base64::prelude::*;
use pumpkin_protocol::Property;
use serde::Deserialize;

// Bit masks for the Java skin pixels that Bedrock requires to be opaque.
// Adapted from Geyser's SkinProvider under the MIT License.
const SKIN_OPAQUE_MASK: &str = "AP//AAAAAAAA//8AAAAAAAD//wAAAAAAAP//AAAAAAAA//8AAAAAAAD//wAAAAAAAP//AAAAAAAA//8AAAAAAP////8AAAAA/////wAAAAD/////AAAAAP////8AAAAA/////wAAAAD/////AAAAAP////8AAAAA/////wAAAADwD/D/D/8AAPAP8P8P/wAA8A/w/w//AADwD/D/D/8AAP///////w8A////////DwD///////8PAP///////w8A////////DwD///////8PAP///////w8A////////DwD///////8PAP///////w8A////////DwD///////8PAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAADwD/APAAAAAPAP8A8AAAAA8A/wDwAAAADwD/APAAAAAP////8AAAAA/////wAAAAD/////AAAAAP////8AAAAA/////wAAAAD/////AAAAAP////8AAAAA/////wAAAAD/////AAAAAP////8AAAAA/////wAAAAD/////AAA=";
const LEGACY_SKIN_OPAQUE_MASK: &str = "AP//AAAAAAAA//8AAAAAAAD//wAAAAAAAP//AAAAAAAA//8AAAAAAAD//wAAAAAAAP//AAAAAAAA//8AAAAAAP////8AAAAA/////wAAAAD/////AAAAAP////8AAAAA/////wAAAAD/////AAAAAP////8AAAAA/////wAAAADwD/D/D/APAPAP8P8P8A8A8A/w/w/wDwDwD/D/D/APAP////////8A/////////wD/////////AP////////8A/////////wD/////////AP////////8A/////////wD/////////AP////////8A/////////wD/////////AA==";

#[derive(Deserialize)]
struct TexturesProperty {
    textures: Textures,
}

#[derive(Deserialize)]
struct Textures {
    #[serde(rename = "SKIN")]
    skin: Option<SkinTexture>,
}

#[derive(Deserialize)]
struct SkinTexture {
    url: String,
    #[serde(default)]
    metadata: Option<SkinMetadata>,
}

#[derive(Deserialize)]
struct SkinMetadata {
    #[serde(default)]
    model: Option<String>,
}

use image::ImageDecoder;
use std::{
    io::Cursor,
    sync::{Arc, LazyLock},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

// Fork limits for Java-to-Bedrock skin conversion; Java PlayerList never downloads skins.
const MAX_SKIN_RESPONSE_BYTES: usize = 256 * 1024;
const MAX_TEXTURE_PROPERTY_BYTES: usize = 16 * 1024;
static SKIN_JOBS: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(8)));

pub async fn fetch_skin(
    properties: &[Property],
) -> Option<pumpkin_protocol::bedrock::client::Skin> {
    let permit = SKIN_JOBS.clone().try_acquire_owned().ok()?;
    let property = properties.iter().find(|p| &*p.name == "textures")?;
    if property.value.len() > MAX_TEXTURE_PROPERTY_BYTES {
        return None;
    }
    let textures: TexturesProperty =
        serde_json::from_slice(&BASE64_STANDARD.decode(property.value.as_bytes()).ok()?).ok()?;
    let texture = textures.textures.skin?;
    let slim = texture.metadata.as_ref().and_then(|m| m.model.as_deref()) == Some("slim");
    let mut response = pumpkin_auth::client_builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .ok()?
        .get(&texture.url)
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_SKIN_RESPONSE_BYTES as u64)
    {
        return None;
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        append_response_chunk(&mut bytes, &chunk)?;
    }
    // The permit lives inside the blocking job if the awaiting connection is cancelled.
    tokio::task::spawn_blocking(move || decode_skin_job(&bytes, texture.url, slim, permit))
        .await
        .ok()
        .flatten()
}

fn append_response_chunk(bytes: &mut Vec<u8>, chunk: &[u8]) -> Option<()> {
    if bytes.len().checked_add(chunk.len())? > MAX_SKIN_RESPONSE_BYTES {
        return None;
    }
    bytes.extend_from_slice(chunk);
    Some(())
}

fn decode_skin_job(
    bytes: &[u8],
    url: String,
    slim: bool,
    _permit: OwnedSemaphorePermit,
) -> Option<pumpkin_protocol::bedrock::client::Skin> {
    decode_skin(bytes, url, slim)
}

fn decode_skin(
    bytes: &[u8],
    url: String,
    slim: bool,
) -> Option<pumpkin_protocol::bedrock::client::Skin> {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(64);
    limits.max_image_height = Some(64);
    limits.max_alloc = Some(MAX_SKIN_RESPONSE_BYTES as u64);
    let decoder = image::codecs::png::PngDecoder::with_limits(Cursor::new(bytes), limits).ok()?;
    let (width, height) = decoder.dimensions();
    if width != 64 || !matches!(height, 32 | 64) {
        return None;
    }
    let mut rgba = image::DynamicImage::from_decoder(decoder)
        .ok()?
        .into_rgba8()
        .into_raw();
    let opaque_mask = BASE64_STANDARD
        .decode(if height == 32 {
            LEGACY_SKIN_OPAQUE_MASK
        } else {
            SKIN_OPAQUE_MASK
        })
        .ok()?;
    for pixel_index in 0..(width * height) as usize {
        if opaque_mask[pixel_index >> 3] & (1 << (pixel_index & 7)) != 0 {
            rgba[pixel_index * 4 + 3] = u8::MAX;
        }
    }
    let mut skin = pumpkin_protocol::bedrock::client::Skin::steve();
    skin.set_slim(slim);
    skin.image_width = width;
    skin.image_height = height;
    skin.skin_data = rgba;
    skin.skin_id.clone_from(&url);
    skin.full_id = url;
    Some(skin)
}

pub fn fetch_skin_blocking(
    properties: &[Property],
) -> Option<pumpkin_protocol::bedrock::client::Skin> {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        tokio::task::block_in_place(|| handle.block_on(fetch_skin(properties)))
    } else {
        tokio::runtime::Runtime::new()
            .ok()?
            .block_on(fetch_skin(properties))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streamed_skin_response_cannot_grow_past_the_byte_cap() {
        let mut bytes = vec![0; MAX_SKIN_RESPONSE_BYTES - 1];
        assert!(append_response_chunk(&mut bytes, &[0]).is_some());
        assert!(append_response_chunk(&mut bytes, &[0]).is_none());
        assert_eq!(bytes.len(), MAX_SKIN_RESPONSE_BYTES);
    }

    #[test]
    fn skin_decoder_accepts_both_layouts_and_rejects_oversized_images() {
        use image::ImageEncoder;
        for (width, height, accepted) in [
            (64, 32, true),
            (64, 64, true),
            (128, 32, false),
            (64, 128, false),
        ] {
            let mut bytes = Vec::new();
            image::codecs::png::PngEncoder::new(&mut bytes)
                .write_image(
                    &vec![0; (width * height * 4) as usize],
                    width,
                    height,
                    image::ExtendedColorType::Rgba8,
                )
                .unwrap();
            assert_eq!(
                decode_skin(&bytes, String::new(), false).is_some(),
                accepted
            );
        }
    }
}
