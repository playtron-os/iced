//! Build and show dropdown menus.
use crate::core::alignment;
use crate::core::border::{self, Border};
use crate::core::keyboard;
use crate::core::layout::{self, Layout};
use crate::core::mouse;
use crate::core::overlay;
use crate::core::renderer;
use crate::core::text::{self, Text};
use crate::core::touch;
use crate::core::widget::Operation;
use crate::core::widget::tree::{self, Tree};
use crate::core::window;
use crate::core::{
    Background, Color, Event, Length, Padding, Pixels, Point, Rectangle, Shadow, Size, Theme,
    Vector,
};
use crate::core::{Element, Shell, Widget};
use crate::scrollable::{self, Scrollable};

use std::cell::Cell;

/// A list of selectable options.
pub struct Menu<'a, 'b, T, Message, Theme = crate::Theme, Renderer = crate::Renderer>
where
    Theme: Catalog,
    Renderer: text::Renderer,
    'b: 'a,
{
    state: &'a mut State,
    options: &'a [T],
    hovered_option: &'a mut Option<usize>,
    to_string: &'a dyn Fn(&T) -> String,
    on_selected: Box<dyn FnMut(T) -> Message + 'a>,
    on_option_hovered: Option<&'a dyn Fn(T) -> Message>,
    width: f32,
    padding: Padding,
    container_padding: Padding,
    text_size: Option<Pixels>,
    letter_spacing: Option<Pixels>,
    line_height: text::LineHeight,
    shaping: text::Shaping,
    ellipsis: text::Ellipsis,
    font: Option<Renderer::Font>,
    class: &'a <Theme as Catalog>::Class<'b>,
    render_icon: Option<&'a dyn Fn(&T, &mut Renderer, Rectangle, &Rectangle)>,
    icon_size: f32,
    keyboard: bool,
}

impl<'a, 'b, T, Message, Theme, Renderer> Menu<'a, 'b, T, Message, Theme, Renderer>
where
    T: Clone,
    Message: 'a,
    Theme: Catalog + 'a,
    Renderer: text::Renderer + 'a,
    'b: 'a,
{
    /// Creates a new [`Menu`] with the given [`State`], a list of options,
    /// the message to produced when an option is selected, and its [`Style`].
    pub fn new(
        state: &'a mut State,
        options: &'a [T],
        hovered_option: &'a mut Option<usize>,
        to_string: &'a dyn Fn(&T) -> String,
        on_selected: impl FnMut(T) -> Message + 'a,
        on_option_hovered: Option<&'a dyn Fn(T) -> Message>,
        class: &'a <Theme as Catalog>::Class<'b>,
    ) -> Self {
        Menu {
            state,
            options,
            hovered_option,
            to_string,
            on_selected: Box::new(on_selected),
            on_option_hovered,
            width: 0.0,
            padding: Padding::ZERO,
            container_padding: Padding::ZERO,
            text_size: None,
            letter_spacing: None,
            line_height: text::LineHeight::default(),
            shaping: text::Shaping::default(),
            ellipsis: text::Ellipsis::default(),
            font: None,
            class,
            render_icon: None,
            icon_size: 0.0,
            keyboard: false,
        }
    }

    /// Sets the width of the [`Menu`].
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// Sets the [`Padding`] of the [`Menu`].
    pub fn padding<P: Into<Padding>>(mut self, padding: P) -> Self {
        self.padding = padding.into();
        self
    }

    /// Sets the container [`Padding`] of the [`Menu`] — the inset between the
    /// menu's border and the option list (the options still scroll within it).
    pub fn container_padding<P: Into<Padding>>(mut self, padding: P) -> Self {
        self.container_padding = padding.into();
        self
    }

    /// Sets the text size of the [`Menu`].
    pub fn text_size(mut self, text_size: impl Into<Pixels>) -> Self {
        self.text_size = Some(text_size.into());
        self
    }

    /// Sets the letter spacing of the text of the [`Menu`].
    pub fn letter_spacing(mut self, letter_spacing: impl Into<Pixels>) -> Self {
        self.letter_spacing = Some(letter_spacing.into());
        self
    }

    /// Sets the text [`text::LineHeight`] of the [`Menu`].
    pub fn line_height(mut self, line_height: impl Into<text::LineHeight>) -> Self {
        self.line_height = line_height.into();
        self
    }

    /// Sets the [`text::Shaping`] strategy of the [`Menu`].
    pub fn shaping(mut self, shaping: text::Shaping) -> Self {
        self.shaping = shaping;
        self
    }

    /// Sets the [`text::Ellipsis`] strategy of the [`Menu`].
    pub fn ellipsis(mut self, ellipsis: text::Ellipsis) -> Self {
        self.ellipsis = ellipsis;
        self
    }

    /// Sets the font of the [`Menu`].
    pub fn font(mut self, font: impl Into<Renderer::Font>) -> Self {
        self.font = Some(font.into());
        self
    }

    /// Sets the icon rendering callback and icon size for the [`Menu`].
    pub fn render_icon(
        mut self,
        icon_size: f32,
        render_icon: &'a dyn Fn(&T, &mut Renderer, Rectangle, &Rectangle),
    ) -> Self {
        self.icon_size = icon_size;
        self.render_icon = Some(render_icon);
        self
    }

    /// Whether the open [`Menu`] takes the keyboard: Up, Down, Home, End, Page Up
    /// and Page Down move the highlight, scrolled into view, and Enter or Space
    /// picks the highlighted option. Off by default, for a widget that steers
    /// the highlight itself, such as a combo box.
    pub fn keyboard(mut self, keyboard: bool) -> Self {
        self.keyboard = keyboard;
        self
    }

    /// Turns the [`Menu`] into an overlay [`Element`] at the given target
    /// position.
    ///
    /// The `target_height` will be used to display the menu either on top
    /// of the target or under it, depending on the screen position and the
    /// dimensions of the [`Menu`].
    pub fn overlay(
        self,
        position: Point,
        viewport: Rectangle,
        target_height: f32,
        menu_height: Length,
    ) -> overlay::Element<'a, Message, Theme, Renderer> {
        overlay::Element::new(Box::new(Overlay::new(
            position,
            viewport,
            self,
            target_height,
            menu_height,
        )))
    }
}

