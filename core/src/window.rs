//! Build window-based GUI applications.
pub mod icon;
pub mod screenshot;
pub mod settings;

mod app_commands;
mod direction;
mod event;
mod id;
mod level;
mod mode;
mod position;
mod redraw_request;
mod user_attention;

pub use app_commands::{AppCommand, AppCommandRequest, AppCommands, AppRecent};
pub use direction::Direction;
pub use event::Event;
pub use icon::Icon;
pub use id::Id;
pub use level::Level;
pub use mode::Mode;
pub use position::Position;
pub use redraw_request::RedrawRequest;
pub use screenshot::Screenshot;
pub use settings::Settings;
pub use user_attention::UserAttention;

/// A compositor-authenticated identity for one mapped window lifetime.
///
/// Available on Kora Wayland compositors. Visibility changes preserve the pair;
/// a true client unmap revokes it and a later mapping gets a new identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Identity {
    /// The identifier shared with the compositor's foreign window list.
    pub identifier: String,
    /// The authenticated process workspace. Empty means the machine plane.
    pub workspace: String,
}
