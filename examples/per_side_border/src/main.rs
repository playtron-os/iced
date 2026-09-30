//! Borders with a different width per side, next to what Chrome draws for the
//! same CSS (`chrome.html`, rendered in `snapshots/chrome.png`).
//!
//! The tests draw the scene at a scale factor of 2 and compare it with Chrome's
//! render, pixel by pixel.
use iced::widget::{container, pin, stack};
use iced::{Border, Color, Element, Fill, color};

pub fn main() -> iced::Result {
    iced::application(|| (), |_: &mut (), _: ()| {}, view)
        .window_size(SIZE)
        .run()
}

fn view(_state: &()) -> Element<'_, ()> {
    scene()
}

/// The logical size of the scene.
const SIZE: (f32, f32) = (520.0, 200.0);

const PAGE: Color = color!(0x101014);
const SURFACE: Color = color!(0x202028);
const LINE: Color = color!(0x4fd1ff);

/// A box of the scene: where it is, its corner radii and its border widths.
struct Case {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    /// Top-left, top-right, bottom-right, bottom-left.
    radius: [f32; 4],
    /// Top, right, bottom, left.
    sides: [f32; 4],
}

/// Keep in sync with `chrome.html`.
const CASES: [Case; 6] = [
    // A compact navigation bar: its bottom divider only.
    Case {
        x: 20.0,
        y: 20.0,
        width: 160.0,
        height: 48.0,
        radius: [0.0; 4],
        sides: [0.0, 0.0, 1.0, 0.0],
    },
    // A drawer: its right edge only.
    Case {
        x: 20.0,
        y: 88.0,
        width: 160.0,
        height: 92.0,
        radius: [0.0; 4],
        sides: [0.0, 1.0, 0.0, 0.0],
    },
    // A rounded card with a heavy top and a light bottom.
    Case {
        x: 200.0,
        y: 20.0,
        width: 140.0,
        height: 72.0,
        radius: [12.0; 4],
        sides: [4.0, 0.0, 1.0, 0.0],
    },
    // A uniform border wider than twice its corner radius.
    Case {
        x: 200.0,
        y: 108.0,
        width: 140.0,
        height: 72.0,
        radius: [2.0; 4],
        sides: [6.0; 4],
    },
    // Every side different, round corners.
    Case {
        x: 360.0,
        y: 20.0,
        width: 140.0,
        height: 72.0,
        radius: [16.0; 4],
        sides: [2.0, 3.0, 6.0, 1.0],
    },
    // One thick side, square corners.
    Case {
        x: 360.0,
        y: 108.0,
        width: 140.0,
        height: 72.0,
        radius: [0.0; 4],
        sides: [0.0, 0.0, 0.0, 4.0],
    },
];

fn scene<'a>() -> Element<'a, ()> {
    let boxes = CASES.iter().map(|case| {
        let [top_left, top_right, bottom_right, bottom_left] = case.radius;
        let [top, right, bottom, left] = case.sides;
        let border = Border::default()
            .color(LINE)
            .rounded(iced::border::Radius {
                top_left,
                top_right,
                bottom_right,
                bottom_left,
            })
            .top(top)
            .right(right)
            .bottom(bottom)
            .left(left);

        pin(container("")
            .width(case.width)
            .height(case.height)
            .style(move |_| container::Style {
                background: Some(SURFACE.into()),
                border,
                ..container::Style::default()
            }))
        .x(case.x)
        .y(case.y)
        .into()
    });

    container(stack(boxes))
        .width(Fill)
        .height(Fill)
        .style(|_| container::background(PAGE))
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
    fn every_side_is_drawn_as_chrome_draws_it() {
        let render = render();
        let chrome = decode(&Path::new(env!("CARGO_MANIFEST_DIR")).join("snapshots/chrome.png"));

        for (index, case) in CASES.iter().enumerate() {
            let (largest, mean) = compare(&render.rgba, &chrome, case);

            // Anti-aliasing differs a little along curves; a missing, misplaced
            // or reshaped edge differs by most of the colour range.
            assert!(
                largest <= 96 && mean < 1.0,
                "case {index} ({}): a pixel differs from Chrome's by {largest}, \
                 {mean:.2} on average (render in {})",
                render.renderer,
                render.path.display(),
            );
        }
    }

    /// The largest difference from Chrome's render of a channel of a pixel in
    /// `case` (and a pixel around it), and the mean over all of them.
    fn compare(render: &[u8], chrome: &[u8], case: &Case) -> (u8, f32) {
        let stride = (SIZE.0 * SCALE) as usize * 4;
        let left = ((case.x - 1.0) * SCALE) as usize;
        let top = ((case.y - 1.0) * SCALE) as usize;
        let right = ((case.x + case.width + 1.0) * SCALE) as usize;
        let bottom = ((case.y + case.height + 1.0) * SCALE) as usize;

        let (mut largest, mut total, mut count) = (0, 0.0, 0);
        for y in top..bottom {
            for x in left..right {
                let at = y * stride + x * 4;
                let delta = (0..3)
                    .map(|channel| render[at + channel].abs_diff(chrome[at + channel]))
                    .max()
                    .unwrap_or_default();
                largest = largest.max(delta);
                total += f32::from(delta);
                count += 1;
            }
        }

        (largest, total / count as f32)
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
        let directory = std::env::temp_dir().join("iced-per-side-border");
        let _ = std::fs::remove_dir_all(&directory);

        let mut ui = Simulator::with_size(Settings::default(), SIZE, scene());
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
