//! These draw on a GPU, so they are ignored by default; run them with
//! `cargo test -p iced_wgpu -- --ignored blur_tests` on a machine with one, and
//! with `WGPU_BACKEND=gl` for the GL backend.
use crate::Renderer;
use crate::core::renderer::{Headless, Quad};
use crate::core::{Color, Font, Pixels, Rectangle, Renderer as _, Size};
use crate::graphics::Viewport;

const WIDTH: u32 = 64;
const HEIGHT: u32 = 32;

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

/// Black and white stripes, four pixels wide, with a blurred panel over the
/// middle.
fn draw(renderer: &mut Renderer) {
    for i in 0..WIDTH / 4 {
        renderer.fill_quad(
            Quad {
                bounds: Rectangle::new([(i * 4) as f32, 0.0].into(), Size::new(4.0, HEIGHT as f32)),
                ..Quad::default()
            },
            if i % 2 == 0 {
                Color::BLACK
            } else {
                Color::WHITE
            },
        );
    }
    renderer.draw_backdrop_blur(
        Rectangle::new([16.0, 4.0].into(), Size::new(32.0, 24.0)),
        8.0,
        [0.0; 4],
        1.0,
        1.0,
    );
}

fn screenshot(frame_usage: wgpu::TextureUsages) -> Vec<u8> {
    let mut renderer = renderer();
    draw(&mut renderer);
    renderer.screenshot_from(
        &Viewport::with_physical_size(Size::new(WIDTH, HEIGHT), 1.0),
        Color::TRANSPARENT,
        frame_usage,
    )
}

fn red(pixels: &[u8], x: u32, y: u32) -> u8 {
    pixels[((y * WIDTH + x) * 4) as usize]
}

/// A GL surface cannot be copied from, so the blur is drawn offscreen there.
/// Copying from it anyway was a validation error that crashed every app with
/// a glass surface on a GL-only machine.
#[test]
#[ignore = "draws on a GPU"]
fn a_frame_that_cannot_be_copied_from_is_blurred_the_same() {
    let copied = screenshot(wgpu::TextureUsages::COPY_SRC);
    let offscreen = screenshot(wgpu::TextureUsages::empty());

    assert_eq!(copied, offscreen);
    // Inside the panel neighbouring stripes run together; outside they stay
    // black and white.
    assert!(red(&offscreen, 32, 16).abs_diff(red(&offscreen, 36, 16)) < 64);
    assert_eq!(red(&offscreen, 2, 16), 0);
    assert_eq!(red(&offscreen, 6, 16), 255);
}

/// A white page under a modal's tint, which is a post-blur layer with no blur
/// of its own; a red dialog over it on an identity blur, as a dialog that
/// keeps nested blurs off itself has; and, when `toast`, a blurred blue panel
/// between the two.
fn modal(renderer: &mut Renderer, toast: bool) {
    let screen = Rectangle::with_size(Size::new(WIDTH as f32, HEIGHT as f32));
    renderer.fill_quad(
        Quad {
            bounds: screen,
            ..Quad::default()
        },
        Color::WHITE,
    );
    renderer.start_post_blur_layer(screen);
    renderer.fill_quad(
        Quad {
            bounds: screen,
            ..Quad::default()
        },
        Color::from_rgba(0.0, 0.0, 0.0, 0.6),
    );
    renderer.end_post_blur_layer();
    if toast {
        let card = Rectangle::new([0.0, 24.0].into(), Size::new(16.0, 8.0));
        renderer.draw_backdrop_blur(card, 8.0, [0.0; 4], 1.0, 1.0);
        renderer.start_post_blur_layer(card);
        renderer.fill_quad(
            Quad {
                bounds: card,
                ..Quad::default()
            },
            Color::from_rgb(0.0, 0.0, 1.0),
        );
        renderer.end_post_blur_layer();
    }
    renderer.draw_backdrop_blur(screen, 0.0, [0.0; 4], 1.0, 1.0);
    renderer.start_post_blur_layer(screen);
    renderer.fill_quad(
        Quad {
            bounds: Rectangle::new([24.0, 4.0].into(), Size::new(32.0, 16.0)),
            ..Quad::default()
        },
        Color::from_rgb(1.0, 0.0, 0.0),
    );
    renderer.end_post_blur_layer();
}

fn modal_screenshot(toast: bool, frame_usage: wgpu::TextureUsages) -> Vec<u8> {
    let mut renderer = renderer();
    modal(&mut renderer, toast);
    renderer.screenshot_from(
        &Viewport::with_physical_size(Size::new(WIDTH, HEIGHT), 1.0),
        Color::TRANSPARENT,
        frame_usage,
    )
}

fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * WIDTH + x) * 4) as usize;
    [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
}

/// A modal's tint is drawn before every blurred surface above it. With a
/// second surface on screen, such as a toast, it used to be drawn after them
/// all, and dimmed the dialog and the toast as it dims the page.
#[test]
#[ignore = "draws on a GPU"]
fn a_tint_under_two_blurred_surfaces_stays_under_them() {
    for usage in [wgpu::TextureUsages::COPY_SRC, wgpu::TextureUsages::empty()] {
        let alone = modal_screenshot(false, usage);
        let with_toast = modal_screenshot(true, usage);

        assert_eq!(pixel(&alone, 40, 12), [255, 0, 0, 255], "{usage:?}");
        assert_eq!(pixel(&with_toast, 40, 12), [255, 0, 0, 255], "{usage:?}");
        // The page under the tint, away from the toast, is unchanged by it.
        assert_eq!(pixel(&with_toast, 60, 2), pixel(&alone, 60, 2), "{usage:?}");
        // The toast, above the tint, is not tinted either.
        assert_eq!(pixel(&with_toast, 8, 28), [0, 0, 255, 255], "{usage:?}");
    }
}
