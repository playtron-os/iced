//! Draw paragraphs.
use crate::core;
use crate::core::alignment;
use crate::core::text::{
    Alignment, Ellipsis, Hit, LineHeight, Shaping, Span, Target, Text, Wrapping,
};
use crate::core::{Font, Pixels, Point, Rectangle, Size};
use crate::text;

use std::fmt;
use std::sync::{self, Arc};

/// A bunch of text.
#[derive(Clone, PartialEq)]
pub struct Paragraph(Arc<Internal>);

#[derive(Clone)]
struct Internal {
    buffer: cosmic_text::Buffer,
    font: Font,
    shaping: Shaping,
    wrapping: Wrapping,
    ellipsis: Ellipsis,
    align_x: Alignment,
    align_y: alignment::Vertical,
    bounds: Size,
    min_bounds: Size,
    truncated: bool,
    version: text::Version,
    hint: bool,
    hint_factor: f32,
    /// The device scale the paragraph was laid out at, when the renderer knew it.
    scale: Option<f32>,
    letter_spacing: Option<f32>,
    selection: Option<Selection>,
}

#[derive(Clone)]
struct Selection {
    start: cosmic_text::Cursor,
    end: cosmic_text::Cursor,
    regions: Vec<Rectangle>,
}

impl Paragraph {
    /// Creates a new empty [`Paragraph`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the buffer of the [`Paragraph`].
    pub fn buffer(&self) -> &cosmic_text::Buffer {
        &self.internal().buffer
    }

    /// The device scale the [`Paragraph`] was laid out at, when the renderer knew it.
    pub fn layout_scale(&self) -> Option<f32> {
        self.internal().scale
    }

    /// Creates a [`Weak`] reference to the [`Paragraph`].
    ///
    /// This is useful to avoid cloning the [`Paragraph`] when
    /// referential guarantees are unnecessary. For instance,
    /// when creating a rendering tree.
    pub fn downgrade(&self) -> Weak {
        let paragraph = self.internal();

        Weak {
            raw: Arc::downgrade(paragraph),
            min_bounds: paragraph.min_bounds,
            align_x: paragraph.align_x,
            align_y: paragraph.align_y,
        }
    }

    fn internal(&self) -> &Arc<Internal> {
        &self.0
    }
}

impl core::text::Paragraph for Paragraph {
    type Font = Font;

    fn with_text(text: Text<&str>) -> Self {
        log::trace!("Allocating plain paragraph: {}", text.content);

        let mut font_system = text::font_system().write().expect("Write font system");

        let (hint, hint_factor) = match text::hint_factor(text.size, text.hint_factor) {
            Some(hint_factor) => (true, hint_factor),
            _ => (false, 1.0),
        };

        let mut buffer = cosmic_text::Buffer::new(
            font_system.raw(),
            cosmic_text::Metrics::new(
                f32::from(text.size) * hint_factor,
                f32::from(text.line_height.to_absolute(text.size)) * hint_factor,
            ),
        );

        if hint {
            buffer.set_hinting(font_system.raw(), cosmic_text::Hinting::Enabled);
        }

        buffer.set_size(
            font_system.raw(),
            Some(text.bounds.width * hint_factor),
            Some(text.bounds.height * hint_factor),
        );

        buffer.set_wrap(font_system.raw(), text::to_wrap(text.wrapping));

        // First measure WITHOUT ellipsis and WITHOUT height limit to get natural bounds
        // (for truncation detection). The height must be unconstrained so we can detect
        // when text would need more vertical space than the bounds allow.
        buffer.set_ellipsize(font_system.raw(), cosmic_text::Ellipsize::None);
        buffer.set_size(
            font_system.raw(),
            Some(text.bounds.width * hint_factor),
            None,
        );

        buffer.set_text(
            font_system.raw(),
            text.content,
            &text::to_attributes_with_spacing(text.font, text.letter_spacing, f32::from(text.size)),
            text::to_shaping(text.shaping, text.content),
            None,
        );

        let width = text::fit_width(
            &mut buffer,
            font_system.raw(),
            text.wrapping,
            text.bounds.width * hint_factor,
        );

        let (natural_min_bounds, _) = text::measure(&buffer);
        let natural_min_bounds = natural_min_bounds / hint_factor;

        // Now restore height limit and set actual ellipsis, then re-measure
        buffer.set_size(
            font_system.raw(),
            Some(width),
            Some(text.bounds.height * hint_factor),
        );
        buffer.set_ellipsize(
            font_system.raw(),
            text::to_ellipsize(text.ellipsis, text.bounds.height * hint_factor),
        );

        let min_bounds = text::align(&mut buffer, font_system.raw(), text.align_x) / hint_factor;

        let truncated = text.ellipsis != Ellipsis::None
            && (natural_min_bounds.width > text.bounds.width
                || natural_min_bounds.height >= text.bounds.height
                || natural_min_bounds != min_bounds);

        Self(Arc::new(Internal {
            buffer,
            hint,
            hint_factor,
            scale: text.hint_factor,
            font: text.font,
            align_x: text.align_x,
            align_y: text.align_y,
            shaping: text.shaping,
            wrapping: text.wrapping,
            ellipsis: text.ellipsis,
            bounds: text.bounds,
            min_bounds,
            truncated,
            version: font_system.version(),
            letter_spacing: text.letter_spacing,
            selection: None,
        }))
    }

