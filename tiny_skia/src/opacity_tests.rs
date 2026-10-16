use crate::Renderer;
use crate::core::renderer::{BackdropFilter, Quad};
use crate::core::{Color, Font, Pixels, Rectangle, Renderer as _, Size};
use crate::graphics::Viewport;

const WIDTH: u32 = 40;
const HEIGHT: u32 = 20;

fn renderer() -> Renderer {
    let mut renderer = Renderer::new(Font::DEFAULT, Pixels(16.0));
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

fn render(renderer: &mut Renderer) -> tiny_skia::Pixmap {
    let mut pixels = tiny_skia::Pixmap::new(WIDTH, HEIGHT).unwrap();
    let viewport = Viewport::with_physical_size(Size::new(WIDTH, HEIGHT), 1.0);
    let mut mask = tiny_skia::Mask::new(WIDTH, HEIGHT).unwrap();
    renderer.draw(
        &mut pixels.as_mut(),
        &mut mask,
        &viewport,
        &[Rectangle::with_size(Size::new(WIDTH as f32, HEIGHT as f32))],
        Color::TRANSPARENT,
    );
    pixels
}

/// The pixel's colour; the renderer stores BGRA, as the window buffer does.
fn pixel(pixels: &tiny_skia::Pixmap, x: u32, y: u32) -> [u8; 3] {
    let color = pixels.pixel(x, y).unwrap().demultiply();
    [color.blue(), color.green(), color.red()]
}

fn close(actual: [u8; 3], expected: [u8; 3]) -> bool {
    actual
        .iter()
        .zip(expected)
        .all(|(a, e)| (i16::from(*a) - i16::from(e)).abs() <= 1)
}

#[test]
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
    assert!(close(label, [128, 128, 255]), "label {label:?}");
    let fill = pixel(&pixels, 2, 2);
    assert!(close(fill, [255, 128, 128]), "fill {fill:?}");
}

#[test]
fn an_opaque_group_draws_as_without_one() {
    let mut grouped = renderer();
    page(&mut grouped);
    grouped.with_opacity(Rectangle::INFINITE, 1.0, button);

    let mut plain = renderer();
    page(&mut plain);
    button(&mut plain);

    assert_eq!(render(&mut grouped).data(), render(&mut plain).data());
}

#[test]
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
    assert!(close(label, [191, 128, 191]), "label {label:?}");
}

#[test]
fn a_group_holding_a_backdrop_scales_each_primitive() {
    let mut renderer = renderer();
    page(&mut renderer);
    renderer.with_opacity(Rectangle::INFINITE, 0.5, |renderer| {
        renderer.draw_backdrop_filter(BackdropFilter {
            bounds: Rectangle::new([0.0, 0.0].into(), Size::new(4.0, 4.0)),
            radius: 2.0,
            border_radius: [0.0; 4],
            fade_direction: 0,
            fade_start: 1.0,
            fade_end: 1.0,
            saturation: 1.0,
            software: true,
        });
        button(renderer);
    });
    let pixels = render(&mut renderer);

    // As before groups: the label at half over the fill at half.
    let label = pixel(&pixels, 20, 10);
    assert!(close(label, [128, 64, 191]), "label {label:?}");
}

#[test]
fn a_changing_fade_is_damage() {
    let record = |opacity: f32| {
        let mut renderer = renderer();
        page(&mut renderer);
        renderer.with_opacity(Rectangle::INFINITE, opacity, button);
        renderer.layers().to_vec()
    };
    let before = record(0.5);
    let after = record(0.25);

    let damage = crate::graphics::damage::diff(
        &before,
        &after,
        |layer| vec![layer.bounds],
        crate::Layer::damage,
    );
    assert!(!damage.is_empty());
}

#[test]
fn a_group_inside_a_gradient_fade_is_composited() {
    let mut renderer = renderer();
    page(&mut renderer);
    renderer.start_gradient_fade(Rectangle::INFINITE, 0, 0.9, 1.0, 0.0);
    renderer.with_opacity(Rectangle::INFINITE, 0.5, button);
    renderer.end_gradient_fade();
    let pixels = render(&mut renderer);

    let label = pixel(&pixels, 20, 10);
    assert!(close(label, [128, 128, 255]), "label {label:?}");
}

#[test]
fn a_group_over_a_backdrop_is_composited() {
    let mut renderer = renderer();
    page(&mut renderer);
    renderer.draw_backdrop_filter(BackdropFilter {
        bounds: Rectangle::new([0.0, 0.0].into(), Size::new(4.0, 4.0)),
        radius: 2.0,
        border_radius: [0.0; 4],
        fade_direction: 0,
        fade_start: 1.0,
        fade_end: 1.0,
        saturation: 1.0,
        software: true,
    });
    renderer.start_post_blur_layer(Rectangle::INFINITE);
    renderer.with_opacity(Rectangle::INFINITE, 0.5, button);
    renderer.end_post_blur_layer();
    let pixels = render(&mut renderer);

    // A dialog's faded button: composited, as outside the dialog.
    let label = pixel(&pixels, 20, 10);
    assert!(close(label, [128, 128, 255]), "label {label:?}");
}
