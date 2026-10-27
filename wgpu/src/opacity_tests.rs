//! These draw on a GPU, so they are ignored by default; run them with
//! `cargo test -p iced_wgpu -- --ignored opacity_tests` on a machine with one.
use crate::Renderer;
use crate::core::renderer::{Headless, Quad};
use crate::core::{Color, Font, Pixels, Rectangle, Renderer as _, Size};
use crate::graphics::Viewport;

const WIDTH: u32 = 40;
const HEIGHT: u32 = 20;

fn renderer() -> Renderer {
    let mut renderer = futures::executor::block_on(<Renderer as Headless>::new(
        Font::DEFAULT,
        Pixels(16.0),
        None,
    ))
    .expect("a GPU adapter");
    renderer.reset(Rectangle::with_size(Size::new(WIDTH as f32, HEIGHT as f32)));
    renderer
}

fn quad(renderer: &mut Renderer, bounds: Rectangle, color: Color) {
    renderer.fill_quad(
        Quad {
            bounds,
            ..Quad::default()
        },
        color,
    );
}

fn screen() -> Rectangle {
    Rectangle::with_size(Size::new(WIDTH as f32, HEIGHT as f32))
}

fn page(renderer: &mut Renderer) {
    quad(
        renderer,
        Rectangle::with_size(Size::new(WIDTH as f32, HEIGHT as f32)),
        Color::WHITE,
    );
}

/// A button: a red fill with a blue label on top.
fn button(renderer: &mut Renderer) {
    quad(
        renderer,
        Rectangle::new([0.0, 0.0].into(), Size::new(WIDTH as f32, HEIGHT as f32)),
        Color::from_rgb(1.0, 0.0, 0.0),
    );
    quad(
        renderer,
        Rectangle::new([10.0, 5.0].into(), Size::new(20.0, 10.0)),
        Color::from_rgb(0.0, 0.0, 1.0),
    );
}

fn render(renderer: &mut Renderer) -> Vec<u8> {
    renderer.screenshot(
        &Viewport::with_physical_size(Size::new(WIDTH, HEIGHT), 1.0),
        Color::TRANSPARENT,
    )
}

/// The pixel's colour.
fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 3] {
    let i = ((y * WIDTH + x) * 4) as usize;
    [pixels[i], pixels[i + 1], pixels[i + 2]]
}

/// A channel blended to `amount`, as stored: blending is in linear light,
/// except with `web-colors`.
fn mix(amount: f32) -> u8 {
    let value = if crate::graphics::color::GAMMA_CORRECTION {
        if amount <= 0.003_130_8 {
            amount * 12.92
        } else {
            1.055 * amount.powf(1.0 / 2.4) - 0.055
        }
    } else {
        amount
    };
    (value * 255.0).round() as u8
}

fn close(actual: [u8; 3], expected: [u8; 3]) -> bool {
    actual
        .iter()
        .zip(expected)
        .all(|(a, e)| (i16::from(*a) - i16::from(e)).abs() <= 2)
}

#[test]
#[ignore = "needs a GPU adapter"]
fn a_faded_group_is_composited_once() {
    let mut renderer = renderer();
    page(&mut renderer);
    renderer.with_opacity(
        Rectangle::with_size(Size::new(WIDTH as f32, HEIGHT as f32)),
        0.5,
        button,
    );
    let pixels = render(&mut renderer);

    // The label at half strength over the page, not over the fill.
    let label = pixel(&pixels, 20, 10);
    assert!(close(label, [mix(0.5), mix(0.5), 255]), "label {label:?}");
    let fill = pixel(&pixels, 2, 2);
    assert!(close(fill, [255, mix(0.5), mix(0.5)]), "fill {fill:?}");
}

