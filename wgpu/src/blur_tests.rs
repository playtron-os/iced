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
