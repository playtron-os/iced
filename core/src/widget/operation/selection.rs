//! Select, copy and clear text across every selectable widget.
//!
//! The runtime drives these: a drag in empty space selects through the text
//! widgets it crosses, and the copy shortcut gathers what they hold.
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

/// Selects the text between `start` and `end`, positions in window coordinates.
pub fn select(start: Point, end: Point, target: Target) -> impl Operation {
    struct Select {
        start: Point,
        end: Point,
        target: Target,
        translation: Vector,
    }

    impl Operation for Select {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
            let translation = self.translation;
            operate(self);
            self.translation = translation;
        }

        fn scrollable(
            &mut self,
            _id: Option<&Id>,
            _bounds: Rectangle,
            _content_bounds: Rectangle,
            translation: Vector,
            _state: &mut dyn Scrollable,
        ) {
            self.translation += translation;
        }

        fn selectable_text(
            &mut self,
            _id: Option<&Id>,
            _bounds: Rectangle,
            state: &mut dyn Selectable,
        ) {
            state.select(
                self.start + self.translation,
                self.end + self.translation,
                self.target,
            );
        }
    }

    Select {
        start,
        end,
        target,
        translation: Vector::ZERO,
    }
}

/// Selects all selectable text.
pub fn select_all() -> impl Operation {
    struct SelectAll;

    impl Operation for SelectAll {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
            operate(self);
        }

        fn selectable_text(
            &mut self,
            _id: Option<&Id>,
            _bounds: Rectangle,
            state: &mut dyn Selectable,
        ) {
            state.select_all();
        }
    }

    SelectAll
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
