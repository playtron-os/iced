//! Elliptical radial gradients next to what Chrome draws for the same CSS
//! `radial-gradient(ellipse …)` (`chrome.html`, rendered in
//! `snapshots/chrome.png`).
//!
//! The tests draw the scene at a scale factor of 2 and compare it with Chrome's
//! render, pixel by pixel.
use iced::gradient::Radial;
use iced::widget::{container, pin, stack};
use iced::{Color, Element, Fill, Gradient, Point, color};

pub fn main() -> iced::Result {
    iced::application(|| (), |_: &mut (), _: ()| {}, view)
        .window_size(SIZE)
        .run()
}

fn view(_state: &()) -> Element<'_, ()> {
    scene()
}

/// The logical size of the scene.
const SIZE: (f32, f32) = (640.0, 300.0);

const PAGE: Color = color!(0x101014);

/// A box of the scene.
struct Case {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    /// Its gradients, bottom first.
    layers: &'static [Layer],
}

/// A `radial-gradient(ellipse …)`, its geometry as ratios of the box.
struct Layer {
    center: (f32, f32),
    radii: (f32, f32),
    stops: &'static [(f32, Color)],
}

/// Keep in sync with `chrome.html`, which lists each box's layers top first.
const CASES: [Case; 3] = [
    // A wide ellipse filling its box.
    Case {
        x: 20.0,
        y: 20.0,
        width: 280.0,
        height: 120.0,
        layers: &[Layer {
            center: (0.5, 0.5),
            radii: (0.5, 0.5),
            stops: &[(0.0, color!(0x4fd1ff)), (1.0, PAGE)],
        }],
    },
    // A tall one off centre, three stops.
    Case {
        x: 20.0,
        y: 160.0,
        width: 280.0,
        height: 120.0,
        layers: &[Layer {
            center: (0.8, 0.2),
            radii: (0.3, 0.9),
            stops: &[
                (0.0, color!(0xff7a3c)),
                (0.5, color!(0x7a3cff)),
                (1.0, PAGE),
            ],
        }],
    },
    // kora-greeter's dusk backdrop: two faded ellipses from the corners.
    Case {
        x: 320.0,
        y: 20.0,
        width: 300.0,
        height: 260.0,
        layers: &[
            Layer {
                center: (1.0, 0.95),
                radii: (0.65, 0.7),
                stops: &[(0.0, color!(0x9d5cff, 0.65)), (0.85, color!(0x9d5cff, 0.0))],
            },
            Layer {
                center: (0.0, 1.0),
                radii: (0.75, 0.8),
                stops: &[(0.0, color!(0x4fd1ff, 0.42)), (0.85, color!(0x4fd1ff, 0.0))],
            },
        ],
    },
];

fn scene<'a>() -> Element<'a, ()> {
    let layers = CASES.iter().flat_map(|case| {
        case.layers.iter().map(move |layer| {
            let gradient = layer.stops.iter().fold(
                Radial::elliptical(
                    Point::new(layer.center.0, layer.center.1),
                    layer.radii.0,
                    layer.radii.1,
                ),
                |gradient, (offset, color)| gradient.add_stop(*offset, *color),
            );

            pin(container("")
                .width(case.width)
                .height(case.height)
                .style(move |_| container::background(Gradient::from(gradient))))
            .x(case.x)
            .y(case.y)
            .into()
        })
    });

    container(stack(layers))
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
    fn every_ellipse_is_drawn_as_chrome_draws_it() {
        let render = render();
        let chrome = decode(&Path::new(env!("CARGO_MANIFEST_DIR")).join("snapshots/chrome.png"));

        for (index, case) in CASES.iter().enumerate() {
            let (largest, mean) = compare(&render.rgba, &chrome, case);

            // Smooth gradients agree to a few levels of rounding; a circle
            // where an ellipse belongs is off by much of a stop's range.
            assert!(
                largest <= 8 && mean < 1.0,
                "case {index} ({}): a pixel differs from Chrome's by {largest}, \
                 {mean:.2} on average (render in {})",
                render.renderer,
                render.path.display(),
            );
        }
    }

    /// The largest difference from Chrome's render of a channel of a pixel in
    /// `case`, and the mean over all of them.
    fn compare(render: &[u8], chrome: &[u8], case: &Case) -> (u8, f32) {
        let stride = (SIZE.0 * SCALE) as usize * 4;
        let columns = (case.x * SCALE) as usize..((case.x + case.width) * SCALE) as usize;
        let rows = (case.y * SCALE) as usize..((case.y + case.height) * SCALE) as usize;

        let (mut largest, mut total, mut count) = (0, 0.0, 0);
        for y in rows {
            for x in columns.clone() {
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
        let directory = std::env::temp_dir().join("iced-elliptical-gradient");
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