    fn with_spans<Link>(text: Text<&[Span<'_, Link>]>) -> Self {
        log::trace!("Allocating rich paragraph: {} spans", text.content.len());

        let mut font_system = text::font_system().write().expect("Write font system");

        let (hint, hint_factor) = match text::hint_factor(text.size, text.hint_factor) {
            Some(hint_factor) => (true, hint_factor),
            _ => (false, 1.0),
        };

        let mut buffer = cosmic_text::Buffer::new(
            font_system.raw(),
            cosmic_text::Metrics::new(
                f32::from(text.size) * hint_factor,
                f32::from(text.line_height.to_absolute(text.size)) * hint_factor,
            ),
        );

        if hint {
            buffer.set_hinting(font_system.raw(), cosmic_text::Hinting::Enabled);
        }

        buffer.set_size(
            font_system.raw(),
            Some(text.bounds.width * hint_factor),
            None,
        );

        buffer.set_wrap(font_system.raw(), text::to_wrap(text.wrapping));

        buffer.set_rich_text(
            font_system.raw(),
            text.content.iter().enumerate().map(|(i, span)| {
                let span_font = span.font.unwrap_or(text.font);
                let span_size = span.size.unwrap_or(text.size);
                let span_letter_spacing = span.letter_spacing.or(text.letter_spacing);
                let attrs = text::to_attributes_with_spacing(
                    span_font,
                    span_letter_spacing,
                    f32::from(span_size),
                );

                let attrs = match (span.size, span.line_height) {
                    (None, None) => attrs,
                    _ => {
                        let size = span.size.unwrap_or(text.size);

                        attrs.metrics(cosmic_text::Metrics::new(
                            f32::from(size) * hint_factor,
                            f32::from(
                                span.line_height
                                    .unwrap_or(text.line_height)
                                    .to_absolute(size),
                            ) * hint_factor,
                        ))
                    }
                };

                let attrs = if let Some(color) = span.color {
                    attrs.color(text::to_color(color))
                } else {
                    attrs
                };

                (span.text.as_ref(), attrs.metadata(i))
            }),
            &text::to_attributes_with_spacing(text.font, text.letter_spacing, f32::from(text.size)),
            text::to_shaping_for_spans(text.shaping, text.content),
            None,
        );

        let width = text::fit_width(
            &mut buffer,
            font_system.raw(),
            text.wrapping,
            text.bounds.width * hint_factor,
        );

        // Measure without ellipsis to get natural bounds (for truncation detection)
        let (natural_min_bounds, _) = text::measure(&buffer);
        let natural_min_bounds = natural_min_bounds / hint_factor;

        // Restore height limit and apply ellipsis truncation
        buffer.set_size(
            font_system.raw(),
            Some(width),
            Some(text.bounds.height * hint_factor),
        );
        buffer.set_ellipsize(
            font_system.raw(),
            text::to_ellipsize(text.ellipsis, text.bounds.height * hint_factor),
        );

        let min_bounds = text::align(&mut buffer, font_system.raw(), text.align_x) / hint_factor;

        let truncated = text.ellipsis != Ellipsis::None
            && (natural_min_bounds.width > text.bounds.width
                || natural_min_bounds.height >= text.bounds.height
                || natural_min_bounds != min_bounds);

        Self(Arc::new(Internal {
            buffer,
            hint,
            hint_factor,
            scale: text.hint_factor,
            font: text.font,
            align_x: text.align_x,
            align_y: text.align_y,
            shaping: text.shaping,
            wrapping: text.wrapping,
            ellipsis: text.ellipsis,
            bounds: text.bounds,
            min_bounds,
            truncated,
            version: font_system.version(),
            letter_spacing: text.letter_spacing,
            selection: None,
        }))
    }

    fn resize(&mut self, new_bounds: Size) {
        let paragraph = Arc::make_mut(&mut self.0);

        let mut font_system = text::font_system().write().expect("Write font system");

        // Measure without ellipsis and without height limit for truncation detection
        paragraph
            .buffer
            .set_ellipsize(font_system.raw(), cosmic_text::Ellipsize::None);
        paragraph.buffer.set_size(
            font_system.raw(),
            Some(new_bounds.width * paragraph.hint_factor),
            None,
        );
        let width = text::fit_width(
            &mut paragraph.buffer,
            font_system.raw(),
            paragraph.wrapping,
            new_bounds.width * paragraph.hint_factor,
        );
        let (natural_min_bounds, _) = text::measure(&paragraph.buffer);
        let natural_min_bounds = natural_min_bounds / paragraph.hint_factor;

        // Restore height limit and re-apply ellipsis
        paragraph.buffer.set_size(
            font_system.raw(),
            Some(width),
            Some(new_bounds.height * paragraph.hint_factor),
        );
        paragraph.buffer.set_ellipsize(
            font_system.raw(),
            text::to_ellipsize(
                paragraph.ellipsis,
                new_bounds.height * paragraph.hint_factor,
            ),
        );

        let min_bounds = text::align(&mut paragraph.buffer, font_system.raw(), paragraph.align_x)
            / paragraph.hint_factor;

        paragraph.bounds = new_bounds;
        paragraph.min_bounds = min_bounds;
        paragraph.truncated = paragraph.ellipsis != Ellipsis::None
            && (natural_min_bounds.width > new_bounds.width
                || natural_min_bounds.height >= new_bounds.height
                || natural_min_bounds != min_bounds);

        // The cursors survive a re-wrap; where their lines fall does not.
        if let Some(selection) = paragraph.selection.as_mut() {
            selection.regions = regions(
                &paragraph.buffer,
                selection.start,
                selection.end,
                paragraph.hint_factor,
            );
        }
    }

    fn compare(&self, text: Text<()>) -> core::text::Difference {
        let font_system = text::font_system().read().expect("Read font system");
        let paragraph = self.internal();
        let metrics = paragraph.buffer.metrics();

        if paragraph.version != font_system.version
            || metrics.font_size != text.size.0 * paragraph.hint_factor
            || metrics.line_height
                != text.line_height.to_absolute(text.size).0 * paragraph.hint_factor
            || paragraph.font != text.font
            || paragraph.shaping != text.shaping
            || paragraph.wrapping != text.wrapping
            || paragraph.ellipsis != text.ellipsis
            || paragraph.align_x != text.align_x
            || paragraph.align_y != text.align_y
            || paragraph.letter_spacing != text.letter_spacing
            || paragraph.hint.then_some(paragraph.hint_factor)
                != text::hint_factor(text.size, text.hint_factor)
            || paragraph.scale != text.hint_factor
        {
            core::text::Difference::Shape
        } else if paragraph.bounds != text.bounds {
            core::text::Difference::Bounds
        } else {
            core::text::Difference::None
        }
    }

    fn hint_factor(&self) -> Option<f32> {
        self.0.hint.then_some(self.0.hint_factor)
    }

    fn letter_spacing(&self) -> Option<f32> {
        self.0.letter_spacing
    }

    fn size(&self) -> Pixels {
        Pixels(self.0.buffer.metrics().font_size / self.0.hint_factor)
    }

    fn font(&self) -> Font {
        self.0.font
    }

    fn line_height(&self) -> LineHeight {
        LineHeight::Absolute(Pixels(
            self.0.buffer.metrics().line_height / self.0.hint_factor,
        ))
    }

    fn align_x(&self) -> Alignment {
        self.internal().align_x
    }

    fn align_y(&self) -> alignment::Vertical {
        self.internal().align_y
    }

    fn wrapping(&self) -> Wrapping {
        self.0.wrapping
    }

    fn ellipsis(&self) -> Ellipsis {
        self.0.ellipsis
    }

    fn shaping(&self) -> Shaping {
        self.0.shaping
    }

    fn bounds(&self) -> Size {
        self.0.bounds
    }

    fn min_bounds(&self) -> Size {
        self.internal().min_bounds
    }

    fn first_baseline(&self) -> Option<f32> {
        let internal = self.internal();
        let run = internal.buffer.layout_runs().next()?;
        let top = run.line_top / internal.hint_factor;

        // Where the renderer draws it at this scale, so baselines line up to the pixel.
        match internal.scale.filter(|scale| *scale > 0.0) {
            Some(scale) => text::browser_baseline(&internal.buffer, scale / internal.hint_factor)
                .map(|below| top + below / scale),
            None => Some(run.line_y / internal.hint_factor),
        }
    }

    fn is_truncated(&self) -> bool {
        self.internal().truncated
    }

    fn select(&mut self, start: Point, end: Point, target: Target) {
        use cosmic_text::Edit;

        let height = self.min_bounds().height;
        let above = |point: Point| point.y < 0.0;
        let below = |point: Point| point.y > height;

        // A drag that passes wholly above or below this text selects none of it.
        if (above(start) && above(end)) || (below(start) && below(end)) {
            self.deselect();
            return;
        }

        // Words and lines grow only around a start inside this text; one that
        // began elsewhere takes everything up to its edge.
        let target = if above(start) || below(start) {
            Target::Character
        } else {
            target
        };

        let paragraph = Arc::make_mut(&mut self.0);
        let hint_factor = paragraph.hint_factor;
        let mut font_system = text::font_system().write().expect("Write font system");

        let mut editor =
            cosmic_text::Editor::new(cosmic_text::BufferRef::Borrowed(&mut paragraph.buffer));

        let x = (start.x * hint_factor) as i32;
        let y = (start.y * hint_factor) as i32;

        editor.action(
            font_system.raw(),
            match target {
                Target::Character => cosmic_text::Action::Click { x, y },
                Target::Word => cosmic_text::Action::DoubleClick { x, y },
                Target::Line => cosmic_text::Action::TripleClick { x, y },
            },
        );

        editor.action(
            font_system.raw(),
            cosmic_text::Action::Drag {
                x: (end.x * hint_factor) as i32,
                y: (end.y * hint_factor) as i32,
            },
        );

        let bounds = editor.selection_bounds();
        drop(editor);

        paragraph.selection = bounds.map(|(start, end)| Selection {
            start,
            end,
            regions: regions(&paragraph.buffer, start, end, hint_factor),
        });
    }

    fn select_all(&mut self) {
        let paragraph = Arc::make_mut(&mut self.0);

        let start = cosmic_text::Cursor::new(0, 0);
        let end = cosmic_text::Cursor::new(
            paragraph.buffer.lines.len().saturating_sub(1),
            paragraph
                .buffer
                .lines
                .last()
                .map(|line| line.text().len())
                .unwrap_or_default(),
        );

        paragraph.selection = Some(Selection {
            start,
            end,
            regions: regions(&paragraph.buffer, start, end, paragraph.hint_factor),
        });
    }

    fn deselect(&mut self) {
        if self.internal().selection.is_some() {
            Arc::make_mut(&mut self.0).selection = None;
        }
    }

    fn selection(&self) -> &[Rectangle] {
        self.internal()
            .selection
            .as_ref()
            .map(|selection| selection.regions.as_slice())
            .unwrap_or_default()
    }

    fn copy(&mut self) -> Option<String> {
        use cosmic_text::Edit;

        let paragraph = Arc::make_mut(&mut self.0);
        let selection = paragraph.selection.clone()?;

        let mut editor =
            cosmic_text::Editor::new(cosmic_text::BufferRef::Borrowed(&mut paragraph.buffer));

        editor.set_selection(cosmic_text::Selection::Normal(selection.start));
        editor.set_cursor(selection.end);
        editor.copy_selection()
    }

    fn hit_test(&self, point: Point) -> Option<Hit> {
        let cursor = self
            .internal()
            .buffer
            .hit(point.x * self.0.hint_factor, point.y * self.0.hint_factor)?;

        Some(Hit::CharOffset(cursor.index))
    }

    fn hit_span(&self, point: Point) -> Option<usize> {
        let internal = self.internal();

        let cursor = internal
            .buffer
            .hit(point.x * self.0.hint_factor, point.y * self.0.hint_factor)?;
        let line = internal.buffer.lines.get(cursor.line)?;

        if cursor.index >= line.text().len() {
            return None;
        }

        let index = match cursor.affinity {
            cosmic_text::Affinity::Before => cursor.index.saturating_sub(1),
            cosmic_text::Affinity::After => cursor.index,
        };

        let mut hit = None;
        let glyphs = line
            .layout_opt()
            .as_ref()?
            .iter()
            .flat_map(|line| line.glyphs.iter());

        for glyph in glyphs {
            if glyph.start <= index && index < glyph.end {
                hit = Some(glyph);
                break;
            }
        }

        Some(hit?.metadata)
    }

    fn span_bounds(&self, index: usize) -> Vec<Rectangle> {
        let internal = self.internal();

        let mut bounds = Vec::new();
        let mut current_bounds = None;

        let glyphs = internal
            .buffer
            .layout_runs()
            .flat_map(|run| {
                let line_top = run.line_top;
                let line_height = run.line_height;

                run.glyphs
                    .iter()
                    .map(move |glyph| (line_top, line_height, glyph))
            })
            .skip_while(|(_, _, glyph)| glyph.metadata != index)
            .take_while(|(_, _, glyph)| glyph.metadata == index)
            // Skip ellipsis glyphs (they have start == end set to the elision boundary)
            .filter(|(_, _, glyph)| !internal.truncated || glyph.start != glyph.end);

        for (line_top, line_height, glyph) in glyphs {
            let y = line_top + glyph.y;

            let new_bounds = || {
                Rectangle::new(
                    Point::new(glyph.x, y),
                    Size::new(glyph.w, glyph.line_height_opt.unwrap_or(line_height)),
                ) * (1.0 / self.0.hint_factor)
            };

            match current_bounds.as_mut() {
                None => {
                    current_bounds = Some(new_bounds());
                }
                Some(current_bounds) if y != current_bounds.y => {
                    bounds.push(*current_bounds);
                    *current_bounds = new_bounds();
                }
                Some(current_bounds) => {
                    current_bounds.width += glyph.w / self.0.hint_factor;
                }
            }
        }

        bounds.extend(current_bounds);
        bounds
    }

    fn selection_bounds(&self, start: usize, end: usize) -> Vec<Rectangle> {
        if start >= end {
            return Vec::new();
        }

        let internal = self.internal();
        let mut bounds = Vec::new();
        let mut current_bounds: Option<Rectangle> = None;

        for run in internal.buffer.layout_runs() {
            let line_top = run.line_top;
            let line_height = run.line_height;

            // Track character position within this run
            let run_start = run.glyphs.first().map(|g| g.start).unwrap_or(0);
            let run_end = run.glyphs.last().map(|g| g.end).unwrap_or(0);

            // Skip runs that don't overlap with selection
            if run_end <= start || run_start >= end {
                continue;
            }

            for glyph in run.glyphs.iter() {
                // Check if this glyph overlaps with selection
                if glyph.end <= start || glyph.start >= end {
                    continue;
                }

                let y = line_top + glyph.y;
                let glyph_height = glyph.line_height_opt.unwrap_or(line_height);

                let new_rect =
                    Rectangle::new(Point::new(glyph.x, y), Size::new(glyph.w, glyph_height))
                        * (1.0 / self.0.hint_factor);

                match current_bounds.as_mut() {
                    None => {
                        current_bounds = Some(new_rect);
                    }
                    Some(curr) if (y - curr.y).abs() > 0.5 => {
                        // Different line, push current and start new
                        bounds.push(*curr);
                        current_bounds = Some(new_rect);
                    }
                    Some(curr) => {
                        // Same line, extend width
                        curr.width += glyph.w / self.0.hint_factor;
                    }
                }
            }
        }

        bounds.extend(current_bounds);
        bounds
    }

    fn grapheme_position(&self, line: usize, index: usize) -> Option<Point> {
        use unicode_segmentation::UnicodeSegmentation;

        let run = self.internal().buffer.layout_runs().nth(line)?;

        // index represents a grapheme, not a glyph
        // Let's find the first glyph for the given grapheme cluster
        let mut last_start = None;
        let mut last_grapheme_count = 0;
        let mut graphemes_seen = 0;

        let glyph = run
            .glyphs
            .iter()
            .find(|glyph| {
                if Some(glyph.start) != last_start {
                    last_grapheme_count = run.text[glyph.start..glyph.end].graphemes(false).count();
                    last_start = Some(glyph.start);
                    graphemes_seen += last_grapheme_count;
                }

                graphemes_seen >= index
            })
            .or_else(|| run.glyphs.last())?;

        let advance = if index == 0 {
            0.0
        } else {
            glyph.w
                * (1.0
                    - graphemes_seen.saturating_sub(index) as f32
                        / last_grapheme_count.max(1) as f32)
        };

        Some(Point::new(
            (glyph.x + glyph.x_offset * glyph.font_size + advance) / self.0.hint_factor,
            (glyph.y - glyph.y_offset * glyph.font_size) / self.0.hint_factor,
        ))
    }
}

impl Default for Paragraph {
    fn default() -> Self {
        Self(Arc::new(Internal::default()))
    }
}

impl fmt::Debug for Paragraph {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let paragraph = self.internal();

