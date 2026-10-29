//! A mouse wheel notch eases a scrollable one line down over a few frames,
//! as a browser does, and further notches add to where it is heading.
//! Touchpads, which report pixels, scroll at once.
//!
//! Skipped with the `wgpu` feature: it builds the renderer without a backend,
//! so the simulator can't make one.
#![cfg(not(feature = "wgpu"))]
use iced_test::Simulator;
use iced_widget::core::animation::SmoothScroll;
use iced_widget::core::time::{Duration, Instant};
use iced_widget::core::{Element, Event, Length, Point, Theme, mouse, window};
use iced_widget::{column, container, scrollable, text};

type Renderer = iced_renderer::Renderer;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Scrolled(f32);

/// 50 rows of 40px in a 200px viewport: 1800px of scroll range.
fn list(smooth: bool) -> Element<'static, Scrolled, Theme, Renderer> {
    let rows = (0..50).map(|i| {
        container(text(format!("row {i}")))
            .height(40)
            .width(Length::Fill)
            .into()
    });

    scrollable(column(rows))
        .height(200)
        .smooth_scroll(smooth)
        .on_scroll(|viewport| Scrolled(viewport.absolute_offset().y))
        .into()
}

fn simulator(smooth: bool) -> Simulator<'static, Scrolled, Theme, Renderer> {
    let mut ui = Simulator::new(list(smooth));
    ui.point_at(Point::new(50.0, 50.0));
    ui
}

fn notch(ui: &mut Simulator<'_, Scrolled, Theme, Renderer>, lines: f32) {
    let _ = ui.simulate([Event::Mouse(mouse::Event::WheelScrolled {
        delta: mouse::ScrollDelta::Lines { x: 0.0, y: -lines },
    })]);
}

fn frame(ui: &mut Simulator<'_, Scrolled, Theme, Renderer>, at: Instant) {
    let _ = ui.simulate([Event::Window(window::Event::RedrawRequested(at))]);
}

fn offsets(ui: Simulator<'_, Scrolled, Theme, Renderer>) -> Vec<f32> {
    ui.into_messages().map(|Scrolled(y)| y).collect()
}

/// Frames at 60 Hz from now on, for this long.
fn frames(ui: &mut Simulator<'_, Scrolled, Theme, Renderer>, from: Instant, count: u32) {
    for i in 1..=count {
        frame(ui, from + Duration::from_millis(16) * i);
    }
}

#[test]
fn the_simulator_lands_a_notch_at_once() {
    let mut ui = simulator(true);
    notch(&mut ui, 1.0);

    assert_eq!(offsets(ui), [SmoothScroll::LINE]);
}

#[test]
fn a_notch_eases_one_line_down() {
    let mut ui = simulator(true);
    SmoothScroll::set_instant(false);

    let start = Instant::now();
    notch(&mut ui, 1.0);
    frames(&mut ui, start, 20);

    let offsets = offsets(ui);

    assert!(offsets.len() >= 5, "eased over frames: {offsets:?}");
    assert!(offsets[0] < 40.0, "the first frame eases in: {offsets:?}");
    assert!(
        offsets.windows(2).all(|pair| pair[0] < pair[1]),
        "{offsets:?}"
    );
    assert_eq!(offsets.last(), Some(&SmoothScroll::LINE));
}

#[test]
fn another_notch_mid_way_carries_on_to_two_lines() {
    let mut ui = simulator(true);
    SmoothScroll::set_instant(false);

    let start = Instant::now();
    notch(&mut ui, 1.0);
    frames(&mut ui, start, 4);
    notch(&mut ui, 1.0);
    frames(&mut ui, start + Duration::from_millis(64), 20);

    let offsets = offsets(ui);

    assert!(
        offsets.windows(2).all(|pair| pair[0] < pair[1]),
        "{offsets:?}"
    );
    assert_eq!(offsets.last(), Some(&(2.0 * SmoothScroll::LINE)));
}

#[test]
fn a_touchpad_scrolls_at_once() {
    let mut ui = simulator(true);
    SmoothScroll::set_instant(false);

    let _ = ui.simulate([Event::Mouse(mouse::Event::WheelScrolled {
        delta: mouse::ScrollDelta::Pixels { x: 0.0, y: -37.0 },
    })]);

    assert_eq!(offsets(ui), [37.0]);
}

#[test]
fn a_touchpad_takes_over_from_an_eased_notch() {
    let mut ui = simulator(true);
    SmoothScroll::set_instant(false);

    let start = Instant::now();
    notch(&mut ui, 1.0);
    frames(&mut ui, start, 3);
    let _ = ui.simulate([Event::Mouse(mouse::Event::WheelScrolled {
        delta: mouse::ScrollDelta::Pixels { x: 0.0, y: -10.0 },
    })]);
    frames(&mut ui, start + Duration::from_millis(48), 20);

    let offsets = offsets(ui);
    let last = *offsets.last().expect("scrolled");

    assert!(
        last < SmoothScroll::LINE,
        "the notch stopped where it was: {offsets:?}"
    );
}

#[test]
fn without_smoothing_a_notch_jumps() {
    let mut ui = simulator(false);
    SmoothScroll::set_instant(false);
    notch(&mut ui, 1.0);

    assert_eq!(offsets(ui), [SmoothScroll::LINE]);
}
