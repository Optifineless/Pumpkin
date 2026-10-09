use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicI8, Ordering},
};

use super::BlockEntity;
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::text::TextComponent;
use pumpkin_util::text::click::ClickEvent;

pub use pumpkin_data::dye_color::DyeColor;

pub struct SignBlockEntity {
    pub front_text: SignText,
    pub back_text: SignText,
    pub is_waxed: AtomicBool,
    pub allow_op_features: AtomicBool,
    position: BlockPos,
    pub currently_editing_player: Arc<Mutex<Option<uuid::Uuid>>>,
}

pub type Text = SignText;

pub struct SignText {
    pub has_glowing_text: AtomicBool,
    color: AtomicI8,
    pub messages: Arc<Mutex<[TextComponent; 4]>>,
    pub filtered_messages: Arc<Mutex<[TextComponent; 4]>>,
}

impl Clone for SignText {
    fn clone(&self) -> Self {
        Self {
            has_glowing_text: AtomicBool::new(self.has_glowing_text.load(Ordering::Relaxed)),
            color: AtomicI8::new(self.color.load(Ordering::Relaxed)),
            messages: self.messages.clone(),
            filtered_messages: self.filtered_messages.clone(),
        }
    }
}

impl Default for SignText {
    fn default() -> Self {
        Self {
            has_glowing_text: AtomicBool::new(false),
            color: AtomicI8::new(DyeColor::Black as i8),
            messages: Arc::new(Mutex::new(Self::empty_messages())),
            filtered_messages: Arc::new(Mutex::new(Self::empty_messages())),
        }
    }
}

#[allow(clippy::fallible_impl_from)]
impl From<SignText> for NbtTag {
    fn from(value: SignText) -> Self {
        let mut nbt = NbtCompound::new();
        nbt.put_bool(
            "has_glowing_text",
            value.has_glowing_text.load(Ordering::Relaxed),
        );
        nbt.put_string("color", value.get_color().name().to_string());

        let messages = value
            .messages
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        nbt.put_list(
            "messages",
            messages
                .iter()
                .map(|s| Self::Compound(s.0.to_nbt_compound()))
                .collect(),
        );

        let filtered_messages = value
            .filtered_messages
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *filtered_messages != *messages {
            nbt.put_list(
                "filtered_messages",
                filtered_messages
                    .iter()
                    .map(|s| Self::Compound(s.0.to_nbt_compound()))
                    .collect(),
            );
        }

        Self::Compound(nbt)
    }
}

impl From<NbtTag> for SignText {
    fn from(tag: NbtTag) -> Self {
        let Some(nbt) = tag.extract_compound() else {
            return Self::default();
        };
        let has_glowing_text = nbt.get_bool("has_glowing_text").unwrap_or(false);
        let color = nbt.get_string("color").unwrap_or("black");
        // SignText.CODEC / ComponentSerialization.CODEC treat NBT strings as literal text.
        let read_messages = |key: &str| {
            let list = nbt.get_list(key);
            std::array::from_fn(|i| {
                list.and_then(|list| list.get(i))
                    .map_or_else(TextComponent::empty, TextComponent::from_nbt)
            })
        };
        let parsed_messages = read_messages("messages");
        let parsed_filtered = if nbt.get_list("filtered_messages").is_some() {
            read_messages("filtered_messages")
        } else {
            parsed_messages.clone()
        };

        Self {
            has_glowing_text: AtomicBool::new(has_glowing_text),
            color: AtomicI8::new(DyeColor::by_name(color).unwrap_or(DyeColor::Black).id() as i8),
            messages: Arc::new(Mutex::new(parsed_messages)),
            filtered_messages: Arc::new(Mutex::new(parsed_filtered)),
        }
    }
}

impl SignText {
    pub const LINES: usize = 4;

    #[must_use]
    pub fn empty_messages() -> [TextComponent; 4] {
        std::array::from_fn(|_| TextComponent::empty())
    }