        f.debug_struct("Paragraph")
            .field("font", &paragraph.font)
            .field("shaping", &paragraph.shaping)
            .field("horizontal_alignment", &paragraph.align_x)
            .field("vertical_alignment", &paragraph.align_y)
            .field("bounds", &paragraph.bounds)
            .field("min_bounds", &paragraph.min_bounds)
            .finish()
    }
}

impl PartialEq for Internal {
    fn eq(&self, other: &Self) -> bool {
        self.font == other.font
            && self.shaping == other.shaping
            && self.align_x == other.align_x
            && self.align_y == other.align_y
            && self.bounds == other.bounds
            && self.min_bounds == other.min_bounds
            && self.letter_spacing == other.letter_spacing
            && self.buffer.metrics() == other.buffer.metrics()
    }
}

impl Default for Internal {
    fn default() -> Self {
        Self {
            buffer: cosmic_text::Buffer::new_empty(cosmic_text::Metrics {
                font_size: 1.0,
                line_height: 1.0,
            }),
            font: Font::default(),
            shaping: Shaping::default(),
            wrapping: Wrapping::default(),
            ellipsis: Ellipsis::default(),
            align_x: Alignment::Default,
            align_y: alignment::Vertical::Top,
            bounds: Size::ZERO,
            min_bounds: Size::ZERO,
            truncated: false,
            version: text::Version::default(),
            hint: false,
            hint_factor: 1.0,
            scale: None,
            letter_spacing: None,
            selection: None,
        }
    }
}