/// The local state of a [`Menu`].
#[derive(Debug)]
pub struct State {
    tree: Tree,
    placement: Cell<Option<bool>>,
    reveal: Cell<bool>,
}

impl State {
    /// Creates a new [`State`] for a [`Menu`].
    pub fn new() -> Self {
        Self {
            tree: Tree::empty(),
            placement: Cell::new(None),
            reveal: Cell::new(false),
        }
    }

    /// Whether the menu went ABOVE its target the last time it was laid out,
    /// or `None` if it has not been laid out yet.
    ///
    /// The widget that owns the menu styles the edge they share from this,
    /// rather than working the direction out a second time: its own bounds
    /// are where it was LAID OUT, and a surface that is laid out in one place
    /// and drawn in another — a modal that centres its dialog — moves the
    /// menu and not the widget, so the two answers differ and the two halves
    /// of one shape square opposite edges.
    pub fn opens_upward(&self) -> Option<bool> {
        self.placement.get()
    }

    /// Forget where the menu last went — it is closed, and the next opening
    /// is laid out again.
    pub fn forget_placement(&self) {
        self.placement.set(None);
    }

    /// Scroll the hovered option into view the next time the menu is laid
    /// out, as when it opens on the selected option.
    pub fn reveal_hovered(&self) {
        self.reveal.set(true);
    }
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

struct Overlay<'a, 'b, Message, Theme, Renderer>
where
    Theme: Catalog,
    Renderer: text::Renderer,
{
    position: Point,
    viewport: Rectangle,
    tree: &'a mut Tree,
    /// Written by `layout`, read by the widget this menu belongs to.
    placement: &'a Cell<Option<bool>>,
    list: Scrollable<'a, Message, Theme, Renderer>,
    hovered_option: &'a Cell<Option<usize>>,
    /// Set when the hovered option must be scrolled into view.
    reveal: &'a Cell<bool>,
    row: Row,
    width: f32,
    container_padding: Padding,
    target_height: f32,
    class: &'a <Theme as Catalog>::Class<'b>,
}

impl<'a, 'b, Message, Theme, Renderer> Overlay<'a, 'b, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: Catalog + scrollable::Catalog + 'a,
    Renderer: text::Renderer + 'a,
    'b: 'a,
{
    pub fn new<T>(
        position: Point,
        viewport: Rectangle,
        menu: Menu<'a, 'b, T, Message, Theme, Renderer>,
        target_height: f32,
        menu_height: Length,
    ) -> Self
    where
        T: Clone,
    {
        let Menu {
            state,
            options,
            hovered_option,
            to_string,
            on_selected,
            on_option_hovered,
            width,
            padding,
            container_padding,
            font,
            text_size,
            letter_spacing,
            line_height,
            shaping,
            ellipsis,
            class,
            render_icon,
            icon_size,
            keyboard,
        } = menu;

        let State {
            tree,
            placement,
            reveal,
        } = state;
        // Shared with the list, which moves it, so the overlay can scroll to it.
        let hovered_option = Cell::from_mut(hovered_option);
        let row = Row {
            text_size,
            line_height,
            padding,
        };

        let list = Scrollable::new(List {
            options,
            hovered_option,
            keyboard,
            reveal,
            row,
            to_string,
            on_selected,
            on_option_hovered,
            font,
            text_size,
            letter_spacing,
            line_height,
            shaping,
            ellipsis,
            padding,
            class,
            render_icon,
            icon_size,
        })
        .height(menu_height);

        tree.diff(&list as &dyn Widget<_, _, _>);

        Self {
            position,
            viewport,
            tree,
            placement,
            list,
            hovered_option,
            reveal,
            row,
            width,
            container_padding,
            target_height,
            class,
        }
    }
}

impl<Message, Theme, Renderer> Overlay<'_, '_, Message, Theme, Renderer>
where
    Theme: Catalog,
    Renderer: text::Renderer,
{
    /// Scrolls the list just far enough to show the hovered option whole.
    fn reveal_hovered(&mut self, layout: Layout<'_>, renderer: &Renderer) {
        let Some(index) = self.hovered_option.get() else {
            return;
        };
        let Some(list_layout) = layout.children().next() else {
            return;
        };
        let height = self.row.height(renderer);
        let top = height * index as f32;

        self.list.operate(
            self.tree,
            list_layout,
            renderer,
            &mut Reveal {
                top,
                bottom: top + height,
            },
        );
    }
}

/// The metrics every option row of a [`Menu`] shares.
#[derive(Debug, Clone, Copy)]
struct Row {
    text_size: Option<Pixels>,
    line_height: text::LineHeight,
    padding: Padding,
}

impl Row {
    fn height<Renderer: text::Renderer>(&self, renderer: &Renderer) -> f32 {
        let text_size = self.text_size.unwrap_or_else(|| renderer.default_size());

        f32::from(self.line_height.to_absolute(text_size)) + self.padding.y()
    }
}

/// Scrolls the list's scrollable so the rows from `top` to `bottom` (offsets
/// into the list) show, moving it as little as it can, as a browser does.
struct Reveal {
    top: f32,
    bottom: f32,
}

impl Operation for Reveal {
    fn scrollable(
        &mut self,
        _id: Option<&crate::core::widget::Id>,
        bounds: Rectangle,
        _content_bounds: Rectangle,
        translation: Vector,
        state: &mut dyn crate::core::widget::operation::Scrollable,
    ) {
        let y = if self.top < translation.y {
            self.top
        } else if self.bottom > translation.y + bounds.height {
            self.bottom - bounds.height
        } else {
            return;
        };

        state.scroll_to(crate::core::widget::operation::scrollable::AbsoluteOffset {
            x: None,
            y: Some(y.max(0.0)),
        });
    }

