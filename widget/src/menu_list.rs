//! Add linear keyboard navigation to a list of items.
//!
//! Wrapping a list with [`menu_list`] gives it the menu key bindings —
//! Up/Down, Home/End, Enter/Space, Escape — without imposing any layout or
//! styling. The caller owns the highlighted index and draws it however it
//! likes; this widget only reports where navigation moved.
//!
//! This complements [`focus_order`](crate::focus_order), which biases the
//! *spatial* focus algorithm. Menus are linear, so they need their own.
//!
//! ```no_run
//! use iced::widget::{column, menu_list, text};
//! use iced::Element;
//!
//! #[derive(Clone)]
//! enum Message {
//!     Highlight(usize),
//!     Activate(usize),
//!     Dismiss,
//! }
//!
//! fn view(highlighted: Option<usize>) -> Element<'static, Message> {
//!     let items = column![text("Profile"), text("Settings")];
//!
//!     menu_list(items, 2)
//!         .highlighted(highlighted)
//!         .on_highlight(Message::Highlight)
//!         .on_activate(Message::Activate)
//!         .on_dismiss(Message::Dismiss)
//!         .into()
//! }
//! ```
use crate::core::widget::Operation;
use crate::core::widget::tree::{self, Tree};
use crate::core::{Element, Event, Layout, Length, Rectangle, Shell, Size, Vector, Widget};
use crate::core::{keyboard, layout, mouse, overlay, renderer};

use crate::core::keyboard::key;

/// A transparent wrapper that gives its content menu keyboard navigation.
///
/// The highlighted index is owned by the caller: pass the current value to
/// [`MenuList::highlighted`] and update it from [`MenuList::on_highlight`].
#[allow(missing_debug_implementations)]
pub struct MenuList<'a, Message, Theme = crate::Theme, Renderer = crate::Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
    item_count: usize,
    highlighted: Option<usize>,
    wrap: bool,
    #[allow(clippy::type_complexity)]
    on_highlight: Option<Box<dyn Fn(usize) -> Message + 'a>>,
    #[allow(clippy::type_complexity)]
    on_activate: Option<Box<dyn Fn(usize) -> Message + 'a>>,
    on_dismiss: Option<Message>,
}

impl<'a, Message, Theme, Renderer> MenuList<'a, Message, Theme, Renderer> {
    /// Creates a new [`MenuList`] over `item_count` navigable items.
    pub fn new(
        content: impl Into<Element<'a, Message, Theme, Renderer>>,
        item_count: usize,
    ) -> Self {
        Self {
            content: content.into(),
            item_count,
            highlighted: None,
            wrap: true,
            on_highlight: None,
            on_activate: None,
            on_dismiss: None,
        }
    }

    /// Sets the currently highlighted item.
    pub fn highlighted(mut self, highlighted: Option<usize>) -> Self {
        self.highlighted = highlighted;
        self
    }

    /// Sets whether navigation wraps past the ends. Defaults to `true`.
    pub fn wrap(mut self, wrap: bool) -> Self {
        self.wrap = wrap;
        self
    }

    /// Sets the message produced when navigation moves the highlight.
    pub fn on_highlight(mut self, on_highlight: impl Fn(usize) -> Message + 'a) -> Self {
        self.on_highlight = Some(Box::new(on_highlight));
        self
    }

    /// Sets the message produced when the highlighted item is activated with
    /// Enter or Space.
    pub fn on_activate(mut self, on_activate: impl Fn(usize) -> Message + 'a) -> Self {
        self.on_activate = Some(Box::new(on_activate));
        self
    }

    /// Sets the message produced when Escape is pressed.
    pub fn on_dismiss(mut self, on_dismiss: Message) -> Self {
        self.on_dismiss = Some(on_dismiss);
        self
    }

