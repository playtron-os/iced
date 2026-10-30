//! Draw text.
pub mod cache;
pub mod editor;
pub mod paragraph;
mod wrap;

pub use cache::Cache;
pub use editor::Editor;
pub use paragraph::Paragraph;

pub use cosmic_text;

use crate::core::alignment;
use crate::core::font::{self, Font};
use crate::core::text::{ADVANCED_AUTO_SHAPING, Alignment, Ellipsis, Shaping, Span, Wrapping};
use crate::core::{Color, Pixels, Point, Rectangle, Size, Transformation};

use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock, Weak};

/// A text primitive.
#[derive(Debug, Clone, PartialEq)]
pub enum Text {
    /// A paragraph.
    #[allow(missing_docs)]
    Paragraph {
        paragraph: paragraph::Weak,
        position: Point,
        color: Color,
        clip_bounds: Rectangle,
        transformation: Transformation,
    },
    /// An editor.
    #[allow(missing_docs)]
    Editor {
        editor: editor::Weak,
        position: Point,
        color: Color,
        clip_bounds: Rectangle,
        transformation: Transformation,
    },
    /// Some cached text.
    Cached {
        /// The contents of the text.
        content: String,
        /// The bounds of the text.
        bounds: Rectangle,
        /// The color of the text.
        color: Color,
        /// The size of the text in logical pixels.
        size: Pixels,
        /// The line height of the text.
        line_height: Pixels,
        /// The font of the text.
        font: Font,
        /// The horizontal alignment of the text.
        align_x: Alignment,
        /// The vertical alignment of the text.
        align_y: alignment::Vertical,
        /// The shaping strategy of the text.
        shaping: Shaping,
        /// The wrapping strategy of the text.
        wrapping: Wrapping,
        /// The ellipsis strategy of the text.
        ellipsis: Ellipsis,
        /// The letter spacing of the text in pixels.
        letter_spacing: Option<f32>,
        /// The clip bounds of the text.
        clip_bounds: Rectangle,
    },
    /// Some raw text.
    #[allow(missing_docs)]
    Raw {
        raw: Raw,
        transformation: Transformation,
    },
}

impl Text {
    /// Scales the color's alpha by `opacity`, as an opacity group does to text
    /// drawn inside it. Raw text keeps its color.
    pub fn scale_opacity(&mut self, opacity: f32) {
        match self {
            Text::Paragraph { color, .. }
            | Text::Editor { color, .. }
            | Text::Cached { color, .. } => color.a *= opacity,
            Text::Raw { .. } => {}
        }
    }

    /// Returns the visible bounds of the [`Text`].
    pub fn visible_bounds(&self) -> Option<Rectangle> {
        match self {
            Text::Paragraph {
                position,
                paragraph,
                clip_bounds,
                transformation,
                ..
            } => Rectangle::new(*position, paragraph.min_bounds)
                .intersection(clip_bounds)
                .map(|bounds| bounds * *transformation),
            Text::Editor {
                editor,
                position,
                clip_bounds,
                transformation,
                ..
            } => Rectangle::new(*position, editor.bounds)
                .intersection(clip_bounds)
                .map(|bounds| bounds * *transformation),
            Text::Cached {
                bounds,
                clip_bounds,
                ..
            } => bounds.intersection(clip_bounds),
            Text::Raw { raw, .. } => Some(raw.clip_bounds),
        }
    }
}

/// The regular variant of the [Fira Sans] font.
///
/// It is loaded as part of the default fonts when the `fira-sans`
/// feature is enabled.
///
/// [Fira Sans]: https://mozilla.github.io/Fira/
#[cfg(feature = "fira-sans")]
pub const FIRA_SANS_REGULAR: &[u8] = include_bytes!("../fonts/FiraSans-Regular.ttf").as_slice();

static FONT_SYSTEM: OnceLock<RwLock<FontSystem>> = OnceLock::new();