#[test]
#[ignore = "needs a GPU adapter"]
fn a_faded_group_keeps_its_rounded_clip() {
    let mut renderer = renderer();
    page(&mut renderer);
    renderer.with_layer_rounded(screen(), 8.0.into(), |renderer| {
        renderer.with_opacity(screen(), 0.5, |renderer| {
            quad(renderer, screen(), Color::from_rgb(1.0, 0.0, 0.0));
        });
    });
    let pixels = render(&mut renderer);

    let corner = pixel(&pixels, 0, 0);
    assert!(close(corner, [255, 255, 255]), "corner {corner:?}");
    let middle = pixel(&pixels, 20, 10);
    assert!(
        close(middle, [255, mix(0.5), mix(0.5)]),
        "middle {middle:?}"
    );
}

#[test]
#[ignore = "needs a GPU adapter"]
fn an_opaque_group_draws_as_without_one() {
    let mut grouped = renderer();
    page(&mut grouped);
    grouped.with_opacity(Rectangle::INFINITE, 1.0, button);

    let mut plain = renderer();
    page(&mut plain);
    button(&mut plain);

    assert_eq!(render(&mut grouped), render(&mut plain));
}

#[test]
#[ignore = "needs a GPU adapter"]
fn nested_groups_composite_inner_then_outer() {
    let mut renderer = renderer();
    page(&mut renderer);
    renderer.with_opacity(Rectangle::INFINITE, 0.5, |renderer| {
        quad(
            renderer,
            Rectangle::with_size(Size::new(WIDTH as f32, HEIGHT as f32)),
            Color::from_rgb(1.0, 0.0, 0.0),
        );
        renderer.with_opacity(Rectangle::INFINITE, 0.5, |renderer| {
            quad(
                renderer,
                Rectangle::new([10.0, 5.0].into(), Size::new(20.0, 10.0)),
                Color::from_rgb(0.0, 0.0, 1.0),
            );
        });
    });
    let pixels = render(&mut renderer);

    // Inner: blue at half over red. Outer: that at half over white.
    let label = pixel(&pixels, 20, 10);
    assert!(
        close(label, [mix(0.75), mix(0.5), mix(0.75)]),
        "label {label:?}"
    );
}

#[test]
#[ignore = "needs a GPU adapter"]
fn a_group_holding_a_backdrop_scales_each_primitive() {
    let mut renderer = renderer();
    page(&mut renderer);
    renderer.with_opacity(Rectangle::INFINITE, 0.5, |renderer| {
        renderer.draw_backdrop_blur(
            Rectangle::new([0.0, 0.0].into(), Size::new(4.0, 4.0)),
            2.0,
            [0.0; 4],
            1.0,
            1.0,
        );
        button(renderer);
    });
    let pixels = render(&mut renderer);

    // As before groups: the label at half over the fill at half.
    let label = pixel(&pixels, 20, 10);
    assert!(
        close(label, [mix(0.5), mix(0.25), mix(0.75)]),
        "label {label:?}"
    );
}

#[test]
#[ignore = "needs a GPU adapter"]
fn a_group_after_a_group_starts_from_the_frame() {
    let mut renderer = renderer();
    page(&mut renderer);
    renderer.with_opacity(Rectangle::INFINITE, 0.5, |renderer| {
        quad(
            renderer,
            Rectangle::new([0.0, 0.0].into(), Size::new(10.0, HEIGHT as f32)),
            Color::from_rgb(1.0, 0.0, 0.0),
        );
    });
    renderer.with_opacity(Rectangle::INFINITE, 0.5, |renderer| {
        quad(
            renderer,
            Rectangle::new([30.0, 0.0].into(), Size::new(10.0, HEIGHT as f32)),
            Color::from_rgb(0.0, 0.0, 1.0),
        );
    });
    let pixels = render(&mut renderer);

    assert!(close(pixel(&pixels, 5, 10), [255, mix(0.5), mix(0.5)]));
    assert!(close(pixel(&pixels, 20, 10), [255, 255, 255]));
    assert!(close(pixel(&pixels, 35, 10), [mix(0.5), mix(0.5), 255]));
}