    #[must_use]
    pub fn new(
        messages: [Box<str>; 4],
        filtered_messages: Option<[Box<str>; 4]>,
        color: DyeColor,
        has_glowing_text: bool,
    ) -> Self {
        let messages = messages.map(|message| TextComponent::text(message.into_string()));
        let filtered = filtered_messages.map_or_else(
            || messages.clone(),
            |lines| lines.map(|message| TextComponent::text(message.into_string())),
        );
        Self {
            has_glowing_text: AtomicBool::new(has_glowing_text),
            color: AtomicI8::new(color.id() as i8),
            messages: Arc::new(Mutex::new(messages)),
            filtered_messages: Arc::new(Mutex::new(filtered)),
        }
    }

    #[must_use]
    pub fn from_messages(messages: [Box<str>; 4]) -> Self {
        Self::new(messages, None, DyeColor::Black, false)
    }

    #[must_use]
    pub fn has_glowing_text(&self) -> bool {
        self.has_glowing_text.load(Ordering::Relaxed)
    }

    pub fn set_has_glowing_text(&self, has_glowing_text: bool) {
        self.has_glowing_text
            .store(has_glowing_text, Ordering::Relaxed);
    }

    #[must_use]
    pub fn get_color(&self) -> DyeColor {
        let c = self.color.load(Ordering::Relaxed);
        if c >= 0 {
            DyeColor::by_id(c as u8).unwrap_or(DyeColor::Black)
        } else {
            DyeColor::Black
        }
    }

    pub fn set_color(&self, color: DyeColor) {
        self.color.store(color.id() as i8, Ordering::Relaxed);
    }

    #[must_use]
    pub fn get_message(&self, index: usize, should_filter: bool) -> Box<str> {
        if index >= Self::LINES {
            return Box::from("");
        }
        let lock = if should_filter {
            self.filtered_messages.lock()
        } else {
            self.messages.lock()
        };
        lock.unwrap_or_else(std::sync::PoisonError::into_inner)[index]
            .clone()
            .get_text()
            .into_boxed_str()
    }

    pub fn set_message(&self, index: usize, message: Box<str>, filtered_message: Option<Box<str>>) {
        if index >= Self::LINES {
            return;
        }
        let filtered = filtered_message.unwrap_or_else(|| message.clone());
        let message = TextComponent::text(message.into_string());
        let filtered = TextComponent::text(filtered.into_string());
        self.messages
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)[index] = message;
        self.filtered_messages
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)[index] = filtered;
    }

    /// Stores submitted lines as literals, retaining styles and matching the filtered copy.
    // SignBlockEntity.updateMessages retains the previous line's style.
    pub fn update_messages(&self, lines: [&str; Self::LINES], should_filter: bool) {
        let original = self.get_messages(should_filter);
        let mut messages = self
            .messages
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for ((message, line), previous) in messages.iter_mut().zip(lines).zip(original) {
            let mut literal = TextComponent::text(line.to_string());
            literal.0.style = previous.0.style;
            *message = literal;
        }
        self.filtered_messages
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone_from(&messages);
    }

    #[must_use]
    pub fn get_messages(&self, should_filter: bool) -> [TextComponent; 4] {
        let lock = if should_filter {
            self.filtered_messages.lock()
        } else {
            self.messages.lock()
        };
        lock.unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    #[must_use]
    pub fn has_message(&self, should_filter: bool) -> bool {
        let messages = self.get_messages(should_filter);
        messages.into_iter().any(|msg| !msg.get_text().is_empty())
    }

    #[must_use]
    pub fn has_any_click_commands(&self, should_filter: bool) -> bool {
        let messages = self.get_messages(should_filter);
        messages.iter().any(|component| {
            matches!(
                component.0.style.click_event,
                Some(ClickEvent::RunCommand { .. })
            )
        })
    }
}

impl BlockEntity for SignBlockEntity {
    fn tick(&self, world: &Arc<crate::world::World>) {
        super::sign::expire_edit_session(world, self.position, &self.currently_editing_player);
    }

