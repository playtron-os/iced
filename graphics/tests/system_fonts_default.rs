//! By default the shared text system also holds the host's installed fonts, which is what
//! makes a capture machine-dependent and `set_system_fonts(false)` worth having.
//!
//! Its own test binary: the text system is built once per process.
use iced_graphics::text;
use iced_graphics::text::cosmic_text::fontdb::Source;

#[test]
fn by_default_the_text_system_holds_the_hosts_fonts_too() {
    let mut font_system = text::font_system().write().expect("Write font system");
    assert!(font_system.system_fonts());

    let from_host = font_system
        .raw()
        .db()
        .faces()
        .filter(|face| !matches!(face.source, Source::Binary(_)))
        .count();
    // A host with no fonts installed (a bare CI image) proves nothing either way.
    eprintln!("{from_host} faces came from the host");
    drop(font_system);

    assert!(
        !text::set_system_fonts(false),
        "already built with the host's fonts"
    );
}