#[test]
#[ignore = "needs a GPU adapter"]
fn a_blur_before_a_scaled_group_keeps_its_strength() {
    let scene = |grouped: bool| {
        let mut renderer = renderer();
        page(&mut renderer);
        quad(
            &mut renderer,
            Rectangle::new([0.0, 0.0].into(), Size::new(8.0, HEIGHT as f32)),
            Color::from_rgb(1.0, 0.0, 0.0),
        );
        renderer.draw_backdrop_blur(
            Rectangle::new([0.0, 0.0].into(), Size::new(16.0, HEIGHT as f32)),
            4.0,
            [0.0; 4],
            1.0,
            1.0,
        );
        if grouped {
            renderer.with_opacity(Rectangle::INFINITE, 0.5, |renderer| {
                renderer.draw_backdrop_blur(
                    Rectangle::new([34.0, 0.0].into(), Size::new(6.0, 8.0)),
                    2.0,
                    [0.0; 4],
                    1.0,
                    1.0,
                );
                quad(
                    renderer,
                    Rectangle::new([34.0, 10.0].into(), Size::new(6.0, 10.0)),
                    Color::from_rgb(0.0, 0.0, 1.0),
                );
            });
        }
        render(&mut renderer)
    };
    let plain = scene(false);
    let grouped = scene(true);

    // The group sits beyond the blur's reach, so only a scaled strength differs.
    for x in 0..16 {
        let (grouped, plain) = (pixel(&grouped, x, 10), pixel(&plain, x, 10));
        assert!(
            close(grouped, plain),
            "x {x}: {grouped:?} against {plain:?}"
        );
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn a_group_inside_a_gradient_fade_is_composited() {
    let mut renderer = renderer();
    page(&mut renderer);
    renderer.start_gradient_fade(screen(), 0, 0.9, 1.0, 0.0);
    renderer.with_opacity(Rectangle::INFINITE, 0.5, button);
    renderer.end_gradient_fade();
    let pixels = render(&mut renderer);

    let label = pixel(&pixels, 20, 10);
    assert!(close(label, [mix(0.5), mix(0.5), 255]), "label {label:?}");
}

#[test]
#[ignore = "needs a GPU adapter"]
fn a_group_holding_a_gradient_fade_is_composited() {
    let mut renderer = renderer();
    page(&mut renderer);
    renderer.with_opacity(Rectangle::INFINITE, 0.5, |renderer| {
        quad(
            renderer,
            Rectangle::with_size(Size::new(WIDTH as f32, HEIGHT as f32)),
            Color::from_rgb(1.0, 0.0, 0.0),
        );
        renderer.start_gradient_fade(screen(), 0, 0.9, 1.0, 0.0);
        renderer.with_layer(screen(), |renderer| {
            quad(
                renderer,
                Rectangle::new([10.0, 5.0].into(), Size::new(20.0, 10.0)),
                Color::from_rgb(0.0, 0.0, 1.0),
            );
        });
        renderer.end_gradient_fade();
    });
    let pixels = render(&mut renderer);

    // The fade lands in the group, which then fades as one.
    let label = pixel(&pixels, 20, 10);
    assert!(close(label, [mix(0.5), mix(0.5), 255]), "label {label:?}");
}

#[test]
#[ignore = "needs a GPU adapter"]
fn a_group_over_a_backdrop_is_composited() {
    let mut renderer = renderer();
    page(&mut renderer);
    renderer.draw_backdrop_blur(screen(), 0.0, [0.0; 4], 1.0, 1.0);
    renderer.start_post_blur_layer(screen());
    renderer.with_opacity(Rectangle::INFINITE, 0.5, button);
    renderer.end_post_blur_layer();
    let pixels = render(&mut renderer);

    // A dialog's faded button: composited, as outside the dialog.
    let label = pixel(&pixels, 20, 10);
    assert!(close(label, [mix(0.5), mix(0.5), 255]), "label {label:?}");
}
