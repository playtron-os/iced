//! Shadows next to what Chrome draws for the same CSS `box-shadow`
//! (`chrome.html`, rendered in `snapshots/chrome.png`).
//!
//! The tests draw the scene at a scale factor of 2 and compare it with Chrome's
//! render, pixel by pixel.
use iced::gradient;
use iced::widget::{container, pin, stack};
use iced::{Background, Border, Color, Element, Fill, Radians, Shadow, Vector, color};

pub fn main() -> iced::Result {
    iced::application(|| (), |_: &mut (), _: ()| {}, view)
        .window_size(SIZE)
        .run()
}

fn view(_state: &()) -> Element<'_, ()> {
    scene()
}

/// The logical size of the scene.
const SIZE: (f32, f32) = (720.0, 400.0);

/// Every card's size.
const CARD: (f32, f32) = (140.0, 90.0);

const PAGE: Color = color!(0xf2f2f5);

/// A card of the scene and its shadow.
struct Case {
    x: f32,
    y: f32,
    /// Top-left, top-right, bottom-right, bottom-left.
    radius: [f32; 4],
    background: Color,
    /// Where a top-to-bottom gradient from `background` ends, if the card has one.
    gradient: Option<Color>,
    border: (f32, Color),
    shadow: Shadow,
}

const fn shadow(offset: (f32, f32), blur: f32, spread: f32, color: Color) -> Shadow {
    Shadow {
        color,
        offset: Vector::new(offset.0, offset.1),
        blur_radius: blur,
        spread_radius: spread,
        inset: false,
    }
}

const fn inset(offset: (f32, f32), blur: f32, spread: f32, color: Color) -> Shadow {
    Shadow {
        inset: true,
        ..shadow(offset, blur, spread, color)
    }
}

const WHITE: Color = Color::WHITE;
const NO_BORDER: (f32, Color) = (0.0, Color::TRANSPARENT);

/// Keep in sync with `chrome.html`.
const CASES: [Case; 8] = [
    // `--shadow-window`: 0 8px 32px -4px.
    Case {
        x: 40.0,
        y: 50.0,
        radius: [12.0; 4],
        background: WHITE,
        gradient: None,
        border: NO_BORDER,
        shadow: shadow((0.0, 8.0), 32.0, -4.0, color!(0x000000, 0.5)),
    },
    // `--shadow-popover`: 0 12px 28px -10px, on a gradient.
    Case {
        x: 210.0,
        y: 50.0,
        radius: [8.0; 4],
        background: WHITE,
        gradient: Some(color!(0xdde6fb)),
        border: NO_BORDER,
        shadow: shadow((0.0, 12.0), 28.0, -10.0, color!(0x000000, 0.6)),
    },
    // A spread ring around square corners stays square.
    Case {
        x: 380.0,
        y: 50.0,
        radius: [0.0; 4],
        background: WHITE,
        gradient: None,
        border: NO_BORDER,
        shadow: shadow((0.0, 0.0), 0.0, 12.0, color!(0x1478ff, 0.5)),
    },
    // Spread wider than the corners, blurred.
    Case {
        x: 550.0,
        y: 50.0,
        radius: [4.0; 4],
        background: WHITE,
        gradient: None,
        border: NO_BORDER,
        shadow: shadow((0.0, 0.0), 16.0, 8.0, color!(0x000000, 0.4)),
    },
    // A translucent card does not show its shadow through itself.
    Case {
        x: 40.0,
        y: 250.0,
        radius: [12.0; 4],
        background: color!(0xffffff, 0.3),
        gradient: None,
        border: NO_BORDER,
        shadow: shadow((0.0, 4.0), 16.0, 0.0, color!(0x000000, 0.6)),
    },
    // An inset shadow sits inside the border.
    Case {
        x: 210.0,
        y: 250.0,
        radius: [12.0; 4],
        background: WHITE,
        gradient: None,
        border: (2.0, color!(0x8a8a96)),
        shadow: inset((0.0, 4.0), 12.0, 0.0, color!(0x000000, 0.5)),
    },
    // A sharp inset: a rim along the top.
    Case {
        x: 380.0,
        y: 250.0,
        radius: [10.0; 4],
        background: WHITE,
        gradient: None,
        border: NO_BORDER,
        shadow: inset((0.0, 2.0), 0.0, 0.0, color!(0xe03030)),
    },
    // Every corner different.
    Case {
        x: 550.0,
        y: 250.0,
        radius: [10.0, 30.0, 0.0, 20.0],
        background: WHITE,
        gradient: None,
        border: NO_BORDER,
        shadow: shadow((0.0, 6.0), 20.0, 0.0, color!(0x000000, 0.45)),
    },
];

