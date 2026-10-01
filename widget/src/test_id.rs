//! Give a widget a stable name that tests can find it by.
//!
//! Wrapping a widget with [`test_id`] reports it to widget operations as a
//! container carrying the given [`Id`], with the wrapped widget's bounds. A
//! selector, or a test driver reading the widget tree, can then find a control
//! by its name, even when it shows no text (an icon-only button, say).
//!
//! The wrapper is otherwise invisible: it draws, lays out, handles events and
//! keeps state exactly as the widget it wraps.
//!
//! ```no_run
//! # mod iced { pub mod widget { pub use iced_widget::*; } pub use iced_widget::Renderer; pub use iced_widget::core::*; }
//! # pub type Element<'a, Message> = iced_widget::core::Element<'a, Message, iced_widget::Theme, iced_widget::Renderer>;
//! #
//! use iced::widget::{button, test_id, text};
//!
//! #[derive(Clone)]
//! enum Message {
//!     NewDocument,
//! }
//!
//! fn view<'a>() -> Element<'a, Message> {
//!     test_id(
//!         "slate.new_document",
//!         button(text("+")).on_press(Message::NewDocument),
//!     )
//!     .into()
//! }
//! ```
use crate::core::layout::{self, Layout};
use crate::core::mouse;
use crate::core::overlay;
use crate::core::renderer;
use crate::core::widget::tree::{self, Tree};
use crate::core::widget::{Id, Operation};
use crate::core::{Element, Event, Length, Rectangle, Shell, Size, Vector, Widget};

/// A transparent wrapper that names its content for tests.
#[allow(missing_debug_implementations)]
pub struct TestId<'a, Message, Theme = crate::Theme, Renderer = crate::Renderer> {
    id: Id,
    content: Element<'a, Message, Theme, Renderer>,
}

impl<'a, Message, Theme, Renderer> TestId<'a, Message, Theme, Renderer> {
    /// Creates a new [`TestId`] wrapper that names `content` with `id`.
    pub fn new(
        id: impl Into<Id>,
        content: impl Into<Element<'a, Message, Theme, Renderer>>,
    ) -> Self {
        Self {
            id: id.into(),
            content: content.into(),
        }
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for TestId<'_, Message, Theme, Renderer>
where
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
        operation.container(Some(&self.id), layout.bounds());
        operation.traverse(&mut |operation| {
            self.content
                .as_widget_mut()
                .operate(tree, layout, renderer, operation);
        });
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

impl<'a, Message, Theme, Renderer> From<TestId<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: renderer::Renderer + 'a,
{
    fn from(test_id: TestId<'a, Message, Theme, Renderer>) -> Self {
        Self::new(test_id)
    }
}

/// Names `content` with `id`, so tests can find it in the widget tree.
///
/// See the [module docs](self) for an example.
pub fn test_id<'a, Message, Theme, Renderer>(
    id: impl Into<Id>,
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
) -> TestId<'a, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    TestId::new(id, content)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::core::Point;
    use crate::core::widget::operation::Focusable;
    use crate::{Space, button};

    /// Records what an operation is told about, in tree order.
    #[derive(Default)]
    struct Recorder {
        seen: Vec<(&'static str, Option<String>, Rectangle)>,
    }

    impl Operation for Recorder {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
            operate(self);
        }

        fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
            self.seen.push((
                "container",
                id.and_then(Id::as_str).map(str::to_owned),
                bounds,
            ));
        }

        fn focusable(&mut self, id: Option<&Id>, bounds: Rectangle, _state: &mut dyn Focusable) {
            self.seen.push((
                "focusable",
                id.and_then(Id::as_str).map(str::to_owned),
                bounds,
            ));
        }
    }

    fn operate(widget: &mut TestId<'_, (), crate::Theme, ()>, bounds: Rectangle) -> Recorder {
        let mut tree = Tree::new(&*widget as &dyn Widget<(), crate::Theme, ()>);
        let node =
            layout::Node::with_children(bounds.size(), vec![layout::Node::new(bounds.size())])
                .move_to(bounds.position());
        let mut recorder = Recorder::default();

        widget.operate(&mut tree, Layout::new(&node), &(), &mut recorder);

        recorder
    }

    #[test]
    fn reports_its_id_with_the_bounds_of_what_it_wraps() {
        let bounds = Rectangle::new(Point::new(10.0, 20.0), Size::new(120.0, 40.0));
        let mut named = test_id("slate.new_document", button(Space::new()).on_press(()));

        let recorder = operate(&mut named, bounds);

        assert_eq!(
            recorder.seen.first(),
            Some(&("container", Some("slate.new_document".to_owned()), bounds))
        );
    }

    #[test]
    fn still_reports_the_widget_it_wraps() {
        let bounds = Rectangle::new(Point::new(10.0, 20.0), Size::new(120.0, 40.0));
        let mut named = test_id("slate.new_document", button(Space::new()).on_press(()));

        let recorder = operate(&mut named, bounds);

        assert!(
            recorder
                .seen
                .iter()
                .any(|(kind, id, b)| *kind == "focusable" && id.is_none() && *b == bounds),
            "the button was not reported: {:?}",
            recorder.seen
        );
    }

    #[test]
    fn keeps_the_state_of_the_widget_it_wraps() {
        let plain: Element<'_, (), crate::Theme, ()> = button(Space::new()).on_press(()).into();
        let named = test_id("slate.new_document", button(Space::new()).on_press(()));

        assert_eq!(
            Widget::<(), crate::Theme, ()>::tag(&named),
            plain.as_widget().tag()
        );
    }
}