    fn traverse(&mut self, _operate: &mut dyn FnMut(&mut dyn Operation)) {}
}

impl<Message, Theme, Renderer> crate::core::Overlay<Message, Theme, Renderer>
    for Overlay<'_, '_, Message, Theme, Renderer>
where
    Theme: Catalog,
    Renderer: text::Renderer,
{
    fn layout(&mut self, renderer: &Renderer, bounds: Size) -> layout::Node {
        let space_below = bounds.height - (self.position.y + self.target_height);
        let space_above = self.position.y;

        let pad = self.container_padding;
        let available = if space_below > space_above {
            space_below
        } else {
            space_above
        };

        // Reserve room for the container padding; the scrollable list is laid
        // out inside it and inset by `pad`.
        let limits = layout::Limits::new(
            Size::ZERO,
            Size::new(
                (bounds.width - self.position.x - pad.x()).max(0.0),
                (available - pad.y()).max(0.0),
            ),
        )
        .width((self.width - pad.x()).max(0.0));

        let list_node = self.list.layout(self.tree, renderer, &limits);
        let list_size = list_node.size();
        let size = Size::new(list_size.width + pad.x(), list_size.height + pad.y());

        let node = layout::Node::with_children(
            size,
            vec![list_node.move_to(Point::new(pad.left, pad.top))],
        );

        // The one decision about which way this menu goes. The widget it
        // belongs to reads it back rather than deciding again (see
        // `State::opens_upward`).
        let downward = space_below > space_above;
        self.placement.set(Some(!downward));

        let node = node.move_to(if downward {
            self.position + Vector::new(0.0, self.target_height)
        } else {
            self.position - Vector::new(0.0, size.height)
        });

        if self.reveal.take() {
            self.reveal_hovered(Layout::new(&node), renderer);
        }

        node
    }

    fn operate(&mut self, layout: Layout<'_>, renderer: &Renderer, operation: &mut dyn Operation) {
        let list_layout = layout.children().next().unwrap();

        self.list
            .operate(self.tree, list_layout, renderer, operation);
    }

    fn update(
        &mut self,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
    ) {
        let list_layout = layout.children().next().unwrap();
        let bounds = list_layout.bounds();

        self.list.update(
            self.tree,
            event,
            list_layout,
            cursor,
            renderer,
            shell,
            &bounds,
        );

        if self.reveal.take() {
            self.reveal_hovered(layout, renderer);
        }
    }

    fn mouse_interaction(
        &self,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        let list_layout = layout.children().next().unwrap();
        self.list
            .mouse_interaction(self.tree, list_layout, cursor, &self.viewport, renderer)
    }

    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        defaults: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        let bounds = layout.bounds();

        let mut style = Catalog::style(theme, self.class);

        // If the menu opened upward (above the pick list), flip the border
        // radius so that the squared corners face the input widget.
        let opens_upward = bounds.y < self.position.y;
        if opens_upward {
            let r = style.border.radius;
            style.border.radius = border::Radius {
                top_left: r.bottom_left,
                top_right: r.bottom_right,
                bottom_left: r.top_left,
                bottom_right: r.top_right,
            };
        }

        const MENU_BACKDROP_BLUR_RADIUS: f32 = 20.0;
        let frosted = matches!(
            style.background,
            Background::Color(color) if color.a > f32::EPSILON && color.a < 1.0
        );

        if frosted {
            renderer.draw_backdrop_blur(
                bounds,
                MENU_BACKDROP_BLUR_RADIUS,
                <[f32; 4]>::from(style.border.radius),
                1.0,
                // Blur only — a pick-list menu takes its colour from the style
                // it was given, so leave the backdrop's saturation alone.
                1.0,
            );
            // Draw the panel + items in a post-blur layer so they composite on
            // top of the blurred backdrop rather than being blurred themselves.
            renderer.start_post_blur_layer(bounds);
        }

        renderer.fill_quad(
            renderer::Quad {
                bounds,
                border: style.border,
                shadow: style.shadow,
                ..renderer::Quad::default()
            },
            style.background,
        );

        let list_layout = layout.children().next().unwrap();
        self.list.draw(
            self.tree,
            renderer,
            theme,
            defaults,
            list_layout,
            cursor,
            &list_layout.bounds(),
        );

        // The border again, over the rows. A menu may be asked for no
        // container padding at all — the rows are then flush with the panel's
        // edge — and the first row's own background painted over the top of
        // the line that closes the panel, and the last row's over the bottom.
        // Drawing it twice is what keeps the panel's edge unbroken without
        // taking the padding decision away from the theme. No shadow on this
        // pass: the one under the panel is already cast.
        if style.border.width > 0.0 {
            renderer.fill_quad(
                renderer::Quad {
                    bounds,
                    border: style.border,
                    ..renderer::Quad::default()
                },
                Background::Color(Color::TRANSPARENT),
            );
        }

        if frosted {
            renderer.end_post_blur_layer();
        }
    }
}

