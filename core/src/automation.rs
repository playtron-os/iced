//! How an app's automation door behaves, when the `automation` feature builds
//! one in and an administrator has switched it on.
//!
//! The door itself lives in the `iced_automation` crate. This is only the
//! app's say in it, carried in [`Settings`](crate::Settings) so an event loop
//! can hand it on. Every number here is clamped to bounds the door enforces,
//! and to any lower limits the device's administrator sets in the switch file;
//! an app can't widen what the administrator allows.
use std::path::PathBuf;
use std::time::Duration;

/// An app's settings for its automation door. `None` everywhere means the
/// door's own defaults.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Config {
    /// The switch file that opens the door, instead of the door's default.
    ///
    /// Fixed when the app is built: nothing at runtime (an environment
    /// variable, a user's setting) can change it, because whoever picks the
    /// path picks "on".
    pub switch: Option<PathBuf>,
    /// How long one step of a request waits for the app to answer.
    pub answer_timeout: Option<Duration>,
    /// How long input waits for the app to settle after each step.
    pub settle_timeout: Option<Duration>,
    /// How long one request may take in all, across its steps.
    pub request_timeout: Option<Duration>,
    /// How many pointer moves a drag is split into.
    pub drag_steps: Option<u16>,
    /// How many clients may be connected at once.
    pub max_clients: Option<usize>,
    /// How long a connected client may send nothing before it is let go.
    pub idle_client_timeout: Option<Duration>,
    /// Which kinds of request the door takes.
    pub ops: Option<Ops>,
}

impl Config {
    /// The door's defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets [`Config::switch`].
    pub fn switch(self, switch: impl Into<PathBuf>) -> Self {
        Self {
            switch: Some(switch.into()),
            ..self
        }
    }

    /// Sets [`Config::answer_timeout`].
    pub fn answer_timeout(self, timeout: Duration) -> Self {
        Self {
            answer_timeout: Some(timeout),
            ..self
        }
    }

    /// Sets [`Config::settle_timeout`].
    pub fn settle_timeout(self, timeout: Duration) -> Self {
        Self {
            settle_timeout: Some(timeout),
            ..self
        }
    }

    /// Sets [`Config::request_timeout`].
    pub fn request_timeout(self, timeout: Duration) -> Self {
        Self {
            request_timeout: Some(timeout),
            ..self
        }
    }

    /// Sets [`Config::drag_steps`].
    pub fn drag_steps(self, steps: u16) -> Self {
        Self {
            drag_steps: Some(steps),
            ..self
        }
    }

    /// Sets [`Config::max_clients`].
    pub fn max_clients(self, clients: usize) -> Self {
        Self {
            max_clients: Some(clients),
            ..self
        }
    }

    /// Sets [`Config::idle_client_timeout`].
    pub fn idle_client_timeout(self, timeout: Duration) -> Self {
        Self {
            idle_client_timeout: Some(timeout),
            ..self
        }
    }

    /// Sets [`Config::ops`].
    pub fn ops(self, ops: Ops) -> Self {
        Self {
            ops: Some(ops),
            ..self
        }
    }
}

/// Kinds of request the door takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ops {
    /// `info`, `tree` and `idle`: reading what the app shows.
    pub read: bool,
    /// `click`, `hover`, `scroll`, `drag` and `leave`: the pointer.
    pub pointer: bool,
    /// `type` and `key`: the keyboard.
    ///
    /// Keys typed into a terminal app run as shell commands, so an app like
    /// that may want to leave this out.
    pub keyboard: bool,
}

impl Ops {
    /// Every kind of request.
    pub const ALL: Self = Self {
        read: true,
        pointer: true,
        keyboard: true,
    };

    /// Reading only: no input goes in.
    pub const READ: Self = Self {
        read: true,
        pointer: false,
        keyboard: false,
    };

    /// What both `self` and `other` allow.
    pub fn and(self, other: Self) -> Self {
        Self {
            read: self.read && other.read,
            pointer: self.pointer && other.pointer,
            keyboard: self.keyboard && other.keyboard,
        }
    }
}

impl Default for Ops {
    fn default() -> Self {
        Self::ALL
    }
}
