//! A row aligned on baselines puts text of different sizes on the same line, as
//! CSS `align-items: baseline` does; a centred row doesn't.
//!
//! They need a renderer that measures text, which this crate's tests don't build by
//! default, so they return early without one. Run them with:
//!
//! ```sh
//! ICED_TEST_BACKEND=tiny-skia cargo test -p iced_widget --test baseline_alignment \
//!     --features iced_renderer/tiny-skia,iced_renderer/wayland,iced_renderer/x11
//! ```
//!
//! For wgpu, set `ICED_TEST_BACKEND=wgpu` and add `iced_renderer/wgpu,iced_renderer/wgpu-bare`.
#![cfg(not(feature = "wgpu"))]
use iced_test::Simulator;
use iced_widget::core::{Color, Element, Settings, Theme};
use iced_widget::{container, row, text};

type Renderer = iced_renderer::Renderer;

/// Whether the renderer measures text; the null one this crate builds by default doesn't.
fn measures_text() -> bool {
    std::any::TypeId::of::<Renderer>() != std::any::TypeId::of::<()>()
}

const BIG: Color = Color::from_rgb(1.0, 0.0, 0.0);
const SMALL: Color = Color::from_rgb(0.0, 0.0, 1.0);

/// Letters that sit on the baseline and have nothing below it.
const FLAT: &str = "HEH";

/// The last pixel row fully inked in `color`, in physical pixels.
fn bottom_row(ui: &mut Simulator<'_, (), Theme, Renderer>, color: Color) -> u32 {
    let snapshot = ui.snapshot(&Theme::Light).expect("snapshot");
    let shot = snapshot.screenshot();
    let width = shot.size.width as usize;
    let want = color.into_rgba8();
    let solid = |pixel: &[u8]| {
        pixel
            .iter()
            .take(3)
            .zip(want.iter())
            .all(|(have, want)| have.abs_diff(*want) < 40)
    };

    shot.rgba
        .chunks_exact(4)
        .enumerate()
        .filter(|(_, pixel)| solid(pixel))
        .map(|(index, _)| (index / width) as u32)
        .max()
        .expect("the text was drawn")
}

fn simulate(element: Element<'_, (), Theme, Renderer>) -> Simulator<'_, (), Theme, Renderer> {
    Simulator::with_size(Settings::default(), (400.0, 120.0), element).scale_factor(1.0)
}

fn pair<'a>() -> (
    Element<'a, (), Theme, Renderer>,
    Element<'a, (), Theme, Renderer>,
) {
    (
        text(FLAT).size(36).color(BIG).into(),
        text(FLAT).size(13).color(SMALL).into(),
    )
}

#[test]
fn baseline_aligned_text_of_two_sizes_sits_on_one_line() {
    if !measures_text() {
        return;
    }

    let (big, small) = pair();
    let mut ui = simulate(row![big, small].spacing(8).align_baseline().into());

    assert_eq!(bottom_row(&mut ui, BIG), bottom_row(&mut ui, SMALL));
}

#[test]
fn a_padded_container_passes_its_texts_baseline_up() {
    if !measures_text() {
        return;
    }

    let (big, small) = pair();
    let padded = container(big).padding([9, 4]);
    let mut ui = simulate(row![padded, small].spacing(8).align_baseline().into());

    assert_eq!(bottom_row(&mut ui, BIG), bottom_row(&mut ui, SMALL));
}

#[test]
fn centred_text_of_two_sizes_does_not() {
    if !measures_text() {
        return;
    }

    let (big, small) = pair();
    let mut ui = simulate(
        row![big, small]
            .spacing(8)
            .align_y(iced_widget::core::alignment::Vertical::Center)
            .into(),
    );

    assert_ne!(bottom_row(&mut ui, BIG), bottom_row(&mut ui, SMALL));
}