    /// The index Up/Down would move to, or `None` when the list is empty or
    /// the move would run off an end with wrapping disabled.
    fn step(&self, forward: bool) -> Option<usize> {
        let last = self.item_count.checked_sub(1)?;

        let Some(current) = self.highlighted else {
            return Some(if forward { 0 } else { last });
        };

        if forward {
            if current >= last {
                self.wrap.then_some(0)
            } else {
                Some(current + 1)
            }
        } else if current == 0 {
            self.wrap.then_some(last)
        } else {
            Some(current - 1)
        }
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for MenuList<'_, Message, Theme, Renderer>
where
    Message: Clone,
    Renderer: renderer::Renderer,
{
    fn tag(&self) -> tree::Tag {
        self.content.as_widget().tag()
    }

    fn state(&self) -> tree::State {
        self.content.as_widget().state()
    }

    fn children(&self) -> Vec<Tree> {
        self.content.as_widget().children()
    }

    fn diff(&self, tree: &mut Tree) {
        self.content.as_widget().diff(tree);
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content.as_widget_mut().layout(tree, renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(tree, layout, renderer, operation);
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
        if let Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(named),
            modifiers,
            ..
        }) = event
        {
            let moved = match named {
                key::Named::ArrowDown => self.step(true),
                key::Named::ArrowUp => self.step(false),
                key::Named::Tab if modifiers.shift() => self.step(false),
                key::Named::Tab => self.step(true),
                key::Named::Home => (self.item_count > 0).then_some(0),
                key::Named::End => self.item_count.checked_sub(1),
                _ => None,
            };

            if let Some(index) = moved {
                if let Some(on_highlight) = &self.on_highlight {
                    shell.publish(on_highlight(index));
                }
                shell.capture_event();
                shell.request_redraw();
                return;
            }

            match named {
                key::Named::Enter | key::Named::Space => {
                    if let Some((on_activate, index)) =
                        self.on_activate.as_ref().zip(self.highlighted)
                    {
                        shell.publish(on_activate(index));
                        shell.capture_event();
                        shell.request_redraw();
                        return;
                    }
                }
                key::Named::Escape => {
                    if let Some(on_dismiss) = &self.on_dismiss {
                        shell.publish(on_dismiss.clone());
                        shell.capture_event();
                        shell.request_redraw();
                        return;
                    }
                }
                _ => {}
            }
        }

        self.content
            .as_widget_mut()
            .update(tree, event, layout, cursor, renderer, shell, viewport);
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content
            .as_widget()
            .mouse_interaction(tree, layout, cursor, viewport, renderer)
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content
            .as_widget()
            .draw(tree, renderer, theme, style, layout, cursor, viewport);
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content
            .as_widget_mut()
            .overlay(tree, layout, renderer, viewport, translation)
    }
}

impl<'a, Message, Theme, Renderer> From<MenuList<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: Clone + 'a,
    Theme: 'a,
    Renderer: renderer::Renderer + 'a,
{
    fn from(menu_list: MenuList<'a, Message, Theme, Renderer>) -> Self {
        Self::new(menu_list)
    }
}

/// Creates a [`MenuList`] that gives `content` menu keyboard navigation over
/// `item_count` items.
///
/// See the [module documentation](self) for an example.
pub fn menu_list<'a, Message, Theme, Renderer>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
    item_count: usize,
) -> MenuList<'a, Message, Theme, Renderer> {
    MenuList::new(content, item_count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    enum Message {}

    fn list(item_count: usize, highlighted: Option<usize>) -> MenuList<'static, Message> {
        menu_list(crate::Space::new(), item_count).highlighted(highlighted)
    }

    #[test]
    fn navigation_starts_at_an_end_when_nothing_is_highlighted() {
        assert_eq!(list(3, None).step(true), Some(0));
        assert_eq!(list(3, None).step(false), Some(2));
    }

    #[test]
    fn navigation_wraps_by_default() {
        assert_eq!(list(3, Some(2)).step(true), Some(0));
        assert_eq!(list(3, Some(0)).step(false), Some(2));
    }

    #[test]
    fn wrap_disabled_stops_at_the_ends() {
        assert_eq!(list(3, Some(2)).wrap(false).step(true), None);
        assert_eq!(list(3, Some(0)).wrap(false).step(false), None);
        assert_eq!(list(3, Some(1)).wrap(false).step(true), Some(2));
    }

    #[test]
    fn an_empty_list_never_navigates() {
        assert_eq!(list(0, None).step(true), None);
        assert_eq!(list(0, Some(0)).step(false), None);
    }
}
