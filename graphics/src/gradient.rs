//! A gradient that can be used as a fill for some geometry.
//!
//! For a gradient that you can use as a background variant for a widget, see [`Gradient`].
use crate::color;
use crate::core::gradient::ColorStop;
use crate::core::{self, Color, Point, Rectangle};

use bytemuck::{Pod, Zeroable};
use half::f16;
use std::cmp::Ordering;

#[derive(Debug, Clone, Copy, PartialEq)]
/// A fill which linearly interpolates colors along a direction.
///
/// For a gradient which can be used as a fill for a background of a widget, see [`crate::core::Gradient`].
pub enum Gradient {
    /// A linear gradient interpolates colors along a direction from its `start` to its `end`
    /// point.
    Linear(Linear),
    /// A radial gradient interpolates colors in a circular pattern from a center point.
    Radial(Radial),
    /// A conic gradient interpolates colors around a center point.
    Conic(Conic),
}

impl From<Linear> for Gradient {
    fn from(gradient: Linear) -> Self {
        Self::Linear(gradient)
    }
}

impl From<Radial> for Gradient {
    fn from(gradient: Radial) -> Self {
        Self::Radial(gradient)
    }
}

impl From<Conic> for Gradient {
    fn from(gradient: Conic) -> Self {
        Self::Conic(gradient)
    }
}

impl Gradient {
    /// Packs the [`Gradient`] for use in shader code.
    pub fn pack(&self) -> Packed {
        match self {
            Gradient::Linear(linear) => linear.pack(),
            Gradient::Radial(radial) => radial.pack(),
            Gradient::Conic(conic) => conic.pack(),
        }
    }
}

/// A linear gradient.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Linear {
    /// The absolute starting position of the gradient.
    pub start: Point,

    /// The absolute ending position of the gradient.
    pub end: Point,

    /// [`ColorStop`]s along the linear gradient direction.
    pub stops: [Option<ColorStop>; 8],
}

impl Linear {
    /// Creates a new [`Linear`] builder.
    pub fn new(start: Point, end: Point) -> Self {
        Self {
            start,
            end,
            stops: [None; 8],
        }
    }

    /// Adds a new [`ColorStop`], defined by an offset and a color, to the gradient.
    ///
    /// Any `offset` that is not within `0.0..=1.0` will be silently ignored.
    ///
    /// Any stop added after the 8th will be silently ignored.
    pub fn add_stop(mut self, offset: f32, color: Color) -> Self {
        if offset.is_finite() && (0.0..=1.0).contains(&offset) {
            let (Ok(index) | Err(index)) = self.stops.binary_search_by(|stop| match stop {
                None => Ordering::Greater,
                Some(stop) => stop.offset.partial_cmp(&offset).unwrap(),
            });

            if index < 8 {
                self.stops[index] = Some(ColorStop { offset, color });
            }
        } else {
            log::warn!("Gradient: ColorStop must be within 0.0..=1.0 range.");
        };

        self
    }

    /// Adds multiple [`ColorStop`]s to the gradient.
    ///
    /// Any stop added after the 8th will be silently ignored.
    pub fn add_stops(mut self, stops: impl IntoIterator<Item = ColorStop>) -> Self {
        for stop in stops {
            self = self.add_stop(stop.offset, stop.color);
        }

        self
    }