struct List<'a, 'b, T, Message, Theme, Renderer>
where
    Theme: Catalog,
    Renderer: text::Renderer,
{
    options: &'a [T],
    hovered_option: &'a Cell<Option<usize>>,
    keyboard: bool,
    reveal: &'a Cell<bool>,
    row: Row,
    to_string: &'a dyn Fn(&T) -> String,
    on_selected: Box<dyn FnMut(T) -> Message + 'a>,
    on_option_hovered: Option<&'a dyn Fn(T) -> Message>,
    padding: Padding,
    text_size: Option<Pixels>,
    letter_spacing: Option<Pixels>,
    line_height: text::LineHeight,
    shaping: text::Shaping,
    ellipsis: text::Ellipsis,
    font: Option<Renderer::Font>,
    class: &'a <Theme as Catalog>::Class<'b>,
    render_icon: Option<&'a dyn Fn(&T, &mut Renderer, Rectangle, &Rectangle)>,
    icon_size: f32,
}

struct ListState {
    is_hovered: Option<bool>,
}

impl<T, Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for List<'_, '_, T, Message, Theme, Renderer>
where
    T: Clone,
    Theme: Catalog,
    Renderer: text::Renderer,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<Option<bool>>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(ListState { is_hovered: None })
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: Length::Fill,
            height: Length::Shrink,
        }
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        use std::f32;

        let text_size = self.text_size.unwrap_or_else(|| renderer.default_size());

        let text_line_height = self.line_height.to_absolute(text_size);

        let size = {
            let intrinsic = Size::new(
                0.0,
                (f32::from(text_line_height) + self.padding.y()) * self.options.len() as f32,
            );

            limits.resolve(Length::Fill, Length::Shrink, intrinsic)
        };

        layout::Node::new(size)
    }

    /// Reports each option as text, over the whole row a press selects it in.
    fn operate(
        &mut self,
        _tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        let bounds = layout.bounds();
        let text_size = self.text_size.unwrap_or_else(|| renderer.default_size());
        let option_height = f32::from(self.line_height.to_absolute(text_size)) + self.padding.y();

        for (i, option) in self.options.iter().enumerate() {
            let row = Rectangle {
                x: bounds.x,
                y: bounds.y + option_height * i as f32,
                width: bounds.width,
                height: option_height,
            };

            operation.text(None, row, &(self.to_string)(option));
        }
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if cursor.is_over(layout.bounds())
                    && let Some(index) = self.hovered_option.get()
                    && let Some(option) = self.options.get(index)
                {
                    shell.publish((self.on_selected)(option.clone()));
                    shell.capture_event();
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if let Some(cursor_position) = cursor.position_in(layout.bounds()) {
                    let text_size = self.text_size.unwrap_or_else(|| renderer.default_size());

                    let option_height =
                        f32::from(self.line_height.to_absolute(text_size)) + self.padding.y();

                    let new_hovered_option = (cursor_position.y / option_height) as usize;

                    if self.hovered_option.get() != Some(new_hovered_option)
                        && let Some(option) = self.options.get(new_hovered_option)
                    {
                        if let Some(on_option_hovered) = self.on_option_hovered {
                            shell.publish(on_option_hovered(option.clone()));
                        }

                        shell.request_redraw();
                    }

                    self.hovered_option.set(Some(new_hovered_option));
                }
            }
            Event::Touch(touch::Event::FingerPressed { .. }) => {
                if let Some(cursor_position) = cursor.position_in(layout.bounds()) {
                    let text_size = self.text_size.unwrap_or_else(|| renderer.default_size());

                    let option_height =
                        f32::from(self.line_height.to_absolute(text_size)) + self.padding.y();

                    self.hovered_option
                        .set(Some((cursor_position.y / option_height) as usize));

                    if let Some(index) = self.hovered_option.get()
                        && let Some(option) = self.options.get(index)
                    {
                        shell.publish((self.on_selected)(option.clone()));
                        shell.capture_event();
                    }
                }
            }
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(key),
                repeat,
                ..
            }) if self.keyboard && !self.options.is_empty() => {
                use keyboard::key::Named;

                let last = self.options.len() - 1;
                let current = self.hovered_option.get().filter(|&index| index <= last);
                // Page Up and Page Down move by the rows that show at once.
                let page = ((viewport.height / self.row.height(renderer)).floor() as usize).max(1);

                let next = match key {
                    Named::ArrowDown => Some(current.map_or(0, |index| (index + 1).min(last))),
                    Named::ArrowUp => Some(current.map_or(last, |index| index.saturating_sub(1))),
                    Named::Home => Some(0),
                    Named::End => Some(last),
                    Named::PageDown => Some(current.map_or(0, |index| (index + page).min(last))),
                    Named::PageUp => Some(current.map_or(0, |index| index.saturating_sub(page))),
                    _ => None,
                };

                if matches!(key, Named::Enter | Named::Space)
                    && !repeat
                    // With nothing highlighted, the pick list closes the menu instead.
                    && let Some(option) = current.and_then(|index| self.options.get(index))
                {
                    shell.publish((self.on_selected)(option.clone()));
                    shell.capture_event();
                } else if let Some(next) = next {
                    if current != Some(next) {
                        self.hovered_option.set(Some(next));
                        self.reveal.set(true);

                        if let Some(on_option_hovered) = self.on_option_hovered
                            && let Some(option) = self.options.get(next)
                        {
                            shell.publish(on_option_hovered(option.clone()));
                        }

                        shell.request_redraw();
                    }

                    shell.capture_event();
                }
            }
            _ => {}
        }

        let state = tree.state.downcast_mut::<ListState>();

        if let Event::Window(window::Event::RedrawRequested(_now)) = event {
            state.is_hovered = Some(cursor.is_over(layout.bounds()));
        } else if state
            .is_hovered
            .is_some_and(|is_hovered| is_hovered != cursor.is_over(layout.bounds()))
        {
            shell.request_redraw();
        }
    }

    fn mouse_interaction(
        &self,
        _tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        let is_mouse_over = cursor.is_over(layout.bounds());

        if is_mouse_over {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }

    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        use crate::pick_list::ICON_TEXT_GAP;

        let style = Catalog::style(theme, self.class);
        let bounds = layout.bounds();

        let text_size = self.text_size.unwrap_or_else(|| renderer.default_size());
        let option_height = f32::from(self.line_height.to_absolute(text_size)) + self.padding.y();

        let offset = viewport.y - bounds.y;
        let start = (offset / option_height) as usize;
        let end = ((offset + viewport.height) / option_height).ceil() as usize;

        let visible_options = &self.options[start..end.min(self.options.len())];

        let icon_offset = if self.render_icon.is_some() && self.icon_size > 0.0 {
            self.icon_size + ICON_TEXT_GAP
        } else {
            0.0
        };

        for (i, option) in visible_options.iter().enumerate() {
            let i = start + i;
            let is_selected = self.hovered_option.get() == Some(i);

            let bounds = Rectangle {
                x: bounds.x,
                y: bounds.y + (option_height * i as f32),
                width: bounds.width,
                height: option_height,
            };

            if is_selected {
                let item_radius = style.item_border_radius.unwrap_or(style.border.radius);

                renderer.fill_quad(
                    renderer::Quad {
                        bounds: Rectangle {
                            x: bounds.x + style.border.width,
                            width: bounds.width - style.border.width * 2.0,
                            ..bounds
                        },
                        border: border::rounded(item_radius),
                        ..renderer::Quad::default()
                    },
                    style.selected_background,
                );
            }

            // Render icon if callback is provided
            if let Some(render_icon) = &self.render_icon
                && self.icon_size > 0.0
            {
                let icon_bounds = Rectangle {
                    x: bounds.x + self.padding.left,
                    y: bounds.center_y() - self.icon_size / 2.0,
                    width: self.icon_size,
                    height: self.icon_size,
                };
                render_icon(option, renderer, icon_bounds, viewport);
            }

            renderer.fill_text(
                Text {
                    content: (self.to_string)(option),
                    bounds: Size::new(bounds.width - self.padding.x() - icon_offset, bounds.height),
                    size: text_size,
                    line_height: self.line_height,
                    font: self.font.unwrap_or_else(|| renderer.default_font()),
                    align_x: text::Alignment::Default,
                    align_y: alignment::Vertical::Center,
                    shaping: self.shaping,
                    wrapping: text::Wrapping::None,
                    ellipsis: self.ellipsis,
                    hint_factor: renderer.scale_factor(),
                    letter_spacing: self.letter_spacing.map(|p| p.0),
                },
                Point::new(
                    bounds.x + self.padding.left + icon_offset,
                    bounds.center_y(),
                ),
                if is_selected {
                    style.selected_text_color
                } else {
                    style.text_color
                },
                *viewport,
            );
        }
    }
}

