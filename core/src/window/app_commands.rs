//! What a window offers the shell's command surfaces, and what the shell asks back.

/// A window's commands and recent items, published as one catalog that
/// replaces the last.
///
/// On Kora the Halo lists them in its app menu and command palette.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppCommands {
    /// Standard verbs the window answers itself (`undo`, `redo`, `cut`, `copy`,
    /// `paste`, `selall`, `markv`, `settings`, `neww`, `find`, `info`), and
    /// whether it can right now.
    pub handles: Vec<(String, bool)>,
    /// The window's own commands, in order.
    pub commands: Vec<AppCommand>,
    /// Things the window can reopen, for the shell's Open Recent.
    pub recents: Vec<AppRecent>,
}

/// One of a window's own commands.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppCommand {
    /// Namespaced with a dot, such as `slate.zoom-in`.
    pub id: String,
    /// Row label.
    pub name: String,
    /// Shortcut hint, display only.
    pub keys: String,
    /// Heading the command is listed under; empty for none.
    pub section: String,
    /// Symbolic icon name; empty for the shell's generic glyph.
    pub icon: String,
    /// Put forward for the shell's compact app menu.
    pub menu: bool,
    /// Toggles rather than fires; `active` says which way.
    pub stateful: bool,
    /// The window itself binds `keys`.
    pub bound: bool,
    /// Whether it can run right now.
    pub enabled: bool,
    /// Whether a stateful command is on.
    pub active: bool,
}

/// An item the window can reopen.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppRecent {
    /// Meaningful only to the window; handed back when picked.
    pub id: String,
    /// The row's title.
    pub label: String,
    /// An optional second line: where it lives, what it is.
    pub sublabel: String,
    /// Milliseconds since the Unix epoch of its last activity.
    pub timestamp_ms: u64,
}

/// What the shell asked a window to do.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AppCommandRequest {
    /// Run the command or standard verb with this id.
    Invoke(String),
    /// Reopen the recent item with this id.
    OpenRecent(String),
}