/// Whether the global [`FontSystem`] is to load the host's fonts; read once, when it is
/// first built.
static SYSTEM_FONTS: AtomicBool = AtomicBool::new(true);

/// Chooses whether the global [`FontSystem`] loads the fonts installed on the host,
/// besides the program's own and iced's built-in ones. See
/// [`Settings::system_fonts`](crate::core::Settings::system_fonts).
///
/// The font system is built once per process, on first use. Turning the host's fonts off
/// still holds after that: their faces are dropped from it, and the program's own keep
/// their ids, so text already laid out with them stays valid. Turning them back on once
/// built does not hold. Returns whether the choice holds.
pub fn set_system_fonts(enabled: bool) -> bool {
    let built_with = || {
        FONT_SYSTEM
            .get()
            .map(|font_system| font_system.read().expect("Read font system").system_fonts)
    };

    if let Some(system_fonts) = built_with() {
        if system_fonts && !enabled {
            font_system()
                .write()
                .expect("Write font system")
                .drop_system_fonts();
            return true;
        }

        return system_fonts == enabled;
    }

    SYSTEM_FONTS.store(enabled, Ordering::SeqCst);

    // Another thread may have built it in between.
    built_with().is_none_or(|system_fonts| system_fonts == enabled)
}

/// Returns the global [`FontSystem`].
pub fn font_system() -> &'static RwLock<FontSystem> {
    FONT_SYSTEM.get_or_init(|| {
        let built_in = [
            cosmic_text::fontdb::Source::Binary(Arc::new(
                include_bytes!("../fonts/Iced-Icons.ttf").as_slice(),
            )),
            #[cfg(feature = "fira-sans")]
            cosmic_text::fontdb::Source::Binary(Arc::new(
                include_bytes!("../fonts/FiraSans-Regular.ttf").as_slice(),
            )),
        ];
        let system_fonts = SYSTEM_FONTS.load(Ordering::SeqCst);

        let raw = if system_fonts {
            cosmic_text::FontSystem::new_with_fonts(built_in)
        } else {
            let mut database = cosmic_text::fontdb::Database::new();

            for source in built_in {
                let _ = database.load_font_source(source);
            }

            bundled_only(database)
        };

        RwLock::new(FontSystem {
            raw,
            loaded_fonts: HashSet::new(),
            version: Version::default(),
            system_fonts,
        })
    })
}

/// The font database the text system holds, shared rather than built again:
/// the fonts bundled with the program, the ones it has loaded, and the
/// system's unless [`set_system_fonts`] turned those off. The SVG rasteriser needs one, and a second database of its own
/// would neither hold an application's own fonts nor agree with the text
/// beside the picture about what "sans-serif" is.
///
/// Cheap: a `Database` holds paths and `Arc`s, and the clone is made again
/// only when a font has been loaded since.
pub fn font_database() -> Arc<cosmic_text::fontdb::Database> {
    static DATABASE: Mutex<Option<(Version, Arc<cosmic_text::fontdb::Database>)>> =
        Mutex::new(None);

    let mut font_system = font_system().write().expect("Write font system");
    let version = font_system.version();

    let mut cached = DATABASE.lock().expect("Lock font database");

    if let Some((at, database)) = cached.as_ref()
        && *at == version
    {
        return Arc::clone(database);
    }

    let database = Arc::new(font_system.raw().db().clone());
    *cached = Some((version, Arc::clone(&database)));

    database
}

/// A [`cosmic_text::FontSystem`] over `database` that resolves the same on every machine:
/// the generic families name faces a program ships or none at all, fallback only searches
/// this database, and the locale that orders the fallback lists is fixed.
fn bundled_only(mut database: cosmic_text::fontdb::Database) -> cosmic_text::FontSystem {
    database.set_monospace_family("Noto Sans Mono");
    database.set_sans_serif_family(if cfg!(feature = "fira-sans") {
        "Fira Sans"
    } else {
        "Open Sans"
    });
    database.set_serif_family("DejaVu Serif");

    cosmic_text::FontSystem::new_with_locale_and_db(String::from("en-US"), database)
}

