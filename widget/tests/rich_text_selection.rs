//! Rich text marked selectable can be selected with the mouse and copied, with
//! no state in the application: the runtime drives the selection across every
//! selectable text it crosses in the scrollable it started in.
//!
//! They need a renderer that measures text, which this crate's tests don't build by
//! default, so they return early without one. Run them with:
//!
//! ```sh
//! ICED_TEST_BACKEND=tiny-skia cargo test -p iced_widget --test rich_text_selection \
//!     --features iced_renderer/tiny-skia,iced_renderer/wayland,iced_renderer/x11
//! ```
//!
//! For wgpu, set `ICED_TEST_BACKEND=wgpu` and add `iced_renderer/wgpu,iced_renderer/wgpu-bare`.
//!
//! Skipped with the `wgpu` feature: it builds the renderer without a backend,
//! so the simulator can't make one.
#![cfg(not(feature = "wgpu"))]
use iced_test::{Simulator, simulator};
use iced_widget::core::keyboard::{self, Modifiers};
use iced_widget::core::text::Span;
use iced_widget::core::{Element, Event, Point, Theme, mouse};
use iced_widget::{column, container, rich_text, row, scrollable, span};

type Renderer = iced_renderer::Renderer;
type Ui<'a> = Simulator<'a, Message, Theme, Renderer>;

/// Whether the renderer measures text; the null one this crate builds by default doesn't.
fn measures_text() -> bool {
    std::any::TypeId::of::<Renderer>() != std::any::TypeId::of::<()>()
}

/// Each text sits in its own band this tall, from the top of the window.
const BAND: f32 = 40.0;

#[derive(Debug, Clone, PartialEq)]
enum Message {
    Link(&'static str),
}

/// A band of selectable (or not) rich text.
fn band<'a>(
    spans: impl AsRef<[Span<'a, &'static str>]> + 'a,
    selectable: bool,
) -> Element<'a, Message, Theme, Renderer> {
    container(
        rich_text(spans)
            .selectable(selectable)
            .on_link_click(Message::Link),
    )
    .height(BAND)
    .into()
}

/// A point on the first line of the given band.
fn on(band: usize, x: f32) -> Point {
    Point::new(x, band as f32 * BAND + 10.0)
}

fn press(ui: &mut Ui<'_>, at: Point) {
    ui.point_at(at);
    let _ = ui.simulate([
        Event::Mouse(mouse::Event::CursorMoved { position: at }),
        Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
    ]);
}

fn release(ui: &mut Ui<'_>) {
    let _ = ui.simulate([Event::Mouse(mouse::Event::ButtonReleased(
        mouse::Button::Left,
    ))]);
}

fn click(ui: &mut Ui<'_>, at: Point) {
    press(ui, at);
    release(ui);
}

fn drag(ui: &mut Ui<'_>, from: Point, to: Point) {
    press(ui, from);
    ui.point_at(to);
    let _ = ui.simulate([Event::Mouse(mouse::Event::CursorMoved { position: to })]);
    release(ui);
}

fn modifiers(ui: &mut Ui<'_>, modifiers: Modifiers) {
    let _ = ui.simulate([Event::Keyboard(keyboard::Event::ModifiersChanged(
        modifiers,
    ))]);
}

/// The platform's command shortcut for `key`: Ctrl, or Cmd on macOS.
fn shortcut(ui: &mut Ui<'_>, key: &str) {
    let mut press = simulator::press_key(keyboard::Key::Character(key.into()), None);

    if let Event::Keyboard(keyboard::Event::KeyPressed { modifiers, .. }) = &mut press {
        *modifiers = Modifiers::COMMAND;
    }

    let _ = ui.simulate([press]);
}

fn three_bands<'a>(selectable: [bool; 3]) -> Ui<'a> {
    Simulator::new(column![
        band([span("Alpha beta gamma")], selectable[0]),
        band([span("Delta epsilon")], selectable[1]),
        band([span("Zeta eta")], selectable[2]),
    ])
}

#[test]
fn a_drag_selects_across_texts_and_the_shortcut_copies_it() {
    if !measures_text() {
        return;
    }

    let mut ui = three_bands([true; 3]);

    drag(&mut ui, on(0, 1.0), on(1, 1000.0));
    shortcut(&mut ui, "c");

    // The third text lies wholly below the drag, so none of it is taken.
    assert_eq!(ui.clipboard(), Some("Alpha beta gamma\nDelta epsilon"));
}

#[test]
fn a_drag_whose_last_move_and_release_arrive_together_still_selects() {
    if !measures_text() {
        return;
    }

    let mut ui = three_bands([true; 3]);
    let (from, to) = (on(0, 1.0), on(1, 1000.0));

    press(&mut ui, from);
    ui.point_at(to);
    let _ = ui.simulate([
        Event::Mouse(mouse::Event::CursorMoved { position: to }),
        Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
    ]);
    shortcut(&mut ui, "c");

    assert_eq!(ui.clipboard(), Some("Alpha beta gamma\nDelta epsilon"));
}

#[test]
fn a_finished_drag_becomes_the_primary_selection() {
    if !measures_text() {
        return;
    }

    let mut ui = three_bands([true; 3]);

    drag(&mut ui, on(1, 1.0), on(2, 1000.0));

    assert_eq!(ui.primary_selection(), Some("Delta epsilon\nZeta eta"));
    assert_eq!(
        ui.clipboard(),
        None,
        "only the shortcut writes the clipboard"
    );
}

