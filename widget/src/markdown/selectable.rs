//! Selectable markdown support.
//!
//! This module provides a wrapper that adds text selection capability to markdown views.

use crate::core::clipboard::{self, Clipboard};
use crate::core::font::Font;
use crate::core::keyboard;
use crate::core::layout;
use crate::core::mouse::{self, click};
use crate::core::renderer;
use crate::core::widget::tree::{self, Tree};
use crate::core::{self, Element, Event, Layout, Length, Point, Rectangle, Shell, Size, Widget};

use super::{Catalog, Item, Style, Text};

/// A wrapper widget that adds text selection capability to markdown content.
///
/// This widget wraps the output of `markdown::view` and adds:
/// - Click and drag to select text
/// - Double-click to select line
/// - Ctrl+A to select all
/// - Ctrl+C to copy selected text
/// - Escape to clear selection
pub struct Selectable<'a, Message, Theme = crate::Theme, Renderer = crate::Renderer>
where
    Theme: Catalog,
    Renderer: core::text::Renderer<Font = Font>,
{
    content: Element<'a, Message, Theme, Renderer>,
    items: &'a [Item],
    width: Length,
    height: Length,
}

impl<'a, Message, Theme, Renderer> Selectable<'a, Message, Theme, Renderer>
where
    Message: Clone + 'a,
    Theme: Catalog + 'a,
    Renderer: core::text::Renderer<Font = Font> + 'a,
{
    /// Creates a new selectable markdown wrapper.
    ///
    /// # Example
    /// ```ignore
    /// use iced::widget::markdown;
    ///
    /// let items: Vec<markdown::Item> = markdown::parse("# Hello").collect();
    /// let view = markdown::Selectable::new(
    ///     markdown::view(&items, Theme::Dark),
    ///     &items,
    /// );
    /// ```
    pub fn new(
        content: impl Into<Element<'a, Message, Theme, Renderer>>,
        items: &'a [Item],
    ) -> Self {
        Self {
            content: content.into(),
            items,
            width: Length::Fill,
            height: Length::Shrink,
        }
    }

    /// Sets the width of the selectable markdown.
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    /// Sets the height of the selectable markdown.
    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.height = height.into();
        self
    }
}

/// Internal state for selection.
#[derive(Debug, Default)]
pub struct State {
    /// Flattened line boundaries: each visual line gets its own (start, end) range.
    /// This includes individual list items, code lines, paragraphs, etc.
    line_boundaries: Vec<(usize, usize)>,
    /// Total text length across all lines.
    total_length: usize,
    /// Selection as (anchor, focus). Anchor is where selection started, focus is current position.
    selection: Option<(usize, usize)>,
    /// Whether we're currently dragging.
    is_dragging: bool,
    /// Last click for double/triple click detection.
    last_click: Option<click::Click>,
    /// Current keyboard modifiers.
    modifiers: keyboard::Modifiers,
    /// Whether the widget has focus.
    is_focused: bool,
}

impl State {
    /// Returns the normalized selection range (start, end) where start <= end.
    pub fn selection_range(&self) -> Option<(usize, usize)> {
        self.selection
            .map(|(anchor, focus)| (anchor.min(focus), anchor.max(focus)))
    }
}

