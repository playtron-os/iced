//! A text editor reports what it holds — or, empty, its placeholder — so a
//! selector (or a test driver reading the tree) can find it and read back
//! what was typed into it.
//!
//! Only the placeholder is checked here: these tests build without a
//! renderer backend, and an editor's content needs one to hold any text.
use iced_test::Simulator;
use iced_widget::core::{Element, Theme};
use iced_widget::text_editor::{Content, TextEditor};

type Renderer = iced_renderer::Renderer;

#[derive(Debug, Clone)]
struct Edited;

#[test]
fn an_empty_editor_is_found_by_its_placeholder() {
    let content = Content::new();
    let editor: Element<'_, Edited, Theme, Renderer> = TextEditor::new(&content)
        .placeholder("Write a note")
        .on_action(|_| Edited)
        .into();
    let mut ui = Simulator::new(editor);

    assert!(ui.find("Write a note").is_ok());
}
