//! Colors that transition progressively.
use crate::{Color, Point, Radians};

use std::cmp::Ordering;

#[derive(Debug, Clone, Copy, PartialEq)]
/// A fill which transitions colors progressively along a direction, either linearly, radially,
/// or conically.
pub enum Gradient {
    /// A linear gradient interpolates colors along a direction at a specific angle.
    Linear(Linear),
    /// A radial gradient interpolates colors in a circular pattern from a center point.
    Radial(Radial),
    /// A conic gradient interpolates colors around a center point (like a color wheel).
    Conic(Conic),
}

impl Gradient {
    /// Scales the alpha channel of the [`Gradient`] by the given factor.
    pub fn scale_alpha(self, factor: f32) -> Self {
        match self {
            Gradient::Linear(linear) => Gradient::Linear(linear.scale_alpha(factor)),
            Gradient::Radial(radial) => Gradient::Radial(radial.scale_alpha(factor)),
            Gradient::Conic(conic) => Gradient::Conic(conic.scale_alpha(factor)),
        }
    }

    /// Linearly interpolates between two [`Gradient`]s by the given amount.
    ///
    /// Returns `None` when the two gradients have no meaningful in-between:
    /// different variants, or stop layouts that don't line up. A caller
    /// animating a fill should step to the target in that case — collapsing an
    /// un-blendable pair to a flat color makes the fill vanish mid-transition.
    pub fn lerp(self, other: Self, amount: f32) -> Option<Self> {
        match (self, other) {
            (Gradient::Linear(from), Gradient::Linear(to)) => {
                from.lerp(to, amount).map(Gradient::Linear)
            }
            (Gradient::Radial(from), Gradient::Radial(to)) => {
                from.lerp(to, amount).map(Gradient::Radial)
            }
            (Gradient::Conic(from), Gradient::Conic(to)) => {
                from.lerp(to, amount).map(Gradient::Conic)
            }
            _ => None,
        }
    }
}

/// Linearly interpolates between two scalars by the given amount.
fn lerp(from: f32, to: f32, amount: f32) -> f32 {
    from + (to - from) * amount
}

/// Linearly interpolates two [`Radians`] by the given amount.
fn lerp_angle(from: Radians, to: Radians, amount: f32) -> Radians {
    Radians(lerp(from.0, to.0, amount))
}

/// Linearly interpolates two [`Point`]s by the given amount.
fn lerp_point(from: Point, to: Point, amount: f32) -> Point {
    Point::new(lerp(from.x, to.x, amount), lerp(from.y, to.y, amount))
}

/// Linearly interpolates two stop lists, or `None` when they don't line up.
///
/// A stop appearing or disappearing has no midpoint — the gradient would have to
/// gain or lose a band partway through — so those pairs are rejected outright.
fn lerp_stops(
    from: &[Option<ColorStop>; 8],
    to: &[Option<ColorStop>; 8],
    amount: f32,
) -> Option<[Option<ColorStop>; 8]> {
    let mut stops = [None; 8];

    for (stop, (from, to)) in stops.iter_mut().zip(from.iter().zip(to.iter())) {
        *stop = match (from, to) {
            (Some(from), Some(to)) => Some(ColorStop {
                offset: lerp(from.offset, to.offset, amount),
                color: from.color.lerp(to.color, amount),
            }),
            (None, None) => None,
            _ => return None,
        };
    }

    Some(stops)
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

#[derive(Debug, Default, Clone, Copy, PartialEq)]
/// A point along the gradient vector where the specified [`color`] is unmixed.
///
/// [`color`]: Self::color
pub struct ColorStop {
    /// Offset along the gradient vector.
    pub offset: f32,

    /// The color of the gradient at the specified [`offset`].
    ///
    /// [`offset`]: Self::offset
    pub color: Color,
}

/// A linear gradient.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Linear {
    /// How the [`Gradient`] is angled within its bounds.
    pub angle: Radians,
    /// [`ColorStop`]s along the linear gradient path.
    pub stops: [Option<ColorStop>; 8],
}

