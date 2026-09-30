//! Box shadows, shaded the way CSS paints a `box-shadow`.
//!
//! The shadow's shape is the quad (its padding box, when inset) moved by the
//! offset and grown by the spread, and it is blurred by a Gaussian whose
//! standard deviation is half the blur radius. An outset shadow only shows
//! outside the quad; an inset one fills the padding box outside that shape.
use crate::core::renderer::Quad;
use crate::core::{Rectangle, Shadow, Transformation};

/// A standard deviation below which a blur is too small to see, in physical
/// pixels; such a shadow is drawn sharp, with the fill's anti-aliasing.
const SHARP: f32 = 0.25;

/// How far past its quad an outset `shadow` paints, before its offset: its
/// spread and three standard deviations (half the blur radius each) of its
/// blur, which hold all but a thousandth of it.
pub fn reach(shadow: &Shadow) -> f32 {
    if shadow.inset {
        return 0.0;
    }

    1.5 * shadow.blur_radius.max(0.0) + shadow.spread_radius.max(0.0)
}

/// The area `quad`'s shadow covers, in its own coordinates.
pub fn bounds(quad: &Quad) -> Rectangle {
    let shadow = quad.shadow;

    if shadow.inset {
        return quad.bounds;
    }

    let reach = reach(&shadow);

    Rectangle {
        x: quad.bounds.x + shadow.offset.x - reach,
        y: quad.bounds.y + shadow.offset.y - reach,
        width: quad.bounds.width + reach * 2.0,
        height: quad.bounds.height + reach * 2.0,
    }
}

/// The part of `quad`'s shadow inside `clip_bounds`, and where it goes.
///
/// `radii` are the quad's corner radii, already fitted to it, and `bounds` is
/// [`bounds`] under `transformation`.
///
/// Shaded per pixel on every draw, so only the damaged part is: shading all of a
/// menu's shadow for a caret blink cost ~88ms a frame at 2x in a debug build.
pub fn pixmap(
    quad: &Quad,
    bounds: Rectangle,
    radii: [f32; 4],
    transformation: Transformation,
    clip_bounds: Rectangle,
) -> Option<(i32, i32, tiny_skia::Pixmap)> {
    let shadow = quad.shadow;
    let scale = transformation.scale_factor();
    let physical = quad.bounds * transformation;
    let radii = radii.map(|radius| radius * scale);
    let offset = (shadow.offset.x * scale, shadow.offset.y * scale);
    let spread = shadow.spread_radius * scale;
    let sigma = shadow.blur_radius.max(0.0) * scale / 2.0;

    // The shadow's pixel grid, cut down to the damage.
    let (x0, y0) = (bounds.x as u32, bounds.y as u32);
    let x1 = (bounds.x + bounds.width).max(0.0).ceil() as u32;
    let y1 = (bounds.y + bounds.height).max(0.0).ceil() as u32;
    let left = x0.max(clip_bounds.x.max(0.0).floor() as u32);
    let top = y0.max(clip_bounds.y.max(0.0).floor() as u32);
    let right = x1.min((clip_bounds.x + clip_bounds.width).max(0.0).ceil() as u32);
    let bottom = y1.min((clip_bounds.y + clip_bounds.height).max(0.0).ceil() as u32);
    let region = tiny_skia::IntSize::from_wh(right.checked_sub(left)?, bottom.checked_sub(top)?)?;

    let quad_shape = Shape {
        lo: (physical.x, physical.y),
        hi: (physical.x + physical.width, physical.y + physical.height),
        rx: radii,
        ry: radii,
    };

    let alphas = if shadow.inset {
        let [top_width, right_width, bottom_width, left_width] =
            quad.border.widths().map(|width| width.max(0.0) * scale);

        // The padding box: inside the border, its corners shrunk by the width of
        // the side they meet on each axis.
        let padding = Shape {
            lo: (physical.x + left_width, physical.y + top_width),
            hi: (
                physical.x + physical.width - right_width,
                physical.y + physical.height - bottom_width,
            ),
            rx: [
                radii[0] - left_width,
                radii[1] - right_width,
                radii[2] - right_width,
                radii[3] - left_width,
            ]
            .map(|radius| radius.max(0.0)),
            ry: [
                radii[0] - top_width,
                radii[1] - top_width,
                radii[2] - bottom_width,
                radii[3] - bottom_width,
            ]
            .map(|radius| radius.max(0.0)),
        };
        let hole = padding.moved(offset).spread(-spread);

        inset(&padding, &hole, sigma, (left, top), region)
    } else {
        let shape = quad_shape.moved(offset).spread(spread);

        outset(&quad_shape, &shape, sigma, (left, top), region)
    };

    // Only alpha varies across a shadow, and it lands on one of 256 values once
    // rounded, so each pixel is a lookup rather than a colour conversion.
    let color = crate::engine::into_color(shadow.color);
    let rgb = color.to_color_u8();
    let shades: [tiny_skia::PremultipliedColorU8; 256] = std::array::from_fn(|alpha| {
        tiny_skia::ColorU8::from_rgba(rgb.red(), rgb.green(), rgb.blue(), alpha as u8).premultiply()
    });
    let opacity = color.alpha() * 255.0;

    let colors: Vec<tiny_skia::PremultipliedColorU8> = alphas
        .into_iter()
        .map(|alpha| shades[(alpha.clamp(0.0, 1.0) * opacity + 0.5) as usize])
        .collect();

    let pixmap = tiny_skia::Pixmap::from_vec(bytemuck::cast_vec(colors), region)?;

    Some((left as i32, top as i32, pixmap))
}