impl<'a, 'b, T, Message, Theme, Renderer> From<List<'a, 'b, T, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    T: Clone,
    Message: 'a,
    Theme: 'a + Catalog,
    Renderer: 'a + text::Renderer,
    'b: 'a,
{
    fn from(list: List<'a, 'b, T, Message, Theme, Renderer>) -> Self {
        Element::new(list)
    }
}

/// The appearance of a [`Menu`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    /// The [`Background`] of the menu.
    pub background: Background,
    /// The [`Border`] of the menu.
    pub border: Border,
    /// The text [`Color`] of the menu.
    pub text_color: Color,
    /// The text [`Color`] of a selected option in the menu.
    pub selected_text_color: Color,
    /// The background [`Color`] of a selected option in the menu.
    pub selected_background: Background,
    /// The [`border::Radius`] of a selected/hovered item in the menu.
    ///
    /// When `None`, falls back to the container's `border.radius`.
    pub item_border_radius: Option<border::Radius>,
    /// The [`Shadow`] of the menu.
    pub shadow: Shadow,
}

/// The theme catalog of a [`Menu`].
pub trait Catalog: scrollable::Catalog {
    /// The item class of the [`Catalog`].
    type Class<'a>;

    /// The default class produced by the [`Catalog`].
    fn default<'a>() -> <Self as Catalog>::Class<'a>;

