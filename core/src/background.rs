use crate::Color;
use crate::gradient::{self, Gradient};

/// The background of some element.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Background {
    /// A solid color.
    Color(Color),
    /// Linearly interpolate between several colors.
    Gradient(Gradient),
    // TODO: Add image variant
}

impl Background {
    /// Scales the alpha channel of the [`Background`] by the given
    /// factor.
    pub fn scale_alpha(self, factor: f32) -> Self {
        match self {
            Self::Color(color) => Self::Color(color.scale_alpha(factor)),
            Self::Gradient(gradient) => Self::Gradient(gradient.scale_alpha(factor)),
        }
    }

    /// Linearly interpolates between two [`Background`]s by the given amount.
    ///
    /// Only fills of the same kind can blend: two colors interpolate, and two
    /// compatible gradients interpolate stop-wise (see [`Gradient::lerp`]). A
    /// color and a gradient — or two gradients that don't line up — have no
    /// representable in-between, so those step straight to `other` rather than
    /// flattening the fill and making it disappear mid-transition.
    pub fn lerp(self, other: Self, amount: f32) -> Self {
        match (self, other) {
            (Self::Color(from), Self::Color(to)) => Self::Color(from.lerp(to, amount)),
            (Self::Gradient(from), Self::Gradient(to)) => {
                from.lerp(to, amount).map_or(other, Self::Gradient)
            }
            _ => other,
        }
    }

    /// Linearly interpolates between two optional [`Background`]s by the given
    /// amount, reading `None` as "no fill".
    ///
    /// Styles across the widget set carry an `Option<Background>`, so animating
    /// one has to blend a fill against the absence of a fill. That side is a
    /// fill fading out to — or in from — nothing, which scaling its alpha
    /// expresses for a gradient just as well as for a flat color. Two present
    /// fills go through [`lerp`](Self::lerp).
    pub fn lerp_maybe(from: Option<Self>, to: Option<Self>, amount: f32) -> Option<Self> {
        match (from, to) {
            (Some(from), Some(to)) => Some(from.lerp(to, amount)),
            (Some(from), None) => Some(from.scale_alpha(1.0 - amount)),
            (None, Some(to)) => Some(to.scale_alpha(amount)),
            (None, None) => None,
        }
    }
}

impl From<Color> for Background {
    fn from(color: Color) -> Self {
        Background::Color(color)
    }
}

impl From<Gradient> for Background {
    fn from(gradient: Gradient) -> Self {
        Background::Gradient(gradient)
    }
}

impl From<gradient::Linear> for Background {
    fn from(gradient: gradient::Linear) -> Self {
        Background::Gradient(Gradient::Linear(gradient))
    }
}

impl From<gradient::Conic> for Background {
    fn from(gradient: gradient::Conic) -> Self {
        Background::Gradient(Gradient::Conic(gradient))
    }
}

#[cfg(test)]
mod tests {
    use super::Background;
    use crate::gradient::Linear;
    use crate::{Color, Gradient};

    fn gradient(start: Color, end: Color) -> Background {
        Background::Gradient(Gradient::Linear(
            Linear::new(0.0).add_stop(0.0, start).add_stop(1.0, end),
        ))
    }

    /// Two flat colors blend, which is the whole point of a fill transition.
    #[test]
    fn colors_blend() {
        let Background::Color(mid) =
            Background::Color(Color::BLACK).lerp(Background::Color(Color::WHITE), 0.5)
        else {
            panic!("expected a flat color");
        };

        assert!((mid.r - 0.5).abs() < 1e-3);
    }

    /// A gradient held constant across states must stay that gradient for the
    /// whole transition — collapsing it to a flat color blinked the fill away.
    #[test]
    fn a_constant_gradient_survives_the_transition() {
        let fill = gradient(Color::BLACK, Color::WHITE);

        for step in 0..=10 {
            let amount = step as f32 / 10.0;

            assert_eq!(
                fill.lerp(fill, amount),
                fill,
                "gradient changed at {amount}"
            );
        }
    }

    /// Two compatible gradients blend stop-wise rather than stepping.
    #[test]
    fn gradients_blend_stop_wise() {
        let from = gradient(Color::BLACK, Color::BLACK);
        let to = gradient(Color::WHITE, Color::WHITE);

        let Background::Gradient(Gradient::Linear(mid)) = from.lerp(to, 0.5) else {
            panic!("expected a linear gradient");
        };

        for stop in mid.stops.iter().flatten() {
            assert!((stop.color.r - 0.5).abs() < 1e-3);
        }
    }

    /// A gradient and a flat color have no representable midpoint, so the pair
    /// steps to the target instead of fading through a collapsed fill.
    #[test]
    fn a_gradient_and_a_color_step_to_the_target() {
        let color = Background::Color(Color::BLACK);
        let fill = gradient(Color::WHITE, Color::WHITE);

        assert_eq!(color.lerp(fill, 0.5), fill);
        assert_eq!(fill.lerp(color, 0.5), color);
    }

    /// A fill appearing where there was none fades in by alpha, and one going
    /// away fades out — a gradient wash included, since a gradient has no flat
    /// midpoint to collapse to.
    #[test]
    fn an_absent_background_fades_the_other_side() {
        let opaque = Color {
            a: 1.0,
            ..Color::BLACK
        };
        let flat = Background::Color(opaque);

        let Some(Background::Color(fading_in)) = Background::lerp_maybe(None, Some(flat), 0.25)
        else {
            panic!("expected a flat color");
        };
        let Some(Background::Color(fading_out)) = Background::lerp_maybe(Some(flat), None, 0.25)
        else {
            panic!("expected a flat color");
        };

        assert!((fading_in.a - 0.25).abs() < 1e-3);
        assert!((fading_out.a - 0.75).abs() < 1e-3);

        let Some(Background::Gradient(Gradient::Linear(wash))) =
            Background::lerp_maybe(None, Some(gradient(opaque, opaque)), 0.25)
        else {
            panic!("expected a linear gradient");
        };

        for stop in wash.stops.iter().flatten() {
            assert!((stop.color.a - 0.25).abs() < 1e-3);
        }
    }

    /// Two absent backgrounds have nothing to draw.
    #[test]
    fn two_absent_backgrounds_stay_absent() {
        assert_eq!(Background::lerp_maybe(None, None, 0.5), None);
    }
}