/// A weak reference to a [`Paragraph`].
#[derive(Debug, Clone)]
pub struct Weak {
    raw: sync::Weak<Internal>,
    /// The minimum bounds of the [`Paragraph`].
    pub min_bounds: Size,
    /// The horizontal alignment of the [`Paragraph`].
    pub align_x: Alignment,
    /// The vertical alignment of the [`Paragraph`].
    pub align_y: alignment::Vertical,
}

impl Weak {
    /// Tries to update the reference into a [`Paragraph`].
    pub fn upgrade(&self) -> Option<Paragraph> {
        self.raw.upgrade().map(Paragraph)
    }
}

impl PartialEq for Weak {
    fn eq(&self, other: &Self) -> bool {
        match (self.raw.upgrade(), other.raw.upgrade()) {
            (Some(p1), Some(p2)) => p1 == p2,
            _ => false,
        }
    }
}

/// The rectangles a selection between two cursors covers, one per visual line,
/// in the paragraph's own (unhinted) coordinates. A line the selection runs
/// past reaches to its end, so a multi-line selection reads as one block.
fn regions(
    buffer: &cosmic_text::Buffer,
    start: cosmic_text::Cursor,
    end: cosmic_text::Cursor,
    hint_factor: f32,
) -> Vec<Rectangle> {
    let scroll = buffer.scroll();
    let mut regions = Vec::new();

    for run in buffer.layout_runs() {
        if run.line_i < start.line || run.line_i > end.line {
            continue;
        }

        let runs_on = end.line > run.line_i;
        let line_break = run.line_height / 2.0;

        let (min, max) = match run.highlight(start, end) {
            Some((x, width)) => {
                let (mut min, mut max) = (x, x + width);

                if runs_on {
                    if run.rtl {
                        min = 0.0;
                    } else {
                        max = run.line_w + line_break;
                    }
                }

                (min, max)
            }
            // An empty line inside the selection still shows a sliver.
            None if run.glyphs.is_empty() && runs_on => (0.0, line_break),
            None => continue,
        };

        regions.push(Rectangle {
            x: (min - scroll.horizontal) / hint_factor,
            y: run.line_top / hint_factor,
            width: (max - min).max(0.0) / hint_factor,
            height: run.line_height / hint_factor,
        });
    }

    regions
}
