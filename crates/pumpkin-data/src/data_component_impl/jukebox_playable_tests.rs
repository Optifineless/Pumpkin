use super::JukeboxPlayableImpl;
use crate::{
    data_component::DataComponent, data_component_impl::DataComponentImpl, item::Item,
    item_stack::ItemStack,
};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};

fn legacy(song: &str) -> NbtTag {
    let mut compound = NbtCompound::new();
    compound.put_string("song", song.to_owned());
    NbtTag::Compound(compound)
}

#[test]
fn b1020b_g08_jukebox_registered_song_codec_and_legacy_conversion() {
    let canonical = NbtTag::String("minecraft:cat".into());
    for fixture in [&canonical, &legacy("minecraft:cat"), &legacy("cat")] {
        let component = JukeboxPlayableImpl::read_data(fixture)
            .expect("canonical and old fork references must resolve the registered cat song");
        assert_eq!(component.song, "cat");
        assert_eq!(component.write_data(), canonical);
    }
    assert!(JukeboxPlayableImpl::read_data(&NbtTag::String("other:cat".into())).is_none());
    assert!(
        JukeboxPlayableImpl::read_data(&NbtTag::String("minecraft:not_a_song".into())).is_none()
    );
    assert!(JukeboxPlayableImpl::read_data(&NbtTag::Compound(NbtCompound::new())).is_none());
}

#[test]
fn b1020b_g08_jukebox_patch_survives_item_stack_serialization() {
    let mut stack = ItemStack::new(1, &Item::MUSIC_DISC_13);
    stack.set_data_component(JukeboxPlayableImpl {
        song: "minecraft:cat",
    });
    stack.set_custom_name("Retained record".to_owned());
    let mut nbt = NbtCompound::new();
    stack.write_item_stack(&mut nbt);
    assert_eq!(
        nbt.get_compound("components")
            .and_then(|components| components.get("minecraft:jukebox_playable")),
        Some(&NbtTag::String("minecraft:cat".into()))
    );
    let restored = ItemStack::read_item_stack(&nbt).expect("explicit song patch must decode");
    assert_eq!(restored.item, &Item::MUSIC_DISC_13);
    assert_eq!(restored.item_count, 1);
    assert_eq!(
        restored
            .get_data_component::<JukeboxPlayableImpl>()
            .map(|component| component.song),
        Some("cat")
    );
    assert_eq!(restored.get_hover_name(), "Retained record");

    stack.remove_data_component(DataComponent::JukeboxPlayable);
    let mut removed_nbt = NbtCompound::new();
    stack.write_item_stack(&mut removed_nbt);
    let removed = ItemStack::read_item_stack(&removed_nbt).expect("removed component must decode");
    assert!(
        removed
            .get_data_component::<JukeboxPlayableImpl>()
            .is_none()
    );
}

#[test]
fn b1020b_g08_default_and_invalid_legacy_jukebox_controls() {
    let disc = ItemStack::new(1, &Item::MUSIC_DISC_CAT);
    let mut nbt = NbtCompound::new();
    disc.write_item_stack(&mut nbt);
    let ordinary = ItemStack::read_item_stack(&nbt).expect("default generated disc must decode");
    assert!(ordinary.patch.is_empty());
    assert!(ordinary.are_equal(&disc));

    nbt.put_compound("components", {
        let mut components = NbtCompound::new();
        components.put("minecraft:jukebox_playable", legacy("minecraft:not_a_song"));
        components
    });
    let invalid = ItemStack::read_item_stack(&nbt).expect("old invalid record must remain present");
    assert_eq!(invalid.item, &Item::MUSIC_DISC_CAT);
    assert_eq!(
        invalid
            .get_data_component::<JukeboxPlayableImpl>()
            .map(|component| component.song),
        Some("")
    );
    let mut resaved = NbtCompound::new();
    invalid.write_item_stack(&mut resaved);
    let restored = ItemStack::read_item_stack(&resaved).expect("stopped old record must resave");
    assert_eq!(
        restored
            .get_data_component::<JukeboxPlayableImpl>()
            .map(|component| component.song),
        Some("")
    );
}
