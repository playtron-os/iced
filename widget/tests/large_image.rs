//! A picture far larger than where it is drawn is reduced to that size first: it
//! draws in its first frame on wgpu, which uploaded it off the render thread and
//! drew nothing there, and its fine detail averages out instead of aliasing.
//! Run them like `baseline_alignment`, with `image`:
//!
//! ```sh
//! ICED_TEST_BACKEND=tiny-skia cargo test -p iced_widget --test large_image \
//!     --features image,iced_renderer/tiny-skia,iced_renderer/wayland,iced_renderer/x11
//! ```
//!
//! For wgpu, set `ICED_TEST_BACKEND=wgpu` and add `iced_renderer/wgpu,iced_renderer/wgpu-bare`.
#![cfg(all(feature = "image", not(feature = "wgpu")))]
use iced_test::Simulator;
use iced_widget::core::image::Handle;
use iced_widget::core::{ContentFit, Element, Settings, Theme};
use iced_widget::{container, image};

type Renderer = iced_renderer::Renderer;

const TILE: f32 = 32.0;
const PAD: f32 = 4.0;

/// Whether the renderer draws; the null one this crate builds by default doesn't.
fn draws() -> bool {
    std::any::TypeId::of::<Renderer>() != std::any::TypeId::of::<()>()
}

/// Black and white columns one pixel wide, 4 bytes a pixel: about 4 MiB and more.
fn stripes(width: u32, height: u32) -> Handle {
    let pixels = (0..width * height)
        .flat_map(|i| {
            let value = if i % 2 == 0 { 0 } else { 255 };
            [value, value, value, 255]
        })
        .collect::<Vec<u8>>();

    Handle::from_rgba(width, height, pixels)
}

/// The device pixel at `(x, y)` of `element` drawn in a `TILE` square, padded.
fn pixels(element: Element<'_, (), Theme, Renderer>, at: &[(f32, f32)]) -> Vec<[u8; 4]> {
    let framed: Element<'_, (), Theme, Renderer> = container(element).padding(PAD).into();
    let canvas = TILE + 2.0 * PAD;
    let mut ui =
        Simulator::with_size(Settings::default(), (canvas, canvas), framed).scale_factor(1.0);
    let snapshot = ui.snapshot(&Theme::Light).expect("snapshot");
    let shot = snapshot.screenshot();
    let width = shot.size.width as usize;

    at.iter()
        .map(|&(x, y)| {
            let at = (y as usize * width + x as usize) * 4;
            [
                shot.rgba[at],
                shot.rgba[at + 1],
                shot.rgba[at + 2],
                shot.rgba[at + 3],
            ]
        })
        .collect()
}

fn is_grey(pixel: [u8; 4]) -> bool {
    pixel[..3].iter().all(|channel| channel.abs_diff(128) <= 2)
}

#[test]
fn a_large_image_drawn_small_shows_its_average() {
    if !draws() {
        return;
    }

    let row = PAD + TILE / 2.0;
    let at: Vec<(f32, f32)> = (0..TILE as u32)
        .map(|x| (PAD + x as f32 + 0.5, row))
        .collect();
    let drawn = pixels(
        // Not a whole multiple of the tile, so sampling lands on single columns.
        image(stripes(1020, 1020)).width(TILE).height(TILE).into(),
        &at,
    );

    assert!(drawn.iter().all(|pixel| is_grey(*pixel)), "{drawn:?}");
}

#[test]
fn a_large_image_keeps_its_rounded_corners_when_it_overflows() {
    if !draws() {
        return;
    }

    let corner = (PAD + 1.0, PAD + 1.0);
    let middle = (PAD + TILE / 2.0, PAD + TILE / 2.0);
    let drawn = pixels(
        image(stripes(1024, 2048))
            .width(TILE)
            .height(TILE)
            .content_fit(ContentFit::Cover)
            .border_radius(8.0)
            .into(),
        &[corner, middle],
    );

    assert_eq!(drawn[0], [255, 255, 255, 255], "the corner is cut away");
    assert!(is_grey(drawn[1]), "{:?}", drawn[1]);
}
