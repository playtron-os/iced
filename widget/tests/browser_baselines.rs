//! Text sits on the device-pixel row the browser draws it on, at 1x, 1.5x and 2x, for
//! every size, line height and quarter-pixel offset below.
//!
//! The expected rows are the last inked row of "HEH" in Geist SemiBold, measured in
//! Google Chrome 154 (`--headless=new --force-device-scale-factor=<scale>`) on a page of
//! absolutely positioned `<div>`s, `font-size: <size>px; line-height: <ratio>`, each
//! `<top>`px down a 60px cell. Run them like `baseline_alignment`:
//!
//! ```sh
//! ICED_TEST_BACKEND=tiny-skia cargo test -p iced_widget --test browser_baselines \
//!     --features iced_renderer/tiny-skia,iced_renderer/wayland,iced_renderer/x11
//! ```
#![cfg(not(feature = "wgpu"))]
use iced_test::Simulator;
use iced_widget::core::text::LineHeight;
use iced_widget::core::{Color, Element, Font, Padding, Settings, Theme, font};
use iced_widget::{container, text};

type Renderer = iced_renderer::Renderer;

const SIZES: [f32; 7] = [11.0, 12.0, 13.0, 14.0, 16.0, 20.0, 28.0];
const LINE_HEIGHTS: [f32; 3] = [1.2, 1.35, 1.5];
const TOPS: [f32; 4] = [0.0, 0.25, 0.5, 0.75];

/// Chrome's rows, case by case: sizes, then line heights, then tops.
const CHROME: [(f32, [u16; 84]); 3] = [
    (
        1.0,
        [
            9, 9, 10, 10, 10, 10, 11, 11, 11, 11, 12, 12, 10, 10, 11, 11, 11, 11, 12, 12, 12, 12,
            13, 13, 11, 11, 12, 12, 12, 12, 13, 13, 13, 13, 14, 14, 12, 12, 13, 13, 13, 13, 14, 14,
            14, 14, 15, 15, 14, 14, 15, 15, 15, 15, 16, 16, 16, 16, 17, 17, 18, 18, 19, 19, 19, 19,
            20, 20, 21, 21, 22, 22, 25, 25, 26, 26, 27, 27, 28, 28, 30, 30, 31, 31,
        ],
    ),
    (
        1.5,
        [
            14, 14, 15, 15, 15, 15, 16, 16, 16, 16, 17, 17, 16, 16, 17, 17, 17, 17, 18, 18, 19, 19,
            20, 20, 17, 17, 18, 18, 19, 19, 20, 20, 20, 20, 21, 21, 18, 18, 19, 19, 20, 20, 21, 21,
            21, 21, 22, 22, 21, 21, 22, 22, 23, 23, 24, 24, 25, 25, 26, 26, 27, 27, 28, 28, 29, 29,
            30, 30, 32, 32, 33, 33, 39, 39, 40, 40, 42, 42, 43, 43, 45, 45, 46, 46,
        ],
    ),
    (
        2.0,
        [
            19, 20, 20, 21, 21, 22, 22, 23, 23, 24, 24, 25, 21, 22, 22, 23, 23, 24, 24, 25, 25, 26,
            26, 27, 23, 24, 24, 25, 25, 26, 26, 27, 27, 28, 28, 29, 25, 26, 26, 27, 27, 28, 28, 29,
            30, 31, 31, 32, 29, 30, 30, 31, 31, 32, 32, 33, 34, 35, 35, 36, 37, 38, 38, 39, 40, 41,
            41, 42, 43, 44, 44, 45, 52, 53, 53, 54, 56, 57, 57, 58, 61, 62, 62, 63,
        ],
    ),
];

/// Whether the renderer measures text; the null one this crate builds by default doesn't.
fn measures_text() -> bool {
    std::any::TypeId::of::<Renderer>() != std::any::TypeId::of::<()>()
}

/// The last row inked darker than mid-grey, in device pixels.
fn last_inked_row(scale: f32, size: f32, line_height: f32, top: f32) -> Option<usize> {
    let geist = Font {
        family: font::Family::Name("Geist"),
        weight: font::Weight::Semibold,
        ..Font::DEFAULT
    };
    let settings = Settings {
        fonts: vec![
            include_bytes!("../../examples/letter_spacing/fonts/Geist-SemiBold.ttf")
                .as_slice()
                .into(),
        ],
        ..Settings::default()
    };
    let line: Element<'_, (), Theme, Renderer> = container(
        text("HEH")
            .size(size)
            .line_height(LineHeight::Relative(line_height))
            .font(geist)
            .color(Color::BLACK),
    )
    .padding(Padding::ZERO.top(top))
    .into();
    let mut ui = Simulator::with_size(settings, (60.0, 60.0), line).scale_factor(scale);
    let snapshot = ui.snapshot(&Theme::Light).expect("snapshot");
    let shot = snapshot.screenshot();
    let width = shot.size.width as usize;

    shot.rgba
        .chunks_exact(4)
        .enumerate()
        .filter(|(_, pixel)| pixel.iter().take(3).map(|v| u32::from(*v)).sum::<u32>() < 384)
        .map(|(index, _)| index / width)
        .max()
}

#[test]
fn text_sits_on_the_rows_chrome_draws_it_on() {
    if !measures_text() {
        return;
    }

    for (scale, rows) in CHROME {
        let mut expected = rows.iter();
        for size in SIZES {
            for line_height in LINE_HEIGHTS {
                for top in TOPS {
                    let chrome = usize::from(*expected.next().expect("a row per case"));
                    assert_eq!(
                        last_inked_row(scale, size, line_height, top),
                        Some(chrome),
                        "{size}px at line height {line_height}, {top}px down, at {scale}x"
                    );
                }
            }
        }
    }
}
