use super::*;

fn arrow(
    input: &mut TextInput<'_, (), Theme, ()>,
    tree: &mut Tree,
    named: keyboard::key::Named,
    modifiers: keyboard::Modifiers,
) -> bool {
    let bounds = Rectangle::with_size(Size::new(300.0, 50.0));
    let node = input.layout(
        tree,
        &(),
        &layout::Limits::new(Size::ZERO, bounds.size()),
        None,
    );
    let key = keyboard::Key::Named(named);
    let event = Event::Keyboard(keyboard::Event::KeyPressed {
        key: key.clone(),
        modified_key: key,
        physical_key: keyboard::key::Physical::Unidentified(
            keyboard::key::NativeCode::Unidentified,
        ),
        location: keyboard::Location::Standard,
        modifiers,
        text: None,
        repeat: false,
    });
    let mut messages = Vec::new();
    input.update(
        tree,
        &Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)),
        Layout::new(&node),
        mouse::Cursor::Unavailable,
        &(),
        &mut Shell::new(&mut messages),
        &bounds,
    );
    let mut shell = Shell::new(&mut messages);
    input.update(
        tree,
        &event,
        Layout::new(&node),
        mouse::Cursor::Unavailable,
        &(),
        &mut shell,
        &bounds,
    );
    shell.is_event_captured()
}

fn focused(input: &TextInput<'_, (), Theme, ()>, at: usize) -> Tree {
    let mut tree = Tree::new(input as &dyn Widget<(), Theme, ()>);
    let state = tree.state.downcast_mut::<State<()>>();
    state.focus();
    state.cursor.move_to(at);
    tree
}

#[test]
fn unmodified_arrows_leave_only_from_the_corresponding_boundary() {
    let mut input = TextInput::new("", "e\u{301}🎮")
        .on_input(|_| ())
        .navigate_on_boundary(true);
    let mut tree = focused(&input, 0);
    assert!(!arrow(
        &mut input,
        &mut tree,
        keyboard::key::Named::ArrowLeft,
        keyboard::Modifiers::empty()
    ));
    assert!(arrow(
        &mut input,
        &mut tree,
        keyboard::key::Named::ArrowRight,
        keyboard::Modifiers::empty()
    ));
    assert!(arrow(
        &mut input,
        &mut tree,
        keyboard::key::Named::ArrowRight,
        keyboard::Modifiers::empty()
    ));
    assert!(!arrow(
        &mut input,
        &mut tree,
        keyboard::key::Named::ArrowRight,
        keyboard::Modifiers::empty()
    ));
    assert!(arrow(
        &mut input,
        &mut tree,
        keyboard::key::Named::ArrowLeft,
        keyboard::Modifiers::empty()
    ));
}

#[test]
fn the_end_sentinel_and_an_empty_field_are_boundaries() {
    for value in ["", "hello"] {
        let mut input = TextInput::new("", value)
            .on_input(|_| ())
            .navigate_on_boundary(true);
        let mut tree = focused(&input, usize::MAX);
        assert!(!arrow(
            &mut input,
            &mut tree,
            keyboard::key::Named::ArrowRight,
            keyboard::Modifiers::empty()
        ));
        if value.is_empty() {
            assert!(!arrow(
                &mut input,
                &mut tree,
                keyboard::key::Named::ArrowLeft,
                keyboard::Modifiers::empty()
            ));
        }
    }
}

#[test]
fn selection_collapses_before_an_arrow_can_leave_the_field() {
    let mut input = TextInput::new("", "hello")
        .on_input(|_| ())
        .navigate_on_boundary(true);
    let mut tree = focused(&input, 0);
    tree.state
        .downcast_mut::<State<()>>()
        .cursor
        .select_range(0, 5);
    assert!(arrow(
        &mut input,
        &mut tree,
        keyboard::key::Named::ArrowLeft,
        keyboard::Modifiers::empty()
    ));
    assert!(!arrow(
        &mut input,
        &mut tree,
        keyboard::key::Named::ArrowLeft,
        keyboard::Modifiers::empty()
    ));
}

