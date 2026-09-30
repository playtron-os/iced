//! Letter-spaced text next to what Chrome draws for the same CSS
//! (`chrome.html`, rendered in `snapshots/chrome.png`).
//!
//! CSS turns ligatures off in spaced text, so Geist's "tt" and "ff" come apart
//! as soon as `letter-spacing` is not zero. The tests draw the scene at a scale
//! factor of 2 and compare where each line's ink ends with Chrome's.
use iced::font::{Family, Weight};
use iced::widget::{container, pin, stack, text};
use iced::{Color, Element, Fill, Font};

pub fn main() -> iced::Result {
    iced::application(|| (), |_: &mut (), _: ()| {}, view)
        .font(GEIST)
        .window_size(SIZE)
        .run()
}

/// Geist SemiBold, which kora-kit sets its labels in (OFL, see `fonts/OFL.txt`).
const GEIST: &[u8] = include_bytes!("../fonts/Geist-SemiBold.ttf");

const FONT: Font = Font {
    family: Family::Name("Geist"),
    weight: Weight::Semibold,
    ..Font::DEFAULT
};

fn view(_state: &()) -> Element<'_, ()> {
    scene()
}

/// The logical size of the scene.
const SIZE: (f32, f32) = (240.0, 100.0);

const LINE: &str = "Settings office";

/// A line of the scene: where it sits and how its letters are spaced.
struct Case {
    y: f32,
    letter_spacing: f32,
}

/// Keep in sync with `chrome.html`.
const CASES: [Case; 3] = [
    // Unspaced: ligatures, as in CSS.
    Case {
        y: 10.0,
        letter_spacing: 0.0,
    },
    // kora-kit's app identity: 14px at -0.02em.
    Case {
        y: 40.0,
        letter_spacing: -0.28,
    },
    Case {
        y: 70.0,
        letter_spacing: 0.5,
    },
];

fn scene<'a>() -> Element<'a, ()> {
    let lines = CASES.iter().map(|case| {
        pin(text(LINE)
            .font(FONT)
            .size(14)
            .line_height(text::LineHeight::Absolute(20.into()))
            .shaping(text::Shaping::Advanced)
            .letter_spacing(case.letter_spacing)
            .color(Color::WHITE))
        .x(10)
        .y(case.y)
        .into()
    });

    container(stack(lines))
        .width(Fill)
        .height(Fill)
        .style(|_| container::background(Color::BLACK))
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    use iced::{Settings, Theme};
    use iced_test::Simulator;

    use std::path::{Path, PathBuf};

    const SCALE: f32 = 2.0;

    #[test]
    fn spaced_lines_are_as_long_as_in_chrome() {
        let render = render();
        let chrome = decode(&Path::new(env!("CARGO_MANIFEST_DIR")).join("snapshots/chrome.png"));

        for (index, case) in CASES.iter().enumerate() {
            let (left, right) = ink(&render.rgba, case);
            let (chrome_left, chrome_right) = ink(&chrome, case);

            // Rasterizers differ in a glyph's anti-aliasing, not in where it sits;
            // a ligature where Chrome has two letters moves the end a whole pixel
            // or more.
            assert!(
                left.abs_diff(chrome_left) <= 1 && right.abs_diff(chrome_right) <= 1,
                "line {index} ({}) has ink from pixel {left} to {right}, Chrome's from \
                 {chrome_left} to {chrome_right} (render in {})",
                render.renderer,
                render.path.display(),
            );
        }
    }

    /// The first and last pixel column of `case`'s line with any ink.
    fn ink(rgba: &[u8], case: &Case) -> (usize, usize) {
        let width = (SIZE.0 * SCALE) as usize;
        let rows = (case.y * SCALE) as usize..((case.y + 20.0) * SCALE) as usize;
        let inked = |x: usize| rows.clone().any(|y| rgba[(y * width + x) * 4] > 64);
        let columns: Vec<usize> = (0..width).filter(|x| inked(*x)).collect();

        (columns[0], columns[columns.len() - 1])
    }

    /// A render of the scene at a scale factor of 2.
    struct Render {
        renderer: String,
        path: PathBuf,
        rgba: Vec<u8>,
    }

    /// Draws the scene with the renderer `ICED_TEST_BACKEND` picks. The PNG is
    /// left in the temporary directory, so a failure can be looked at.
    fn render() -> Render {
        let directory = std::env::temp_dir().join("iced-letter-spacing");
        let _ = std::fs::remove_dir_all(&directory);

        let settings = Settings {
            fonts: vec![GEIST.into()],
            ..Settings::default()
        };
        let mut ui = Simulator::with_size(settings, SIZE, scene());
        let snapshot = ui.snapshot(&Theme::Dark).expect("take a snapshot");
        // A snapshot writes itself out when there is nothing to compare it with.
        assert!(
            snapshot
                .matches_image(directory.join("scene"))
                .expect("write the snapshot")
        );

        let path = std::fs::read_dir(&directory)
            .expect("list the snapshot directory")
            .find_map(|entry| Some(entry.ok()?.path()))
            .expect("find the snapshot");
        let renderer = path
            .file_stem()
            .and_then(|stem| stem.to_str()?.strip_prefix("scene-"))
            .unwrap_or_default()
            .to_owned();
        let rgba = decode(&path);

        Render {
            renderer,
            path,
            rgba,
        }
    }

    /// The pixels of a PNG, as RGBA.
    fn decode(path: &Path) -> Vec<u8> {
        let file = std::fs::File::open(path).expect("open the image");
        let mut reader = png::Decoder::new(std::io::BufReader::new(file))
            .read_info()
            .expect("read the image");
        let mut pixels = vec![0; reader.output_buffer_size().expect("size the image")];
        let info = reader.next_frame(&mut pixels).expect("decode the image");
        pixels.truncate(info.buffer_size());

        match info.color_type {
            png::ColorType::Rgba => pixels,
            png::ColorType::Rgb => pixels
                .chunks_exact(3)
                .flat_map(|rgb| [rgb[0], rgb[1], rgb[2], u8::MAX])
                .collect(),
            other => panic!("{} is {other:?}, not RGB(A)", path.display()),
        }
    }
}