/// An outset shadow's coverage of `region` (at `origin`): `shape` blurred by
/// `sigma`, and none of it under the quad.
fn outset(
    quad: &Shape,
    shape: &Shape,
    sigma: f32,
    origin: (u32, u32),
    region: tiny_skia::IntSize,
) -> Vec<f32> {
    let (width, height) = (region.width() as usize, region.height() as usize);
    let mut alphas = vec![0.0; width * height];
    let blurred = (sigma >= SHARP).then(|| Blur::new(shape, sigma, origin.0, width));

    // A pixel a whole pixel inside the quad, clear of its corners, is covered
    // by it whatever the shadow, so each row skips that span.
    let max_radius = quad.rx.iter().copied().fold(0.0, f32::max);
    let interior = (quad.lo.0 + max_radius + 1.0, quad.hi.0 - max_radius - 1.0);

    for (row, alphas) in alphas.chunks_exact_mut(width).enumerate() {
        let y = (origin.1 + row as u32) as f32 + 0.5;
        let column = |x: f32| ((x - origin.0 as f32).max(0.0) as usize).min(width);
        let (from, to) = if y > quad.lo.1 + 1.0 && y < quad.hi.1 - 1.0 {
            let from = column(interior.0.ceil());
            (from, column(interior.1.floor()).max(from))
        } else {
            (width, width)
        };

        for span in [0..from, to..width] {
            match &blurred {
                Some(blurred) => blurred.row(y, &mut alphas[span.clone()], span.start),
                None => {
                    for (x, alpha) in span.clone().zip(&mut alphas[span.clone()]) {
                        let x = (origin.0 + x as u32) as f32 + 0.5;
                        *alpha = shape.coverage(x, y);
                    }
                }
            }

            if y < quad.lo.1 - 1.0 || y > quad.hi.1 + 1.0 {
                continue;
            }

            for (x, alpha) in span.clone().zip(&mut alphas[span]) {
                let x = (origin.0 + x as u32) as f32 + 0.5;

                if x > quad.lo.0 - 1.0 && x < quad.hi.0 + 1.0 {
                    *alpha *= 1.0 - coverage(quad.distance(x, y));
                }
            }
        }
    }

    alphas
}

