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