#[test]
fn modified_arrows_remain_editing_shortcuts_at_a_boundary() {
    for modifiers in [
        keyboard::Modifiers::SHIFT,
        keyboard::Modifiers::CTRL,
        keyboard::Modifiers::ALT,
        keyboard::Modifiers::LOGO,
    ] {
        let mut input = TextInput::new("", "hello")
            .on_input(|_| ())
            .navigate_on_boundary(true);
        let mut tree = focused(&input, 0);
        assert!(arrow(
            &mut input,
            &mut tree,
            keyboard::key::Named::ArrowLeft,
            modifiers
        ));
    }
}

#[test]
fn ordinary_text_fields_still_capture_arrows_at_their_boundaries() {
    let mut input = TextInput::new("", "hello").on_input(|_| ());
    let mut tree = focused(&input, 0);
    assert!(arrow(
        &mut input,
        &mut tree,
        keyboard::key::Named::ArrowLeft,
        keyboard::Modifiers::empty()
    ));
    tree.state
        .downcast_mut::<State<()>>()
        .cursor
        .move_to(usize::MAX);
    assert!(arrow(
        &mut input,
        &mut tree,
        keyboard::key::Named::ArrowRight,
        keyboard::Modifiers::empty()
    ));
}

fn input_method(input: &mut TextInput<'_, (), Theme, ()>, tree: &mut Tree) -> InputMethod {
    let bounds = Rectangle::with_size(Size::new(300.0, 50.0));
    let node = input.layout(
        tree,
        &(),
        &layout::Limits::new(Size::ZERO, bounds.size()),
        None,
    );
    let mut messages = Vec::new();
    let mut shell = Shell::new(&mut messages);
    input.update(
        tree,
        &Event::Window(window::Event::RedrawRequested(Instant::now())),
        Layout::new(&node),
        mouse::Cursor::Unavailable,
        &(),
        &mut shell,
        &bounds,
    );
    shell.input_method().clone()
}

fn surrounding_of(
    input: &mut TextInput<'_, (), Theme, ()>,
    tree: &mut Tree,
) -> Option<input_method::Surrounding> {
    match input_method(input, tree) {
        InputMethod::Enabled { surrounding, .. } => surrounding,
        InputMethod::Disabled => panic!("a focused field enables the input method"),
    }
}

fn around(text: &str, cursor: usize, anchor: usize) -> Option<input_method::Surrounding> {
    Some(input_method::Surrounding {
        text: text.to_owned(),
        cursor,
        anchor,
    })
}

#[test]
fn a_focused_field_tells_the_input_method_the_text_around_its_caret() {
    let mut input = TextInput::new("Search", "Find").on_input(|_| ());
    let mut tree = focused(&input, 4);
    assert_eq!(surrounding_of(&mut input, &mut tree), around("Find", 4, 4));

    let mut empty = TextInput::new("Search", "").on_input(|_| ());
    let mut tree = focused(&empty, 0);
    assert_eq!(surrounding_of(&mut empty, &mut tree), around("", 0, 0));
}

#[test]
fn surrounding_offsets_are_bytes_and_the_anchor_is_where_a_selection_began() {
    // "e" + a combining accent is one grapheme of 3 bytes; the gamepad is 4.
    let mut input = TextInput::new("", "e\u{301}🎮 go").on_input(|_| ());
    let mut tree = focused(&input, 2);
    let text = "e\u{301}🎮 go";
    assert_eq!(surrounding_of(&mut input, &mut tree), around(text, 7, 7));

    // Selected leftwards from the end: the caret is at the gamepad, the anchor at the end.
    tree.state
        .downcast_mut::<State<()>>()
        .cursor
        .select_range(5, 1);
    assert_eq!(surrounding_of(&mut input, &mut tree), around(text, 3, 10));
}

#[test]
fn a_secure_field_never_tells_its_text() {
    let mut input = TextInput::new("Password", "hunter2")
        .on_input(|_| ())
        .secure(true);
    let mut tree = focused(&input, 7);
    match input_method(&mut input, &mut tree) {
        InputMethod::Enabled {
            purpose,
            surrounding,
            ..
        } => {
            assert_eq!(purpose, input_method::Purpose::Secure);
            assert_eq!(surrounding, None);
        }
        InputMethod::Disabled => panic!("a focused field enables the input method"),
    }
}