    /// The default class for the scrollable of the [`Menu`].
    fn default_scrollable<'a>() -> <Self as scrollable::Catalog>::Class<'a> {
        <Self as scrollable::Catalog>::default()
    }

    /// The [`Style`] of a class with the given status.
    fn style(&self, class: &<Self as Catalog>::Class<'_>) -> Style;
}

/// A styling function for a [`Menu`].
pub type StyleFn<'a, Theme> = Box<dyn Fn(&Theme) -> Style + 'a>;

impl Catalog for Theme {
    type Class<'a> = StyleFn<'a, Self>;

    fn default<'a>() -> StyleFn<'a, Self> {
        Box::new(default)
    }

    fn style(&self, class: &StyleFn<'_, Self>) -> Style {
        class(self)
    }
}

/// The default style of the list of a [`Menu`].
pub fn default(theme: &Theme) -> Style {
    let palette = theme.extended_palette();

    Style {
        background: palette.background.weak.color.into(),
        border: Border {
            width: 1.0,
            radius: 0.0.into(),
            color: palette.background.strong.color,
            ..Default::default()
        },
        text_color: palette.background.weak.text,
        selected_text_color: palette.primary.strong.text,
        selected_background: palette.primary.strong.color.into(),
        item_border_radius: None,
        shadow: Shadow::default(),
    }
}