/// An inset shadow's coverage of `region` (at `origin`): the `padding` box
/// outside `hole` blurred by `sigma`.
fn inset(
    padding: &Shape,
    hole: &Shape,
    sigma: f32,
    origin: (u32, u32),
    region: tiny_skia::IntSize,
) -> Vec<f32> {
    let (width, height) = (region.width() as usize, region.height() as usize);
    let mut alphas = vec![0.0; width * height];
    let blurred = (sigma >= SHARP).then(|| Blur::new(hole, sigma, origin.0, width));

    for (row, alphas) in alphas.chunks_exact_mut(width).enumerate() {
        let y = (origin.1 + row as u32) as f32 + 0.5;

        if let Some(blurred) = &blurred {
            blurred.row(y, alphas, 0);
        }

        for (x, alpha) in alphas.iter_mut().enumerate() {
            let x = (origin.0 + x as u32) as f32 + 0.5;
            let inside = coverage(padding.distance(x, y));

            *alpha = if blurred.is_some() {
                inside * (1.0 - *alpha)
            } else {
                // Coincident edges cancel rather than casting a halo.
                (inside - hole.coverage(x, y)).max(0.0)
            };
        }
    }

    alphas
}

/// Coverage of a pixel whose centre is `distance` outside an edge.
fn coverage(distance: f32) -> f32 {
    (0.5 - distance).clamp(0.0, 1.0)
}

/// A box whose corners are quarter ellipses, in physical pixels.
#[derive(Debug, Clone, Copy)]
struct Shape {
    lo: (f32, f32),
    hi: (f32, f32),
    /// Horizontal radii: top-left, top-right, bottom-right, bottom-left.
    rx: [f32; 4],
    /// Vertical radii, in the same order.
    ry: [f32; 4],
}

impl Shape {
    fn moved(self, (x, y): (f32, f32)) -> Self {
        Self {
            lo: (self.lo.0 + x, self.lo.1 + y),
            hi: (self.hi.0 + x, self.hi.1 + y),
            ..self
        }
    }

    /// The shape grown by `outset` on every side (shrunk when negative), with
    /// the corner radii CSS gives a spread shadow.
    fn spread(self, outset: f32) -> Self {
        Self {
            lo: (self.lo.0 - outset, self.lo.1 - outset),
            hi: (self.hi.0 + outset, self.hi.1 + outset),
            rx: self.rx.map(|radius| spread_radius(radius, outset)),
            ry: self.ry.map(|radius| spread_radius(radius, outset)),
        }
    }

    fn is_empty(&self) -> bool {
        self.hi.0 <= self.lo.0 || self.hi.1 <= self.lo.1
    }

    /// The horizontal extent of the shape at height `y`.
    fn span(&self, y: f32) -> (f32, f32) {
        let (top, bottom) = (y - self.lo.1, self.hi.1 - y);
        let inset = |corner: usize, depth: f32| {
            let (rx, ry) = (self.rx[corner], self.ry[corner]);

            if depth >= ry || rx <= 0.0 {
                return 0.0;
            }

            let t = 1.0 - depth / ry;
            rx * (1.0 - (1.0 - t * t).max(0.0).sqrt())
        };

        (
            self.lo.0 + inset(0, top).max(inset(3, bottom)),
            self.hi.0 - inset(1, top).max(inset(2, bottom)),
        )
    }

    /// Signed distance from (`x`, `y`) to the shape's outline: exact along its
    /// sides, and a first-order estimate that is exact on the curve at a corner.
    fn distance(&self, x: f32, y: f32) -> f32 {
        let half = ((self.hi.0 - self.lo.0) / 2.0, (self.hi.1 - self.lo.1) / 2.0);
        let p = (x - (self.lo.0 + half.0), y - (self.lo.1 + half.1));
        let corner = match (p.0 > 0.0, p.1 > 0.0) {
            (false, false) => 0,
            (true, false) => 1,
            (true, true) => 2,
            (false, true) => 3,
        };
        let (rx, ry) = (self.rx[corner], self.ry[corner]);
        let q = (p.0.abs() - half.0 + rx, p.1.abs() - half.1 + ry);

        if rx > 0.0 && ry > 0.0 && q.0 > 0.0 && q.1 > 0.0 {
            let k0 = ((q.0 / rx).powi(2) + (q.1 / ry).powi(2)).sqrt();
            let k1 = ((q.0 / (rx * rx)).powi(2) + (q.1 / (ry * ry)).powi(2)).sqrt();

            return k0 * (k0 - 1.0) / k1;
        }

        let d = (p.0.abs() - half.0, p.1.abs() - half.1);

        d.0.max(d.1).min(0.0) + (d.0.max(0.0).powi(2) + d.1.max(0.0).powi(2)).sqrt()
    }