/// A set of system fonts.
pub struct FontSystem {
    raw: cosmic_text::FontSystem,
    loaded_fonts: HashSet<usize>,
    version: Version,
    system_fonts: bool,
}

impl FontSystem {
    /// Returns the raw [`cosmic_text::FontSystem`].
    pub fn raw(&mut self) -> &mut cosmic_text::FontSystem {
        &mut self.raw
    }

    /// Loads a font from its bytes.
    pub fn load_font(&mut self, bytes: Cow<'static, [u8]>) {
        if let Cow::Borrowed(bytes) = bytes {
            let address = bytes.as_ptr() as usize;

            if !self.loaded_fonts.insert(address) {
                return;
            }
        }

        let _ = self
            .raw
            .db_mut()
            .load_font_source(cosmic_text::fontdb::Source::Binary(Arc::new(
                bytes.into_owned(),
            )));

        self.version = Version(self.version.0 + 1);
    }

    /// Drops the host's faces, keeping the program's own and iced's built-in ones under the
    /// ids they had. The caches are rebuilt over the same database.
    fn drop_system_fonts(&mut self) {
        let mut database = self.raw.db().clone();
        let from_host: Vec<_> = database
            .faces()
            .filter(|face| !matches!(face.source, cosmic_text::fontdb::Source::Binary(_)))
            .map(|face| face.id)
            .collect();

        for id in from_host {
            database.remove_face(id);
        }

        self.raw = bundled_only(database);
        self.system_fonts = false;
        self.version = Version(self.version.0 + 1);
    }

    /// Whether this [`FontSystem`] holds the fonts installed on the host. See
    /// [`set_system_fonts`].
    pub fn system_fonts(&self) -> bool {
        self.system_fonts
    }

    /// Returns an iterator over the family names of all font faces
    /// in the font database.
    pub fn families(&self) -> impl Iterator<Item = &str> {
        self.raw
            .db()
            .faces()
            .filter_map(|face| face.families.first())
            .map(|(name, _)| name.as_str())
    }

    /// Returns the current [`Version`] of the [`FontSystem`].
    ///
    /// Loading a font will increase the version of a [`FontSystem`].
    pub fn version(&self) -> Version {
        self.version
    }
}

/// A version number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Version(u32);

/// A weak reference to a [`cosmic_text::Buffer`] that can be drawn.
#[derive(Debug, Clone)]
pub struct Raw {
    /// A weak reference to a [`cosmic_text::Buffer`].
    pub buffer: Weak<cosmic_text::Buffer>,
    /// The position of the text.
    pub position: Point,
    /// The color of the text.
    pub color: Color,
    /// The clip bounds of the text.
    pub clip_bounds: Rectangle,
}

impl PartialEq for Raw {
    fn eq(&self, _other: &Self) -> bool {
        // TODO: There is no proper way to compare raw buffers
        // For now, no two instances of `Raw` text will be equal.
        // This should be fine, but could trigger unnecessary redraws
        // in the future.
        false
    }
}

/// Measures the dimensions of the given [`cosmic_text::Buffer`].
pub fn measure(buffer: &cosmic_text::Buffer) -> (Size, bool) {
    let (width, height, has_rtl) =
        buffer
            .layout_runs()
            .fold((0.0, 0.0, false), |(width, height, has_rtl), run| {
                (
                    run.line_w.max(width),
                    height + run.line_height,
                    has_rtl || run.rtl,
                )
            });

    (Size::new(width, height), has_rtl)
}

