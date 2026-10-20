use super::*;
use crate::entity::{Entity, EntityBase, decoration::display::TextDisplayEntity};
use pumpkin_nbt::tag::NbtTag;
use pumpkin_protocol::{
    ClientPacket, VarInt,
    java::client::play::{CSetEntityMetadata, Metadata},
    ser::NetworkReadExt,
};
use pumpkin_util::{
    math::vector3::Vector3,
    text::{TextComponent, TextContent, hover::HoverEvent},
    translation::Locale,
    version::JavaMinecraftVersion,
};
use std::io::Cursor;

const VERSION: JavaMinecraftVersion = JavaMinecraftVersion::V_26_3;
const UUID: &str = "00000000-0000-0000-0000-000000000001";

fn mixed_text(length: usize) -> TextComponent {
    TextComponent::text("A".repeat(length)).add_child(TextComponent::from_content(
        TextContent::Custom {
            key: "x".into(),
            with: vec![],
            locale: Locale::EnUs,
        },
    ))
}

fn assert_saved_text(saved: &NbtCompound) {
    let name_tag = saved.get("CustomName").unwrap();
    let name_text = name_tag.extract_string().or_else(|| {
        name_tag
            .extract_compound()
            .and_then(|name| name.get_string("text"))
    });
    let display: serde_json::Value =
        serde_json::from_str(saved.get_string("text").unwrap()).unwrap();
    assert_eq!(
        (
            name_text.map(str::len),
            display["text"].as_str().map(str::len)
        ),
        (Some(40_000), Some(40_000))
    );
    let text = "A".repeat(40_000);
    assert!(name_text == Some(text.as_str()) && display["text"].as_str() == Some(text.as_str()));
    let name = name_tag.extract_compound().unwrap();
    assert_eq!(
        name.get_list("extra").unwrap(),
        &[NbtTag::String("x".into())]
    );
    assert_eq!(display["extra"][0]["text"], "x");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoreboard_regression_mixed_entity_text_save_reload_is_lossless() {
    let directory = tempfile::tempdir().unwrap();
    with_worlds(directory.path(), None, |_server, worlds| async move {
        let make_display = || {
            TextDisplayEntity::new(Entity::new(
                worlds[0].clone(),
                Vector3::default(),
                &pumpkin_data::entity::EntityType::TEXT_DISPLAY,
            ))
        };
        let display = make_display();
        display.get_entity().set_custom_name(mixed_text(40_000));
        display.set_text(mixed_text(40_000));
        let mut saved = NbtCompound::new();
        EntityBase::write_nbt(display.as_ref(), &mut saved);
        assert_saved_text(&saved);
        let restored = make_display();
        EntityBase::read_nbt_non_mut(restored.as_ref(), &saved);
        let mut resaved = NbtCompound::new();
        EntityBase::write_nbt(restored.as_ref(), &mut resaved);
        assert_saved_text(&resaved);
    })
    .await;
}

fn metadata_component(component: TextComponent) -> NbtCompound {
    let tracked = pumpkin_data::tracked_data::entity::DATA_CUSTOM_NAME;
    let mut metadata = Vec::new();
    Metadata::new(tracked, Some(component))
        .write(&mut metadata, &VERSION)
        .unwrap();
    metadata.push(255);
    let packet = CSetEntityMetadata::new(VarInt(42), metadata.into_boxed_slice());
    let mut bytes = Vec::new();
    packet.write_packet_data(&mut bytes, &VERSION).unwrap();
    let mut input = Cursor::new(bytes.as_slice());
    assert_eq!(input.get_var_int().unwrap(), VarInt(42));
    assert_eq!(input.get_u8().unwrap(), tracked.get(&VERSION));
    assert_eq!(input.get_var_int().unwrap().0, tracked.r#type.id(VERSION));
    assert!(input.get_bool().unwrap());
    let tag = input.get_nbt_with_version(&VERSION).unwrap().unwrap();
    assert_eq!(input.get_u8().unwrap(), 255);
    assert_eq!(input.position() as usize, bytes.len());
    tag.extract_compound().unwrap().clone()
}

#[test]
fn scoreboard_regression_hover_metadata_retains_complete_identifiers() {
    for length in [1, 32_360] {
        let component =
            metadata_component(mixed_text(length).hover_event(HoverEvent::ShowEntity {
                id: "minecraft:pig".into(),
                uuid: UUID.into(),
                name: None,
            }));
        let event = component.get_compound("hover_event");
        if length == 1 {
            assert!(event.is_some(), "Affordable hover event was discarded");
        }
        if let Some(event) = event {
            assert_eq!(event.get_string("id"), Some("minecraft:pig"));
            assert_eq!(event.get_string("uuid"), Some(UUID));
            assert!(uuid::Uuid::parse_str(event.get_string("uuid").unwrap()).is_ok());
        }
    }
}
