//! A capture never sees the host's fonts, whatever the program's settings ask for, so
//! it draws the same on every machine.
//!
//! Its own test binary: the text system is built once per process.
use iced_test::core::layout::{self, Layout};
use iced_test::core::renderer;
use iced_test::core::widget::{Tree, Widget};
use iced_test::core::{Element, Length, Rectangle, Settings, Size, Theme, mouse};
use iced_test::renderer::graphics::text;
use iced_test::renderer::graphics::text::cosmic_text::fontdb::Source;
use iced_test::{Simulator, renderer::Renderer};

struct Blank;

impl<Message> Widget<Message, Theme, Renderer> for Blank {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::Node::new(limits.max())
    }

    fn draw(
        &self,
        _tree: &Tree,
        _renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        _layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
    }
}

#[test]
fn a_simulator_holds_only_the_programs_fonts_even_when_settings_ask_for_the_hosts() {
    let settings = Settings {
        system_fonts: true,
        ..Settings::default()
    };
    let _ui: Simulator<'_, ()> = Simulator::with_settings(settings, Element::new(Blank));

    let mut font_system = text::font_system().write().expect("Write font system");
    assert!(!font_system.system_fonts());
    assert!(
        font_system
            .raw()
            .db()
            .faces()
            .all(|face| matches!(face.source, Source::Binary(_))),
        "a face came from the host"
    );
}
