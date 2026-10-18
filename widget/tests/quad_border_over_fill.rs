//! A quad's border is painted over its background, as CSS and tiny-skia paint it:
//! one quad with both must look like its background with a border-only quad on top.
//! Run them like `baseline_alignment`:
//!
//! ```sh
//! ICED_TEST_BACKEND=tiny-skia cargo test -p iced_widget --test quad_border_over_fill \
//!     --features iced_renderer/tiny-skia,iced_renderer/wayland,iced_renderer/x11
//! ```
//!
//! For wgpu, set `ICED_TEST_BACKEND=wgpu` and add `iced_renderer/wgpu,iced_renderer/wgpu-bare`.
#![cfg(not(feature = "wgpu"))]
use iced_test::Simulator;
use iced_widget::core::gradient::{Gradient, Linear};
use iced_widget::core::{Background, Border, Color, Element, Settings, Theme};
use iced_widget::{Space, container, stack};

type Renderer = iced_renderer::Renderer;

const SIDE: f32 = 40.0;
const PAD: f32 = 10.0;
const BORDER: f32 = 4.0;

/// Whether the renderer draws; the null one this crate builds by default doesn't.
fn draws() -> bool {
    std::any::TypeId::of::<Renderer>() != std::any::TypeId::of::<()>()
}

fn quad<'a>(
    background: Option<Background>,
    border: Option<Color>,
) -> Element<'a, (), Theme, Renderer> {
    container(Space::new().width(SIDE).height(SIDE))
        .style(move |_| container::Style {
            background,
            border: border.map_or_else(Border::default, |color| Border {
                color,
                width: BORDER,
                ..Border::default()
            }),
            ..container::Style::default()
        })
        .into()
}

/// The device pixels at the middle of the border's band and inside it.
fn border_and_inside(element: Element<'_, (), Theme, Renderer>) -> [[u8; 4]; 2] {
    let framed: Element<'_, (), Theme, Renderer> = container(element).padding(PAD).into();
    let canvas = SIDE + 2.0 * PAD;
    let mut ui =
        Simulator::with_size(Settings::default(), (canvas, canvas), framed).scale_factor(1.0);
    let snapshot = ui.snapshot(&Theme::Light).expect("snapshot");
    let shot = snapshot.screenshot();
    let width = shot.size.width as usize;
    let pixel = |x: f32, y: f32| {
        let at = (y as usize * width + x as usize) * 4;
        [
            shot.rgba[at],
            shot.rgba[at + 1],
            shot.rgba[at + 2],
            shot.rgba[at + 3],
        ]
    };
    let middle = PAD + SIDE / 2.0;
    [pixel(PAD + BORDER / 2.0, middle), pixel(middle, middle)]
}

fn assert_layered(background: impl Into<Background>, border: Color) {
    let background = background.into();
    let single = border_and_inside(quad(Some(background), Some(border)));
    let layered =
        border_and_inside(stack![quad(Some(background), None), quad(None, Some(border))].into());
    for (name, (single, layered)) in ["border", "inside"]
        .into_iter()
        .zip(single.iter().zip(&layered))
    {
        let off = single
            .iter()
            .zip(layered)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap_or(0);
        assert!(
            off <= 1,
            "{name}: one quad drew {single:?}, the border over its background {layered:?} \
             ({border:?} over {background:?})"
        );
    }
}

#[test]
fn a_translucent_border_is_painted_over_an_opaque_background() {
    if !draws() {
        return;
    }
    assert_layered(Color::BLACK, Color::from_rgba(1.0, 1.0, 1.0, 0.25));
}

#[test]
fn a_translucent_border_is_painted_over_a_translucent_background() {
    if !draws() {
        return;
    }
    assert_layered(
        Color::from_rgba(0.0, 0.0, 0.0, 0.5),
        Color::from_rgba(1.0, 0.0, 0.0, 0.5),
    );
}

#[test]
fn a_translucent_border_is_painted_over_a_gradient() {
    if !draws() {
        return;
    }
    let black = Linear::new(0.0)
        .add_stop(0.0, Color::BLACK)
        .add_stop(1.0, Color::BLACK);
    assert_layered(
        Gradient::Linear(black),
        Color::from_rgba(1.0, 1.0, 1.0, 0.25),
    );
}

#[test]
fn an_opaque_border_covers_its_background() {
    if !draws() {
        return;
    }
    assert_layered(
        Color::from_rgb(0.2, 0.6, 0.4),
        Color::from_rgb(0.0, 0.5, 1.0),
    );
}
