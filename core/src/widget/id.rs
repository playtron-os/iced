use std::borrow;
use std::sync::atomic::{self, AtomicUsize};

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

/// The identifier of a generic widget.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Id(Internal);

impl Id {
    /// Creates a new [`Id`] from a static `str`.
    pub const fn new(id: &'static str) -> Self {
        Self(Internal::Custom(borrow::Cow::Borrowed(id)))
    }

    /// Creates a unique [`Id`].
    ///
    /// This function produces a different [`Id`] every time it is called.
    pub fn unique() -> Self {
        let id = NEXT_ID.fetch_add(1, atomic::Ordering::Relaxed);

        Self(Internal::Unique(id))
    }

    /// Returns the name of the [`Id`], if it was created from a string.
    ///
    /// Unique ids have no name and return `None`.
    pub fn as_str(&self) -> Option<&str> {
        match &self.0 {
            Internal::Custom(name) => Some(name),
            Internal::Unique(_) => None,
        }
    }
}

impl From<&'static str> for Id {
    fn from(value: &'static str) -> Self {
        Self::new(value)
    }
}

impl From<String> for Id {
    fn from(value: String) -> Self {
        Self(Internal::Custom(borrow::Cow::Owned(value)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Internal {
    Unique(usize),
    Custom(borrow::Cow<'static, str>),
}

#[cfg(test)]
mod tests {
    use super::Id;

    #[test]
    fn unique_generates_different_ids() {
        let a = Id::unique();
        let b = Id::unique();

        assert_ne!(a, b);
    }

    #[test]
    fn as_str_returns_the_name_of_custom_ids_only() {
        assert_eq!(
            Id::new("slate.new_document").as_str(),
            Some("slate.new_document")
        );
        assert_eq!(
            Id::from(String::from("files.trash")).as_str(),
            Some("files.trash")
        );
        assert_eq!(Id::unique().as_str(), None);
    }
}