impl Linear {
    /// Creates a new [`Linear`] gradient with the given angle in [`Radians`].
    pub fn new(angle: impl Into<Radians>) -> Self {
        Self {
            angle: angle.into(),
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
            log::warn!("Gradient color stop must be within 0.0..=1.0 range.");
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

    /// Scales the alpha channel of the [`Linear`] gradient by the given
    /// factor.
    pub fn scale_alpha(mut self, factor: f32) -> Self {
        for stop in self.stops.iter_mut().flatten() {
            stop.color.a *= factor;
        }

        self
    }

    /// Linearly interpolates between two [`Linear`] gradients by the given
    /// amount, or `None` when their stop layouts don't line up.
    pub fn lerp(self, other: Self, amount: f32) -> Option<Self> {
        Some(Self {
            angle: lerp_angle(self.angle, other.angle, amount),
            stops: lerp_stops(&self.stops, &other.stops, amount)?,
        })
    }
}

/// A radial gradient that interpolates colors in a circular pattern.
///
/// The gradient radiates from a center point outward.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Radial {
    /// The center point of the gradient, as a ratio of the bounds (0.0-1.0).
    /// For example, (0.5, 0.5) is the center, (0.0, 1.0) is the bottom-left.
    pub center: Point,
    /// The horizontal radius of the ellipse, as a ratio of the bounds width (0.0-1.0).
    pub radius_x: f32,
    /// The vertical radius of the ellipse, as a ratio of the bounds height (0.0-1.0).
    pub radius_y: f32,
    /// [`ColorStop`]s along the radial gradient path.
    pub stops: [Option<ColorStop>; 8],
}

impl Radial {
    /// Creates a new [`Radial`] gradient with the given center and radius.
    ///
    /// # Arguments
    /// * `center` - Center point as ratios (0.0-1.0) of the bounds
    /// * `radius` - Radius as a ratio of the smaller dimension
    pub fn new(center: Point, radius: f32) -> Self {
        Self {
            center,
            radius_x: radius,
            radius_y: radius,
            stops: [None; 8],
        }
    }

    /// Creates a new elliptical [`Radial`] gradient with separate x and y radii.
    ///
    /// # Arguments
    /// * `center` - Center point as ratios (0.0-1.0) of the bounds
    /// * `radius_x` - Horizontal radius as a ratio of bounds width
    /// * `radius_y` - Vertical radius as a ratio of bounds height
    pub fn elliptical(center: Point, radius_x: f32, radius_y: f32) -> Self {
        Self {
            center,
            radius_x,
            radius_y,
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
            log::warn!("Gradient color stop must be within 0.0..=1.0 range.");
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

    /// Scales the alpha channel of the [`Radial`] gradient by the given factor.
    pub fn scale_alpha(mut self, factor: f32) -> Self {
        for stop in self.stops.iter_mut().flatten() {
            stop.color.a *= factor;
        }

        self
    }

    /// Linearly interpolates between two [`Radial`] gradients by the given
    /// amount, or `None` when their stop layouts don't line up.
    pub fn lerp(self, other: Self, amount: f32) -> Option<Self> {
        Some(Self {
            center: lerp_point(self.center, other.center, amount),
            radius_x: lerp(self.radius_x, other.radius_x, amount),
            radius_y: lerp(self.radius_y, other.radius_y, amount),
            stops: lerp_stops(&self.stops, &other.stops, amount)?,
        })
    }
}

/// A conic (angular/sweep) gradient that interpolates colors around a center point.
///
/// The gradient sweeps around the center, like a color wheel. The angle determines
/// where the gradient starts (0 radians = right/3 o'clock, PI/2 = bottom, etc.).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Conic {
    /// The center point of the gradient, as a ratio of the bounds (0.0-1.0).
    /// For example, (0.5, 0.5) is the center.
    pub center: Point,
    /// The starting angle of the gradient in radians.
    /// 0 = right (3 o'clock), PI/2 = bottom, PI = left, 3*PI/2 = top.
    pub angle: Radians,
    /// [`ColorStop`]s along the conic gradient path (0.0 = start angle, 1.0 = full rotation back to start).
    pub stops: [Option<ColorStop>; 8],
}

impl Conic {
    /// Creates a new [`Conic`] gradient with the given center and starting angle.
    ///
    /// # Arguments
    /// * `center` - Center point as ratios (0.0-1.0) of the bounds
    /// * `angle` - Starting angle in radians
    pub fn new(center: Point, angle: impl Into<Radians>) -> Self {
        Self {
            center,
            angle: angle.into(),
            stops: [None; 8],
        }
    }

