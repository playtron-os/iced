//! Backdrop luminance events from the compositor.
//!
//! A surface that draws content straight onto the wallpaper -- a desktop
//! greeting, a widget label -- cannot pick a legible foreground colour from the
//! active colour scheme alone: dark mode chooses light text, which vanishes over
//! a light wallpaper. These events report how light the compositor found the
//! backdrop under each zone the surface marked, so it can choose for itself.

/// Backdrop luminance events sent to layer shell surfaces.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Fresh readings for one or more zones.
    ///
    /// Each entry is a zone index -- as the surface added it to its region --
    /// paired with the relative luminance behind it, where 0.0 is black and 1.0
    /// is white. Only zones whose reading actually changed are included, so a
    /// surface should update the ones named and leave the rest alone.
    ///
    /// Readings arrive batched rather than one per zone so a surface with
    /// several independently styled zones restyles in a single frame.
    Updated {
        /// The changed zones, as `(index, luminance)`.
        readings: Vec<(u32, f32)>,
    },
}