impl<'a, Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for Selectable<'a, Message, Theme, Renderer>
where
    Message: Clone + 'a,
    Theme: Catalog + 'a,
    Renderer: core::text::Renderer<Font = Font> + 'a,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: self.width,
            height: self.height,
        }
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let state = tree.state.downcast_mut::<State>();

        // Compute flattened line boundaries - each visual line gets its own range
        state.line_boundaries.clear();
        let mut offset = 0;

        for item in self.items.iter() {
            flatten_item_boundaries(item, &mut state.line_boundaries, &mut offset);
        }
        state.total_length = offset;

        // Layout the inner content
        let child_limits = limits.width(self.width).height(self.height);
        let child_node =
            self.content
                .as_widget_mut()
                .layout(&mut tree.children[0], renderer, &child_limits);

        layout::Node::with_children(child_node.size(), vec![child_node])
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        defaults: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        if !layout.bounds().intersects(viewport) {
            return;
        }

        // Draw the inner content FIRST
        if let Some(child_layout) = layout.children().next() {
            self.content.as_widget().draw(
                &tree.children[0],
                renderer,
                theme,
                defaults,
                child_layout,
                cursor,
                viewport,
            );
        }

        let state = tree.state.downcast_ref::<State>();

        // Draw selection highlights AFTER content (as overlay) so it's visible over code blocks
        if let Some((anchor, focus)) = state.selection {
            let sel_start = anchor.min(focus);
            let sel_end = anchor.max(focus);

            if sel_start != sel_end && !state.line_boundaries.is_empty() {
                if let Some(child_layout) = layout.children().next() {
                    // Collect visual lines
                    let mut visual_lines: Vec<Rectangle> = Vec::new();
                    collect_visual_lines(child_layout, &mut visual_lines);

                    // Expand to match boundary count
                    let expanded_lines = expand_visual_lines_simple(&visual_lines, state.line_boundaries.len());

                    // Draw selection for each line
                    for (i, line_bounds) in expanded_lines.iter().enumerate() {
                        if let Some((line_start, line_end)) = state.line_boundaries.get(i) {
                            draw_line_selection(
                                renderer,
                                *line_bounds,
                                sel_start,
                                sel_end,
                                *line_start,
                                *line_end,
                            );
                        }
                    }
                }
            }
        }
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();

        // Handle selection events FIRST
        let mut event_handled = false;

        match event {
            Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                state.modifiers = *modifiers;
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(position) = cursor.position_in(layout.bounds()) {
                    state.is_focused = true;
                    state.is_dragging = true;

                    let click = click::Click::new(position, mouse::Button::Left, state.last_click);

                    match click.kind() {
                        click::Kind::Single => {
                            if let Some(offset) = position_to_offset(position, layout, state) {
                                if state.modifiers.shift() {
                                    if let Some((anchor, _)) = state.selection {
                                        state.selection = Some((anchor, offset));
                                    } else {
                                        state.selection = Some((offset, offset));
                                    }
                                } else {
                                    state.selection = Some((offset, offset));
                                }
                            }
                        }
                        click::Kind::Double | click::Kind::Triple => {
                            if let Some(offset) = position_to_offset(position, layout, state) {
                                let (start, end) = find_line_boundaries(offset, state);
                                state.selection = Some((start, end));
                            }
                            event_handled = true; // Capture double/triple clicks
                        }
                    }

                    state.last_click = Some(click);
                    shell.request_redraw();
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if state.is_dragging {
                    if let Some(position) = cursor.position_in(layout.bounds()) {
                        if let Some(offset) = position_to_offset(position, layout, state) {
                            if let Some((anchor, _)) = state.selection {
                                state.selection = Some((anchor, offset));
                                shell.request_redraw();
                            }
                        }
                    }
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.is_dragging = false;
            }
            Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. })
                if state.is_focused =>
            {
                match (modifiers.command(), key) {
                    (true, keyboard::Key::Character(c)) if c.as_str() == "a" => {
                        state.selection = Some((0, state.total_length));
                        shell.request_redraw();
                        shell.capture_event();
                        event_handled = true;
                    }
                    (true, keyboard::Key::Character(c)) if c.as_str() == "c" => {
                        if let Some((start, end)) = state.selection_range() {
                            let selected_text = extract_text(start, end, self.items, state);
                            clipboard.write(clipboard::Kind::Standard, selected_text);
                        }
                        shell.capture_event();
                        event_handled = true;
                    }
                    (false, keyboard::Key::Named(keyboard::key::Named::Escape)) => {
                        state.selection = None;
                        state.is_focused = false;
                        shell.request_redraw();
                        shell.capture_event();
                        event_handled = true;
                    }
                    _ => {}
                }
            }
            _ => {}
        }

        // Let the inner content handle events only if we didn't fully handle them
        if !event_handled {
            if let Some(child_layout) = layout.children().next() {
                self.content.as_widget_mut().update(
                    &mut tree.children[0],
                    event,
                    child_layout,
                    cursor,
                    renderer,
                    clipboard,
                    shell,
                    viewport,
                );
            }
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        // Check inner content first
        if let Some(child_layout) = layout.children().next() {
            let inner = self.content.as_widget().mouse_interaction(
                &tree.children[0],
                child_layout,
                cursor,
                viewport,
                renderer,
            );
            if inner != mouse::Interaction::default() {
                return inner;
            }
        }

        if cursor.is_over(layout.bounds()) {
            mouse::Interaction::Text
        } else {
            mouse::Interaction::default()
        }
    }
}

