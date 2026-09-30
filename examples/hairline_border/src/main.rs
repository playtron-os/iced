//! Borders thinner than a device pixel, and ones between whole pixels, next to
//! what Chrome draws for the same CSS (`chrome.html`, rendered at 1x, 1.5x and
//! 2x in `snapshots/`).
//!
//! CSS snaps a border width to device pixels: down to a whole pixel, and a
//! border thinner than one pixel up to one. The tests draw the scene at each
//! scale and compare it with Chrome's render, pixel by pixel.
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
const SIZE: (f32, f32) = (380.0, 180.0);

/// Every box's size.
const BOX: (f32, f32) = (100.0, 60.0);

const PAGE: Color = color!(0x101014);
const SURFACE: Color = color!(0x202028);
const LINE: Color = color!(0xe8e8ee);

/// A box of the scene, placed on even coordinates so it lands on whole
/// pixels at 1.5x too.
struct Case {
    x: f32,
    y: f32,
    radius: f32,
    /// Top, right, bottom, left.
    sides: [f32; 4],
}

/// Keep in sync with `chrome.html`.
const CASES: [Case; 6] = [
    // `--border-width-hairline`.
    Case {
        x: 20.0,
        y: 20.0,
        radius: 0.0,
        sides: [0.5; 4],
    },
    Case {
        x: 140.0,
        y: 20.0,
        radius: 0.0,
        sides: [0.25; 4],
    },
    // `--border-width-medium`: a whole pixel at 1x, two at 1.5x, three at 2x.
    Case {
        x: 260.0,
        y: 20.0,
        radius: 0.0,
        sides: [1.5; 4],
    },
    Case {
        x: 20.0,
        y: 100.0,
        radius: 8.0,
        sides: [0.5; 4],
    },
    // A hairline divider.
    Case {
        x: 140.0,
        y: 100.0,
        radius: 0.0,
        sides: [0.0, 0.0, 0.5, 0.0],
    },
    Case {
        x: 260.0,
        y: 100.0,
        radius: 0.0,
        sides: [2.5; 4],
    },
];

fn scene<'a>() -> Element<'a, ()> {
    let boxes = CASES.iter().map(|case| {
        let [top, right, bottom, left] = case.sides;
        let border = if case.sides.iter().all(|side| *side == top) {
            Border::default().width(top)
        } else {
            Border::default()
                .top(top)
                .right(right)
                .bottom(bottom)
                .left(left)
        }
        .color(LINE)
        .rounded(case.radius);

        pin(container("")
            .width(BOX.0)
            .height(BOX.1)
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

    #[test]
    fn borders_snap_to_device_pixels_as_in_chrome() {
        let mut failures = Vec::new();

        for scale in [1.0, 1.5, 2.0] {
            let render = render(scale);
            let chrome = decode(
                &Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join(format!("snapshots/chrome-{scale}x.png")),
            );

            for (index, case) in CASES.iter().enumerate() {
                let (largest, mean) = compare(&render.rgba, &chrome, case, scale);

                // A line at half strength, or a pixel too wide, is off by
                // around half the colour range.
                if largest > 24 || mean > 0.5 {
                    failures.push(format!(
                        "case {index} at {scale}x ({}): a pixel differs from Chrome's by \
                         {largest}, {mean:.2} on average (render in {})",
                        render.renderer,
                        render.path.display(),
                    ));
                }
            }
        }

        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// The largest difference from Chrome's render of a channel of a pixel along
    /// the sides of `case` (and a pixel around it), and the mean over all of
    /// them. Corners are left out: how a curve or a join is anti-aliased differs
    /// between rasterizers, and that is not the width of the line.
    fn compare(render: &[u8], chrome: &[u8], case: &Case, scale: f32) -> (u8, f32) {
        let stride = (SIZE.0 * scale) as usize * 4;
        let (left, top) = (case.x - 1.0, case.y - 1.0);
        let (right, bottom) = (case.x + BOX.0 + 1.0, case.y + BOX.1 + 1.0);
        let corner = case.radius + 2.0;

        let (mut largest, mut total, mut count) = (0, 0.0, 0);
        for y in (top * scale) as usize..(bottom * scale) as usize {
            for x in (left * scale) as usize..(right * scale) as usize {
                let (logical_x, logical_y) = ((x as f32 + 0.5) / scale, (y as f32 + 0.5) / scale);
                let near_x = logical_x < left + corner || logical_x > right - corner;
                let near_y = logical_y < top + corner || logical_y > bottom - corner;
                if near_x && near_y {
                    continue;
                }

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

    /// A render of the scene.
    struct Render {
        renderer: String,
        path: PathBuf,
        rgba: Vec<u8>,
    }

    /// Draws the scene at `scale` with the renderer `ICED_TEST_BACKEND` picks.
    /// The PNG is left in the temporary directory, so a failure can be looked at.
    fn render(scale: f32) -> Render {
        let directory = std::env::temp_dir()
            .join("iced-hairline-border")
            .join(format!("{scale}x"));
        let _ = std::fs::remove_dir_all(&directory);

        let mut ui = Simulator::with_size(Settings::default(), SIZE, scene()).scale_factor(scale);
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
