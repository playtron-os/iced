//! Line breaking for [`Wrapping::Balanced`] and [`Wrapping::Pretty`], as Blink
//! breaks CSS `text-wrap: balance` and `text-wrap: pretty`.
//!
//! Both start from the greedy breaks. Blink's `ScoreLineBreaker` then scores every
//! way of breaking the lines it may change, much like Knuth–Plass, and keeps the
//! best one if it needs as many lines; `balance` falls back to the narrowest width
//! that keeps the line count. The chosen breaks reach cosmic-text as
//! [`cosmic_text::BufferLine::set_wrap_breaks`].
use crate::core::text::Wrapping;

/// The most lines `balance` evens out; longer text wraps greedily.
pub const BALANCE_MAX_LINES: usize = 6;

/// The last lines of a paragraph `pretty` may break again.
const PRETTY_MAX_LINES: usize = 4;

const SCORE_INFINITY: f32 = f32::MAX;
const SCORE_OVERFULL: f32 = 1e12;
const LAST_LINE_PENALTY_MULTIPLIER: f32 = 4.0;
const ORPHANS_PENALTY: f32 = 10_000.0;

/// A `LayoutUnit`, Blink's unit for widths: what a line may exceed its width by
/// and still fit.
const LAYOUT_UNIT: f32 = 1.0 / 64.0;

/// Lays `buffer` out as `wrapping` asks within `max_width`, and returns the width
/// the text now wraps at.
pub fn fit(
    buffer: &mut cosmic_text::Buffer,
    font_system: &mut cosmic_text::FontSystem,
    wrapping: Wrapping,
    max_width: f32,
) -> f32 {
    // Breaks chosen at another width no longer apply.
    let mut stale = false;
    for line in &mut buffer.lines {
        stale |= line.set_wrap_breaks(Vec::new());
    }

    let width = if max_width.is_finite() {
        match wrapping {
            Wrapping::Pretty => {
                pretty(buffer, font_system, max_width);
                max_width
            }
            Wrapping::Balanced => balance(buffer, font_system, max_width),
            _ => max_width,
        }
    } else {
        max_width
    };

    if stale
        || buffer
            .lines
            .iter()
            .any(|line| !line.wrap_breaks().is_empty())
    {
        buffer.shape_until_scroll(font_system, false);
    }

    width
}

/// A place a line may end, as Blink's `LineBreakCandidate` keeps it.
#[derive(Debug, Clone, Copy)]
struct Candidate {
    /// Where the next line starts, as a byte offset.
    offset: usize,
    /// The pen position at the next line's start, after any spaces.
    no_break: f32,
    /// Where a line ending here ends, before any spaces.
    if_break: f32,
    penalty: f32,
}

/// One paragraph's greedy lines: where each starts and how wide it is.
struct Greedy {
    starts: Vec<usize>,
    widths: Vec<f32>,
}

impl Greedy {
    /// Whether a word too wide for the line overflows it. Blink then breaks
    /// the paragraph greedily: its scores and bisection cannot handle overflow.
    fn overflows(&self, width: f32) -> bool {
        self.widths.iter().any(|line| *line > width + LAYOUT_UNIT)
    }
}

fn greedy(
    buffer: &mut cosmic_text::Buffer,
    font_system: &mut cosmic_text::FontSystem,
    index: usize,
) -> Option<Greedy> {
    let layout = buffer.line_layout(font_system, index)?;
    let mut lines = Greedy {
        starts: Vec::with_capacity(layout.len()),
        widths: Vec::with_capacity(layout.len()),
    };

    for line in layout {
        lines
            .starts
            .push(line.glyphs.iter().map(|glyph| glyph.start).min()?);
        lines.widths.push(line.w);
    }

    Some(lines)
}

/// The break opportunities of a paragraph's text from `from` to its end, with a
/// sentinel at each end, measured from `from`. `None` for right-to-left or
/// mixed-direction text, which wraps greedily.
fn candidates(
    buffer: &mut cosmic_text::Buffer,
    font_system: &mut cosmic_text::FontSystem,
    index: usize,
    from: usize,
) -> Option<Vec<Candidate>> {
    let advance = cosmic_text::Advance {
        font_size: buffer.metrics().font_size,
        match_mono_width: buffer.monospace_width(),
        hinting: buffer.hinting(),
    };
    let end = buffer.lines.get(index)?.text().len();
    let shape = buffer.line_shape(font_system, index)?;

    if shape.rtl {
        return None;
    }

    let mut candidates = vec![Candidate {
        offset: from,
        no_break: 0.0,
        if_break: 0.0,
        penalty: 0.0,
    }];
    let (mut origin, mut x, mut line_end) = (None, 0.0_f32, None);

    for span in &shape.spans {
        if span.level.is_rtl() {
            return None;
        }

        for word in &span.words {
            let Some(start) = word.glyphs.iter().map(|glyph| glyph.start).min() else {
                continue;
            };
            let width = word.width_px(advance);

            if start < from {
                x += width;
                continue;
            }

            let origin = *origin.get_or_insert(x);

            if !word.blank {
                if let Some(line_end) = line_end {
                    candidates.push(Candidate {
                        offset: start,
                        no_break: x - origin,
                        if_break: line_end - origin,
                        penalty: 0.0,
                    });
                }

                line_end = Some(x + width);
            }

            x += width;
        }
    }

    let last = line_end
        .zip(origin)
        .map_or(0.0, |(end, origin)| end - origin);

    candidates.push(Candidate {
        offset: end,
        no_break: last,
        if_break: last,
        penalty: 0.0,
    });

    Some(candidates)
}