    fn coverage(&self, x: f32, y: f32) -> f32 {
        if self.is_empty() {
            0.0
        } else {
            coverage(self.distance(x, y))
        }
    }
}

/// CSS's radius for a corner of a shadow grown by `outset` (shrunk when
/// negative): a corner tighter than the outset grows by less, so a square
/// corner stays square.
fn spread_radius(radius: f32, outset: f32) -> f32 {
    if outset > 0.0 && radius < outset {
        let r = radius / outset - 1.0;

        radius + outset * (1.0 + r * r * r)
    } else {
        (radius + outset).max(0.0)
    }
}

/// A [`Shape`] blurred by a Gaussian, shaded a row at a time over a fixed set
/// of columns.
///
/// Rows clear of the corners separate into a column factor and a row factor, so
/// they integrate exactly. The rows a corner curves through are cut into slabs
/// no taller than a third of a standard deviation, each of which separates in
/// turn; a pixel sums the few slabs within four deviations of its row.
struct Blur {
    sigma: f32,
    /// The straight rows, and each column's share of them.
    straight: Option<((f32, f32), Vec<f32>)>,
    slabs: Vec<Slab>,
    /// Each slab's column shares, one run of `width` per slab.
    columns: Vec<f32>,
    width: usize,
}

struct Slab {
    top: f32,
    bottom: f32,
}

impl Blur {
    fn new(shape: &Shape, sigma: f32, left: u32, width: usize) -> Self {
        let empty = Self {
            sigma,
            straight: None,
            slabs: Vec::new(),
            columns: Vec::new(),
            width,
        };

        if shape.is_empty() {
            return empty;
        }

        // Past four deviations from both ends a column sees all of a span or
        // none of it.
        let reach = 4.0 * sigma;
        let share = |(from, to): (f32, f32), column: usize| {
            let x = (left + column as u32) as f32 + 0.5;

            if x < from - reach || x > to + reach {
                0.0
            } else if x > from + reach && x < to - reach {
                1.0
            } else {
                normal_cdf((to - x) / sigma) - normal_cdf((from - x) / sigma)
            }
        };

        let (lo, hi) = (shape.lo.1, shape.hi.1);
        let top = shape.ry[0].max(shape.ry[1]);
        let bottom = shape.ry[2].max(shape.ry[3]);

        let (curved, straight) = if top + bottom >= hi - lo {
            (vec![(lo, hi)], None)
        } else {
            (
                vec![(lo, lo + top), (hi - bottom, hi)],
                Some((lo + top, hi - bottom)),
            )
        };

        let mut slabs = Vec::new();
        let mut columns = Vec::new();

        for (from, to) in curved {
            if to <= from {
                continue;
            }

            let count = ((to - from) / (sigma / 3.0)).ceil().clamp(8.0, 768.0) as usize;
            let height = (to - from) / count as f32;

            for i in 0..count {
                let slab = Slab {
                    top: from + i as f32 * height,
                    bottom: from + (i + 1) as f32 * height,
                };
                let spans = [
                    shape.span(slab.top),
                    shape.span((slab.top + slab.bottom) / 2.0),
                    shape.span(slab.bottom),
                ];

                // Simpson's rule across the slab tames the corner flattening out
                // along its edge.
                columns.extend((0..width).map(|column| {
                    (share(spans[0], column)
                        + 4.0 * share(spans[1], column)
                        + share(spans[2], column))
                        / 6.0
                }));
                slabs.push(slab);
            }
        }

        Self {
            straight: straight.map(|rows| {
                let span = (shape.lo.0, shape.hi.0);
                (rows, (0..width).map(|column| share(span, column)).collect())
            }),
            slabs,
            columns,
            ..empty
        }
    }