    fn resource_location(&self) -> &'static str {
        Self::ID
    }

    fn get_position(&self) -> BlockPos {
        self.position
    }

    fn from_nbt(nbt: &pumpkin_nbt::compound::NbtCompound, position: BlockPos) -> Self
    where
        Self: Sized,
    {
        let front_text = nbt
            .get("front_text")
            .cloned()
            .map(SignText::from)
            .unwrap_or_default();
        let back_text = nbt
            .get("back_text")
            .cloned()
            .map(SignText::from)
            .unwrap_or_default();
        let is_waxed = nbt.get_bool("is_waxed").unwrap_or(false);
        Self {
            position,
            front_text,
            back_text,
            is_waxed: AtomicBool::new(is_waxed),
            allow_op_features: AtomicBool::new(nbt.get_bool("allow_op_features").unwrap_or(false)),
            currently_editing_player: Arc::new(Mutex::new(None)),
        }
    }

    fn write_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put("front_text", self.front_text.clone());
        nbt.put("back_text", self.back_text.clone());
        nbt.put_bool("is_waxed", self.is_waxed.load(Ordering::Relaxed));
        // SignBlockEntity.saveAdditional omits the default false trust flag.
        if self.allow_op_features.load(Ordering::Relaxed) {
            nbt.put_bool("allow_op_features", true);
        }
    }

    fn chunk_data_nbt(&self) -> Option<NbtCompound> {
        let mut nbt = NbtCompound::new();
        self.write_nbt(&mut nbt);
        Some(nbt)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl SignBlockEntity {
    pub const ID: &'static str = "minecraft:sign";

    #[must_use]
    pub fn new(position: BlockPos, is_front: bool, messages: [Box<str>; 4]) -> Self {
        Self {
            position,
            is_waxed: AtomicBool::new(false),
            allow_op_features: AtomicBool::new(false),
            front_text: if is_front {
                SignText::from_messages(messages.clone())
            } else {
                SignText::default()
            },
            back_text: if is_front {
                SignText::default()
            } else {
                SignText::from_messages(messages)
            },
            currently_editing_player: Arc::new(Mutex::new(None)),
        }
    }

    #[must_use]
    pub fn empty(position: BlockPos) -> Self {
        Self {
            position,
            is_waxed: AtomicBool::new(false),
            allow_op_features: AtomicBool::new(false),
            front_text: SignText::default(),
            back_text: SignText::default(),
            currently_editing_player: Arc::new(Mutex::new(None)),
        }
    }
}

pub enum SignEntityRef<'a> {
    Sign(&'a SignBlockEntity),
    Hanging(&'a super::hanging_sign::HangingSignBlockEntity),
}

impl<'a> SignEntityRef<'a> {
    pub fn from_block_entity(entity: &'a dyn BlockEntity) -> Option<Self> {
        entity
            .as_any()
            .downcast_ref::<SignBlockEntity>()
            .map(Self::Sign)
            .or_else(|| {
                entity
                    .as_any()
                    .downcast_ref::<super::hanging_sign::HangingSignBlockEntity>()
                    .map(Self::Hanging)
            })
    }

    #[must_use]
    pub const fn front_text(&self) -> &'a SignText {
        match self {
            Self::Sign(s) => &s.front_text,
            Self::Hanging(s) => &s.front_text,
        }
    }

    #[must_use]
    pub const fn back_text(&self) -> &'a SignText {
        match self {
            Self::Sign(s) => &s.back_text,
            Self::Hanging(s) => &s.back_text,
        }
    }

    #[must_use]
    pub const fn get_text(&self, is_front: bool) -> &'a SignText {
        if is_front {
            self.front_text()
        } else {
            self.back_text()
        }
    }

    #[must_use]
    pub fn is_waxed(&self) -> bool {
        match self {
            Self::Sign(s) => s.is_waxed.load(Ordering::Relaxed),
            Self::Hanging(s) => s.is_waxed.load(Ordering::Relaxed),
        }
    }

    /// Whether trusted block entity NBT enabled operator click actions, independently of wax.
    #[must_use]
    pub fn allow_op_features(&self) -> bool {
        match self {
            Self::Sign(s) => s.allow_op_features.load(Ordering::Relaxed),
            Self::Hanging(s) => s.allow_op_features.load(Ordering::Relaxed),
        }
    }

    pub fn set_waxed(&self, waxed: bool) {
        match self {
            Self::Sign(s) => s.is_waxed.store(waxed, Ordering::Relaxed),
            Self::Hanging(s) => s.is_waxed.store(waxed, Ordering::Relaxed),
        }
    }

    #[must_use]
    pub const fn currently_editing_player(&self) -> &'a Arc<Mutex<Option<uuid::Uuid>>> {
        match self {
            Self::Sign(s) => &s.currently_editing_player,
            Self::Hanging(s) => &s.currently_editing_player,
        }
    }
}

