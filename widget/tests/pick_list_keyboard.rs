//! A focused pick list works from the keyboard, as a browser's select does:
//! Enter or Space opens it on the selected option, the arrows, Home and End
//! move the highlight, Enter or Space picks it and Escape closes the menu.
//! Its focus comes from a parent that owns it, such as a focus ring, through
//! `button::style_focus`.
//!
//! Skipped with the `wgpu` feature: it builds the renderer without a backend,
//! so the simulator can't make one.
#![cfg(not(feature = "wgpu"))]
use iced_test::{Error, Simulator, simulator};
use iced_widget::core::keyboard::{self, key::Named};
use iced_widget::core::{Element, Event, Length, Theme, event};
use iced_widget::{button, column, pick_list};

type Renderer = iced_renderer::Renderer;
type Ui<'a> = Simulator<'a, Message, Theme, Renderer>;

#[derive(Debug, Clone, PartialEq)]
enum Message {
    Picked(String),
    Opened,
    Closed,
}

fn fruits() -> Vec<String> {
    ["Apple", "Orange", "Pear"].map(str::to_owned).to_vec()
}

fn view<'a>(
    options: &'a [String],
    selected: Option<&'a String>,
    menu_height: Length,
) -> Element<'a, Message, Theme, Renderer> {
    column![
        pick_list(selected, options, String::clone)
            .on_select(Message::Picked)
            .on_open(Message::Opened)
            .on_close(Message::Closed)
            .placeholder("Fruit")
            .menu_height(menu_height),
    ]
    .into()
}

/// What a focus ring around the pick list does when it takes the focus.
fn focus(ui: &mut Ui<'_>) {
    ui.operate(&mut button::style_focus::<()>(true));
}

fn tap(ui: &mut Ui<'_>, key: Named) -> event::Status {
    ui.tap_key(keyboard::Key::Named(key))
}

/// Whether the option's text is in the menu and shows in its scrolled list.
fn shows(ui: &mut Ui<'_>, label: &str) -> bool {
    ui.find(label)
        .ok()
        .and_then(|found| found.visible_bounds())
        .is_some()
}

#[test]
fn enter_opens_down_moves_and_enter_picks() -> Result<(), Error> {
    let options = fruits();
    let mut ui = simulator(view(&options, Some(&options[0]), Length::Shrink));

    focus(&mut ui);
    assert_eq!(tap(&mut ui, Named::Enter), event::Status::Captured);
    let _ = ui.find("Orange")?;

    assert_eq!(tap(&mut ui, Named::ArrowDown), event::Status::Captured);
    assert_eq!(tap(&mut ui, Named::Enter), event::Status::Captured);

    assert!(ui.find("Orange").is_err(), "picking closes the menu");
    assert_eq!(
        ui.into_messages().collect::<Vec<_>>(),
        [Message::Opened, Message::Picked("Orange".to_owned())]
    );

    Ok(())
}

#[test]
fn space_opens_and_escape_closes_without_picking() -> Result<(), Error> {
    let options = fruits();
    let mut ui = simulator(view(&options, Some(&options[0]), Length::Shrink));

    focus(&mut ui);
    let _ = tap(&mut ui, Named::Space);
    let _ = ui.find("Orange")?;

    let _ = tap(&mut ui, Named::ArrowDown);
    assert_eq!(tap(&mut ui, Named::Escape), event::Status::Captured);

    assert!(ui.find("Orange").is_err(), "Escape closes the menu");
    assert_eq!(
        ui.into_messages().collect::<Vec<_>>(),
        [Message::Opened, Message::Closed]
    );

    Ok(())
}