/// Moves a [`cosmic_text::Buffer`]'s physical `position` so its first baseline lands on
/// a whole device pixel, `scale` device pixels to a buffer unit.
///
/// A paragraph laid out at this scale (`browser`) takes the baseline the browser draws,
/// [`browser_baseline`] below its top rounded to a pixel, as its layout expects. Other
/// text rounds its own baseline once. Renderers truncate the top and round the
/// baseline's offset apart, so the top moves to suit.
pub fn snap_baseline(
    buffer: &cosmic_text::Buffer,
    position: Point,
    scale: f32,
    browser: bool,
) -> Point {
    let Some(run) = buffer.layout_runs().next() else {
        return position;
    };
    let offset = run.line_y * scale;
    let baseline = match browser_baseline(buffer, scale).filter(|_| browser) {
        Some(below) => (position.y + run.line_top * scale + 1e-3).round() + below,
        None => (position.y + offset).round(),
    };

    Point::new(position.x, baseline - offset.round())
}

/// How far below its line's top the browser draws a buffer's first baseline, in whole
/// device pixels at `scale` device pixels to a buffer unit.
///
/// Blink rounds the ascent and descent to whole pixels at the device's font size, then
/// splits the leading with the half above floored.
pub fn browser_baseline(buffer: &cosmic_text::Buffer, scale: f32) -> Option<f32> {
    let run = buffer.layout_runs().next()?;
    let line = buffer.lines.first()?.layout_opt()?.first()?;
    let ascent = (line.max_ascent * scale).round();
    let descent = (line.max_descent * scale).round();

    Some(ascent + ((run.line_height * scale - ascent - descent) / 2.0).floor())
}

/// Aligns the given [`cosmic_text::Buffer`] with the given [`Alignment`]
/// and returns its minimum [`Size`].
pub fn align(
    buffer: &mut cosmic_text::Buffer,
    font_system: &mut cosmic_text::FontSystem,
    alignment: Alignment,
) -> Size {
    let (min_bounds, has_rtl) = measure(buffer);
    let mut needs_relayout = has_rtl;

    if let Some(align) = to_align(alignment) {
        let has_multiple_lines = buffer.lines.len() > 1
            || buffer
                .lines
                .first()
                .is_some_and(|line| line.layout_opt().is_some_and(|layout| layout.len() > 1));

        if has_multiple_lines {
            for line in &mut buffer.lines {
                let _ = line.set_align(Some(align));
            }

            needs_relayout = true;
        } else if let Some(line) = buffer.lines.first_mut() {
            needs_relayout = line.set_align(None);
        }
    }

    // TODO: Avoid relayout with some changes to `cosmic-text` (?)
    if needs_relayout {
        log::trace!("Relayouting paragraph...");

        buffer.set_size(font_system, Some(min_bounds.width), Some(min_bounds.height));
    }

    min_bounds
}

/// Returns the attributes of the given [`Font`].
pub fn to_attributes(font: Font) -> cosmic_text::Attrs<'static> {
    let attributes = cosmic_text::Attrs::new()
        .family(to_family(font.family))
        .weight(to_weight(font.weight))
        .stretch(to_stretch(font.stretch))
        .style(to_style(font.style));

    if font.features.is_empty() {
        return attributes;
    }

    let mut features = cosmic_text::FontFeatures::new();
    if font.features.contains(font::Features::TABULAR_FIGURES) {
        let _ = features.enable(cosmic_text::FeatureTag::new(b"tnum"));
    }

    attributes.font_features(features)
}

/// Returns the attributes of the given [`Font`] with optional letter spacing.
///
/// Letter spacing is specified in pixels and will be converted to EM units
/// (relative to font size) for the underlying text engine.
pub fn to_attributes_with_spacing(
    font: Font,
    letter_spacing_px: Option<f32>,
    font_size: f32,
) -> cosmic_text::Attrs<'static> {
    let attrs = to_attributes(font);
    match letter_spacing_px {
        Some(px) if font_size > 0.0 => with_letter_spacing(attrs, px / font_size),
        _ => attrs,
    }
}

