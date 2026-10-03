//! Dropdown menus and tooltips are overlays. Widget operations must reach what
//! they show, so a selector (or a test driver reading the tree) can find it.
//! A closed pick list reports what it shows, so it can be found to open.
//!
//! Skipped with the `wgpu` feature: it builds the renderer without a backend,
//! so the simulator can't make one.
#![cfg(not(feature = "wgpu"))]
use iced_test::{Error, Simulator, simulator};
use iced_widget::core::widget::operation::Focusable;
use iced_widget::core::widget::{Id, Operation};
use iced_widget::core::{Element, Event, Rectangle, Theme, mouse};
use iced_widget::{button, column, pick_list, text, text_input, tooltip};

type Renderer = iced_renderer::Renderer;

/// Points at the text and moves the mouse there, as a person would before
/// clicking: menus pick the option under the pointer when it moves.
fn hover(ui: &mut Simulator<'_, Message, Theme, Renderer>, label: &str) -> Result<(), Error> {
    let found = ui.find(label)?;
    // The simulator measures text without the system's fonts, so a short
    // label can come out empty; its place is still where it is drawn.
    let position = found
        .visible_bounds()
        .unwrap_or_else(|| found.bounds())
        .center();

    ui.point_at(position);
    let _ = ui.simulate([Event::Mouse(mouse::Event::CursorMoved { position })]);

    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
enum Message {
    Picked(String),
    Hovered,
}

#[test]
fn a_dropdowns_options_can_be_found_and_picked_by_their_text() -> Result<(), Error> {
    let options = vec!["Apple".to_owned(), "Orange".to_owned(), "Pear".to_owned()];
    let view: Element<'_, Message, Theme, Renderer> = column![
        pick_list(None::<&String>, options.as_slice(), String::clone)
            .on_select(Message::Picked)
            .placeholder("Fruit"),
    ]
    .into();

    let mut ui = simulator(view);

    assert!(
        ui.find("Orange").is_err(),
        "a closed dropdown shows nothing"
    );

    let _ = ui.click("Fruit")?;
    let _ = ui.find("Orange")?;

    hover(&mut ui, "Pear")?;
    let _ = ui.click("Pear")?;

    assert_eq!(
        ui.into_messages().collect::<Vec<_>>(),
        [Message::Picked("Pear".to_owned())]
    );

    Ok(())
}

#[test]
fn an_open_tooltip_can_be_read() -> Result<(), Error> {
    let view: Element<'_, Message, Theme, Renderer> = column![tooltip(
        button(text("Add")).on_press(Message::Hovered),
        text("Add a channel"),
        tooltip::Position::Bottom,
    )]
    .into();

    let mut ui = simulator(view);

    assert!(ui.find("Add a channel").is_err(), "nothing until hovered");

    hover(&mut ui, "Add")?;

    let _ = ui.find("Add a channel")?;

    Ok(())
}

#[test]
fn the_selected_option_is_picked_from_the_menu_not_the_closed_list() -> Result<(), Error> {
    let options = vec!["Apple".to_owned(), "Pear".to_owned()];
    let selected = "Pear".to_owned();
    let view: Element<'_, Message, Theme, Renderer> = column![
        pick_list(Some(&selected), options.as_slice(), String::clone).on_select(Message::Picked),
    ]
    .into();

    let mut ui = simulator(view);

    let _ = ui.click("Pear")?;
    hover(&mut ui, "Pear")?;
    let _ = ui.click("Pear")?;

    assert_eq!(
        ui.into_messages().collect::<Vec<_>>(),
        [Message::Picked("Pear".to_owned())]
    );

    Ok(())
}

#[test]
fn nothing_in_an_open_tooltip_joins_focus() -> Result<(), Error> {
    struct Focusables(usize);

    impl Operation for Focusables {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
            operate(self);
        }

        fn focusable(&mut self, _id: Option<&Id>, _bounds: Rectangle, _state: &mut dyn Focusable) {
            self.0 += 1;
        }
    }

    let view: Element<'_, Message, Theme, Renderer> = column![tooltip(
        button(text("Add")).on_press(Message::Hovered),
        column![text("Add a channel"), text_input("Name", "")],
        tooltip::Position::Bottom,
    )]
    .into();

    let mut ui = simulator(view);
    let mut before = Focusables(0);
    ui.operate(&mut before);

    hover(&mut ui, "Add")?;
    let _ = ui.find("Add a channel")?;

    let mut after = Focusables(0);
    ui.operate(&mut after);

    assert_eq!(before.0, after.0);

    Ok(())
}

#[cfg(feature = "lazy")]
#[test]
fn a_dropdown_inside_lazy_can_still_be_picked_by_text() -> Result<(), Error> {
    let options = vec!["Apple".to_owned(), "Pear".to_owned()];
    let view: Element<'_, Message, Theme, Renderer> = column![iced_widget::lazy(0, move |_| {
        pick_list(None::<&String>, options.clone(), String::clone)
            .on_select(Message::Picked)
            .placeholder("Fruit")
    })]
    .into();

    let mut ui = simulator(view);

    let _ = ui.click("Fruit")?;
    hover(&mut ui, "Pear")?;
    let _ = ui.click("Pear")?;

    assert_eq!(
        ui.into_messages().collect::<Vec<_>>(),
        [Message::Picked("Pear".to_owned())]
    );

    Ok(())
}