fn scene<'a>() -> Element<'a, ()> {
    let cards = CASES.iter().map(|case| {
        let [top_left, top_right, bottom_right, bottom_left] = case.radius;
        let border = Border::default()
            .width(case.border.0)
            .color(case.border.1)
            .rounded(iced::border::Radius {
                top_left,
                top_right,
                bottom_right,
                bottom_left,
            });
        let background = match case.gradient {
            Some(end) => Background::Gradient(
                gradient::Linear::new(Radians::PI)
                    .add_stop(0.0, case.background)
                    .add_stop(1.0, end)
                    .into(),
            ),
            None => Background::Color(case.background),
        };
        let shadow = case.shadow;

        pin(container("")
            .width(CARD.0)
            .height(CARD.1)
            .style(move |_| container::Style {
                background: Some(background),
                border,
                shadow,
                ..container::Style::default()
            }))
        .x(case.x)
        .y(case.y)
        .into()
    });

    container(stack(cards))
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
    fn every_shadow_is_drawn_as_chrome_draws_it() {
        let render = render();
        let chrome = decode(&Path::new(env!("CARGO_MANIFEST_DIR")).join("snapshots/chrome.png"));

        for (index, case) in CASES.iter().enumerate() {
            let Difference {
                largest,
                mean,
                visible,
            } = compare(&render.rgba, &chrome, case);

            // A shadow's falloff agrees to a couple of levels; only the edges of a
            // sharp inset, anti-aliased by each rasterizer its own way, stray
            // further, and only in a few pixels at their corners.
            assert!(
                mean < 0.6 && visible <= 32 && largest <= 48,
                "case {index} ({}): {visible} pixels differ visibly from Chrome's, \
                 one by {largest}, {mean:.2} on average (render in {})",
                render.renderer,
                render.path.display(),
            );
        }
    }

    /// How the pixels the shadow of `case` reaches differ from Chrome's, channel
    /// by channel.
    struct Difference {
        largest: u8,
        mean: f32,
        /// How many pixels differ by more than 12.
        visible: usize,
    }

    /// Compares the pixels the shadow of `case` reaches with Chrome's render.
    ///
    /// Pixels on the card's own edge are left out: how a curve is anti-aliased
    /// differs between rasterizers, and that is not the shadow.
    fn compare(render: &[u8], chrome: &[u8], case: &Case) -> Difference {
        let shadow = case.shadow;
        let reach = 1.5 * shadow.blur_radius
            + shadow.spread_radius.max(0.0)
            + shadow.offset.x.abs().max(shadow.offset.y.abs())
            + 1.0;
        let stride = (SIZE.0 * SCALE) as usize * 4;
        let columns = ((case.x - reach) * SCALE).max(0.0) as usize
            ..((case.x + CARD.0 + reach) * SCALE).min(SIZE.0 * SCALE) as usize;
        let rows = ((case.y - reach) * SCALE).max(0.0) as usize
            ..((case.y + CARD.1 + reach) * SCALE).min(SIZE.1 * SCALE) as usize;

        let (mut largest, mut total, mut count, mut visible) = (0, 0.0, 0, 0);
        for y in rows {
            for x in columns.clone() {
                let center = ((x as f32 + 0.5) / SCALE, (y as f32 + 0.5) / SCALE);
                if outline_distance(case, center).abs() * SCALE <= 1.0 {
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
                visible += usize::from(delta > 12);
            }
        }

        Difference {
            largest,
            mean: total / count as f32,
            visible,
        }
    }

    /// Signed distance from `point` to the card's rounded outline.
    fn outline_distance(case: &Case, point: (f32, f32)) -> f32 {
        let half = (CARD.0 / 2.0, CARD.1 / 2.0);
        let p = (point.0 - case.x - half.0, point.1 - case.y - half.1);
        let radius = match (p.0 > 0.0, p.1 > 0.0) {
            (false, false) => case.radius[0],
            (true, false) => case.radius[1],
            (true, true) => case.radius[2],
            (false, true) => case.radius[3],
        };
        let q = (p.0.abs() - half.0 + radius, p.1.abs() - half.1 + radius);

        q.0.max(q.1).min(0.0) + q.0.max(0.0).hypot(q.1.max(0.0)) - radius
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
        let directory = std::env::temp_dir().join("iced-box-shadow");
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