#[test]
fn a_double_click_selects_a_word() {
    if !measures_text() {
        return;
    }

    let mut ui = three_bands([true; 3]);

    click(&mut ui, on(0, 2.0));
    click(&mut ui, on(0, 2.0));
    shortcut(&mut ui, "c");

    assert_eq!(ui.clipboard(), Some("Alpha"));
}

#[test]
fn a_shift_click_extends_from_where_the_selection_started() {
    if !measures_text() {
        return;
    }

    let mut ui = three_bands([true; 3]);

    click(&mut ui, on(0, 1.0));
    modifiers(&mut ui, Modifiers::SHIFT);
    click(&mut ui, on(1, 1000.0));
    modifiers(&mut ui, Modifiers::empty());
    shortcut(&mut ui, "c");

    assert_eq!(ui.clipboard(), Some("Alpha beta gamma\nDelta epsilon"));
}

#[test]
fn a_click_in_empty_space_clears_the_selection() {
    if !measures_text() {
        return;
    }

    let mut ui = three_bands([true; 3]);

    drag(&mut ui, on(0, 1.0), on(1, 1000.0));
    click(&mut ui, on(5, 1.0));
    shortcut(&mut ui, "c");

    assert_eq!(ui.clipboard(), None);
}

#[test]
fn text_not_marked_selectable_is_left_out() {
    if !measures_text() {
        return;
    }

    let mut ui = three_bands([true, false, true]);

    drag(&mut ui, on(0, 1.0), on(2, 1000.0));
    shortcut(&mut ui, "c");

    assert_eq!(ui.clipboard(), Some("Alpha beta gamma\nZeta eta"));
}

#[test]
fn select_all_takes_every_selectable_text_once_selection_is_in_use() {
    if !measures_text() {
        return;
    }

    let mut ui = three_bands([true, true, false]);

    shortcut(&mut ui, "a");
    shortcut(&mut ui, "c");
    assert_eq!(ui.clipboard(), None, "nothing is selected before a click");

    click(&mut ui, on(0, 1.0));
    shortcut(&mut ui, "a");
    shortcut(&mut ui, "c");

    assert_eq!(ui.clipboard(), Some("Alpha beta gamma\nDelta epsilon"));
}

#[test]
fn a_click_on_a_link_follows_it_and_selects_nothing() {
    if !measures_text() {
        return;
    }

    let mut ui = Simulator::new(column![band(
        [span("Open").link("docs"), span(" the guide")],
        true,
    )]);

    click(&mut ui, on(0, 2.0));
    shortcut(&mut ui, "c");

    assert_eq!(ui.clipboard(), None);
    assert_eq!(
        ui.into_messages().collect::<Vec<_>>(),
        [Message::Link("docs")]
    );
}

/// Two scrolling panes side by side: a long text wrapped in a narrow one, and
/// two bands in a wide one from x = [`PANE`].
fn panes<'a>() -> Ui<'a> {
    Simulator::new(row![
        scrollable(
            rich_text([span("Alpha beta gamma delta epsilon zeta")])
                .selectable(true)
                .on_link_click(Message::Link),
        )
        .width(PANE),
        scrollable(column![
            band([span("Eta theta")], true),
            band([span("Iota")], true),
        ])
        .width(3.0 * PANE),
    ])
}

/// The width of the narrow pane of [`panes`].
const PANE: f32 = 100.0;

#[test]
fn a_drag_in_one_scrollable_leaves_the_text_beside_it_alone() {
    if !measures_text() {
        return;
    }

    let mut ui = panes();

    drag(&mut ui, on(0, PANE + 1.0), on(1, 4.0 * PANE - 1.0));
    shortcut(&mut ui, "c");

    assert_eq!(ui.clipboard(), Some("Eta theta\nIota"));
}

#[test]
fn select_all_stays_in_the_scrollable_the_selection_started_in() {
    if !measures_text() {
        return;
    }

    let mut ui = panes();

    click(&mut ui, on(0, PANE + 1.0));
    shortcut(&mut ui, "a");
    shortcut(&mut ui, "c");

    assert_eq!(ui.clipboard(), Some("Eta theta\nIota"));
}

#[test]
fn a_drag_that_starts_outside_every_scrollable_reaches_into_them() {
    if !measures_text() {
        return;
    }

    let mut ui = Simulator::new(column![
        band([span("Alpha beta gamma")], true),
        scrollable(band([span("Delta epsilon")], true)).height(BAND),
    ]);

    drag(&mut ui, on(0, 1.0), on(1, 1000.0));
    shortcut(&mut ui, "c");

    assert_eq!(ui.clipboard(), Some("Alpha beta gamma\nDelta epsilon"));
}

#[test]
fn text_after_a_scrolled_scrollable_selects_where_it_is_drawn() {
    if !measures_text() {
        return;
    }

    let mut ui = Simulator::new(column![
        scrollable(column![
            band([span("Alpha beta gamma")], true),
            band([span("Delta epsilon")], true),
        ])
        .height(BAND),
        band([span("Omega")], true),
    ]);

    ui.point_at(on(0, 1.0));
    let _ = ui.simulate([
        Event::Mouse(mouse::Event::CursorMoved {
            position: on(0, 1.0),
        }),
        Event::Mouse(mouse::Event::WheelScrolled {
            delta: mouse::ScrollDelta::Pixels { x: 0.0, y: -BAND },
        }),
    ]);
    drag(&mut ui, on(1, 1.0), on(1, 1000.0));
    shortcut(&mut ui, "c");

    assert_eq!(ui.clipboard(), Some("Omega"));
}