/// Selection highlight color
const SELECTION_COLOR: core::Color = core::Color::from_rgba(0.3, 0.5, 0.9, 0.35);

/// Draws selection highlight for a single line.
fn draw_line_selection<Renderer: core::Renderer>(
    renderer: &mut Renderer,
    line_bounds: Rectangle,
    sel_start: usize,
    sel_end: usize,
    line_start: usize,
    line_end: usize,
) {
    if sel_end <= line_start || sel_start >= line_end {
        return;
    }

    let line_len = line_end - line_start;
    if line_len == 0 {
        return;
    }

    // Clamp selection to this line's range
    let local_sel_start = sel_start.saturating_sub(line_start);
    let local_sel_end = (sel_end - line_start).min(line_len);

    // Convert character positions to visual positions (ratio-based)
    let start_ratio = local_sel_start as f32 / line_len as f32;
    let end_ratio = local_sel_end as f32 / line_len as f32;

    let sel_x = line_bounds.x + (start_ratio * line_bounds.width);
    let sel_width = (end_ratio - start_ratio) * line_bounds.width;

    let selection_bounds = Rectangle {
        x: sel_x,
        y: line_bounds.y,
        width: sel_width,
        height: line_bounds.height,
    };

    renderer.fill_quad(
        renderer::Quad {
            bounds: selection_bounds,
            ..Default::default()
        },
        SELECTION_COLOR,
    );
}

/// Flattens item boundaries into individual lines.
/// Each visual line (paragraph, heading, list item content, code block line) gets its own entry.
fn flatten_item_boundaries(
    item: &Item,
    boundaries: &mut Vec<(usize, usize)>,
    offset: &mut usize,
) {
    match item {
        Item::Heading(_, text) | Item::Paragraph(text) => {
            let len = spans_text_length(text) + 1;
            boundaries.push((*offset, *offset + len));
            *offset += len;
        }
        Item::CodeBlock { code, .. } => {
            // Each line of code gets its own boundary
            for line in code.lines() {
                let len = line.len() + 1;
                boundaries.push((*offset, *offset + len));
                *offset += len;
            }
            // Handle case where code doesn't end with newline
            if !code.ends_with('\n') && !code.is_empty() {
                *offset += 1;
            }
        }
        Item::List { bullets, .. } => {
            for bullet in bullets {
                for sub_item in bullet.items() {
                    flatten_item_boundaries(sub_item, boundaries, offset);
                }
            }
        }
        Item::Quote(items) => {
            for sub_item in items {
                flatten_item_boundaries(sub_item, boundaries, offset);
            }
        }
        Item::Image { alt, .. } => {
            let len = spans_text_length(alt) + 1;
            boundaries.push((*offset, *offset + len));
            *offset += len;
        }
        Item::Rule => {
            boundaries.push((*offset, *offset + 1));
            *offset += 1;
        }
        Item::Table { columns, rows } => {
            for col in columns {
                for header_item in &col.header {
                    flatten_item_boundaries(header_item, boundaries, offset);
                }
            }
            for row in rows {
                for cell in row.cells() {
                    for cell_item in cell {
                        flatten_item_boundaries(cell_item, boundaries, offset);
                    }
                }
            }
        }
    }
}

fn spans_text_length(text: &Text) -> usize {
    let style = Style::from_palette(crate::core::theme::Palette::DARK);
    let spans = text.spans(style);
    spans.iter().map(|s| s.text.len()).sum()
}

