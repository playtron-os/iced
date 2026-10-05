use super::*;
use crate::Renderer;
use crate::core::{Color, Font, Pixels, Renderer as _, Size, Transformation, renderer::Quad};
use crate::graphics::Viewport;

fn rect(x: f32, y: f32, width: f32, height: f32) -> Rectangle {
    Rectangle {
        x,
        y,
        width,
        height,
    }
}

fn renderer(width: f32, height: f32) -> Renderer {
    let mut renderer = Renderer::new(Font::DEFAULT, Pixels(16.0));
    renderer.reset(rect(0.0, 0.0, width, height));
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

fn stripes(renderer: &mut Renderer, width: u32, height: u32, phase: u32) {
    for x in (0..width).step_by(2) {
        let color = if (x / 2 + phase).is_multiple_of(2) {
            Color::BLACK
        } else {
            Color::WHITE
        };
        quad(renderer, rect(x as f32, 0.0, 2.0, height as f32), color);
    }
}

fn filter(bounds: Rectangle) -> BackdropFilter {
    BackdropFilter {
        bounds,
        radius: 6.0,
        border_radius: [0.0; 4],
        fade_direction: 0,
        fade_start: 1.0,
        fade_end: 1.0,
        saturation: 1.0,
        software: true,
    }
}

fn frame(
    renderer: &mut Renderer,
    pixels: &mut tiny_skia::Pixmap,
    scale: f32,
    damage: &[Rectangle],
) {
    let viewport = Viewport::with_physical_size(Size::new(pixels.width(), pixels.height()), scale);
    let mut mask = tiny_skia::Mask::new(pixels.width(), pixels.height()).unwrap();
    renderer.draw(
        &mut pixels.as_mut(),
        &mut mask,
        &viewport,
        damage,
        Color::TRANSPARENT,
    );
}

fn render(renderer: &mut Renderer, width: u32, height: u32, scale: f32) -> tiny_skia::Pixmap {
    let mut pixels = tiny_skia::Pixmap::new(width, height).unwrap();
    frame(
        renderer,
        &mut pixels,
        scale,
        &[rect(0.0, 0.0, width as f32 / scale, height as f32 / scale)],
    );
    pixels
}

fn rgba(pixels: &tiny_skia::Pixmap, x: u32, y: u32) -> [u8; 4] {
    let color = pixels.pixel(x, y).unwrap();
    [color.blue(), color.green(), color.red(), color.alpha()]
}

#[test]
fn disabled_software_filter_is_pixel_exact_and_reset_discards_pending_state() {
    let bounds = rect(8.0, 4.0, 48.0, 24.0);
    let mut plain = renderer(64.0, 32.0);
    stripes(&mut plain, 64, 32, 0);
    quad(
        &mut plain,
        rect(20.0, 8.0, 12.0, 12.0),
        Color::from_rgb(1.0, 0.0, 0.0),
    );
    let expected = render(&mut plain, 64, 32, 1.0);
    let mut tested = renderer(64.0, 32.0);
    tested.draw_backdrop_filter(filter(bounds));
    tested.start_post_blur_layer(bounds);
    tested.reset(rect(0.0, 0.0, 64.0, 32.0));
    stripes(&mut tested, 64, 32, 0);
    tested.draw_backdrop_filter(BackdropFilter {
        software: false,
        ..filter(bounds)
    });
    tested.start_post_blur_layer(bounds);
    quad(
        &mut tested,
        rect(20.0, 8.0, 12.0, 12.0),
        Color::from_rgb(1.0, 0.0, 0.0),
    );
    tested.end_post_blur_layer();
    assert_eq!(render(&mut tested, 64, 32, 1.0).data(), expected.data());
    assert_eq!(
        tested.layers().len(),
        1,
        "default batching remains unchanged"
    );
}

#[test]
fn software_blur_is_visible_but_its_foreground_and_restored_parent_stay_sharp() {
    let mut tested = renderer(64.0, 48.0);
    stripes(&mut tested, 64, 48, 0);
    tested.start_layer_rounded(rect(4.0, 4.0, 48.0, 40.0), 10.0.into());
    let bounds = rect(0.0, 0.0, 64.0, 48.0);
    tested.draw_backdrop_filter(filter(bounds));
    tested.start_post_blur_layer(bounds);
    quad(
        &mut tested,
        rect(24.0, 20.0, 12.0, 12.0),
        Color::from_rgb(0.0, 1.0, 0.0),
    );
    tested.end_post_blur_layer();
    tested.end_layer();
    quad(
        &mut tested,
        rect(0.0, 8.0, 32.0, 8.0),
        Color::from_rgb(1.0, 0.0, 0.0),
    );
    let pixels = render(&mut tested, 64, 48, 1.0);
    assert!(
        (60..195).contains(&rgba(&pixels, 42, 24)[0]),
        "backdrop stripes are blurred"
    );
    assert_eq!(rgba(&pixels, 28, 24), [0, 255, 0, 255]);
    assert_eq!(
        rgba(&pixels, 20, 10),
        [255, 0, 0, 255],
        "later parent content is never sampled by the filter"
    );
    assert_eq!(
        rgba(&pixels, 2, 10),
        [255, 0, 0, 255],
        "the enclosing clip was restored"
    );
    assert_eq!(
        rgba(&pixels, 4, 4),
        [0, 0, 0, 255],
        "rounded caller clip still applies"
    );
}

#[test]
fn the_filter_keeps_its_own_rounded_corners() {
    let mut tested = renderer(64.0, 48.0);
    stripes(&mut tested, 64, 48, 0);
    tested.draw_backdrop_filter(BackdropFilter {
        border_radius: [8.0; 4],
        ..filter(rect(16.0, 12.0, 32.0, 24.0))
    });
    let pixels = render(&mut tested, 64, 48, 1.0);
    assert_eq!(rgba(&pixels, 16, 12), [0, 0, 0, 255]);
    assert!((60..195).contains(&rgba(&pixels, 28, 20)[0]));
    assert_eq!(rgba(&pixels, 14, 20), [255, 255, 255, 255]);
}

#[test]
fn transforms_scale_the_filter_radius_and_corners_before_device_scaling() {
    let mut transformed = renderer(50.0, 40.0);
    stripes(&mut transformed, 50, 40, 0);
    transformed
        .start_transformation(Transformation::translate(5.0, 3.0) * Transformation::scale(2.0));
    transformed.draw_backdrop_filter(BackdropFilter {
        radius: 3.0,
        border_radius: [2.0; 4],
        ..filter(rect(2.0, 2.0, 14.0, 12.0))
    });
    transformed.end_transformation();
    let mut explicit = renderer(50.0, 40.0);
    stripes(&mut explicit, 50, 40, 0);
    explicit.draw_backdrop_filter(BackdropFilter {
        radius: 6.0,
        border_radius: [4.0; 4],
        ..filter(rect(9.0, 7.0, 28.0, 24.0))
    });
    assert_eq!(
        render(&mut transformed, 100, 80, 2.0).data(),
        render(&mut explicit, 100, 80, 2.0).data()
    );
}

#[test]
fn saturation_respects_bgr_storage_premultiplication_and_filter_opacity() {
    let bounds = rect(0.0, 0.0, 16.0, 16.0);
    for (alpha, opacity, expected) in [
        (1.0, 1.0, [54, 54, 54, 255]),
        (0.5, 1.0, [27, 27, 27, 128]),
        (0.5, 0.5, [78, 14, 14, 128]),
        (0.5, 0.0, [128, 0, 0, 128]),
    ] {
        let mut tested = renderer(16.0, 16.0);
        quad(&mut tested, bounds, Color::from_rgba(1.0, 0.0, 0.0, alpha));
        tested.start_opacity(bounds, opacity);
        tested.draw_backdrop_filter(BackdropFilter {
            radius: 0.0,
            saturation: 0.0,
            ..filter(bounds)
        });
        tested.end_opacity();
        assert_eq!(rgba(&render(&mut tested, 16, 16, 1.0), 8, 8), expected);
    }
}

#[test]
fn all_fade_directions_interpolate_with_the_original_backdrop() {
    for direction in 0..6 {
        let mut tested = renderer(16.0, 16.0);
        let bounds = rect(0.0, 0.0, 16.0, 16.0);
        quad(&mut tested, bounds, Color::from_rgb(1.0, 0.0, 0.0));
        tested.draw_backdrop_filter(BackdropFilter {
            radius: 0.0,
            saturation: 0.0,
            fade_direction: direction,
            fade_start: 0.0,
            fade_end: 0.5,
            ..filter(bounds)
        });
        let pixels = render(&mut tested, 16, 16, 1.0);
        let (top, bottom, left, right, middle) = (
            rgba(&pixels, 8, 1)[0],
            rgba(&pixels, 8, 14)[0],
            rgba(&pixels, 1, 8)[0],
            rgba(&pixels, 14, 8)[0],
            rgba(&pixels, 8, 8)[0],
        );
        match direction {
            0 => assert!(top < bottom),
            1 => assert!(top > bottom),
            2 => assert!(left < right),
            3 => assert!(left > right),
            4 => {
                assert_eq!(top, bottom);
                assert!(middle < top);
            }
            _ => {
                assert_eq!(left, right);
                assert!(middle < left);
            }
        }
    }
}

fn scene(renderer: &mut Renderer, phase: u32) {
    stripes(renderer, 64, 48, phase);
    let bounds = rect(8.0, 4.0, 48.0, 40.0);
    renderer.draw_backdrop_filter(filter(bounds));
    renderer.start_post_blur_layer(bounds);
    quad(
        renderer,
        rect(24.0, 12.0, 16.0, 20.0),
        Color::from_rgb(1.0, 0.0, 0.0),
    );
    renderer.end_post_blur_layer();
}

#[test]
fn sibling_and_nested_filters_keep_the_recorded_order_even_after_layer_merging() {
    let mut tested = renderer(64.0, 48.0);
    stripes(&mut tested, 64, 48, 0);
    let mut reference = render(&mut tested, 64, 48, 1.0);
    let bounds = rect(0.0, 0.0, 64.0, 48.0);
    let mut mask = tiny_skia::Mask::new(64, 48).unwrap();
    crate::engine::adjust_clip_mask(&mut mask, bounds);
    let paint = |pixels: &mut tiny_skia::Pixmap, area: Rectangle, color: Color| {
        pixels.fill_rect(
            tiny_skia::Rect::from_xywh(area.x, area.y, area.width, area.height).unwrap(),
            &tiny_skia::Paint {
                shader: tiny_skia::Shader::SolidColor(crate::engine::into_color(color)),
                ..tiny_skia::Paint::default()
            },
            tiny_skia::Transform::identity(),
            None,
        );
    };
    let parent = filter(rect(8.0, 4.0, 48.0, 40.0));
    tested.draw_backdrop_filter(parent);
    tested.start_post_blur_layer(parent.bounds);
    draw(&mut reference.as_mut(), &parent, 1.0, 1.0, bounds, &mask);
    let green = Color::from_rgb(0.0, 1.0, 0.0);
    let green_bounds = rect(12.0, 10.0, 40.0, 28.0);
    quad(&mut tested, green_bounds, green);
    paint(&mut reference, green_bounds, green);
    let nested = BackdropFilter {
        radius: 4.0,
        saturation: 0.5,
        ..filter(rect(18.0, 8.0, 24.0, 32.0))
    };
    tested.draw_backdrop_filter(nested);
    tested.start_post_blur_layer(nested.bounds);
    draw(&mut reference.as_mut(), &nested, 1.0, 1.0, bounds, &mask);
    let red = Color::from_rgb(1.0, 0.0, 0.0);
    let red_bounds = rect(24.0, 14.0, 8.0, 8.0);
    quad(&mut tested, red_bounds, red);
    paint(&mut reference, red_bounds, red);
    tested.end_post_blur_layer();
    let blue = Color::from_rgb(0.0, 0.0, 1.0);
    let blue_bounds = rect(20.0, 24.0, 8.0, 8.0);
    quad(&mut tested, blue_bounds, blue);
    paint(&mut reference, blue_bounds, blue);
    tested.end_post_blur_layer();
    let sibling = BackdropFilter {
        radius: 5.0,
        border_radius: [6.0; 4],
        ..filter(rect(36.0, 0.0, 24.0, 40.0))
    };
    tested.draw_backdrop_filter(sibling);
    tested.start_post_blur_layer(sibling.bounds);
    draw(&mut reference.as_mut(), &sibling, 1.0, 1.0, bounds, &mask);
    let yellow = Color::from_rgb(1.0, 1.0, 0.0);
    let yellow_bounds = rect(44.0, 12.0, 8.0, 8.0);
    quad(&mut tested, yellow_bounds, yellow);
    paint(&mut reference, yellow_bounds, yellow);
    tested.end_post_blur_layer();
    tested.layers.merge();
    assert_eq!(
        tested
            .layers
            .iter()
            .filter(|layer| layer.backdrop.is_some())
            .count(),
        3
    );
    assert_eq!(render(&mut tested, 64, 48, 1.0).data(), reference.data());
}

#[test]
fn repeated_partial_damage_never_feeds_back_previous_foreground() {
    let mut tested = renderer(64.0, 48.0);
    scene(&mut tested, 0);
    let expected = render(&mut tested, 64, 48, 1.0);
    let mut reused = expected.clone();
    for _ in 0..8 {
        frame(&mut tested, &mut reused, 1.0, &[rect(24.0, 12.0, 1.0, 1.0)]);
        assert_eq!(reused.data(), expected.data());
    }
    tested.reset(rect(0.0, 0.0, 64.0, 48.0));
    scene(&mut tested, 1);
    let changed = render(&mut tested, 64, 48, 1.0);
    frame(&mut tested, &mut reused, 1.0, &[rect(0.0, 0.0, 1.0, 1.0)]);
    assert_eq!(reused.data(), changed.data());
}

#[test]
fn removing_the_last_filter_also_redraws_changed_content_outside_its_bounds() {
    let screen = rect(0.0, 0.0, 64.0, 48.0);
    let mut tested = renderer(64.0, 48.0);
    stripes(&mut tested, 64, 48, 0);
    tested.draw_backdrop_filter(filter(rect(8.0, 4.0, 20.0, 20.0)));
    let mut reused = render(&mut tested, 64, 48, 1.0);
    let previous = tested.layers().to_vec();

    tested.reset(screen);
    stripes(&mut tested, 64, 48, 0);
    tested.start_layer(screen);
    quad(
        &mut tested,
        rect(40.0, 24.0, 16.0, 16.0),
        Color::from_rgb(1.0, 0.0, 0.0),
    );
    tested.end_layer();
    tested.start_layer(screen);
    tested.end_layer();
    assert_eq!(previous.len(), tested.layers().len());
    let damage = crate::graphics::damage::diff(
        &previous,
        tested.layers(),
        |layer| vec![layer.bounds],
        crate::Layer::damage,
    );
    let damage = crate::graphics::damage::group(damage, screen);
    let expected = render(&mut tested, 64, 48, 1.0);
    frame(&mut tested, &mut reused, 1.0, &damage);
    assert_eq!(reused.data(), expected.data());
    assert_eq!(rgba(&reused, 44, 28), [255, 0, 0, 255]);
}

#[test]
fn box_passes_match_direct_sampling_even_beyond_image_edges() {
    let (width, height) = (7, 5);
    let source: Vec<Pixel> = (0..width * height)
        .map(|i| {
            [
                (i % width) as f32 * 30.0,
                (i / width) as f32 * 35.0,
                30.0,
                255.0,
            ]
        })
        .collect();
    for radius in [0, 1, 4, 13] {
        for horizontal in [true, false] {
            let mut output = vec![[0.0; 4]; source.len()];
            box_pass(&source, &mut output, width, height, radius, horizontal);
            for y in 0..height {
                for x in 0..width {
                    let mut expected = [0.0_f64; 4];
                    for offset in -(radius as isize)..=radius as isize {
                        let sx = if horizontal {
                            (x as isize + offset).clamp(0, width as isize - 1) as usize
                        } else {
                            x
                        };
                        let sy = if horizontal {
                            y
                        } else {
                            (y as isize + offset).clamp(0, height as isize - 1) as usize
                        };
                        for (channel, value) in expected.iter_mut().enumerate() {
                            *value += f64::from(source[sy * width + sx][channel]);
                        }
                    }
                    for (channel, value) in expected.iter().enumerate() {
                        assert!(
                            (f64::from(output[y * width + x][channel])
                                - value / (2 * radius + 1) as f64)
                                .abs()
                                < 0.0001
                        );
                    }
                }
            }
        }
    }
    let constant = vec![[10.0, 20.0, 30.0, 100.0]; 35];
    let mut output = constant.clone();
    box_pass(&constant, &mut output, width, height, usize::MAX / 32, true);
    assert_eq!(
        output, constant,
        "cost is bounded by pixels, even for enormous radii"
    );
}
