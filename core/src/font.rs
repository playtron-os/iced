//! Load and use fonts.
use std::hash::Hash;

/// A font.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Font {
    /// The [`Family`] of the [`Font`].
    pub family: Family,
    /// The [`Weight`] of the [`Font`].
    pub weight: Weight,
    /// The [`Stretch`] of the [`Font`].
    pub stretch: Stretch,
    /// The [`Style`] of the [`Font`].
    pub style: Style,
    /// The OpenType [`Features`] the [`Font`] turns on.
    pub features: Features,
}

impl Font {
    /// A non-monospaced sans-serif font with normal [`Weight`].
    pub const DEFAULT: Font = Font {
        family: Family::SansSerif,
        weight: Weight::Normal,
        stretch: Stretch::Normal,
        style: Style::Normal,
        features: Features::NONE,
    };

    /// A monospaced font with normal [`Weight`].
    pub const MONOSPACE: Font = Font {
        family: Family::Monospace,
        ..Self::DEFAULT
    };

    /// Creates a [`Font`] with the given [`Family::Name`] and default attributes.
    pub const fn new(name: &'static str) -> Self {
        Self {
            family: Family::Name(name),
            ..Self::DEFAULT
        }
    }

    /// Creates a [`Font`] with the given [`Family::Name`] and default attributes.
    ///
    /// The same as [`Font::new`].
    pub const fn with_name(name: &'static str) -> Self {
        Self::new(name)
    }

    /// Creates a [`Font`] with the given [`Family`] and default attributes.
    pub fn with_family(family: impl Into<Family>) -> Self {
        Font {
            family: family.into(),
            ..Self::DEFAULT
        }
    }

    /// Sets the [`Weight`] of the [`Font`].
    pub const fn weight(self, weight: Weight) -> Self {
        Self { weight, ..self }
    }

    /// Sets the [`Stretch`] of the [`Font`].
    pub const fn stretch(self, stretch: Stretch) -> Self {
        Self { stretch, ..self }
    }

    /// Sets the [`Style`] of the [`Font`].
    pub const fn style(self, style: Style) -> Self {
        Self { style, ..self }
    }

    /// Returns this [`Font`] with every figure the same width, so numbers
    /// that change in place keep their columns (CSS `tabular-nums`).
    pub const fn tabular_figures(self) -> Self {
        Font {
            features: self.features.union(Features::TABULAR_FIGURES),
            ..self
        }
    }
}

impl From<&'static str> for Font {
    fn from(name: &'static str) -> Self {
        Font::new(name)
    }
}

impl From<Family> for Font {
    fn from(family: Family) -> Self {
        Font::with_family(family)
    }
}

/// A set of OpenType features a [`Font`] turns on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Features(u8);

impl Features {
    /// No features beyond the font's defaults.
    pub const NONE: Self = Self(0);

    /// Tabular figures (`tnum`): every digit takes the same advance.
    pub const TABULAR_FIGURES: Self = Self(1);

    /// Returns the features in either set.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Returns whether every feature in `other` is in this set.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Returns whether the set is empty.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// A font family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Family {
    /// The name of a font family of choice.
    Name(&'static str),

    /// Serif fonts represent the formal text style for a script.
    Serif,

    /// Glyphs in sans-serif fonts, as the term is used in CSS, are generally low
    /// contrast and have stroke endings that are plain — without any flaring,
    /// cross stroke, or other ornamentation.
    #[default]
    SansSerif,

    /// Glyphs in cursive fonts generally use a more informal script style, and
    /// the result looks more like handwritten pen or brush writing than printed
    /// letterwork.
    Cursive,

    /// Fantasy fonts are primarily decorative or expressive fonts that contain
    /// decorative or expressive representations of characters.
    Fantasy,

    /// The sole criterion of a monospace font is that all glyphs have the same
    /// fixed width.
    Monospace,
}

impl Family {
    /// A list of all the different standalone family variants.
    pub const VARIANTS: &[Self] = &[
        Self::Serif,
        Self::SansSerif,
        Self::Cursive,
        Self::Fantasy,
        Self::Monospace,
    ];

    /// Creates a [`Family::Name`] from the given string.
    ///
    /// The name is interned in a global cache and never freed.
    pub fn name(name: &str) -> Self {
        use rustc_hash::FxHashSet;
        use std::sync::{LazyLock, Mutex};

        static NAMES: LazyLock<Mutex<FxHashSet<&'static str>>> = LazyLock::new(Mutex::default);

        let mut names = NAMES.lock().expect("lock font name cache");

        let Some(name) = names.get(name) else {
            let name: &'static str = name.to_owned().leak();
            let _ = names.insert(name);

            return Self::Name(name);
        };

        Self::Name(name)
    }
}

impl From<&str> for Family {
    fn from(name: &str) -> Self {
        Family::name(name)
    }
}

impl std::fmt::Display for Family {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Family::Name(name) => name,
            Family::Serif => "Serif",
            Family::SansSerif => "Sans-serif",
            Family::Cursive => "Cursive",
            Family::Fantasy => "Fantasy",
            Family::Monospace => "Monospace",
        })
    }
}

/// The weight of some text.
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Weight {
    Thin,
    ExtraLight,
    Light,
    #[default]
    Normal,
    Medium,
    Semibold,
    Bold,
    ExtraBold,
    Black,
}

/// The width of some text.
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Stretch {
    UltraCondensed,
    ExtraCondensed,
    Condensed,
    SemiCondensed,
    #[default]
    Normal,
    SemiExpanded,
    Expanded,
    ExtraExpanded,
    UltraExpanded,
}

/// The style of some text.
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Style {
    #[default]
    Normal,
    Italic,
    Oblique,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_interned_once() {
        let first = Family::name(&String::from("Kora Test Sans"));
        let second = Family::from(String::from("Kora Test Sans").as_str());

        let (Family::Name(first), Family::Name(second)) = (first, second) else {
            panic!("expected named families");
        };

        assert_eq!(first, "Kora Test Sans");
        assert!(std::ptr::eq(first, second));
    }

    #[test]
    fn with_name_is_new() {
        assert_eq!(Font::with_name("Inter"), Font::new("Inter"));
        assert_eq!(Font::from("Inter"), Font::new("Inter"));
    }

    #[test]
    fn setters_change_one_attribute() {
        let font = Font::with_family("Inter")
            .weight(Weight::Bold)
            .stretch(Stretch::Condensed)
            .style(Style::Italic);

        assert_eq!(
            font,
            Font {
                family: Family::Name("Inter"),
                weight: Weight::Bold,
                stretch: Stretch::Condensed,
                style: Style::Italic,
                features: Features::NONE,
            }
        );
        assert_eq!(
            Font::from(Family::Monospace).tabular_figures(),
            Font::MONOSPACE.tabular_figures()
        );
    }

    #[test]
    fn a_family_shows_its_name() {
        let shown: Vec<String> = Family::VARIANTS.iter().map(ToString::to_string).collect();

        assert_eq!(
            shown,
            ["Serif", "Sans-serif", "Cursive", "Fantasy", "Monospace"]
        );
        assert_eq!(Family::name("Fira Code").to_string(), "Fira Code");
    }
}