    /// Adds a new [`ColorStop`], defined by an offset and a color, to the gradient.
    ///
    /// The offset represents the position around the circle (0.0 = start angle, 1.0 = full rotation).
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
            log::warn!("Gradient color stop must be within 0.0..=1.0 range.");
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

    /// Scales the alpha channel of the [`Conic`] gradient by the given factor.
    pub fn scale_alpha(mut self, factor: f32) -> Self {
        for stop in self.stops.iter_mut().flatten() {
            stop.color.a *= factor;
        }

        self
    }

    /// Linearly interpolates between two [`Conic`] gradients by the given
    /// amount, or `None` when their stop layouts don't line up.
    pub fn lerp(self, other: Self, amount: f32) -> Option<Self> {
        Some(Self {
            center: lerp_point(self.center, other.center, amount),
            angle: lerp_angle(self.angle, other.angle, amount),
            stops: lerp_stops(&self.stops, &other.stops, amount)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Conic, Gradient, Linear, Radial};
    use crate::{Color, Point};

    /// Every scalar of a linear gradient travels with the blend.
    #[test]
    fn linear_interpolates_angle_and_stops() {
        let from = Linear::new(0.0).add_stop(0.0, Color::BLACK);
        let to = Linear::new(2.0).add_stop(1.0, Color::WHITE);

        let mid = from.lerp(to, 0.5).expect("matching stop layouts");

        assert!((mid.angle.0 - 1.0).abs() < 1e-3);
        assert!((mid.stops[0].unwrap().offset - 0.5).abs() < 1e-3);
        assert!((mid.stops[0].unwrap().color.r - 0.5).abs() < 1e-3);
    }

    /// Radial carries its centre and both radii across.
    #[test]
    fn radial_interpolates_geometry() {
        let from = Radial::new(Point::new(0.0, 0.0), 0.0).add_stop(0.0, Color::BLACK);
        let to = Radial::elliptical(Point::new(1.0, 1.0), 1.0, 0.5).add_stop(0.0, Color::WHITE);

        let mid = from.lerp(to, 0.5).expect("matching stop layouts");

        assert!((mid.center.x - 0.5).abs() < 1e-3);
        assert!((mid.radius_x - 0.5).abs() < 1e-3);
        assert!((mid.radius_y - 0.25).abs() < 1e-3);
    }

    /// Conic carries its centre and start angle across.
    #[test]
    fn conic_interpolates_geometry() {
        let from = Conic::new(Point::new(0.0, 0.0), 0.0).add_stop(0.0, Color::BLACK);
        let to = Conic::new(Point::new(1.0, 1.0), 2.0).add_stop(0.0, Color::WHITE);

        let mid = from.lerp(to, 0.5).expect("matching stop layouts");

        assert!((mid.center.y - 0.5).abs() < 1e-3);
        assert!((mid.angle.0 - 1.0).abs() < 1e-3);
    }

    /// A stop that only one side has would make the gradient gain or lose a
    /// band partway through, so the pair is rejected.
    #[test]
    fn a_differing_stop_count_has_no_midpoint() {
        let one = Linear::new(0.0).add_stop(0.0, Color::BLACK);
        let two = Linear::new(0.0)
            .add_stop(0.0, Color::BLACK)
            .add_stop(1.0, Color::WHITE);

        assert!(one.lerp(two, 0.5).is_none());
    }

    /// Variants don't blend into each other either.
    #[test]
    fn differing_variants_have_no_midpoint() {
        let linear = Gradient::Linear(Linear::new(0.0).add_stop(0.0, Color::BLACK));
        let radial = Gradient::Radial(Radial::new(Point::ORIGIN, 1.0).add_stop(0.0, Color::BLACK));

        assert!(linear.lerp(radial, 0.5).is_none());
    }
}
