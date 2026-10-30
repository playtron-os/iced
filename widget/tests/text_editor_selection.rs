//! A selection across lines is one solid band: each line's highlight is snapped
//! to whole pixels, so lines at fractional heights leave no seam between them.
//!
//! It needs a renderer that draws, which this crate's tests don't build by
//! default, so it returns early without one. Run it like `baseline_alignment`:
//!
//! ```sh
//! ICED_TEST_BACKEND=tiny-skia cargo test -p iced_widget --test text_editor_selection \
//!     --features iced_renderer/tiny-skia,iced_renderer/wayland,iced_renderer/x11
//! ```
//!
//! For wgpu, set `ICED_TEST_BACKEND=wgpu` and add `iced_renderer/wgpu,iced_renderer/wgpu-bare`.
#![cfg(not(feature = "wgpu"))]
use iced_test::{Simulator, simulator};
use iced_widget::core::{Element, Point, Settings, Theme};
use iced_widget::text_editor::{Action, Content, TextEditor};

type Renderer = iced_renderer::Renderer;

/// Whether the renderer draws; the null one this crate builds by default doesn't.
fn draws() -> bool {
    std::any::TypeId::of::<Renderer>() != std::any::TypeId::of::<()>()
}

const PADDING: f32 = 5.0;
const SIZE: f32 = 16.0;

/// 20.8px lines: the second one starts between two pixel rows.
const LINE_HEIGHT: f32 = 1.3;

#[test]
fn a_selection_across_lines_has_no_seam() {
    if !draws() {
        return;
    }

    // Spaces: the selection shows and no glyph draws over it.
    let mut content = Content::with_text("        \n        ");
    content.perform(Action::SelectAll);

    let editor: Element<'_, (), Theme, Renderer> = TextEditor::new(&content)
        .size(SIZE)
        .line_height(LINE_HEIGHT)
        .padding(PADDING)
        .on_action(|_| ())
        .into();

    let mut ui =
        Simulator::with_size(Settings::default(), (200.0, 100.0), editor).scale_factor(1.0);

    // The click focuses the editor. Its action is never performed, so
    // everything stays selected.
    ui.point_at(Point::new(150.0, 20.0));
    let _ = ui.simulate(simulator::click());

    let snapshot = ui.snapshot(&Theme::Light).expect("snapshot");
    let shot = snapshot.screenshot();
    let width = shot.size.width as usize;
    let x = PADDING as usize + 2;
    let pixel = |y: usize| {
        let at = (y * width + x) * 4;
        <[u8; 4]>::try_from(&shot.rgba[at..at + 4]).expect("a pixel")
    };

    let top = PADDING as usize + 1;
    let bottom = (PADDING + 2.0 * SIZE * LINE_HEIGHT) as usize - 1;
    let band = pixel(top);

    assert_ne!(band, pixel(1), "the selection is drawn");

    for y in top..bottom {
        assert_eq!(pixel(y), band, "row {y} of the selection");
    }
}
