//! Highlight text.
use crate::Color;

use std::ops::Range;

/// A type capable of highlighting text.
///
/// A [`Highlighter`] highlights lines in sequence. When a line changes,
/// it must be notified and the lines after the changed one must be fed
/// again to the [`Highlighter`].
pub trait Highlighter: 'static {
    /// The settings to configure the [`Highlighter`].
    type Settings: PartialEq + Clone;

    /// The output of the [`Highlighter`].
    type Highlight;

    /// The highlight iterator type.
    type Iterator<'a>: Iterator<Item = (Range<usize>, Self::Highlight)>
    where
        Self: 'a;

    /// Creates a new [`Highlighter`] from its [`Self::Settings`].
    fn new(settings: &Self::Settings) -> Self;

    /// Updates the [`Highlighter`] with some new [`Self::Settings`].
    fn update(&mut self, new_settings: &Self::Settings);

    /// Notifies the [`Highlighter`] that the line at the given index has changed.
    fn change_line(&mut self, line: usize);

    /// Highlights the given line.
    ///
    /// If a line changed prior to this, the first line provided here will be the
    /// line that changed.
    fn highlight_line(&mut self, line: &str) -> Self::Iterator<'_>;

    /// Returns the current line of the [`Highlighter`].
    ///
    /// If `change_line` has been called, this will normally be the least index
    /// that changed.
    fn current_line(&self) -> usize;

    /// The size a highlight lays its text out at, when it is not the editor's.
    ///
    /// A highlight with [`Metrics`] makes its line as tall as it needs, which is
    /// how a heading is larger than the text around it. `None`, the default,
    /// keeps the editor's size and line height.
    fn metrics(&self, _highlight: &Self::Highlight) -> Option<Metrics> {
        None
    }
}

/// The font size and line height of highlighted text, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    /// The font size.
    pub size: f32,
    /// The height of a line.
    pub line_height: f32,
}

/// A highlighter that highlights nothing.
#[derive(Debug, Clone, Copy)]
pub struct PlainText;

impl Highlighter for PlainText {
    type Settings = ();
    type Highlight = ();

    type Iterator<'a> = std::iter::Empty<(Range<usize>, Self::Highlight)>;

    fn new(_settings: &Self::Settings) -> Self {
        Self
    }

    fn update(&mut self, _new_settings: &Self::Settings) {}

    fn change_line(&mut self, _line: usize) {}

    fn highlight_line(&mut self, _line: &str) -> Self::Iterator<'_> {
        std::iter::empty()
    }

    fn current_line(&self) -> usize {
        usize::MAX
    }
}

/// The format of some text.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Format<Font> {
    /// The [`Color`] of the text.
    pub color: Option<Color>,
    /// The `Font` of the text.
    pub font: Option<Font>,
    /// Whether the text should be underlined.
    pub underline: bool,
    /// Whether the text should have a strikethrough.
    pub strikethrough: bool,
    /// An optional highlight [`Color`] drawn behind the text.
    pub background: Option<Color>,
}

impl<Font> Default for Format<Font> {
    fn default() -> Self {
        Self {
            color: None,
            font: None,
            underline: false,
            strikethrough: false,
            background: None,
        }
    }
}
