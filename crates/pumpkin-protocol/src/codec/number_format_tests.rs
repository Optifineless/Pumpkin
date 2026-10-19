use std::io::Cursor;

use crate::{
    ClientPacket, NumberFormat, VarInt,
    java::client::play::{CUpdateObjectives, CUpdateScore, Mode, RenderType},
    ser::NetworkReadExt,
};
use pumpkin_nbt::{NbtCompound, tag::NbtTag};
use pumpkin_util::{
    text::{TextComponent, color::NamedColor, style::Style},
    version::JavaMinecraftVersion,
};

const VERSION: JavaMinecraftVersion = JavaMinecraftVersion::V_26_3;

fn styled() -> NumberFormat {
    NumberFormat::Styled(Style::default().color_named(NamedColor::Red).bold())
}

fn assert_style(input: &mut Cursor<Vec<u8>>) {
    assert!(input.get_bool().unwrap());
    assert_eq!(input.get_var_int().unwrap().0, 1);
    let Some(NbtTag::Compound(fields)) = input.get_nbt_with_version(&VERSION).unwrap() else {
        panic!("styled number format must carry its style compound");
    };
    // StyledFormat.TYPE -> Style.Serializer.TRUSTED_STREAM_CODEC writes only style fields.
    let mut expected = NbtCompound::new();
    expected.put_string("color", "red".into());
    expected.put_bool("bold", true);
    assert_eq!(fields, expected);
    assert_eq!(input.position() as usize, input.get_ref().len());
}

fn read_component(input: &mut Cursor<Vec<u8>>) -> TextComponent {
    TextComponent::from_nbt(&input.get_nbt_with_version(&VERSION).unwrap().unwrap())
}

#[test]
fn scoreboard_number_format_score_packet_writes_style_nbt() {
    let packet = CUpdateScore::new(
        "Holder".into(),
        "tally".into(),
        VarInt(9),
        None,
        Some(styled()),
    );
    let mut bytes = Vec::new();
    packet.write_packet_data(&mut bytes, &VERSION).unwrap();
    let mut input = Cursor::new(bytes);
    assert_eq!(&*input.get_str().unwrap(), "Holder");
    assert_eq!(&*input.get_str().unwrap(), "tally");
    assert_eq!(input.get_var_int().unwrap().0, 9);
    assert!(!input.get_bool().unwrap());
    assert_style(&mut input);
}

#[test]
fn scoreboard_number_format_objective_packet_preserves_style_fields() {
    let packet = CUpdateObjectives::new(
        "tally".into(),
        Mode::Update,
        TextComponent::text("Tally"),
        RenderType::Integer,
        Some(styled()),
    );
    let mut bytes = Vec::new();
    packet.write_packet_data(&mut bytes, &VERSION).unwrap();
    let mut input = Cursor::new(bytes);
    assert_eq!(&*input.get_str().unwrap(), "tally");
    assert_eq!(input.get_u8().unwrap(), 2);
    assert_eq!(read_component(&mut input), TextComponent::text("Tally"));
    assert_eq!(input.get_var_int().unwrap().0, 0);
    assert_style(&mut input);
}

#[test]
fn scoreboard_number_format_fixed_uses_component_wire_encoding() {
    let format = NumberFormat::Fixed(TextComponent::text("Many"));
    let mut bytes = Vec::new();
    format.write(&mut bytes).unwrap();
    let mut input = Cursor::new(bytes);
    assert_eq!(input.get_var_int().unwrap().0, 2);
    assert_eq!(read_component(&mut input), TextComponent::text("Many"));
    assert_eq!(input.position() as usize, input.get_ref().len());
}