#[test]
fn the_highlight_stops_at_either_end() {
    let options = fruits();
    let mut ui = simulator(view(&options, Some(&options[1]), Length::Shrink));

    focus(&mut ui);
    let _ = tap(&mut ui, Named::Enter);
    let _ = tap(&mut ui, Named::ArrowUp);
    let _ = tap(&mut ui, Named::ArrowUp);
    let _ = tap(&mut ui, Named::Enter);

    // Nothing rebuilds the view, so it reopens on Orange again.
    let _ = tap(&mut ui, Named::Enter);
    let _ = tap(&mut ui, Named::ArrowDown);
    let _ = tap(&mut ui, Named::ArrowDown);
    let _ = tap(&mut ui, Named::Space);

    let _ = tap(&mut ui, Named::Enter);
    let _ = tap(&mut ui, Named::End);
    let _ = tap(&mut ui, Named::Home);
    let _ = tap(&mut ui, Named::Enter);

    assert_eq!(
        ui.into_messages().collect::<Vec<_>>(),
        [
            Message::Opened,
            Message::Picked("Apple".to_owned()),
            Message::Opened,
            Message::Picked("Pear".to_owned()),
            Message::Opened,
            Message::Picked("Apple".to_owned()),
        ]
    );
}

#[test]
fn without_focus_the_keys_go_on_to_focus_navigation() {
    let options = fruits();
    let mut ui = simulator(view(&options, Some(&options[0]), Length::Shrink));

    assert_eq!(tap(&mut ui, Named::Enter), event::Status::Ignored);
    assert!(ui.find("Orange").is_err(), "the menu stays closed");

    focus(&mut ui);
    // Closed, the arrows still move the focus.
    assert_eq!(tap(&mut ui, Named::ArrowDown), event::Status::Ignored);

    ui.operate(&mut button::style_focus::<()>(false));
    assert_eq!(tap(&mut ui, Named::Space), event::Status::Ignored);

    assert_eq!(ui.into_messages().count(), 0);
}

#[test]
fn tab_closes_the_menu_and_still_moves_the_focus() -> Result<(), Error> {
    let options = fruits();
    let mut ui = simulator(view(&options, Some(&options[0]), Length::Shrink));

    focus(&mut ui);
    let _ = tap(&mut ui, Named::Enter);
    let _ = ui.find("Orange")?;

    assert_eq!(tap(&mut ui, Named::Tab), event::Status::Ignored);
    assert!(ui.find("Orange").is_err(), "Tab closes the menu");
    assert_eq!(
        ui.into_messages().collect::<Vec<_>>(),
        [Message::Opened, Message::Closed]
    );

    Ok(())
}

#[test]
fn a_held_enter_opens_once_and_picks_nothing() -> Result<(), Error> {
    let options = fruits();
    let mut ui = simulator(view(&options, Some(&options[0]), Length::Shrink));

    focus(&mut ui);
    let press = |repeat| {
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(Named::Enter),
            modified_key: keyboard::Key::Named(Named::Enter),
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::default(),
            repeat,
            text: None,
        })
    };

    let _ = ui.simulate([press(false)]);
    let _ = ui.simulate([press(true)]);
    let _ = ui.simulate([press(true)]);

    let _ = ui.find("Orange")?;
    assert_eq!(ui.into_messages().collect::<Vec<_>>(), [Message::Opened]);

    Ok(())
}

#[test]
fn the_highlight_is_scrolled_into_view() {
    let options: Vec<String> = (0..30).map(|i| format!("Option {i}")).collect();
    let mut ui = simulator(view(&options, Some(&options[0]), Length::Fixed(100.0)));

    focus(&mut ui);
    let _ = tap(&mut ui, Named::Enter);
    assert!(shows(&mut ui, "Option 0"));
    assert!(!shows(&mut ui, "Option 29"), "the menu holds a few rows");

    let _ = tap(&mut ui, Named::End);
    assert!(shows(&mut ui, "Option 29"));
    assert!(!shows(&mut ui, "Option 0"));

    let _ = tap(&mut ui, Named::PageUp);
    let _ = tap(&mut ui, Named::Home);
    assert!(shows(&mut ui, "Option 0"));
}

#[test]
fn the_menu_opens_scrolled_to_the_selected_option() {
    let options: Vec<String> = (0..30).map(|i| format!("Option {i}")).collect();
    let mut ui = simulator(view(&options, Some(&options[25]), Length::Fixed(100.0)));

    focus(&mut ui);
    let _ = tap(&mut ui, Named::Enter);

    assert!(shows(&mut ui, "Option 25"));
    assert!(!shows(&mut ui, "Option 0"));
}