/// Returns `attributes` spaced by `letter_spacing` EM.
///
/// Spaced text also drops the ligatures and contextual alternates CSS drops
/// from it (`liga`, `clig` and `calt`, as Chromium does): a ligature draws two
/// letters as one glyph, so the space between them would be lost.
pub fn with_letter_spacing(
    attributes: cosmic_text::Attrs<'static>,
    letter_spacing: f32,
) -> cosmic_text::Attrs<'static> {
    let attributes = attributes.letter_spacing(letter_spacing);

    if letter_spacing == 0.0 {
        return attributes;
    }

    // Keep the font's own features, such as tabular figures.
    let mut features = attributes.font_features.clone();
    let _ = features
        .disable(cosmic_text::FeatureTag::STANDARD_LIGATURES)
        .disable(cosmic_text::FeatureTag::CONTEXTUAL_LIGATURES)
        .disable(cosmic_text::FeatureTag::CONTEXTUAL_ALTERNATES);

    attributes.font_features(features)
}

fn to_family(family: font::Family) -> cosmic_text::Family<'static> {
    match family {
        font::Family::Name(name) => cosmic_text::Family::Name(name),
        font::Family::SansSerif => cosmic_text::Family::SansSerif,
        font::Family::Serif => cosmic_text::Family::Serif,
        font::Family::Cursive => cosmic_text::Family::Cursive,
        font::Family::Fantasy => cosmic_text::Family::Fantasy,
        font::Family::Monospace => cosmic_text::Family::Monospace,
    }
}

fn to_weight(weight: font::Weight) -> cosmic_text::Weight {
    match weight {
        font::Weight::Thin => cosmic_text::Weight::THIN,
        font::Weight::ExtraLight => cosmic_text::Weight::EXTRA_LIGHT,
        font::Weight::Light => cosmic_text::Weight::LIGHT,
        font::Weight::Normal => cosmic_text::Weight::NORMAL,
        font::Weight::Medium => cosmic_text::Weight::MEDIUM,
        font::Weight::Semibold => cosmic_text::Weight::SEMIBOLD,
        font::Weight::Bold => cosmic_text::Weight::BOLD,
        font::Weight::ExtraBold => cosmic_text::Weight::EXTRA_BOLD,
        font::Weight::Black => cosmic_text::Weight::BLACK,
    }
}

fn to_stretch(stretch: font::Stretch) -> cosmic_text::Stretch {
    match stretch {
        font::Stretch::UltraCondensed => cosmic_text::Stretch::UltraCondensed,
        font::Stretch::ExtraCondensed => cosmic_text::Stretch::ExtraCondensed,
        font::Stretch::Condensed => cosmic_text::Stretch::Condensed,
        font::Stretch::SemiCondensed => cosmic_text::Stretch::SemiCondensed,
        font::Stretch::Normal => cosmic_text::Stretch::Normal,
        font::Stretch::SemiExpanded => cosmic_text::Stretch::SemiExpanded,
        font::Stretch::Expanded => cosmic_text::Stretch::Expanded,
        font::Stretch::ExtraExpanded => cosmic_text::Stretch::ExtraExpanded,
        font::Stretch::UltraExpanded => cosmic_text::Stretch::UltraExpanded,
    }
}

fn to_style(style: font::Style) -> cosmic_text::Style {
    match style {
        font::Style::Normal => cosmic_text::Style::Normal,
        font::Style::Italic => cosmic_text::Style::Italic,
        font::Style::Oblique => cosmic_text::Style::Oblique,
    }
}

fn to_align(alignment: Alignment) -> Option<cosmic_text::Align> {
    match alignment {
        Alignment::Default => None,
        Alignment::Left => Some(cosmic_text::Align::Left),
        Alignment::Center => Some(cosmic_text::Align::Center),
        Alignment::Right => Some(cosmic_text::Align::Right),
        Alignment::Justified => Some(cosmic_text::Align::Justified),
    }
}

/// Converts some [`Shaping`] strategy to a [`cosmic_text::Shaping`] strategy.
pub fn to_shaping(shaping: Shaping, text: &str) -> cosmic_text::Shaping {
    match shaping {
        Shaping::Auto => {
            if !ADVANCED_AUTO_SHAPING && text.is_ascii() {
                cosmic_text::Shaping::Basic
            } else {
                cosmic_text::Shaping::Advanced
            }
        }
        Shaping::Basic => cosmic_text::Shaping::Basic,
        Shaping::Advanced => cosmic_text::Shaping::Advanced,
    }
}