/// Blink's `ScoreLineBreaker`: the line starts after the first that score best,
/// if they need `lines` lines.
fn optimize(
    mut candidates: Vec<Candidate>,
    width: f32,
    font_size: f32,
    balanced: bool,
    lines: usize,
) -> Option<Vec<usize>> {
    // Two sentinels and at least three places to break.
    if candidates.len() < 5 {
        return None;
    }

    // A single word alone on the last line costs more.
    let last = candidates.len() - 2;
    candidates[last].penalty += ORPHANS_PENALTY;

    let hyphen_penalty = width * font_size * 2.0;
    let line_penalty = hyphen_penalty * 2.0;
    let fit = width + LAYOUT_UNIT;

    #[derive(Clone, Copy)]
    struct Score {
        score: f32,
        previous: usize,
        line: usize,
    }

    let mut scores = vec![Score {
        score: 0.0,
        previous: 0,
        line: 0,
    }];
    let mut active = 0;

    for end in 1..candidates.len() {
        let end_candidate = candidates[end];
        let is_last = end == candidates.len() - 1;
        let start_edge = end_candidate.if_break - fit;
        let (mut best, mut best_previous, mut best_hope) = (SCORE_INFINITY, 0, 0.0);

        // A start that overflows here overflows every later end too, so the
        // next end begins past it; this one still looks at all of them.
        let first = active;

        for start in first..end {
            let start_score = scores[start].score;

            if start_score + best_hope >= best {
                continue;
            }

            let start_candidate = candidates[start];
            let delta = start_candidate.no_break - start_edge;

            let (width_score, additional_penalty) = if delta < 0.0 {
                (SCORE_OVERFULL, 0.0)
            } else if is_last && !balanced {
                (0.0, LAST_LINE_PENALTY_MULTIPLIER * start_candidate.penalty)
            } else {
                (delta * delta, 0.0)
            };

            if delta < 0.0 {
                active = start + 1;
            } else {
                best_hope = width_score;
            }

            let score = start_score + width_score + additional_penalty;

            if score <= best {
                best = score;
                best_previous = start;
            }
        }

        scores.push(Score {
            score: best + end_candidate.penalty + line_penalty,
            previous: best_previous,
            line: scores[best_previous].line + 1,
        });
    }

    if scores.last()?.line != lines {
        return None;
    }

    let mut starts = Vec::with_capacity(lines);
    let mut index = scores[candidates.len() - 1].previous;

    while index > 0 {
        starts.push(candidates[index].offset);
        index = scores[index].previous;
    }

    starts.reverse();

    Some(starts)
}

/// `pretty`, paragraph by paragraph: when the greedy breaks leave a short word
/// alone on the last line, its last lines break again.
fn pretty(buffer: &mut cosmic_text::Buffer, font_system: &mut cosmic_text::FontSystem, width: f32) {
    let font_size = buffer.metrics().font_size;

    for index in 0..buffer.lines.len() {
        let Some(lines) = greedy(buffer, font_system, index) else {
            continue;
        };
        let count = lines.starts.len();

        if count < 2 || lines.widths[count - 1] >= width / 3.0 || lines.overflows(width) {
            continue;
        }

        let first = count.saturating_sub(PRETTY_MAX_LINES);
        let Some(candidates) = candidates(buffer, font_system, index, lines.starts[first]) else {
            continue;
        };

        // Only a last line with no place to break in it.
        let last_start = lines.starts[count - 1];
        let end = candidates.last().map_or(0, |candidate| candidate.offset);

        if candidates
            .iter()
            .any(|candidate| candidate.offset > last_start && candidate.offset < end)
        {
            continue;
        }

        let Some(starts) = optimize(candidates, width, font_size, false, count - first) else {
            continue;
        };

        let mut breaks = lines.starts[1..=first].to_vec();
        breaks.extend(starts);

        let _ = buffer.lines[index].set_wrap_breaks(breaks);
    }
}

/// `balance`: its scored breaks when they keep the line count, or else the
/// narrowest width that does.
fn balance(
    buffer: &mut cosmic_text::Buffer,
    font_system: &mut cosmic_text::FontSystem,
    width: f32,
) -> f32 {
    if buffer.lines.len() != 1 {
        return width;
    }

    let Some(lines) = greedy(buffer, font_system, 0) else {
        return width;
    };
    let count = lines.starts.len();

    if !(2..=BALANCE_MAX_LINES).contains(&count) || lines.overflows(width) {
        return width;
    }

    let font_size = buffer.metrics().font_size;

    if let Some(starts) = candidates(buffer, font_system, 0, 0)
        .and_then(|candidates| optimize(candidates, width, font_size, true, count))
    {
        let _ = buffer.lines[0].set_wrap_breaks(starts);
        return width;
    }

    bisect(buffer, font_system, width, &lines.widths)
}

