use super::*;
use crate::argument_builder::{argument, command, literal};
use crate::argument_types::core::string::StringArgumentType;
use crate::errors::command_syntax_error::CommandSyntaxErrorContext;
use std::sync::Mutex;

#[derive(Clone, Default)]
struct Output(Arc<Mutex<Vec<TextComponent>>>);

impl CommandSource for Output {
    fn send_message(&self, message: TextComponent) {
        self.0.lock().unwrap().push(message);
    }
}

#[test]
fn unicode_error_context_uses_ten_utf16_units_and_safe_cursor_boundaries() {
    for (input, cursor, expected) in [
        ("😀éx", 0, "😀éx"),
        ("😀éx", 1, "😀éx"),
        ("😀éx", 3, "😀éx"),
        ("😀éx", 4, "😀éx"),
        ("😀éx", 5, "😀éx"),
        ("😀éx", 7, "😀éx"),
        ("msg \"😀😀😀", 17, "...sg \"😀😀😀"),
        ("abcdefghij😀é", 16, "...defghij😀é"),
        ("abcdefghij😀é", usize::MAX, "...defghij😀é"),
        ("abcdefghij中é", usize::MAX, "...cdefghij中é"),
        ("abcdefghije\u{301}", usize::MAX, "...cdefghije\u{301}"),
        ("abcdefghij😀😀", usize::MAX, "...efghij😀😀"),
        // A straddling surrogate pair is omitted whole on a safe UTF-8 boundary.
        ("abcdefghi😀123456789", usize::MAX, "...123456789"),
    ] {
        let output = Output::default();
        let error = CommandSyntaxError::create(
            &DISPATCHER_UNKNOWN_ARGUMENT,
            TextComponent::text("syntax error"),
            &CommandSyntaxErrorContext {
                input: input.into(),
                cursor,
            },
        );
        CommandDispatcher::<Output>::send_error_to_source(&output, error, input);
        let messages = output.0.lock().unwrap();
        assert_eq!(messages.len(), 2);
        let component = &messages[1].0;
        assert_eq!(
            messages[1]
                .0
                .clone()
                .get_text(pumpkin_util::translation::Locale::EnUs),
            format!("{expected}<--[HERE]"),
            "{input:?}, cursor {cursor}"
        );
        let marker = component.extra.last().unwrap();
        assert_eq!(marker.style.color, Some(Color::Named(NamedColor::Red)));
        assert_eq!(marker.style.italic, Some(true));
        assert_eq!(component.style.color, Some(Color::Named(NamedColor::Gray)));
        assert!(component.style.click_event.is_some());
    }
}

#[test]
fn malformed_quoted_unicode_reports_a_syntax_error() {
    let mut dispatcher = CommandDispatcher::<Output>::new();
    dispatcher.register(
        command("msg", "msg").then(argument("target", StringArgumentType::QuotablePhrase)),
    );
    let output = Output::default();
    dispatcher.handle_command(&output, "msg \"😀😀😀");
    assert_eq!(output.0.lock().unwrap().len(), 2);
}

#[test]
fn disabled_root_commands_are_rejected_after_redirects_and_preparsed_execution() {
    let mut dispatcher = CommandDispatcher::new();
    let executor: fn(&CommandContext) -> crate::node::CommandExecutorResult = |_| Ok(1);
    let root = dispatcher.register_with_aliases(
        command("simple", "simple")
            .executes(executor)
            .then(literal("arg").executes(executor)),
        &["alias"],
    );
    dispatcher.register(command("redirect", "redirect").redirect(Redirection::Root));
    dispatcher.register(command("local", "local").redirect(Redirection::Local(root.into())));
    let source = DummySource::dummy();
    assert_eq!(dispatcher.execute_input("local arg", &source), Ok(1));
    // Parse on an available dispatcher, then execute on its disabled clone.
    let parsed = dispatcher.parse_input("redirect simple", &source);
    let mut disabled = dispatcher.clone();
    disabled.disable_command("simple");
    assert!(disabled.execute(parsed).is_err());
    for input in [
        "simple",
        "alias",
        "redirect simple",
        "redirect alias",
        "local arg",
    ] {
        assert!(disabled.execute_input(input, &source).is_err(), "{input}");
    }
    assert!(disabled.suggest("redirect sim", &source).is_empty());
    assert!(disabled.suggest("redirect ali", &source).is_empty());
    assert!(disabled.suggest("sim", &source).is_empty());
}

#[test]
fn review3_unavailable_redirect_error_points_to_child_start() {
    let mut dispatcher = CommandDispatcher::new();
    let executor: fn(&CommandContext) -> crate::node::CommandExecutorResult = |_| Ok(1);
    let target =
        dispatcher.register(command("target", "target").then(literal("arg").executes(executor)));
    dispatcher.register(
        command("outer", "outer")
            .then(literal("local").redirect(Redirection::Local(target.into()))),
    );
    dispatcher.disable_command("target");
    let source = DummySource::dummy();
    let parsed = dispatcher.parse_input("outer local arg", &source);
    let error = parsed
        .errors
        .values()
        .next()
        .expect("unavailable redirect error");
    assert_eq!(error.context.as_ref().unwrap().cursor, "outer ".len());
}