/// Converts a screen position to a global character offset.
fn position_to_offset(position: Point, layout: Layout<'_>, state: &State) -> Option<usize> {
    if state.line_boundaries.is_empty() {
        return Some(0);
    }

    // Convert relative position to absolute screen coordinates
    let abs_position = Point::new(
        position.x + layout.bounds().x,
        position.y + layout.bounds().y,
    );

    // Get the child layout (the column of items)
    let child_layout = layout.children().next()?;

    // Collect visual lines and expand to match boundary count
    let mut visual_lines: Vec<Rectangle> = Vec::new();
    collect_visual_lines(child_layout, &mut visual_lines);

    // Expand visual lines to match boundary count
    let expanded_lines = expand_visual_lines_simple(&visual_lines, state.line_boundaries.len());

    // Find which line contains the click position
    for (i, bounds) in expanded_lines.iter().enumerate() {
        if abs_position.y >= bounds.y
            && abs_position.y < bounds.y + bounds.height
            && abs_position.x >= bounds.x - 10.0
            && abs_position.x < bounds.x + bounds.width + 10.0
        {
            if let Some((line_start, line_end)) = state.line_boundaries.get(i) {
                let line_length = line_end - line_start;
                if line_length == 0 {
                    return Some(*line_start);
                }
                let local_x = (abs_position.x - bounds.x).max(0.0);
                let ratio = (local_x / bounds.width.max(1.0)).clamp(0.0, 1.0);
                let estimated_offset = (ratio * line_length as f32) as usize;
                return Some(*line_start + estimated_offset.min(line_length));
            }
        }
    }

    // If click is between lines or outside, find the closest one by Y
    let mut best_idx = 0;
    let mut best_distance = f32::MAX;

    for (i, bounds) in expanded_lines.iter().enumerate() {
        let center_y = bounds.y + bounds.height / 2.0;
        let distance = (abs_position.y - center_y).abs();
        if distance < best_distance {
            best_distance = distance;
            best_idx = i;
        }
    }

    if let Some((line_start, line_end)) = state.line_boundaries.get(best_idx) {
        let bounds = &expanded_lines[best_idx];
        let line_length = line_end - line_start;
        let local_x = (abs_position.x - bounds.x).max(0.0);
        let ratio = (local_x / bounds.width.max(1.0)).clamp(0.0, 1.0);
        let estimated_offset = (ratio * line_length as f32) as usize;
        return Some(*line_start + estimated_offset.min(line_length));
    }

    Some(0)
}

/// Simple expansion that ensures we have exactly `expected_count` lines.
/// If we have fewer visual lines, distribute them and fill gaps with estimated positions.
fn expand_visual_lines_simple(visual_lines: &[Rectangle], expected_count: usize) -> Vec<Rectangle> {
    if visual_lines.is_empty() || expected_count == 0 {
        return vec![Rectangle::default(); expected_count];
    }

    if visual_lines.len() >= expected_count {
        return visual_lines[..expected_count].to_vec();
    }

    // We have fewer visual lines than needed
    // Strategy: use visual lines as anchors and interpolate positions for extras
    let mut result = Vec::with_capacity(expected_count);

    // Calculate the average line height from existing lines
    let avg_height: f32 = visual_lines.iter().map(|r| r.height).sum::<f32>() / visual_lines.len() as f32;

    // Get the last visual line to extrapolate from
    let last_line = visual_lines.last().unwrap();

    // First, add all existing visual lines
    result.extend_from_slice(visual_lines);

    // Then add estimated lines below the last one
    let mut y = last_line.y + last_line.height;
    while result.len() < expected_count {
        result.push(Rectangle {
            x: last_line.x,
            y,
            width: last_line.width,
            height: avg_height,
        });
        y += avg_height;
    }

    result
}

