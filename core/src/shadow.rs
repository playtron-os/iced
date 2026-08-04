use crate::{Color, Vector};

/// A shadow.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Shadow {
    /// The color of the shadow.
    pub color: Color,

    /// The offset of the shadow.
    pub offset: Vector,

    /// The blur radius of the shadow.
    pub blur_radius: f32,

    /// The spread radius of the shadow.
    ///
    /// Positive values expand the shadow outward (larger than the element).
    /// Negative values contract the shadow inward (smaller than the element).
    pub spread_radius: f32,

    /// Whether the shadow is inset (inside the element) or outset (outside).
    /// Default is `false` (outset shadow).
    pub inset: bool,
}

impl Shadow {
    /// Creates a new outset (default) shadow.
    pub fn new(color: Color, offset: Vector, blur_radius: f32) -> Self {
        Self {
            color,
            offset,
            blur_radius,
            spread_radius: 0.0,
            inset: false,
        }
    }

    /// Creates a new inset shadow.
    pub fn inset(color: Color, offset: Vector, blur_radius: f32) -> Self {
        Self {
            color,
            offset,
            blur_radius,
            spread_radius: 0.0,
            inset: true,
        }
    }

    /// Sets the spread radius of the shadow.
    pub fn with_spread(mut self, spread_radius: f32) -> Self {
        self.spread_radius = spread_radius;
        self
    }

    /// Sets whether the shadow is inset.
    pub fn with_inset(mut self, inset: bool) -> Self {
        self.inset = inset;
        self
    }

    /// Returns the [`Shadow`] with a fully transparent color.
    ///
    /// The geometry is kept, so this is the "absent" end of a fade — a shadow
    /// appearing or disappearing rather than one moving.
    pub const fn transparent(self) -> Self {
        Self {
            color: Color {
                a: 0.0,
                ..self.color
            },
            ..self
        }
    }

    /// Linearly interpolates between two [`Shadow`]s by the given amount.
    ///
    /// `inset` has no representable in-between — an inset shadow and an outset one
    /// are drawn by different branches entirely — so it steps to `other`'s.
    pub const fn lerp(self, other: Self, amount: f32) -> Self {
        Self {
            color: self.color.lerp(other.color, amount),
            offset: self.offset.lerp(other.offset, amount),
            blur_radius: self.blur_radius + (other.blur_radius - self.blur_radius) * amount,
            spread_radius: self.spread_radius + (other.spread_radius - self.spread_radius) * amount,
            inset: other.inset,
        }
    }

    /// Linearly interpolates between two stacks of [`Shadow`]s by the given
    /// amount, pairing them up layer by layer.
    ///
    /// Elevation scales stack several layers, and the two ends of a transition
    /// need not use the same number of them. A layer present at only one end keeps
    /// that end's geometry and fades its alpha from (or to) zero, so a stack can
    /// grow and shrink across a transition.
    ///
    /// The result always holds one layer per index rather than both stacks
    /// concatenated: overlapping translucent layers composite, so drawing both
    /// would darken the shadow mid-transition even when the endpoints match.
    pub fn lerp_stacks(from: &[Self], to: &[Self], amount: f32) -> Vec<Self> {
        (0..from.len().max(to.len()))
            .filter_map(|i| match (from.get(i), to.get(i)) {
                (Some(from), Some(to)) => Some(from.lerp(*to, amount)),
                (Some(from), None) => Some(from.lerp(from.transparent(), amount)),
                (None, Some(to)) => Some(to.transparent().lerp(*to, amount)),
                // Unreachable: `i` is below the longer stack's length.
                (None, None) => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shadow(alpha: f32, blur: f32) -> Shadow {
        Shadow {
            color: Color::from_rgba(0.0, 0.0, 0.0, alpha),
            offset: Vector::new(0.0, 2.0),
            blur_radius: blur,
            spread_radius: -1.0,
            inset: false,
        }
    }

    #[test]
    fn lerp_stacks_pairs_layers_instead_of_concatenating() {
        // Two overlapping translucent layers composite to more than either one,
        // so a stack transition that drew both ends would visibly darken halfway
        // through even when the endpoints are identical.
        let from = vec![shadow(0.1, 6.0), shadow(0.1, 15.0)];
        let to = vec![shadow(0.2, 6.0), shadow(0.2, 15.0)];

        assert_eq!(Shadow::lerp_stacks(&from, &to, 0.5).len(), 2);
        assert_eq!(Shadow::lerp_stacks(&from, &to, 0.0), from);
        assert_eq!(Shadow::lerp_stacks(&from, &to, 1.0), to);
    }

    #[test]
    fn lerp_stacks_fades_unpaired_layers_from_transparent() {
        // A stack that grows mid-transition (an extra ring layer appearing, say)
        // must fade the new layer in by alpha while it already holds its final
        // geometry — interpolating its geometry from a neighbour would slide it
        // into place from the wrong shape.
        let from = vec![shadow(0.1, 6.0)];
        let to = vec![shadow(0.1, 6.0), shadow(0.4, 20.0)];

        let mid = Shadow::lerp_stacks(&from, &to, 0.5);
        assert_eq!(mid.len(), 2);
        assert!((mid[1].color.a - 0.2).abs() < 1e-6, "alpha fades in");
        assert!(
            (mid[1].blur_radius - 20.0).abs() < 1e-6,
            "geometry is already final"
        );

        // Endpoints stay exact in both directions.
        assert_eq!(Shadow::lerp_stacks(&from, &to, 1.0), to);
        assert_eq!(Shadow::lerp_stacks(&to, &from, 1.0)[1], to[1].transparent());
    }
}