    /// Shades the row at height `y` into `alphas`, which start at column `from`.
    fn row(&self, y: f32, alphas: &mut [f32], from: usize) {
        let weight = |top: f32, bottom: f32| {
            normal_cdf((bottom - y) / self.sigma) - normal_cdf((top - y) / self.sigma)
        };
        let to = from + alphas.len();

        match &self.straight {
            Some(((top, bottom), shares)) => {
                let weight = weight(*top, *bottom);

                for (alpha, share) in alphas.iter_mut().zip(&shares[from..to]) {
                    *alpha = weight * share;
                }
            }
            None => alphas.fill(0.0),
        }

        let reach = 4.0 * self.sigma;

        for (slab, shares) in self.slabs.iter().zip(self.columns.chunks_exact(self.width)) {
            if slab.bottom < y - reach || slab.top > y + reach {
                continue;
            }

            let weight = weight(slab.top, slab.bottom);

            for (alpha, share) in alphas.iter_mut().zip(&shares[from..to]) {
                *alpha += weight * share;
            }
        }
    }
}

/// The standard normal CDF, from Abramowitz and Stegun's erf 7.1.26 (error
/// under 1.5e-7).
fn normal_cdf(x: f32) -> f32 {
    let z = x.abs() * std::f32::consts::FRAC_1_SQRT_2;
    let t = 1.0 / (1.0 + 0.327_591_1 * z);
    let polynomial = t
        * (0.254_829_6
            + t * (-0.284_496_74 + t * (1.421_413_7 + t * (-1.453_152 + t * 1.061_405_4))));
    let erf = 1.0 - polynomial * (-z * z).exp();

    if x >= 0.0 {
        0.5 + 0.5 * erf
    } else {
        0.5 - 0.5 * erf
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Border, Color, Vector};

    fn quad(shadow: Shadow, radius: f32) -> Quad {
        Quad {
            bounds: Rectangle {
                x: 40.0,
                y: 30.0,
                width: 120.0,
                height: 80.0,
            },
            border: Border {
                radius: radius.into(),
                ..Border::default()
            },
            shadow,
            ..Quad::default()
        }
    }

    fn black(offset: (f32, f32), blur: f32, spread: f32) -> Shadow {
        Shadow {
            color: Color::BLACK,
            offset: Vector::new(offset.0, offset.1),
            blur_radius: blur,
            spread_radius: spread,
            inset: false,
        }
    }

    /// The shadow's alpha at each pixel, and where its pixmap starts.
    fn shade(quad: &Quad) -> (i32, i32, tiny_skia::Pixmap) {
        let transformation = Transformation::IDENTITY;
        let radii = [quad.border.radius.top_left; 4];
        let everything = Rectangle {
            x: 0.0,
            y: 0.0,
            width: 1000.0,
            height: 1000.0,
        };

        pixmap(
            quad,
            bounds(quad) * transformation,
            radii,
            transformation,
            everything,
        )
        .unwrap()
    }

    /// Off a straight edge a blur falls off as a Gaussian whose standard
    /// deviation is half the blur radius, as CSS specifies.
    #[test]
    fn a_straight_edge_falls_off_as_half_the_blur_radius() {
        let quad = quad(black((0.0, 0.0), 24.0, 0.0), 0.0);
        let (x, y, pixmap) = shade(&quad);
        let row = (70 - y) as u32;

        for column in 160..200 {
            let distance = column as f32 + 0.5 - 160.0;
            let expected = 1.0 - normal_cdf(distance / 12.0);
            let alpha = pixmap.pixel((column - x) as u32, row).unwrap().alpha();

            assert!(
                (f32::from(alpha) - expected * 255.0).abs() <= 1.0,
                "{distance}px out: {alpha}, not {}",
                expected * 255.0
            );
        }
    }

    /// The slabs a corner is cut into add up to what integrating the blur over
    /// the shape directly gives.
    #[test]
    fn a_blurred_corner_matches_a_direct_integration() {
        let shapes = [
            Shape {
                lo: (20.0, 15.0),
                hi: (100.0, 75.0),
                rx: [0.0, 8.0, 20.0, 30.0],
                ry: [0.0, 8.0, 20.0, 30.0],
            },
            Shape {
                lo: (20.0, 15.0),
                hi: (100.0, 75.0),
                rx: [12.0, 12.0, 4.0, 4.0],
                ry: [8.0, 8.0, 10.0, 10.0],
            },
        ];

        for shape in shapes {
            for sigma in [0.5, 2.0, 8.0, 16.0] {
                let blur = Blur::new(&shape, sigma, 0, 120);
                let mut row = vec![0.0; 120];

                for y in (0..90).step_by(3) {
                    let y = y as f32 + 0.5;
                    blur.row(y, &mut row, 0);

                    for x in (0..120).step_by(3) {
                        let expected = integrate(&shape, sigma, x as f32 + 0.5, y);

                        assert!(
                            (row[x] - expected).abs() * 255.0 < 1.5,
                            "sigma {sigma} at ({x}, {y}): {} instead of {expected}",
                            row[x]
                        );
                    }
                }
            }
        }
    }

    /// The blur of `shape` at (`x`, `y`), integrated row by row in fine steps.
    fn integrate(shape: &Shape, sigma: f32, x: f32, y: f32) -> f32 {
        let from = shape.lo.1.max(y - 6.0 * sigma);
        let to = shape.hi.1.min(y + 6.0 * sigma);
        let steps = 2000;
        let step = (to - from) / steps as f32;

        (0..steps)
            .map(|i| {
                let top = from + i as f32 * step;
                let (left, right) = shape.span(top + step / 2.0);
                let weight = normal_cdf((top + step - y) / sigma) - normal_cdf((top - y) / sigma);

                weight * (normal_cdf((right - x) / sigma) - normal_cdf((left - x) / sigma))
            })
            .sum()
    }

    /// A spread shadow's corners follow CSS: a square corner stays square, and
    /// one tighter than the spread grows by less than it.
    #[test]
    fn spread_corners_grow_as_css_grows_them() {
        assert_eq!(spread_radius(0.0, 12.0), 0.0);
        assert!((spread_radius(4.0, 12.0) - 12.444).abs() < 1e-3);
        assert_eq!(spread_radius(20.0, 8.0), 28.0);
        assert_eq!(spread_radius(24.0, -8.0), 16.0);
        assert_eq!(spread_radius(4.0, -8.0), 0.0);
    }

    /// An outset shadow shows only outside its quad, even under a clear fill.
    #[test]
    fn an_outset_shadow_is_not_painted_under_its_quad() {
        let quad = quad(black((0.0, 6.0), 16.0, 4.0), 12.0);
        let (x, y, pixmap) = shade(&quad);

        for (column, row) in [(100, 70), (45, 80), (155, 105), (100, 108)] {
            let pixel = pixmap.pixel((column - x) as u32, (row - y) as u32).unwrap();

            assert_eq!(pixel.alpha(), 0, "({column}, {row})");
        }
        assert!(
            pixmap
                .pixel((100 - x) as u32, (113 - y) as u32)
                .unwrap()
                .alpha()
                > 100
        );
    }

    /// An inset shadow whose hole is the padding box itself paints nothing,
    /// not even a halo along the edge.
    #[test]
    fn an_inset_shadow_without_offset_blur_or_spread_is_invisible() {
        let shadow = Shadow {
            inset: true,
            ..black((0.0, 0.0), 0.0, 0.0)
        };

        for scale in [1.0, 1.5, 2.0] {
            let quad = quad(shadow, 12.0);
            let transformation = Transformation::scale(scale);
            let everything = Rectangle {
                x: 0.0,
                y: 0.0,
                width: 1000.0,
                height: 1000.0,
            };
            let (_, _, pixmap) = pixmap(
                &quad,
                bounds(&quad) * transformation,
                [12.0; 4],
                transformation,
                everything,
            )
            .unwrap();

            assert!(
                pixmap.pixels().iter().all(|pixel| pixel.alpha() == 0),
                "{scale}x"
            );
        }
    }
}
