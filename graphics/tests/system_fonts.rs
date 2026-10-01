//! With the host's fonts off, the shared text system holds only the program's own fonts.
//!
//! Its own test binary: the text system is built once per process.
use iced_graphics::text;
use iced_graphics::text::cosmic_text::fontdb::Source;

use std::borrow::Cow;

/// A face the host may well have installed, loaded here from the program's own bytes.
const BUNDLED: &[u8] = include_bytes!("../fonts/Iced-Icons.ttf");

#[test]
fn without_system_fonts_the_text_system_holds_only_the_programs_fonts() {
    assert!(
        text::set_system_fonts(false),
        "nothing built the text system yet"
    );

    let mut font_system = text::font_system().write().expect("Write font system");
    font_system.load_font(Cow::Borrowed(BUNDLED));
    assert!(!font_system.system_fonts());

    let faces: Vec<_> = font_system.raw().db().faces().collect();
    assert!(!faces.is_empty(), "the built-in fonts are there");
    for face in &faces {
        assert!(
            matches!(face.source, Source::Binary(_)),
            "{:?} came from the host: {:?}",
            face.families,
            face.source
        );
    }
    drop(font_system);

    // Built now, so the choice can no longer change.
    assert!(!text::set_system_fonts(true));
    assert!(text::set_system_fonts(false));
}