/// Converts some [`Shaping`] strategy to a [`cosmic_text::Shaping`] strategy for rich text.
/// Checks if all spans are ASCII to decide whether Basic shaping can be used.
pub fn to_shaping_for_spans<'a, Link>(
    shaping: Shaping,
    spans: &[Span<'a, Link>],
) -> cosmic_text::Shaping {
    match shaping {
        Shaping::Auto => {
            if !ADVANCED_AUTO_SHAPING && spans.iter().all(|span| span.text.is_ascii()) {
                cosmic_text::Shaping::Basic
            } else {
                cosmic_text::Shaping::Advanced
            }
        }
        Shaping::Basic => cosmic_text::Shaping::Basic,
        Shaping::Advanced => cosmic_text::Shaping::Advanced,
    }
}

/// Converts some [`Wrapping`] strategy to a [`cosmic_text::Wrap`] strategy.
pub fn to_wrap(wrapping: Wrapping) -> cosmic_text::Wrap {
    match wrapping {
        Wrapping::None => cosmic_text::Wrap::None,
        Wrapping::Word => cosmic_text::Wrap::Word,
        Wrapping::Glyph => cosmic_text::Wrap::Glyph,
        Wrapping::WordOrGlyph => cosmic_text::Wrap::WordOrGlyph,
        Wrapping::Balanced | Wrapping::Pretty => cosmic_text::Wrap::Word,
    }
}

/// Lays `buffer` out as its [`Wrapping`] asks, given the widest it may be, and
/// returns the width it wraps at.
///
/// [`Wrapping::Balanced`] and [`Wrapping::Pretty`] break lines as Blink breaks
/// CSS `text-wrap: balance` and `pretty`; balancing may narrow the width when its
/// scored breaks do not keep the line count. Every other strategy keeps
/// `max_width`.
pub fn fit_width(
    buffer: &mut cosmic_text::Buffer,
    font_system: &mut cosmic_text::FontSystem,
    wrapping: Wrapping,
    max_width: f32,
) -> f32 {
    wrap::fit(buffer, font_system, wrapping, max_width)
}

/// Converts some [`Ellipsis`] strategy to a [`cosmic_text::Ellipsize`] strategy.
pub fn to_ellipsize(ellipsis: Ellipsis, max_height: f32) -> cosmic_text::Ellipsize {
    let limit = cosmic_text::EllipsizeHeightLimit::Height(max_height);

    match ellipsis {
        Ellipsis::None => cosmic_text::Ellipsize::None,
        Ellipsis::Start => cosmic_text::Ellipsize::Start(limit),
        Ellipsis::Middle => cosmic_text::Ellipsize::Middle(limit),
        Ellipsis::End => cosmic_text::Ellipsize::End(limit),
    }
}

/// Converts some [`Color`] to a [`cosmic_text::Color`].
pub fn to_color(color: Color) -> cosmic_text::Color {
    let [r, g, b, a] = color.into_rgba8();

    cosmic_text::Color::rgba(r, g, b, a)
}

/// Returns the ideal hint factor given the size and scale factor of some text.
pub fn hint_factor(_size: Pixels, _scale_factor: Option<f32>) -> Option<f32> {
    // TODO: Fix hinting in `cosmic-text`
    // const MAX_HINTING_SIZE: f32 = 18.0;

    // let hint_factor = scale_factor?;

    // if size.0 * hint_factor < MAX_HINTING_SIZE {
    //     Some(hint_factor)
    // } else {
    //     None
    // }

    None // Disable all text hinting for now
}

/// A text renderer coupled to `iced_graphics`.
pub trait Renderer {
    /// Draws the given [`Raw`] text.
    fn fill_raw(&mut self, raw: Raw);
}