    /// Packs the [`Gradient`] for use in shader code.
    pub fn pack(&self) -> Packed {
        let mut colors = [[0u32; 2]; 8];
        let mut offsets = [f16::from(0u8); 8];

        for (index, stop) in self.stops.iter().enumerate() {
            let [r, g, b, a] = color::pack(stop.map_or(Color::default(), |s| s.color)).components();

            colors[index] = [
                pack_f16s([f16::from_f32(r), f16::from_f32(g)]),
                pack_f16s([f16::from_f32(b), f16::from_f32(a)]),
            ];

            offsets[index] = stop.map_or(f16::from_f32(2.0), |s| f16::from_f32(s.offset));
        }

        let offsets = [
            pack_f16s([offsets[0], offsets[1]]),
            pack_f16s([offsets[2], offsets[3]]),
            pack_f16s([offsets[4], offsets[5]]),
            pack_f16s([offsets[6], offsets[7]]),
        ];

        let direction = [self.start.x, self.start.y, self.end.x, self.end.y];

        Packed {
            colors,
            offsets,
            direction,
            gradient_type: 0, // Linear
            _padding: [0; 3],
        }
    }
}

/// A radial gradient that interpolates colors in a circular pattern.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Radial {
    /// The center point of the gradient in absolute coordinates.
    pub center: Point,

    /// The horizontal radius of the ellipse.
    pub radius_x: f32,

    /// The vertical radius of the ellipse.
    pub radius_y: f32,

    /// [`ColorStop`]s along the radial gradient path.
    pub stops: [Option<ColorStop>; 8],
}

impl Radial {
    /// Creates a new [`Radial`] gradient with the given center and radius.
    pub fn new(center: Point, radius: f32) -> Self {
        Self {
            center,
            radius_x: radius,
            radius_y: radius,
            stops: [None; 8],
        }
    }

    /// Creates a new elliptical [`Radial`] gradient.
    pub fn elliptical(center: Point, radius_x: f32, radius_y: f32) -> Self {
        Self {
            center,
            radius_x,
            radius_y,
            stops: [None; 8],
        }
    }

    /// Adds a new [`ColorStop`], defined by an offset and a color, to the gradient.
    pub fn add_stop(mut self, offset: f32, color: Color) -> Self {
        if offset.is_finite() && (0.0..=1.0).contains(&offset) {
            let (Ok(index) | Err(index)) = self.stops.binary_search_by(|stop| match stop {
                None => Ordering::Greater,
                Some(stop) => stop.offset.partial_cmp(&offset).unwrap(),
            });

            if index < 8 {
                self.stops[index] = Some(ColorStop { offset, color });
            }
        } else {
            log::warn!("Gradient: ColorStop must be within 0.0..=1.0 range.");
        };

        self
    }

    /// Adds multiple [`ColorStop`]s to the gradient.
    pub fn add_stops(mut self, stops: impl IntoIterator<Item = ColorStop>) -> Self {
        for stop in stops {
            self = self.add_stop(stop.offset, stop.color);
        }

        self
    }

    /// Packs the [`Gradient`] for use in shader code.
    pub fn pack(&self) -> Packed {
        let mut colors = [[0u32; 2]; 8];
        let mut offsets = [f16::from(0u8); 8];

        for (index, stop) in self.stops.iter().enumerate() {
            let [r, g, b, a] = color::pack(stop.map_or(Color::default(), |s| s.color)).components();

            colors[index] = [
                pack_f16s([f16::from_f32(r), f16::from_f32(g)]),
                pack_f16s([f16::from_f32(b), f16::from_f32(a)]),
            ];

            offsets[index] = stop.map_or(f16::from_f32(2.0), |s| f16::from_f32(s.offset));
        }

        let offsets = [
            pack_f16s([offsets[0], offsets[1]]),
            pack_f16s([offsets[2], offsets[3]]),
            pack_f16s([offsets[4], offsets[5]]),
            pack_f16s([offsets[6], offsets[7]]),
        ];

        Packed {
            colors,
            offsets,
            // Pack radial params into direction: center.xy, radius.xy
            direction: [self.center.x, self.center.y, self.radius_x, self.radius_y],
            gradient_type: 1, // Radial
            _padding: [0; 3],
        }
    }
}

/// A conic (angular/sweep) gradient that interpolates colors around a center point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Conic {
    /// The center point of the gradient in absolute coordinates.
    pub center: Point,

    /// The starting angle of the gradient in radians.
    pub angle: f32,

    /// [`ColorStop`]s along the conic gradient path (0.0 = start angle, 1.0 = full rotation).
    pub stops: [Option<ColorStop>; 8],
}

