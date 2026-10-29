//! Select, copy and clear text across every selectable widget.
//!
//! The runtime drives these: a drag in empty space selects through the text
//! widgets it crosses inside the scrollable it started in, and the copy
//! shortcut gathers what they hold.
use crate::text::Target;
use crate::widget::Id;
use crate::widget::operation::{Operation, Outcome, Scrollable};
use crate::{Point, Rectangle, Vector};

/// The state of a widget whose text can be selected.
pub trait Selectable {
    /// Selects the text between two positions in layout coordinates, as a drag
    /// from `start` to `end` would.
    fn select(&mut self, start: Point, end: Point, target: Target);

    /// Selects all the text of the widget.
    fn select_all(&mut self);

    /// Clears the selection of the widget.
    fn deselect(&mut self);

    /// Returns the selected text, if any.
    fn copy(&mut self) -> Option<String>;
}

/// Where a selection may reach: the scrollable it started in, so a drag in
/// one pane leaves the text of the panes beside it alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Scope(
    /// The scrollable's place in the order a traversal meets them, or `None`
    /// for the whole window.
    Option<usize>,
);

/// Finds the [`Scope`] of a selection that starts at `at`, a position in
/// window coordinates: the innermost scrollable under it.
pub fn scope(at: Point) -> impl Operation<Scope> {
    struct Find {
        at: Point,
        /// Whether `at` lies inside every scrollable around the current widgets.
        walk: Walk<bool>,
        found: Option<usize>,
    }

    impl Operation<Scope> for Find {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<Scope>)) {
            let outer = self.walk.enter();
            operate(self);
            self.walk.leave(outer);
        }

        fn scrollable(
            &mut self,
            _id: Option<&Id>,
            bounds: Rectangle,
            _content_bounds: Rectangle,
            translation: Vector,
            _state: &mut dyn Scrollable,
        ) {
            let under = self.walk.state && bounds.contains(self.at + self.walk.translation);
            let order = self.walk.scrollable(translation, under);

            if under {
                self.found = Some(order);
            }
        }

        fn finish(&self) -> Outcome<Scope> {
            Outcome::Some(Scope(self.found))
        }
    }

    Find {
        at,
        walk: Walk::new(true),
        found: None,
    }
}

/// Selects the text between `start` and `end`, positions in window coordinates,
/// and clears the selection of the text outside `scope`.
pub fn select(start: Point, end: Point, target: Target, scope: Scope) -> impl Operation {
    struct Select {
        start: Point,
        end: Point,
        target: Target,
        scope: Scope,
        /// Whether the current widgets are inside the scope.
        walk: Walk<bool>,
    }

    impl Operation for Select {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
            let outer = self.walk.enter();
            operate(self);
            self.walk.leave(outer);
        }

        fn scrollable(
            &mut self,
            _id: Option<&Id>,
            _bounds: Rectangle,
            _content_bounds: Rectangle,
            translation: Vector,
            _state: &mut dyn Scrollable,
        ) {
            self.walk.scrollable_in(translation, self.scope);
        }

        fn selectable_text(
            &mut self,
            _id: Option<&Id>,
            _bounds: Rectangle,
            state: &mut dyn Selectable,
        ) {
            if !self.walk.state {
                state.deselect();
                return;
            }

            state.select(
                self.start + self.walk.translation,
                self.end + self.walk.translation,
                self.target,
            );
        }
    }

    Select {
        start,
        end,
        target,
        scope,
        walk: Walk::new(scope.0.is_none()),
    }
}

/// Selects all the selectable text inside `scope`, and clears the rest.
pub fn select_all(scope: Scope) -> impl Operation {
    struct SelectAll {
        scope: Scope,
        /// Whether the current widgets are inside the scope.
        walk: Walk<bool>,
    }

    impl Operation for SelectAll {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
            let outer = self.walk.enter();
            operate(self);
            self.walk.leave(outer);
        }

        fn scrollable(
            &mut self,
            _id: Option<&Id>,
            _bounds: Rectangle,
            _content_bounds: Rectangle,
            translation: Vector,
            _state: &mut dyn Scrollable,
        ) {
            self.walk.scrollable_in(translation, self.scope);
        }

        fn selectable_text(
            &mut self,
            _id: Option<&Id>,
            _bounds: Rectangle,
            state: &mut dyn Selectable,
        ) {
            if self.walk.state {
                state.select_all();
            } else {
                state.deselect();
            }
        }
    }

    SelectAll {
        scope,
        walk: Walk::new(scope.0.is_none()),
    }
}

/// Clears every selection.
pub fn deselect() -> impl Operation {
    struct Deselect;

    impl Operation for Deselect {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
            operate(self);
        }

        fn selectable_text(
            &mut self,
            _id: Option<&Id>,
            _bounds: Rectangle,
            state: &mut dyn Selectable,
        ) {
            state.deselect();
        }
    }

    Deselect
}

/// Gathers the selected text of every widget, one line per widget.
pub fn copy() -> impl Operation<String> {
    struct Copy {
        text: String,
    }

    impl Operation<String> for Copy {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<String>)) {
            operate(self);
        }

        fn selectable_text(
            &mut self,
            _id: Option<&Id>,
            _bounds: Rectangle,
            state: &mut dyn Selectable,
        ) {
            let Some(text) = state.copy().filter(|text| !text.is_empty()) else {
                return;
            };

            if !self.text.is_empty() {
                self.text.push('\n');
            }

            self.text.push_str(&text);
        }

        fn finish(&self) -> Outcome<String> {
            if self.text.is_empty() {
                Outcome::None
            } else {
                Outcome::Some(self.text.clone())
            }
        }
    }

    Copy {
        text: String::new(),
    }
}

/// A traversal's place among scrollables, with `S` an operation's own state
/// for the widgets inside the current one.
#[derive(Debug, Clone, Copy)]
struct Walk<S> {
    /// Added to a window position to compare it with the current widgets' bounds.
    translation: Vector,
    state: S,
    /// How many scrollables the traversal has met.
    met: usize,
    /// The content of the scrollable met last, which the next traversal enters.
    next: Option<(Vector, S)>,
}

impl<S: Copy> Walk<S> {
    fn new(state: S) -> Self {
        Self {
            translation: Vector::ZERO,
            state,
            met: 0,
            next: None,
        }
    }

    /// Meets a scrollable whose content is scrolled by `translation` and gets
    /// `state`, and returns its order.
    fn scrollable(&mut self, translation: Vector, state: S) -> usize {
        self.next = Some((self.translation + translation, state));
        self.met += 1;
        self.met - 1
    }

    /// Steps into the content of the scrollable met last, if any, and returns
    /// what [`Walk::leave`] restores.
    fn enter(&mut self) -> (Vector, S) {
        let outer = (self.translation, self.state);

        if let Some((translation, state)) = self.next.take() {
            self.translation = translation;
            self.state = state;
        }

        outer
    }

    fn leave(&mut self, (translation, state): (Vector, S)) {
        self.translation = translation;
        self.state = state;
    }
}

impl Walk<bool> {
    /// Meets a scrollable, whose content is inside `scope` when it is the
    /// scope or lies inside it.
    fn scrollable_in(&mut self, translation: Vector, scope: Scope) {
        let inside = self.state || scope.0 == Some(self.met);
        let _ = self.scrollable(translation, inside);
    }
}
