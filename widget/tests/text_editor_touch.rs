//! A finger works a text editor like the left mouse button: a tap focuses it
//! and places the caret, two and three quick taps select a word and a line,
//! and a drag selects.
//!
//! Skipped with the `wgpu` feature: it builds the renderer without a backend,
//! so the simulator can't make one.
#![cfg(not(feature = "wgpu"))]
use iced_test::Simulator;
use iced_widget::core::{Element, Event, Point, Theme, touch};
use iced_widget::text_editor::{Action, Content, Edit, TextEditor};

type Renderer = iced_renderer::Renderer;

/// The editor's default padding.
const PADDING: f32 = 5.0;

fn editor(content: &Content<Renderer>) -> Simulator<'_, Action, Theme, Renderer> {
    let editor: Element<'_, Action, Theme, Renderer> = TextEditor::new(content)
        .height(100)
        .on_action(|action| action)
        .into();

    Simulator::new(editor)
}

fn finger(id: u64) -> touch::Finger {
    touch::Finger(id)
}

fn press(at: Point) -> Event {
    Event::Touch(touch::Event::FingerPressed {
        id: finger(0),
        position: at,
    })
}

fn moved(to: Point) -> Event {
    Event::Touch(touch::Event::FingerMoved {
        id: finger(0),
        position: to,
    })
}

fn lift(at: Point) -> Event {
    Event::Touch(touch::Event::FingerLifted {
        id: finger(0),
        position: at,
    })
}

fn tap(at: Point) -> [Event; 2] {
    [press(at), lift(at)]
}

/// Where a window position lands inside the editor's text area.
fn inside(at: Point) -> Point {
    Point::new(at.x - PADDING, at.y - PADDING)
}

#[test]
fn a_tap_places_the_caret() {
    let content = Content::new();
    let mut ui = editor(&content);
    let at = Point::new(40.0, 20.0);

    let _ = ui.simulate(tap(at));

    assert_eq!(
        ui.into_messages().collect::<Vec<_>>(),
        [Action::Click(inside(at))]
    );
}

#[test]
fn quick_taps_select_a_word_then_a_line() {
    let content = Content::new();
    let mut ui = editor(&content);
    let at = Point::new(40.0, 20.0);

    let _ = ui.simulate(tap(at));
    let _ = ui.simulate(tap(at));
    let _ = ui.simulate(tap(at));

    assert_eq!(
        ui.into_messages().collect::<Vec<_>>(),
        [
            Action::Click(inside(at)),
            Action::SelectWord,
            Action::SelectLine
        ]
    );
}

#[test]
fn a_drag_selects_until_the_finger_lifts() {
    let content = Content::new();
    let mut ui = editor(&content);
    let from = Point::new(40.0, 20.0);
    let to = Point::new(90.0, 60.0);

    let _ = ui.simulate([press(from), moved(to), lift(to)]);
    // Once lifted, a moving finger no longer selects.
    let _ = ui.simulate([moved(Point::new(120.0, 60.0))]);

    assert_eq!(
        ui.into_messages().collect::<Vec<_>>(),
        [Action::Click(inside(from)), Action::Drag(inside(to))]
    );
}

#[test]
fn a_drag_past_the_edge_stops_at_it() {
    let content = Content::new();
    let mut ui = editor(&content);
    let from = Point::new(40.0, 20.0);

    let _ = ui.simulate([press(from), moved(Point::new(40.0, 400.0))]);

    assert_eq!(
        ui.into_messages().collect::<Vec<_>>(),
        [
            Action::Click(inside(from)),
            Action::Drag(inside(Point::new(40.0, 100.0)))
        ]
    );
}

#[test]
fn a_tap_focuses_and_a_tap_elsewhere_unfocuses() {
    let content = Content::new();
    let mut ui = editor(&content);
    let at = Point::new(40.0, 20.0);

    let _ = ui.simulate(tap(at));
    let _ = ui.typewrite("a");
    let _ = ui.simulate(tap(Point::new(40.0, 300.0)));
    let _ = ui.typewrite("b");

    assert_eq!(
        ui.into_messages().collect::<Vec<_>>(),
        [Action::Click(inside(at)), Action::Edit(Edit::Insert('a'))]
    );
}