impl Conic {
    /// Creates a new [`Conic`] gradient with the given center and starting angle.
    pub fn new(center: Point, angle: f32) -> Self {
        Self {
            center,
            angle,
            stops: [None; 8],
        }
    }

    /// Adds a new [`ColorStop`], defined by an offset and a color, to the gradient.
    pub fn add_stop(mut self, offset: f32, color: Color) -> Self {
        if offset.is_finite() && (0.0..=1.0).contains(&offset) {
            let (Ok(index) | Err(index)) = self.stops.binary_search_by(|stop| match stop {
                None => Ordering::Greater,
                Some(stop) => stop.offset.partial_cmp(&offset).unwrap(),
            });

            if index < 8 {
                self.stops[index] = Some(ColorStop { offset, color });
            }
        } else {
            log::warn!("Gradient: ColorStop must be within 0.0..=1.0 range.");
        };

        self
    }

    /// Adds multiple [`ColorStop`]s to the gradient.
    pub fn add_stops(mut self, stops: impl IntoIterator<Item = ColorStop>) -> Self {
        for stop in stops {
            self = self.add_stop(stop.offset, stop.color);
        }

        self
    }

    /// Packs the [`Gradient`] for use in shader code.
    pub fn pack(&self) -> Packed {
        let mut colors = [[0u32; 2]; 8];
        let mut offsets = [f16::from(0u8); 8];

        for (index, stop) in self.stops.iter().enumerate() {
            let [r, g, b, a] = color::pack(stop.map_or(Color::default(), |s| s.color)).components();

            colors[index] = [
                pack_f16s([f16::from_f32(r), f16::from_f32(g)]),
                pack_f16s([f16::from_f32(b), f16::from_f32(a)]),
            ];

            offsets[index] = stop.map_or(f16::from_f32(2.0), |s| f16::from_f32(s.offset));
        }

        let offsets = [
            pack_f16s([offsets[0], offsets[1]]),
            pack_f16s([offsets[2], offsets[3]]),
            pack_f16s([offsets[4], offsets[5]]),
            pack_f16s([offsets[6], offsets[7]]),
        ];

        Packed {
            colors,
            offsets,
            // Pack conic params into direction: center.xy, angle.z (w unused)
            direction: [self.center.x, self.center.y, self.angle, 0.0],
            gradient_type: 2, // Conic
            _padding: [0; 3],
        }
    }
}

/// Packed [`Gradient`] data for use in shader code.
#[derive(Debug, Copy, Clone, PartialEq, Zeroable, Pod)]
#[repr(C)]
pub struct Packed {
    // 8 colors, each channel = 16 bit float, 2 colors packed into 1 u32
    colors: [[u32; 2]; 8],
    // 8 offsets, 8x 16 bit floats packed into 4 u32s
    offsets: [u32; 4],
    // For linear: start.x, start.y, end.x, end.y
    // For radial: center.x, center.y, radius.x, radius.y
    // For conic: center.x, center.y, angle, 0.0
    direction: [f32; 4],
    // Gradient type: 0 = linear, 1 = radial, 2 = conic
    gradient_type: u32,
    // Padding to maintain 16-byte alignment (3 u32s)
    _padding: [u32; 3],
}