#[cfg(test)]
mod tests {
    use super::*;

    // Run with and without `iced_core/advanced-auto-shaping`.
    #[test]
    fn auto_shaping_is_advanced_for_ascii_only_with_the_feature() {
        let (basic, advanced) = (cosmic_text::Shaping::Basic, cosmic_text::Shaping::Advanced);
        let ascii = if ADVANCED_AUTO_SHAPING {
            advanced
        } else {
            basic
        };

        let plain: [Span<'_>; 2] = [Span::new("Move "), Span::new("to Trash")];
        assert_eq!(to_shaping(Shaping::Auto, "Move to Trash"), ascii);
        assert_eq!(to_shaping_for_spans(Shaping::Auto, &plain), ascii);

        let curly: [Span<'_>; 2] = [Span::new("can’t "), Span::new("be undone")];
        assert_eq!(to_shaping(Shaping::Auto, "can’t be undone"), advanced);
        assert_eq!(to_shaping_for_spans(Shaping::Auto, &curly), advanced);

        assert_eq!(to_shaping(Shaping::Basic, "can’t"), basic);
        assert_eq!(to_shaping_for_spans(Shaping::Basic, &curly), basic);
        assert_eq!(to_shaping(Shaping::Advanced, "Trash"), advanced);
        assert_eq!(to_shaping_for_spans(Shaping::Advanced, &plain), advanced);
    }

    fn feature(attributes: &cosmic_text::Attrs<'_>, tag: [u8; 4]) -> Option<u32> {
        attributes
            .font_features
            .features
            .iter()
            .rev()
            .find(|feature| *feature.tag.as_bytes() == tag)
            .map(|feature| feature.value)
    }

    #[test]
    fn tabular_figures_ask_the_font_for_tnum() {
        assert_eq!(feature(&to_attributes(Font::DEFAULT), *b"tnum"), None);
        assert_eq!(
            feature(&to_attributes(Font::DEFAULT.tabular_figures()), *b"tnum"),
            Some(1)
        );
    }

    #[test]
    fn letter_spacing_keeps_tabular_figures() {
        let spaced = with_letter_spacing(to_attributes(Font::DEFAULT.tabular_figures()), 0.1);

        assert_eq!(feature(&spaced, *b"tnum"), Some(1));
        assert_eq!(feature(&spaced, *b"liga"), Some(0));
    }

    /// The advance of `text` alone in Fira Sans, shaped with `font`'s features.
    #[cfg(feature = "fira-sans")]
    fn advance(text: &str, font: Font) -> f32 {
        let mut database = cosmic_text::fontdb::Database::new();
        database.load_font_data(FIRA_SANS_REGULAR.to_vec());
        database.set_sans_serif_family("Fira Sans");
        let mut font_system =
            cosmic_text::FontSystem::new_with_locale_and_db("en-US".into(), database);

        let mut buffer =
            cosmic_text::Buffer::new(&mut font_system, cosmic_text::Metrics::new(16.0, 20.0));
        buffer.set_text(
            &mut font_system,
            text,
            &to_attributes(font),
            cosmic_text::Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(&mut font_system, false);

        buffer.layout_runs().map(|run| run.line_w).sum()
    }

    #[cfg(feature = "fira-sans")]
    #[test]
    fn tabular_figures_give_every_digit_one_width() {
        let proportional = Font::DEFAULT;
        let tabular = Font::DEFAULT.tabular_figures();

        assert_ne!(
            advance("1", proportional),
            advance("0", proportional),
            "Fira Sans figures are proportional by default"
        );
        assert_eq!(advance("1", tabular), advance("0", tabular));
        assert_eq!(advance("1111", tabular), advance("8888", tabular));
    }

    /// `text` wrapped in Fira Sans as `wrapping` asks, no wider than
    /// `max_width`: the width it chose and each line's words.
    #[cfg(feature = "fira-sans")]
    fn wrap(text: &str, wrapping: Wrapping, max_width: f32) -> (f32, Vec<String>) {
        let mut database = cosmic_text::fontdb::Database::new();
        database.load_font_data(FIRA_SANS_REGULAR.to_vec());
        database.set_sans_serif_family("Fira Sans");
        let mut font_system =
            cosmic_text::FontSystem::new_with_locale_and_db("en-US".into(), database);

        let mut buffer =
            cosmic_text::Buffer::new(&mut font_system, cosmic_text::Metrics::new(16.0, 20.0));
        buffer.set_wrap(&mut font_system, to_wrap(wrapping));
        buffer.set_size(&mut font_system, Some(max_width), None);
        buffer.set_text(
            &mut font_system,
            text,
            &to_attributes(Font::DEFAULT),
            cosmic_text::Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(&mut font_system, false);

        let width = fit_width(&mut buffer, &mut font_system, wrapping, max_width);
        let lines = buffer
            .layout_runs()
            .map(|run| {
                let start = run
                    .glyphs
                    .iter()
                    .map(|glyph| glyph.start)
                    .min()
                    .unwrap_or(0);
                let end = run.glyphs.iter().map(|glyph| glyph.end).max().unwrap_or(0);

                run.text[start..end].trim().to_owned()
            })
            .collect();

        (width, lines)
    }

    #[cfg(feature = "fira-sans")]
    const HEADING: &str = "Turn a personal idea into a project.";

    #[cfg(feature = "fira-sans")]
    #[test]
    fn balanced_lines_come_out_about_even() {
        // Just too narrow for one line: a word wrap leaves the last word alone.
        let max_width = advance(HEADING, Font::DEFAULT) - 1.0;

        let (word_width, word) = wrap(HEADING, Wrapping::Word, max_width);
        assert_eq!(word_width, max_width);
        assert_eq!(word, ["Turn a personal idea into a", "project."]);

        // Breaks move; the width stays, as in a browser.
        let (width, balanced) = wrap(HEADING, Wrapping::Balanced, max_width);
        assert_eq!(width, max_width);
        assert_eq!(balanced, ["Turn a personal", "idea into a project."]);
    }

    #[cfg(feature = "fira-sans")]
    #[test]
    fn pretty_text_does_not_end_on_one_word() {
        let max_width = advance(HEADING, Font::DEFAULT) - 1.0;

        let (width, pretty) = wrap(HEADING, Wrapping::Pretty, max_width);
        assert_eq!(width, max_width);
        assert_eq!(pretty, ["Turn a personal idea into", "a project."]);
    }

    #[cfg(feature = "fira-sans")]
    #[test]
    fn text_that_already_reads_well_keeps_its_width() {
        let width = advance("Turn a personal idea", Font::DEFAULT) + 4.0;

        let (pretty_width, pretty) = wrap(HEADING, Wrapping::Pretty, width);
        assert_eq!(pretty_width, width);
        assert_eq!(pretty, ["Turn a personal idea", "into a project."]);

        let one_line = advance(HEADING, Font::DEFAULT) + 4.0;
        assert_eq!(wrap(HEADING, Wrapping::Balanced, one_line).0, one_line);
        assert_eq!(wrap(HEADING, Wrapping::Pretty, one_line).0, one_line);
    }

    #[cfg(feature = "fira-sans")]
    #[test]
    fn long_or_broken_text_wraps_as_words() {
        let long = HEADING.repeat(8);
        let max_width = advance(HEADING, Font::DEFAULT);
        let (width, lines) = wrap(&long, Wrapping::Balanced, max_width);
        assert!(lines.len() > wrap::BALANCE_MAX_LINES);
        assert_eq!(width, max_width);

        let broken = "Turn a personal idea\ninto a project, then share it";
        let max_width = advance("Turn a personal idea", Font::DEFAULT);
        assert_eq!(wrap(broken, Wrapping::Balanced, max_width).0, max_width);
        assert_eq!(wrap(broken, Wrapping::Pretty, max_width).0, max_width);
    }
}
