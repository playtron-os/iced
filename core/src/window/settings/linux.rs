//! Platform specific settings for Linux.

/// The platform specific window settings of an application.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PlatformSpecific {
    /// Sets the application id of the window.
    ///
    /// As a best practice, it is suggested to select an application id that match
    /// the basename of the application’s .desktop file.
    pub application_id: String,

    /// Whether bypass the window manager mapping for x11 windows
    ///
    /// This flag is particularly useful for creating UI elements that need precise
    /// positioning and immediate display without window manager interference.
    pub override_redirect: bool,

    /// Wayland-only: an xdg-foreign handle of a toplevel exported by another
    /// client, to be made the parent of this window.
    ///
    /// This is the bare handle string (no `wayland:` prefix) obtained by the
    /// other client via `zxdg_exporter_v2` — for example the `parent_window` an
    /// xdg-desktop-portal `FileChooser` backend receives. The window is imported
    /// via `zxdg_importer_v2` and set as a child of that toplevel, so the
    /// compositor places it as a dialog over the requesting application's window.
    /// Ignored on X11 and on compositors without `zxdg_importer_v2`.
    pub wayland_parent: Option<String>,

    /// Wayland-only: let Kora's Halo header float over the window's top edge
    /// instead of reserving room above it (`kora_halo_header_manager_v1`,
    /// overlay mode).
    ///
    /// For a window laid out chromeless — full-bleed content whose first row
    /// already sits clear of the Halo's hot zone. The mode is sent before the
    /// window's initial commit, the only time the protocol takes it. Ignored
    /// on X11 and on compositors without the global; ask
    /// `window::is_halo_header_overlay` which one the window got.
    pub halo_header_overlay: bool,
}
