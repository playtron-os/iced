//! By default the shared text system also holds the host's installed fonts, which is what
//! makes a capture machine-dependent and `set_system_fonts(false)` worth having. Turning
//! them off after it is built drops the host's faces and keeps the program's own.
//!
//! Its own test binary: the text system is built once per process.
use iced_graphics::text;
use iced_graphics::text::cosmic_text::fontdb::Source;

use std::borrow::Cow;

/// A face of the program's own, loaded before the host's fonts are dropped.
const BUNDLED: &[u8] = include_bytes!("../fonts/Iced-Icons.ttf");

#[test]
fn turning_the_hosts_fonts_off_once_built_drops_them_and_keeps_the_programs() {
    let mut font_system = text::font_system().write().expect("Write font system");
    assert!(font_system.system_fonts());
    font_system.load_font(Cow::Owned(BUNDLED.to_vec()));

    let from_host = font_system
        .raw()
        .db()
        .faces()
        .filter(|face| !matches!(face.source, Source::Binary(_)))
        .count();
    // A host with no fonts installed (a bare CI image) proves nothing either way.
    eprintln!("{from_host} faces came from the host");
    let ours: Vec<_> = font_system
        .raw()
        .db()
        .faces()
        .filter(|face| matches!(face.source, Source::Binary(_)))
        .map(|face| face.id)
        .collect();
    let version = font_system.version();
    drop(font_system);

    assert!(
        text::set_system_fonts(false),
        "turning the host's fonts off holds once built"
    );

    let mut font_system = text::font_system().write().expect("Write font system");
    assert!(!font_system.system_fonts());
    assert!(
        font_system.version() > version,
        "caches keyed on the version refresh"
    );
    for face in font_system.raw().db().faces() {
        assert!(
            matches!(face.source, Source::Binary(_)),
            "{:?} came from the host: {:?}",
            face.families,
            face.source
        );
    }
    for id in ours {
        assert!(
            font_system.raw().db().face(id).is_some(),
            "a face of the program's own keeps its id"
        );
    }
    drop(font_system);

    // The host's fonts do not come back.
    assert!(!text::set_system_fonts(true));
}