// SignBlockEntity.tick / playerIsTooFarAwayToEdit use the block interaction range plus four.
pub(super) fn expire_edit_session(
    world: &Arc<crate::world::World>,
    position: BlockPos,
    editor: &Mutex<Option<uuid::Uuid>>,
) {
    clear_invalid_player_who_may_edit(editor, |id| {
        world
            .get_player_by_uuid(id)
            .is_none_or(|player| !player.can_interact_with_block_at(&position, 4.0))
    });
}

// SignBlockEntity.clearInvalidPlayerWhoMayEdit checks the player without holding the editor lock.
fn clear_invalid_player_who_may_edit(
    editor: &Mutex<Option<uuid::Uuid>>,
    too_far: impl FnOnce(uuid::Uuid) -> bool,
) {
    let id = *editor
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(id) = id
        && too_far(id)
    {
        let mut editor = editor
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *editor == Some(id) {
            *editor = None;
        }
    }
}

/// Applies sign item NBT only for a creative gamemaster, as BlockItem.updateCustomBlockEntityTag.
pub(crate) fn apply_trusted_item_data(
    entity: &Arc<dyn BlockEntity>,
    stack: &pumpkin_data::item_stack::ItemStack,
    player: &crate::entity::player::Player,
    world: &Arc<crate::world::World>,
) {
    if !player.can_use_game_master_blocks() {
        return;
    }
    let Some(sign) = SignEntityRef::from_block_entity(&**entity) else {
        return;
    };
    let Some(data) =
        stack.get_data_component::<pumpkin_data::data_component_impl::BlockEntityDataImpl>()
    else {
        return;
    };
    if data
        .nbt
        .get_string("id")
        .map(|id| id.strip_prefix("minecraft:").unwrap_or(id))
        != entity.resource_location().strip_prefix("minecraft:")
    {
        return;
    }
    let mut nbt = NbtCompound::new();
    entity.write_nbt(&mut nbt);
    // TypedEntityData.loadInto merges custom data, then reloads only when it changed.
    let old_nbt = nbt.clone();
    let mut custom_data = data.nbt.clone();
    custom_data.child_tags.remove("id");
    nbt.merge(&custom_data);
    if nbt == old_nbt {
        return;
    }
    let loaded: Arc<dyn BlockEntity> = match sign {
        SignEntityRef::Sign(_) => Arc::new(SignBlockEntity::from_nbt(&nbt, entity.get_position())),
        SignEntityRef::Hanging(_) => Arc::new(
            super::hanging_sign::HangingSignBlockEntity::from_nbt(&nbt, entity.get_position()),
        ),
    };
    // SignBlockEntity.loadAdditional leaves the editing session untouched.
    if let Some(loaded_sign) = SignEntityRef::from_block_entity(&*loaded) {
        *loaded_sign
            .currently_editing_player()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = *sign
            .currently_editing_player()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
    }
    world.add_block_entity(loaded);
}

#[cfg(test)]
mod session_tests {
    use super::*;

    #[test]
    fn review_sign_distance_check_releases_lock_and_preserves_new_editor() {
        let first = uuid::Uuid::new_v4();
        let second = uuid::Uuid::new_v4();
        let editor = Mutex::new(Some(first));
        clear_invalid_player_who_may_edit(&editor, |id| {
            assert_eq!(id, first);
            *editor
                .try_lock()
                .expect("distance check held the editor lock") = Some(second);
            true
        });
        assert_eq!(*editor.lock().unwrap(), Some(second));
        clear_invalid_player_who_may_edit(&editor, |_| true);
        assert_eq!(*editor.lock().unwrap(), None);
    }
}
