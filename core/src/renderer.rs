//! Write your own renderer.
#[cfg(debug_assertions)]
mod null;

use crate::{
    Background, Border, Color, Rectangle, Shadow, Size, Transformation, Vector,
};

/// A component that can be used by widgets to draw themselves on a screen.
pub trait Renderer {
    /// Starts recording a new layer.
    fn start_layer(&mut self, bounds: Rectangle);

    /// Ends recording a new layer.
    ///
    /// The new layer will clip its contents to the provided `bounds`.
    fn end_layer(&mut self);

    /// Draws the primitives recorded in the given closure in a new layer.
    ///
    /// The layer will clip its contents to the provided `bounds`.
    fn with_layer(&mut self, bounds: Rectangle, f: impl FnOnce(&mut Self)) {
        self.start_layer(bounds);
        f(self);
        self.end_layer();
    }

    /// Starts recording with a new [`Transformation`].
    fn start_transformation(&mut self, transformation: Transformation);

    /// Ends recording a new layer.
    ///
    /// The new layer will clip its contents to the provided `bounds`.
    fn end_transformation(&mut self);

    /// Applies a [`Transformation`] to the primitives recorded in the given closure.
    fn with_transformation(
        &mut self,
        transformation: Transformation,
        f: impl FnOnce(&mut Self),
    ) {
        self.start_transformation(transformation);
        f(self);
        self.end_transformation();
    }

    /// Applies a translation to the primitives recorded in the given closure.
    fn with_translation(
        &mut self,
        translation: Vector,
        f: impl FnOnce(&mut Self),
    ) {
        self.with_transformation(
            Transformation::translate(translation.x, translation.y),
            f,
        );
    }

    /// Starts recording a new opacity group.
    ///
    /// All primitives drawn until [`end_opacity`](Self::end_opacity) is called
    /// will have the given opacity applied.
    ///
    /// Opacity values should be in the range `0.0` (fully transparent) to `1.0` (fully opaque).
    fn start_opacity(&mut self, _bounds: Rectangle, _opacity: f32) {}

    /// Ends recording the current opacity group.
    fn end_opacity(&mut self) {}

    /// Draws the primitives recorded in the given closure with the specified opacity.
    fn with_opacity(
        &mut self,
        bounds: Rectangle,
        opacity: f32,
        f: impl FnOnce(&mut Self),
    ) {
        self.start_opacity(bounds, opacity);
        f(self);
        self.end_opacity();
    }

    /// Fills a [`Quad`] with the provided [`Background`].
    fn fill_quad(&mut self, quad: Quad, background: impl Into<Background>);

    /// Clears all of the recorded primitives in the [`Renderer`].
    fn clear(&mut self);
}

/// A polygon with four sides.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quad {
    /// The bounds of the [`Quad`].
    pub bounds: Rectangle,

    /// The [`Border`] of the [`Quad`]. The border is drawn on the inside of the [`Quad`].
    pub border: Border,

    /// The [`Shadow`] of the [`Quad`].
    pub shadow: Shadow,
}

impl Default for Quad {
    fn default() -> Self {
        Self {
            bounds: Rectangle::with_size(Size::ZERO),
            border: Border::default(),
            shadow: Shadow::default(),
        }
    }
}

/// The styling attributes of a [`Renderer`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    /// The color to apply to symbolic icons.
    pub icon_color: Color,
    /// The text color
    pub text_color: Color,
    /// The scale factor
    pub scale_factor: f64,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            icon_color: Color::BLACK,
            text_color: Color::BLACK,
            scale_factor: 1.0,
        }
    }
}