/// Collects visual line bounds by flattening the layout structure.
/// Stops at content-level widgets (paragraphs, headings) without diving into their span internals.
fn collect_visual_lines(layout: Layout<'_>, lines: &mut Vec<Rectangle>) {
    let children: Vec<_> = layout.children().collect();
    let bounds = layout.bounds();

    if children.is_empty() {
        // True leaf
        if bounds.width > 20.0 && bounds.height > 5.0 {
            lines.push(bounds);
        }
    } else if children.len() == 1 {
        // Single child - recurse but use parent bounds if child is much smaller
        // (indicates padding that we should include for hit testing)
        let child_bounds = children[0].bounds();
        let child_children: Vec<_> = children[0].children().collect();

        // If child has no children (is a leaf) and is significantly smaller, use parent bounds
        if child_children.is_empty() && child_bounds.height < bounds.height * 0.9 {
            lines.push(bounds);
        } else {
            collect_visual_lines(children[0], lines);
        }
    } else {
        // Multiple children - check if they're on the same line (internal spans) or different lines (structure)
        let first_y = children[0].bounds().y;
        let all_same_line = children
            .iter()
            .all(|c| (c.bounds().y - first_y).abs() < 5.0);

        if all_same_line && bounds.width > 20.0 {
            // Children are internal spans - use parent bounds
            lines.push(bounds);
        } else {
            // Children are structural - recurse
            for child in children {
                collect_visual_lines(child, lines);
            }
        }
    }
}

/// Finds line boundaries containing the offset.
fn find_line_boundaries(offset: usize, state: &State) -> (usize, usize) {
    for (start, end) in &state.line_boundaries {
        if offset >= *start && offset < *end {
            return (*start, *end);
        }
    }
    (0, state.total_length)
}

/// Extracts plain text from a selection range.
fn extract_text(start: usize, end: usize, items: &[Item], state: &State) -> String {
    // Collect all text lines flattened
    let mut all_lines = Vec::new();
    for item in items {
        collect_item_lines(item, &mut all_lines);
    }

    let mut result = String::new();

    for (i, line_text) in all_lines.iter().enumerate() {
        let Some((line_start, line_end)) = state.line_boundaries.get(i) else {
            continue;
        };

        if end <= *line_start || start >= *line_end {
            continue;
        }

        let local_start = start.saturating_sub(*line_start);
        let local_end = (end - *line_start).min(line_end - line_start);

        if local_start < line_text.len() {
            let end_idx = local_end.min(line_text.len());
            result.push_str(&line_text[local_start..end_idx]);
        }
    }

    result
}

/// Collects text lines from an item in flattened order (matching flatten_item_boundaries).
fn collect_item_lines(item: &Item, lines: &mut Vec<String>) {
    match item {
        Item::Heading(_, text) | Item::Paragraph(text) => {
            lines.push(extract_spans_text(text) + "\n");
        }
        Item::CodeBlock { code, .. } => {
            for line in code.lines() {
                lines.push(line.to_string() + "\n");
            }
        }
        Item::List { bullets, .. } => {
            for bullet in bullets {
                for sub_item in bullet.items() {
                    collect_item_lines(sub_item, lines);
                }
            }
        }
        Item::Quote(items) => {
            for sub_item in items {
                collect_item_lines(sub_item, lines);
            }
        }
        Item::Image { alt, .. } => {
            lines.push(extract_spans_text(alt) + "\n");
        }
        Item::Rule => {
            lines.push("\n".to_string());
        }
        Item::Table { columns, rows } => {
            for col in columns {
                for header_item in &col.header {
                    collect_item_lines(header_item, lines);
                }
            }
            for row in rows {
                for cell in row.cells() {
                    for cell_item in cell {
                        collect_item_lines(cell_item, lines);
                    }
                }
            }
        }
    }
}

fn extract_spans_text(text: &Text) -> String {
    let style = Style::from_palette(crate::core::theme::Palette::DARK);
    let spans = text.spans(style);
    spans.iter().map(|s| s.text.as_ref()).collect()
}

impl<'a, Message, Theme, Renderer> From<Selectable<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: Clone + 'a,
    Theme: Catalog + 'a,
    Renderer: core::text::Renderer<Font = Font> + 'a,
{
    fn from(widget: Selectable<'a, Message, Theme, Renderer>) -> Self {
        Element::new(widget)
    }
}