/// Blink's `ParagraphLineBreaker` bisection, in `LayoutUnit`s: the narrowest
/// width, to a pixel, at which the text still wraps into as many lines.
fn bisect(
    buffer: &mut cosmic_text::Buffer,
    font_system: &mut cosmic_text::FontSystem,
    width: f32,
    widths: &[f32],
) -> f32 {
    let units = |pixels: f32| (pixels / LAYOUT_UNIT).round() as i64;
    let count = widths.len();

    let average = widths.iter().map(|width| units(*width)).sum::<i64>() / count as i64;
    let mut lower = units(average as f32 * LAYOUT_UNIT * 0.8);
    let mut upper = units(width);
    let epsilon = units(1.0);

    while lower + epsilon < upper {
        let middle = (upper + lower) / 2;
        buffer.set_size(font_system, Some(middle as f32 * LAYOUT_UNIT), None);

        if buffer.layout_runs().count() <= count {
            upper = middle;
        } else {
            lower = middle;
        }
    }

    let width = upper as f32 * LAYOUT_UNIT;
    buffer.set_size(font_system, Some(width), None);

    width
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::BTreeMap;

    /// Chrome's line starts; see the header of the file.
    const CHROME: &str = include_str!("wrap/chrome.txt");

    const GEIST: &[u8] = include_bytes!("../../../examples/letter_spacing/fonts/Geist-Regular.ttf");

    struct Case<'a> {
        mode: &'a str,
        size: f32,
        width: f32,
        text: usize,
        starts: Vec<usize>,
    }

    fn corpus() -> (Vec<&'static str>, Vec<Case<'static>>) {
        let (mut texts, mut cases) = (Vec::new(), Vec::new());

        for line in CHROME.lines().filter(|line| !line.starts_with('#')) {
            if let Some(text) = line.strip_prefix("text ") {
                let (_, text) = text.split_once(' ').expect("an index then the text");
                texts.push(text);
                continue;
            }

            let fields: Vec<&str> = line.split(' ').collect();
            cases.push(Case {
                mode: fields[0],
                size: fields[1].parse().expect("a size"),
                width: fields[2].parse().expect("a width"),
                text: fields[3].parse().expect("a text index"),
                starts: fields[4]
                    .split(',')
                    .map(|start| start.parse().expect("a start"))
                    .collect(),
            });
        }

        (texts, cases)
    }

    fn font_system() -> cosmic_text::FontSystem {
        let mut database = cosmic_text::fontdb::Database::new();
        database.load_font_data(GEIST.to_vec());
        database.set_sans_serif_family("Geist");
        cosmic_text::FontSystem::new_with_locale_and_db("en-US".into(), database)
    }

    /// Where each line of `text` starts, as character offsets, laid out as
    /// `iced_graphics` lays out a paragraph.
    fn starts(
        font_system: &mut cosmic_text::FontSystem,
        text: &str,
        size: f32,
        width: f32,
        wrapping: Wrapping,
    ) -> Vec<usize> {
        let mut buffer =
            cosmic_text::Buffer::new(font_system, cosmic_text::Metrics::new(size, size * 1.5));
        buffer.set_wrap(font_system, super::super::to_wrap(wrapping));
        buffer.set_size(font_system, Some(width), None);
        buffer.set_text(
            font_system,
            text,
            &cosmic_text::Attrs::new().family(cosmic_text::Family::Name("Geist")),
            cosmic_text::Shaping::Advanced,
            None,
        );
        let _ = fit(&mut buffer, font_system, wrapping, width);

        buffer
            .layout_runs()
            .filter_map(|run| run.glyphs.iter().map(|glyph| glyph.start).min())
            .map(|byte| text[..byte].chars().count())
            .collect()
    }

    fn wrapping(mode: &str) -> Wrapping {
        match mode {
            "pretty" => Wrapping::Pretty,
            "balance" => Wrapping::Balanced,
            _ => Wrapping::Word,
        }
    }

    #[test]
    fn lines_break_where_chrome_breaks_them() {
        let (texts, cases) = corpus();
        let mut font_system = font_system();
        let mut matched: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
        let mut misses = Vec::new();

        for case in &cases {
            let text = texts[case.text];
            let ours = starts(
                &mut font_system,
                text,
                case.size,
                case.width,
                wrapping(case.mode),
            );
            let tally = matched.entry(case.mode).or_default();
            tally.1 += 1;

            if ours == case.starts {
                tally.0 += 1;
            } else {
                misses.push(format!(
                    "{} {}px {}px #{}: chrome {:?} ours {:?}",
                    case.mode, case.size, case.width, case.text, case.starts, ours
                ));
            }
        }

        for (mode, (hit, total)) in &matched {
            eprintln!("{mode}: {hit} of {total}");
        }
        for miss in misses.iter().take(40) {
            eprintln!("{miss}");
        }

        assert!(
            misses.is_empty(),
            "{} cases break differently",
            misses.len()
        );
    }
}