/// Creates a new [`Packed`] gradient for use in shader code.
pub fn pack(gradient: &core::Gradient, bounds: Rectangle) -> Packed {
    match gradient {
        core::Gradient::Linear(linear) => {
            let mut colors = [[0u32; 2]; 8];
            let mut offsets = [f16::from(0u8); 8];

            for (index, stop) in linear.stops.iter().enumerate() {
                let [r, g, b, a] =
                    color::pack(stop.map_or(Color::default(), |s| s.color)).components();

                colors[index] = [
                    pack_f16s([f16::from_f32(r), f16::from_f32(g)]),
                    pack_f16s([f16::from_f32(b), f16::from_f32(a)]),
                ];

                offsets[index] = stop.map_or(f16::from_f32(2.0), |s| f16::from_f32(s.offset));
            }

            let offsets = [
                pack_f16s([offsets[0], offsets[1]]),
                pack_f16s([offsets[2], offsets[3]]),
                pack_f16s([offsets[4], offsets[5]]),
                pack_f16s([offsets[6], offsets[7]]),
            ];

            let (start, end) = linear.angle.to_distance(&bounds);

            let direction = [start.x, start.y, end.x, end.y];

            Packed {
                colors,
                offsets,
                direction,
                gradient_type: 0, // Linear
                _padding: [0; 3],
            }
        }
        core::Gradient::Radial(radial) => {
            let mut colors = [[0u32; 2]; 8];
            let mut offsets = [f16::from(0u8); 8];

            for (index, stop) in radial.stops.iter().enumerate() {
                let [r, g, b, a] =
                    color::pack(stop.map_or(Color::default(), |s| s.color)).components();

                colors[index] = [
                    pack_f16s([f16::from_f32(r), f16::from_f32(g)]),
                    pack_f16s([f16::from_f32(b), f16::from_f32(a)]),
                ];

                offsets[index] = stop.map_or(f16::from_f32(2.0), |s| f16::from_f32(s.offset));
            }

            let offsets = [
                pack_f16s([offsets[0], offsets[1]]),
                pack_f16s([offsets[2], offsets[3]]),
                pack_f16s([offsets[4], offsets[5]]),
                pack_f16s([offsets[6], offsets[7]]),
            ];

            // Convert relative center and radius to absolute coordinates
            let center_x = bounds.x + radial.center.x * bounds.width;
            let center_y = bounds.y + radial.center.y * bounds.height;
            let radius_x = radial.radius_x * bounds.width;
            let radius_y = radial.radius_y * bounds.height;

            Packed {
                colors,
                offsets,
                // Pack radial params into direction: center.xy, radius.xy
                direction: [center_x, center_y, radius_x, radius_y],
                gradient_type: 1, // Radial
                _padding: [0; 3],
            }
        }
        core::Gradient::Conic(conic) => {
            let mut colors = [[0u32; 2]; 8];
            let mut offsets = [f16::from(0u8); 8];

            for (index, stop) in conic.stops.iter().enumerate() {
                let [r, g, b, a] =
                    color::pack(stop.map_or(Color::default(), |s| s.color)).components();

                colors[index] = [
                    pack_f16s([f16::from_f32(r), f16::from_f32(g)]),
                    pack_f16s([f16::from_f32(b), f16::from_f32(a)]),
                ];

                offsets[index] = stop.map_or(f16::from_f32(2.0), |s| f16::from_f32(s.offset));
            }

            let offsets = [
                pack_f16s([offsets[0], offsets[1]]),
                pack_f16s([offsets[2], offsets[3]]),
                pack_f16s([offsets[4], offsets[5]]),
                pack_f16s([offsets[6], offsets[7]]),
            ];

            // Convert relative center to absolute coordinates
            let center_x = bounds.x + conic.center.x * bounds.width;
            let center_y = bounds.y + conic.center.y * bounds.height;

            Packed {
                colors,
                offsets,
                // Pack conic params into direction: center.xy, angle.z (w unused)
                direction: [center_x, center_y, conic.angle.0, 0.0],
                gradient_type: 2, // Conic
                _padding: [0; 3],
            }
        }
    }
}

/// Packs two f16s into one u32.
fn pack_f16s(f: [f16; 2]) -> u32 {
    let one = (f[0].to_bits() as u32) << 16;
    let two = f[1].to_bits() as u32;

    one | two
}
