//! The device's special key, resolved by the compositor.
//!
//! The key — the HUMAIN button — is usually bound to a modifier the compositor
//! also needs for its own chords, so only the compositor can tell a tap from
//! the start of one. It resolves the gesture and sends the meaning rather than
//! the key. Register a window with
//! [`register_special_action`](crate::window::register_special_action) to
//! receive these.

/// A resolved gesture on the special key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// Tapped. Focus the text input; no voice is involved.
    Activate,
    /// A hold began. Start capturing audio.
    HoldStart,
    /// The hold ended. Stop capturing and process what was captured.
    HoldEnd,
    /// The gesture was abandoned. Discard any capture rather than process it.
    Cancel,
}
