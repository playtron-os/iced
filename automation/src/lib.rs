//! A door that lets a test driver on the same device read and press the widgets
//! of a running app, the way a person would.
//!
//! The door stays shut unless an administrator has switched it on for the whole
//! device, by creating the switch file ([`DEFAULT_SWITCH`], or the app's
//! [`Config::switch`]) owned by root and writable by no one else. Nothing the
//! user's own processes can set (an environment variable, a setting, a file in
//! their home) opens it or moves it: on Wayland one app may not press buttons
//! in another, and the door must not become a way around that. The switch is
//! read again for every connection, so removing it shuts the door for every
//! new client without restarting the app.
//!
//! An empty switch file opens the door with its defaults. Otherwise the file
//! is JSON, and lets the administrator narrow what the door does; a file the
//! door can't read as that keeps it shut:
//!
//! ```json
//! {
//!   "max": { "answer_ms": 5000, "settle_ms": 500, "request_ms": 30000,
//!            "steps": 8, "clients": 8, "idle_client_ms": 120000 },
//!   "ops": ["read", "pointer", "keyboard"],
//!   "apps": ["org.example.app"]
//! }
//! ```
//!
//! Every key is optional. `max` caps the app's settings ([`Config`]) and what
//! a request may ask for; `ops` lists the kinds of request taken (`read` is
//! `info`, `tree` and `idle`; `pointer` is `click`, `hover`, `scroll`, `drag`
//! and `leave`; `keyboard` is `type` and `key`); `apps` lists the app ids that
//! open a door at all.
//!
//! When the switch is on, the app listens on a Unix socket only its own user
//! can reach, at `$XDG_RUNTIME_DIR/iced-automation/<app id>-<pid>.sock`, and
//! answers one JSON object per line:
//!
//! | request | reply |
//! |---|---|
//! | `{"op":"info"}` | the app id, pid and its windows and popups |
//! | `{"op":"tree"}` | every named widget, text, input and focusable, with bounds |
//! | `{"op":"idle"}` | whether the app has nothing left to process or draw |
//! | `{"op":"click","id":"…"}`, `{"op":"click","text":"…"}` | clicks the match |
//! | `{"op":"hover", …}` | moves the pointer onto the match |
//! | `{"op":"scroll", …, "dy":-3}` | scrolls the wheel over the match, in lines (negative is down) |
//! | `{"op":"drag", …, "to":{…}}` | presses on the match, moves to `to` (a target, or `dx`/`dy`), releases |
//! | `{"op":"type","text":"…"}` | types the text |
//! | `{"op":"key","key":"Enter","modifiers":["ctrl"]}` | presses a key |
//! | `{"op":"leave"}` | moves the door's pointer off the surface it is on |
//!
//! Clicks and keys go in as input events, into the same queue a person's input
//! reaches the widgets through, never by calling the app's handlers. They skip
//! the windowing layer itself, so, for instance, a click outside a Wayland popup
//! doesn't dismiss it. Positions are surface-relative logical pixels. Pointer
//! requests also take `"x"` and `"y"`, and any request can name a `"window"`
//! from `info`.
//!
//! Any request may also give `"timeout_ms"` (how long it may take in all),
//! `"settle_ms"` (how long to wait for the app to settle after each step; `0`
//! doesn't wait) and, for a drag, `"steps"`. The app's [`Config`] gives the
//! values a request leaves out; whatever a request gives is clamped to the
//! administrator's `max` and the door's own bounds.
//!
//! # What the door can do
//!
//! The door reads what an app shows and puts input into it, as its own user.
//! That is more than "read and press": keys typed into a terminal app run as
//! shell commands, so for a confined process that can reach the socket, the
//! door of such an app is a shell. An app like that can leave the keyboard out
//! with [`Config::ops`], and an administrator can for every app. The tree also
//! carries what text inputs and editors hold (a password input's dots, so its
//! length; a text editor's whole content).
//!
//! Only the same user can reach the socket (a folder of their own, mode 0700,
//! in their private runtime folder), and the door bounds what one client can
//! cost the app: one request line of at most 64 KiB, a ceiling on clients, a
//! poll no faster than every 16 ms, and clients that send nothing for a while
//! are let go.
//!
//! # For event loops
//!
//! This crate owns the switch, the socket and the protocol; it knows nothing
//! about any one event loop. A loop opens the door once with [`start_with`]
//! (passing the app's [`Config`]) or [`start`], keeping the [`Open`] it returns
//! for as long as it runs, and calls [`serve`] once per pass, before it hands
//! out its pending events. [`serve`] gives it each request as an [`Ask`]; it
//! answers with an [`Answer`], reading its widget trees with a [`Collector`].
//! [`Ask`], [`Answer`] and [`Surface`] may grow: match with a wildcard and
//! answer what you don't know with [`Answer::Unsupported`].
#![cfg(target_os = "linux")]

use iced_core as core;

use crate::core::keyboard::{self, key};
use crate::core::mouse;
use crate::core::widget::operation::{Focusable, Hidden, PaintedOffset, Scrollable, TextInput};
use crate::core::widget::{Id, Operation};
use crate::core::window;
use crate::core::{Event, Point, Rectangle, Size, SmolStr, Vector};

use serde_json::{Value, json};

pub use crate::core::automation::{Config, Ops};

use std::cell::Cell;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

/// The switch file that opens the door, unless the app's [`Config`] names
/// another: `/etc/iced/automation-enabled`, or the path in the
/// `ICED_AUTOMATION_SWITCH` environment variable when iced was built. A
/// distribution that keeps its switch elsewhere can also link this path to
/// it: the door follows links root put there, and a link to nothing keeps it
/// shut.
pub const DEFAULT_SWITCH: &str = match option_env!("ICED_AUTOMATION_SWITCH") {
    Some(path) => path,
    None => "/etc/iced/automation-enabled",
};

/// The longest request line read, in bytes.
const MAX_REQUEST: u64 = 64 * 1024;

/// The longest switch file read, in bytes.
const MAX_SWITCH: u64 = 64 * 1024;

/// How long before a client gives up the event loop stops taking its request,
/// so that a request the client was told timed out never runs later.
const ANSWER_MARGIN: Duration = Duration::from_millis(250);

/// How often the door asks a busy app whether it has settled, at most.
const POLL: Duration = Duration::from_millis(16);

/// The door's numbers: its defaults, the bounds nothing can set it outside of,
/// and the limits in force for a client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Limits {
    /// How long a step of a request waits for the app to answer.
    answer: Duration,
    /// How long input waits for the app to settle after each step.
    settle: Duration,
    /// How long one request may take in all, across its steps (a long
    /// `type`, say). After that, or once the client has hung up, the rest is
    /// not sent.
    request: Duration,
    /// How many pointer moves a drag is split into.
    drag_steps: u16,
    /// How many clients may be connected at once.
    clients: usize,
    /// How long a connected client may send nothing before it is let go.
    idle_client: Duration,
}

const DEFAULTS: Limits = Limits {
    answer: Duration::from_secs(5),
    settle: Duration::from_millis(500),
    request: Duration::from_secs(30),
    drag_steps: 8,
    clients: 8,
    idle_client: Duration::from_secs(120),
};

/// Nothing sets the door below these...
const HARD_MIN: Limits = Limits {
    answer: Duration::from_millis(500),
    settle: Duration::ZERO,
    request: Duration::from_secs(1),
    drag_steps: 1,
    clients: 1,
    idle_client: Duration::from_secs(1),
};

/// ...or above these, whatever the app or the switch file says.
const HARD_MAX: Limits = Limits {
    answer: Duration::from_secs(60),
    settle: Duration::from_secs(10),
    request: Duration::from_secs(600),
    drag_steps: 200,
    clients: 32,
    idle_client: Duration::from_secs(3600),
};

/// What an administrator's switch file says (see the crate docs).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Policy {
    /// The highest values allowed; `None` leaves the door's own bound.
    max: Max,
    ops: Ops,
    /// The apps that open a door; `None` is every app.
    apps: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Max {
    answer: Option<Duration>,
    settle: Option<Duration>,
    request: Option<Duration>,
    drag_steps: Option<u16>,
    clients: Option<usize>,
    idle_client: Option<Duration>,
}

impl Policy {
    fn allows(&self, app_id: &str) -> bool {
        self.apps
            .as_ref()
            .is_none_or(|apps| apps.iter().any(|app| app == app_id))
    }
}

/// Clamps `wanted` (else `default`) to `[min, min(admin, max)]`. An
/// administrator's limit below the hard minimum still wins.
fn clamp<T: Ord + Copy>(wanted: Option<T>, default: T, min: T, admin: Option<T>, max: T) -> T {
    let ceiling = admin.map_or(max, |admin| admin.min(max));
    let floor = min.min(ceiling);

    wanted.unwrap_or(default).clamp(floor, ceiling)
}

impl Limits {
    /// The limits for an app with `config`, under `max`.
    fn resolve(config: &Config, max: &Max) -> Self {
        Self {
            answer: clamp(
                config.answer_timeout,
                DEFAULTS.answer,
                HARD_MIN.answer,
                max.answer,
                HARD_MAX.answer,
            ),
            settle: clamp(
                config.settle_timeout,
                DEFAULTS.settle,
                HARD_MIN.settle,
                max.settle,
                HARD_MAX.settle,
            ),
            request: clamp(
                config.request_timeout,
                DEFAULTS.request,
                HARD_MIN.request,
                max.request,
                HARD_MAX.request,
            ),
            drag_steps: clamp(
                config.drag_steps,
                DEFAULTS.drag_steps,
                HARD_MIN.drag_steps,
                max.drag_steps,
                HARD_MAX.drag_steps,
            ),
            clients: clamp(
                config.max_clients,
                DEFAULTS.clients,
                HARD_MIN.clients,
                max.clients,
                HARD_MAX.clients,
            ),
            idle_client: clamp(
                config.idle_client_timeout,
                DEFAULTS.idle_client,
                HARD_MIN.idle_client,
                max.idle_client,
                HARD_MAX.idle_client,
            ),
        }
    }
}

/// Requests waiting for the event loop, while the door is open.
static INBOX: Mutex<Option<mpsc::Receiver<Job>>> = Mutex::new(None);

/// Whether the door is open in this process.
static OPEN: AtomicBool = AtomicBool::new(false);

type Wake = Arc<dyn Fn() + Send + Sync>;

/// An open door. Dropping it shuts the door: it removes the socket, and every
/// connected client is told the app has stopped. Keep it while the loop runs.
#[derive(Debug)]
#[must_use = "dropping the door removes its socket at once"]
pub struct Open {
    path: PathBuf,
}

impl Open {
    /// Where the socket is.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        OPEN.store(false, Ordering::SeqCst);

        // Dropping the inbox makes every waiting client's request fail at once
        // ("the app has stopped"), instead of each waiting out its timeout.
        if let Ok(mut inbox) = INBOX.lock() {
            *inbox = None;
        }

        let _ = fs::remove_file(&self.path);
    }
}

/// Opens the door with the door's defaults, if the device's switch is on.
/// See [`start_with`].
#[must_use = "dropping the door removes its socket at once"]
pub fn start(app_id: Option<&str>, wake: impl Fn() + Send + Sync + 'static) -> Option<Open> {
    start_with(Config::default(), app_id, wake)
}

/// Opens the door if the device's switch is on and lets this app in.
/// Otherwise does nothing at all, and returns `None`.
///
/// `config` is the app's say in how the door behaves ([`Config`]); the
/// administrator's switch file can narrow it further. The socket is named
/// after `app_id` (the Wayland app id, when the app sets one), or else the name
/// the program was started as. `wake` must make the event loop run a pass
/// soon, so that it calls [`serve`]; it is called from the door's own threads.
#[must_use = "dropping the door removes its socket at once"]
pub fn start_with(
    config: Config,
    app_id: Option<&str>,
    wake: impl Fn() + Send + Sync + 'static,
) -> Option<Open> {
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    let program = std::env::args_os()
        .next()
        .and_then(|arg| {
            Path::new(&arg)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .filter(|name| !name.is_empty());

    open_door(
        config,
        &[0],
        runtime_dir,
        app_id.or(program.as_deref()),
        Arc::new(wake),
    )
}

/// Whether the door is open in this process. Cheap: one atomic load.
pub fn is_open() -> bool {
    OPEN.load(Ordering::Relaxed)
}

/// Answers the requests waiting for the event loop, by calling `answer` for
/// each. A loop calls it once per pass, before it hands out its pending events,
/// so anything injected is processed in the same pass, exactly as if the
/// compositor had sent it. Does nothing when the door is shut.
///
/// Requests the client has already given up on are dropped unasked, except
/// ones that let go of a key or button an earlier request pressed.
pub fn serve(mut answer: impl FnMut(Ask) -> Answer) {
    if !is_open() {
        return;
    }

    let Ok(inbox) = INBOX.lock() else {
        return;
    };

    if let Some(inbox) = inbox.as_ref() {
        answer_all(inbox, &mut answer);
    }
}

fn answer_all(inbox: &mpsc::Receiver<Job>, answer: &mut impl FnMut(Ask) -> Answer) {
    while let Ok(job) = inbox.try_recv() {
        if !job.cleanup && Instant::now() > job.deadline {
            continue;
        }

        let _ = job.answer.send(answer(job.ask));
    }
}

/// Whether a surface will draw soon, for answering [`Ask::Idle`]: a frame was
/// asked for (`requested_at`) and not yet drawn, or a timed redraw
/// (`redraw_at`, an animation step, say) is less than a frame away.
///
/// `requested_at` is when the oldest frame still undrawn was asked for: a
/// loop sets it on the first request after a draw and keeps it until the next
/// draw, so frames asked for again and again (messages arriving several times
/// a second for a minimised window) don't keep the app busy forever.
///
/// A frame asked for long ago and never drawn means the compositor isn't
/// drawing the surface (it is hidden), and a timed redraw further off (a text
/// cursor blinking) is not work in progress, so neither keeps the app busy.
pub fn frame_due(requested_at: Option<Instant>, redraw_at: Option<Instant>, now: Instant) -> bool {
    const FRAME: Duration = Duration::from_millis(50);
    const UNDRAWN: Duration = Duration::from_millis(250);

    requested_at.is_some_and(|requested| now.saturating_duration_since(requested) < UNDRAWN)
        || redraw_at.is_some_and(|at| at <= now + FRAME)
}

/// Everything a connection needs to know about the door it came in through.
struct Shared {
    config: Config,
    switch: PathBuf,
    /// The owners a switch may have: root, except in tests.
    trusted: Vec<u32>,
    app_id: String,
    wake: Wake,
    jobs: mpsc::Sender<Job>,
    /// The surface the door's pointer is on, for every client.
    pointed: Arc<Mutex<Option<window::Id>>>,
}

fn open_door(
    config: Config,
    trusted: &[u32],
    runtime_dir: Option<PathBuf>,
    app_id: Option<&str>,
    wake: Wake,
) -> Option<Open> {
    let switch = config
        .switch
        .clone()
        .unwrap_or_else(|| PathBuf::from(DEFAULT_SWITCH));
    let app_id = app_id.unwrap_or("app").to_owned();

    let policy = match read_policy(&switch, trusted) {
        Ok(policy) => policy,
        Err(Shut::Off) => return None,
        Err(Shut::Refused(why)) => {
            log::warn!("automation: door stays shut: {why}");
            return None;
        }
    };

    if !policy.allows(&app_id) {
        log::info!(
            "automation: door stays shut: {} doesn't list {app_id}",
            switch.display()
        );
        return None;
    }

    // Before anything touches the socket folder: a second door would bind,
    // and then remove, the first one's socket.
    if is_open() {
        log::warn!("automation: the door is already open in this process");
        return None;
    }

    let Some(runtime_dir) = runtime_dir else {
        log::warn!(
            "automation: {} is on, but XDG_RUNTIME_DIR is unset; door stays shut",
            switch.display()
        );
        return None;
    };

    let (listener, path) = match listen(&runtime_dir, Some(&app_id)) {
        Ok(opened) => opened,
        Err(error) => {
            log::warn!(
                "automation: {} is on, but the door could not open: {error}",
                switch.display()
            );
            return None;
        }
    };

    let (jobs, inbox) = mpsc::channel();

    {
        let Ok(mut slot) = INBOX.lock() else {
            let _ = fs::remove_file(&path);
            return None;
        };

        if slot.is_some() || OPEN.swap(true, Ordering::SeqCst) {
            log::warn!("automation: the door is already open in this process");
            let _ = fs::remove_file(&path);
            return None;
        }

        *slot = Some(inbox);
    }

    let open = Open { path };

    log::warn!(
        "automation: door open at {} because {} is on",
        open.path.display(),
        switch.display()
    );

    let shared = Arc::new(Shared {
        config,
        switch,
        trusted: trusted.to_vec(),
        app_id,
        wake,
        jobs,
        pointed: Arc::new(Mutex::new(None)),
    });

    let _ = thread::Builder::new()
        .name("iced-automation".into())
        .spawn(move || accept(&listener, &shared))
        .ok()?;

    Some(open)
}

/// Why the switch doesn't open the door.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Shut {
    /// There is no switch, or not one root put there.
    Off,
    /// There is one, but it can't be read as a switch file.
    Refused(String),
}

/// What the switch at `switch` allows, if it is on.
fn read_policy(switch: &Path, trusted: &[u32]) -> Result<Policy, Shut> {
    if !switch_is_on(switch, trusted) {
        return Err(Shut::Off);
    }

    let mut contents = String::new();

    let _ = fs::File::open(switch)
        .and_then(|file| file.take(MAX_SWITCH + 1).read_to_string(&mut contents))
        .map_err(|error| Shut::Refused(format!("{}: {error}", switch.display())))?;

    if contents.len() as u64 > MAX_SWITCH {
        return Err(Shut::Refused(format!("{} is too long", switch.display())));
    }

    parse_policy(&contents).map_err(|error| Shut::Refused(format!("{}: {error}", switch.display())))
}

/// Reads a switch file's contents: empty is the defaults; anything else must
/// be the JSON the crate docs describe, with no key the door doesn't know.
fn parse_policy(contents: &str) -> Result<Policy, String> {
    if contents.trim().is_empty() {
        return Ok(Policy::default());
    }

    let value: Value =
        serde_json::from_str(contents).map_err(|error| format!("not JSON: {error}"))?;
    let Value::Object(fields) = value else {
        return Err("must be a JSON object".into());
    };

    let mut policy = Policy::default();

    for (key, value) in &fields {
        match key.as_str() {
            "max" => {
                let Value::Object(max) = value else {
                    return Err("\"max\" must be an object".into());
                };

                for (key, value) in max {
                    let number = value
                        .as_u64()
                        .ok_or_else(|| format!("\"max.{key}\" must be a whole number"))?;
                    let millis = Some(Duration::from_millis(number));

                    match key.as_str() {
                        "answer_ms" => policy.max.answer = millis,
                        "settle_ms" => policy.max.settle = millis,
                        "request_ms" => policy.max.request = millis,
                        "idle_client_ms" => policy.max.idle_client = millis,
                        "steps" => {
                            policy.max.drag_steps = Some(u16::try_from(number).unwrap_or(u16::MAX));
                        }
                        "clients" => {
                            policy.max.clients =
                                Some(usize::try_from(number).unwrap_or(usize::MAX));
                        }
                        _ => return Err(format!("unknown key \"max.{key}\"")),
                    }
                }
            }
            "ops" => {
                let Value::Array(names) = value else {
                    return Err("\"ops\" must be a list".into());
                };

                policy.ops = Ops::NONE;

                for name in names {
                    match name.as_str() {
                        Some("read") => policy.ops.read = true,
                        Some("pointer") => policy.ops.pointer = true,
                        Some("keyboard") => policy.ops.keyboard = true,
                        _ => return Err(format!("unknown op kind {name}")),
                    }
                }
            }
            "apps" => {
                let Value::Array(names) = value else {
                    return Err("\"apps\" must be a list".into());
                };

                policy.apps = Some(
                    names
                        .iter()
                        .map(|name| {
                            name.as_str()
                                .map(str::to_owned)
                                .ok_or_else(|| "\"apps\" must list app ids".to_owned())
                        })
                        .collect::<Result<_, _>>()?,
                );
            }
            _ => return Err(format!("unknown key \"{key}\"")),
        }
    }

    Ok(policy)
}

/// Whether `switch` is a file that only root could have put there. (`trusted`
/// is the owners that count as root: just root, except in tests.)
///
/// The path is walked one step at a time, following symlinks by hand. Every
/// folder, symlink and the file itself must belong to root, the file must not
/// be writable by anyone else, and no folder on the way may be writable by
/// anyone else unless it is sticky (as `/nix/store` is), where others cannot
/// replace what root put there. So a NixOS `environment.etc` link into the
/// store counts, and a link to a file a user could recreate does not.
fn switch_is_on(switch: &Path, trusted: &[u32]) -> bool {
    use std::collections::VecDeque;
    use std::ffi::OsString;
    use std::path::Component;

    fn parts(path: &Path) -> impl Iterator<Item = OsString> + '_ {
        path.components().filter_map(|component| match component {
            Component::Normal(name) => Some(name.to_owned()),
            Component::ParentDir => Some(OsString::from("..")),
            Component::RootDir | Component::CurDir | Component::Prefix(_) => None,
        })
    }

    let root_folder = |metadata: &fs::Metadata| {
        metadata.is_dir()
            && trusted.contains(&metadata.uid())
            && (metadata.mode() & 0o022 == 0 || metadata.mode() & 0o1000 != 0)
    };

    if !switch.is_absolute() || !fs::symlink_metadata("/").is_ok_and(|root| root_folder(&root)) {
        return false;
    }

    let mut here = PathBuf::from("/");
    let mut rest: VecDeque<OsString> = parts(switch).collect();
    let mut links = 0;

    while let Some(part) = rest.pop_front() {
        if part == ".." {
            let _ = here.pop();
            continue;
        }

        let next = here.join(&part);
        let Ok(metadata) = fs::symlink_metadata(&next) else {
            return false;
        };

        if !trusted.contains(&metadata.uid()) {
            return false;
        }

        if metadata.file_type().is_symlink() {
            links += 1;

            let Ok(target) = fs::read_link(&next) else {
                return false;
            };

            if links > 40 {
                return false;
            }

            if target.is_absolute() {
                here = PathBuf::from("/");
            }

            for part in parts(&target).collect::<Vec<_>>().into_iter().rev() {
                rest.push_front(part);
            }

            continue;
        }

        if rest.is_empty() {
            return metadata.is_file() && metadata.mode() & 0o022 == 0;
        }

        if !root_folder(&metadata) {
            return false;
        }

        here = next;
    }

    false
}

fn listen(runtime_dir: &Path, app_id: Option<&str>) -> io::Result<(UnixListener, PathBuf)> {
    let user = rustix::process::geteuid().as_raw();

    // Both folders must belong to the user the app runs as. Otherwise someone
    // else could swap them for links while the door sets itself up (say, root
    // running an app with a user's environment).
    let runtime = fs::symlink_metadata(runtime_dir)?;

    if runtime.uid() != user {
        return Err(io::Error::other(format!(
            "{} does not belong to the user the app runs as",
            runtime_dir.display()
        )));
    }

    // As the XDG spec requires: private to the user.
    if runtime.mode() & 0o077 != 0 {
        return Err(io::Error::other(format!(
            "{} is open to other users (mode {:o}); it must be 0700",
            runtime_dir.display(),
            runtime.mode() & 0o777
        )));
    }

    let dir = runtime_dir.join("iced-automation");

    match fs::DirBuilder::new().mode(0o700).create(&dir) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }

    let metadata = fs::symlink_metadata(&dir)?;

    if !metadata.is_dir() || metadata.uid() != user {
        return Err(io::Error::other(format!(
            "{} is not a folder owned by the user",
            dir.display()
        )));
    }

    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    sweep(&dir);

    let name: String = app_id
        .unwrap_or("app")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();

    let suffix = format!("-{}.sock", std::process::id());
    let name = fit_name(&name, &dir, &suffix)?;
    let path = dir.join(format!("{name}{suffix}"));

    match fs::remove_file(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    let listener = UnixListener::bind(&path)?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;

    Ok((listener, path))
}

/// A socket path must fit in `sun_path` (108 bytes, with its NUL). A name too
/// long for what `dir` and `suffix` leave keeps its start, so it can still be
/// found by its app id, and ends in a hash of the whole of it.
fn fit_name(name: &str, dir: &Path, suffix: &str) -> io::Result<String> {
    const SUN_PATH: usize = 107;
    const HASH: usize = 9; // "~" and 8 hex digits

    let room = SUN_PATH
        .checked_sub(dir.as_os_str().len() + 1 + suffix.len())
        .filter(|room| *room > HASH)
        .ok_or_else(|| {
            io::Error::other(format!(
                "{} is too long a path for a socket in it",
                dir.display()
            ))
        })?;

    if name.len() <= room {
        return Ok(name.to_owned());
    }

    // FNV-1a: stable and short; this only tells long names apart.
    let hash = name.bytes().fold(0x811c_9dc5_u32, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
    });

    // `name` is ASCII by now, so any byte is a character boundary.
    Ok(format!("{}~{hash:08x}", &name[..room - HASH]))
}

/// Removes the sockets of apps that are no longer running.
fn sweep(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name
            .to_str()
            .and_then(|name| name.strip_suffix(".sock"))
            .and_then(|stem| stem.rsplit_once('-'))
            .and_then(|(_, pid)| pid.parse::<u32>().ok())
        else {
            continue;
        };

        if !Path::new("/proc").join(pid.to_string()).exists() {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn accept(listener: &UnixListener, shared: &Arc<Shared>) {
    let clients = Arc::new(AtomicUsize::new(0));

    for stream in listener.incoming() {
        if !is_open() {
            if let Ok(stream) = stream {
                refuse(stream, "the app has stopped");
            }
            return;
        }

        let Ok(stream) = stream else {
            continue;
        };

        // The switch is read again for every client: removing it, or
        // narrowing what it allows, takes effect without restarting the app.
        let policy = match read_policy(&shared.switch, &shared.trusted) {
            Ok(policy) if policy.allows(&shared.app_id) => policy,
            Ok(_) => {
                refuse(
                    stream,
                    &format!(
                        "the door is shut: {} doesn't list this app",
                        shared.switch.display()
                    ),
                );
                continue;
            }
            Err(Shut::Off) => {
                refuse(
                    stream,
                    &format!("the door is shut: {} is off", shared.switch.display()),
                );
                continue;
            }
            Err(Shut::Refused(why)) => {
                refuse(stream, &format!("the door is shut: {why}"));
                continue;
            }
        };

        let limits = Limits::resolve(&shared.config, &policy.max);

        if clients.fetch_add(1, Ordering::SeqCst) >= limits.clients {
            let _ = clients.fetch_sub(1, Ordering::SeqCst);
            refuse(stream, "too many clients");
            continue;
        }

        // A client that sends nothing for a while gives its place up, and so
        // does one that stops reading its answers.
        let _ = stream.set_read_timeout(Some(limits.idle_client));
        let _ = stream.set_write_timeout(Some(limits.idle_client));

        let door = Door::new(
            shared.jobs.clone(),
            shared.wake.clone(),
            shared.app_id.clone(),
            stream.try_clone().ok(),
            limits,
            policy.max,
            shared.config.ops.unwrap_or_default().and(policy.ops),
            shared.pointed.clone(),
        );
        let leaving = clients.clone();

        let spawned = thread::Builder::new()
            .name("iced-automation-client".into())
            .spawn(move || {
                door.converse(stream);
                let _ = leaving.fetch_sub(1, Ordering::SeqCst);
            });

        if spawned.is_err() {
            let _ = clients.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

/// Turns a client away with `why`. Its first request is read (briefly) before
/// the socket closes, so a client that writes before it reads gets the reason
/// rather than a broken pipe.
fn refuse(mut stream: UnixStream, why: &str) {
    let _ = writeln!(stream, "{}", json!({ "error": why }));
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
    let _ = (&stream).take(MAX_REQUEST).read_to_end(&mut Vec::new());
}

/// A request handed to the event loop, with where to send the answer.
struct Job {
    ask: Ask,
    answer: mpsc::Sender<Answer>,
    /// The loop drops the request after this, unless it is `cleanup`.
    deadline: Instant,
    /// Lets go of a key, button or modifier that an earlier request pressed.
    /// Always delivered, even late, so nothing stays held down.
    cleanup: bool,
}

/// What the door asks the event loop. More may be added: match with a
/// wildcard, and answer what you don't know with [`Answer::Unsupported`].
#[derive(Debug)]
#[non_exhaustive]
pub enum Ask {
    /// Describe every surface: answer [`Answer::Info`].
    Info,
    /// Read every surface's widget tree with a [`Collector`]: answer
    /// [`Answer::Tree`].
    Tree,
    /// Whether nothing is pending and no frame is due: answer [`Answer::Idle`].
    Idle,
    /// Which window has keyboard focus (or else the first): answer
    /// [`Answer::Focused`].
    Focused,
    /// Put `events` into the pending events of surface `window`, after placing
    /// its cursor at `cursor`, if given: answer [`Answer::Injected`], `false`
    /// if there is no such surface.
    ///
    /// First, if `leave` names a surface, the door's pointer leaves it: forget
    /// the cursor placed there and give it a `CursorLeft`, so hover styles and
    /// tooltips go, as when a person's pointer moves off.
    #[non_exhaustive]
    Inject {
        /// The window or popup the events are for.
        window: window::Id,
        /// Where its cursor goes, in surface-relative logical pixels.
        cursor: Option<Point>,
        /// The events, in order.
        events: Vec<Event>,
        /// The surface the door's pointer leaves first, if any.
        leave: Option<window::Id>,
    },
}

/// The event loop's answer to an [`Ask`].
#[derive(Debug)]
#[non_exhaustive]
pub enum Answer {
    /// Every window and popup.
    Info(Vec<Surface>),
    /// What a [`Collector`] found on every surface.
    Tree(Vec<Node>),
    /// Whether the app is idle.
    Idle(bool),
    /// The window keys go to.
    Focused(Option<window::Id>),
    /// Whether the surface was found and the events queued.
    Injected(bool),
    /// The loop doesn't know this [`Ask`] (it was added after the loop was
    /// written).
    Unsupported,
}

/// A window or popup, as [`Answer::Info`] lists it. Made with
/// [`Surface::new`]; it may gain fields.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Surface {
    /// Its id.
    pub window: window::Id,
    /// Whether it is a popup.
    pub popup: bool,
    /// Its logical size.
    pub size: Size,
    /// Its scale factor.
    pub scale_factor: f32,
    /// Whether it has keyboard focus.
    pub focused: bool,
}

impl Surface {
    /// A surface, as [`Answer::Info`] lists it.
    pub fn new(
        window: window::Id,
        popup: bool,
        size: Size,
        scale_factor: f32,
        focused: bool,
    ) -> Self {
        Self {
            window,
            popup,
            size,
            scale_factor,
            focused,
        }
    }
}

/// Something a [`Collector`] found in a widget tree.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    window: window::Id,
    popup: bool,
    kind: &'static str,
    id: Option<String>,
    text: Option<String>,
    bounds: Rectangle,
    visible: Option<Rectangle>,
    focused: bool,
}

/// One client's side of the door, run on its own thread.
struct Door {
    jobs: mpsc::Sender<Job>,
    wake: Wake,
    app_id: String,
    /// The client's socket, to notice when it hangs up mid-request.
    peer: Option<UnixStream>,
    /// The limits in force for this client.
    limits: Limits,
    /// The administrator's caps, which a request's own numbers can't exceed.
    max: Max,
    /// The kinds of request this client may make.
    ops: Ops,
    /// When the request being handled must be done by.
    deadline: Cell<Option<Instant>>,
    /// How long the request being handled waits for the app to settle.
    settle: Cell<Duration>,
    /// How many moves the drag being handled is split into.
    steps: Cell<u16>,
    /// The surface this client last pressed a button on, with the one the app
    /// said had keyboard focus at that moment. Keys go to the pressed surface,
    /// as they would after a person's click, until the app's focus moves.
    /// Each client keeps its own, so one client's click never redirects
    /// another's keys.
    last_press: Cell<Option<(window::Id, Option<window::Id>)>>,
    /// The surface the door's pointer is on, so it can leave it when it
    /// moves to another (or on `leave`). The app has one pointer, so this is
    /// shared by every client of the door.
    pointed: Arc<Mutex<Option<window::Id>>>,
}

impl Door {
    fn new(
        jobs: mpsc::Sender<Job>,
        wake: Wake,
        app_id: String,
        peer: Option<UnixStream>,
        limits: Limits,
        max: Max,
        ops: Ops,
        pointed: Arc<Mutex<Option<window::Id>>>,
    ) -> Self {
        Self {
            jobs,
            wake,
            app_id,
            peer,
            limits,
            max,
            ops,
            deadline: Cell::new(None),
            settle: Cell::new(limits.settle),
            steps: Cell::new(limits.drag_steps),
            last_press: Cell::new(None),
            pointed,
        }
    }

    fn converse(&self, stream: UnixStream) {
        let Ok(mut writer) = stream.try_clone() else {
            return;
        };
        let mut reader = BufReader::new(stream);

        loop {
            let mut line = String::new();

            match (&mut reader).take(MAX_REQUEST).read_line(&mut line) {
                Ok(0) => return,
                Ok(_) => {}
                Err(error) => {
                    match error.kind() {
                        io::ErrorKind::InvalidData => {
                            let _ =
                                writeln!(writer, "{}", json!({ "error": "request is not UTF-8" }));
                        }
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => {
                            let _ = writeln!(
                                writer,
                                "{}",
                                json!({ "error": format!(
                                    "closed: nothing sent for {} ms",
                                    self.limits.idle_client.as_millis()
                                ) })
                            );
                        }
                        _ => {}
                    }

                    return;
                }
            }

            if !line.ends_with('\n') && line.len() as u64 >= MAX_REQUEST {
                let _ = writeln!(writer, "{}", json!({ "error": "request too long" }));
                return;
            }

            if line.trim().is_empty() {
                continue;
            }

            let reply = match serde_json::from_str::<Value>(&line) {
                Ok(request) => self
                    .handle(&request)
                    .unwrap_or_else(|error| json!({ "error": error })),
                Err(error) => json!({ "error": format!("not JSON: {error}") }),
            };

            if writeln!(writer, "{reply}").is_err() {
                return;
            }
        }
    }

    /// A request's own number under `key`, clamped to the administrator's
    /// cap (`admin`) and the door's bounds; `default` when it gives none.
    fn requested<T: Ord + Copy>(
        request: &Value,
        key: &str,
        read: impl Fn(u64) -> T,
        default: T,
        min: T,
        admin: Option<T>,
        max: T,
    ) -> Result<T, String> {
        let wanted = match &request[key] {
            Value::Null => None,
            value => {
                Some(read(value.as_u64().ok_or_else(|| {
                    format!("\"{key}\" must be a whole number")
                })?))
            }
        };

        Ok(clamp(wanted, default, min, admin, max))
    }

    fn handle(&self, request: &Value) -> Result<Value, String> {
        let op = request["op"].as_str().ok_or("missing \"op\"")?;

        let allowed = match op {
            "info" | "tree" | "idle" => self.ops.read,
            "click" | "hover" | "scroll" | "drag" | "leave" => self.ops.pointer,
            "type" | "key" => self.ops.keyboard,
            _ => return Err(format!("unknown op {op:?}")),
        };

        if !allowed {
            return Err(format!(
                "\"{op}\" isn't allowed for this app on this device"
            ));
        }

        let millis = Duration::from_millis;
        let request_timeout = Self::requested(
            request,
            "timeout_ms",
            millis,
            self.limits.request,
            HARD_MIN.request,
            self.max.request,
            HARD_MAX.request,
        )?;

        self.settle.set(Self::requested(
            request,
            "settle_ms",
            millis,
            self.limits.settle,
            HARD_MIN.settle,
            self.max.settle,
            HARD_MAX.settle,
        )?);
        self.steps.set(Self::requested(
            request,
            "steps",
            |steps| u16::try_from(steps).unwrap_or(u16::MAX),
            self.limits.drag_steps,
            HARD_MIN.drag_steps,
            self.max.drag_steps,
            HARD_MAX.drag_steps,
        )?);
        self.deadline.set(Some(Instant::now() + request_timeout));

        match op {
            "info" => self.info(),
            "tree" => self
                .tree()
                .map(|nodes| Value::Array(nodes.iter().map(node_json).collect())),
            "idle" => self.idle().map(|idle| json!({ "idle": idle })),
            "click" | "hover" => {
                let (window, point) = self.locate(request)?;

                self.pointer(window, point, op == "click")?;

                Ok(json!({ "ok": true, "window": window.to_string(), "x": point.x, "y": point.y }))
            }
            "scroll" => {
                let (window, point) = self.locate(request)?;
                let lines = |key: &str| {
                    #[allow(clippy::cast_possible_truncation)]
                    request[key].as_f64().map_or(0.0, |lines| lines as f32)
                };
                let delta = mouse::ScrollDelta::Lines {
                    x: lines("dx"),
                    y: lines("dy"),
                };

                self.pointer(window, point, false)?;
                self.inject(
                    window,
                    Some(point),
                    vec![Event::Mouse(mouse::Event::WheelScrolled { delta })],
                )?;

                Ok(json!({ "ok": true, "window": window.to_string(), "x": point.x, "y": point.y }))
            }
            "leave" => {
                let window = match self.named_window(request)? {
                    Some(window) => Some(window),
                    None => self.pointed(),
                };

                if let Some(window) = window {
                    // Leaving another surface than the one pointed at keeps
                    // that one, so it still gets its own leave later.
                    if self.pointed() == Some(window) {
                        self.point(None);
                    }

                    // A surface that has gone has nothing to leave.
                    let _ = self.send(
                        Ask::Inject {
                            window,
                            cursor: None,
                            events: Vec::new(),
                            leave: Some(window),
                        },
                        true,
                    )?;
                    self.settle();
                }

                Ok(json!({ "ok": true, "left": window.map(|window| window.to_string()) }))
            }
            "drag" => {
                let (window, from) = self.locate(request)?;

                let to = match &request["to"] {
                    Value::Object(target) => {
                        if request.get("dx").is_some() || request.get("dy").is_some() {
                            return Err("give \"to\", or \"dx\"/\"dy\", not both".into());
                        }

                        // A target without a window of its own is in the drag's.
                        let mut target = target.clone();
                        let _ = target
                            .entry("window")
                            .or_insert_with(|| Value::String(window.to_string()));

                        let (to_window, to) = self
                            .locate(&Value::Object(target))
                            .map_err(|error| format!("\"to\": {error}"))?;

                        if to_window != window {
                            return Err("a drag must start and end in the same window".into());
                        }

                        to
                    }
                    Value::Null => {
                        #[allow(clippy::cast_possible_truncation)]
                        let offset = |key: &str| request[key].as_f64().map(|value| value as f32);

                        match (offset("dx"), offset("dy")) {
                            (None, None) => {
                                return Err(
                                    "\"drag\" needs \"to\", or \"dx\" and \"dy\" in pixels".into(),
                                );
                            }
                            (dx, dy) => {
                                Point::new(from.x + dx.unwrap_or(0.0), from.y + dy.unwrap_or(0.0))
                            }
                        }
                    }
                    _ => return Err("\"to\" must be an object, like {\"text\":\"…\"}".into()),
                };

                self.drag(window, from, to)?;

                Ok(json!({
                    "ok": true,
                    "window": window.to_string(),
                    "from": { "x": from.x, "y": from.y },
                    "to": { "x": to.x, "y": to.y },
                }))
            }
            "type" => {
                let text = request["text"].as_str().ok_or("\"type\" needs \"text\"")?;
                let window = self.keyboard_window(request)?;

                // One batch of presses and releases, settled once: as fast as
                // the app takes it, and nothing is left held down, whatever
                // happens to the request.
                let physical_key = key::Physical::Unidentified(key::NativeCode::Unidentified);
                let events = text
                    .chars()
                    .flat_map(|c| {
                        let (key, text) = match c {
                            ' ' => (keyboard::Key::Named(key::Named::Space), " ".to_owned()),
                            '\n' | '\r' => {
                                (keyboard::Key::Named(key::Named::Enter), "\r".to_owned())
                            }
                            '\t' => (keyboard::Key::Named(key::Named::Tab), "\t".to_owned()),
                            c => (
                                keyboard::Key::Character(SmolStr::new(c.to_string())),
                                c.to_string(),
                            ),
                        };

                        [
                            Event::Keyboard(keyboard::Event::KeyPressed {
                                key: key.clone(),
                                modified_key: key.clone(),
                                physical_key,
                                location: keyboard::Location::Standard,
                                modifiers: keyboard::Modifiers::empty(),
                                text: Some(SmolStr::new(text)),
                                repeat: false,
                            }),
                            Event::Keyboard(keyboard::Event::KeyReleased {
                                key: key.clone(),
                                modified_key: key,
                                physical_key,
                                location: keyboard::Location::Standard,
                                modifiers: keyboard::Modifiers::empty(),
                            }),
                        ]
                    })
                    .collect();

                self.inject(window, None, events)?;

                Ok(json!({ "ok": true }))
            }
            "key" => {
                let name = request["key"].as_str().ok_or("\"key\" needs \"key\"")?;
                let modifiers = modifiers(&request["modifiers"])?;
                let key = parse_key(name)?;
                let window = self.keyboard_window(request)?;

                let text = match &key {
                    keyboard::Key::Character(c)
                        if !modifiers.intersects(
                            keyboard::Modifiers::CTRL
                                | keyboard::Modifiers::ALT
                                | keyboard::Modifiers::LOGO,
                        ) =>
                    {
                        Some(if modifiers.shift() {
                            c.to_uppercase()
                        } else {
                            c.to_string()
                        })
                    }
                    keyboard::Key::Named(key::Named::Enter) => Some("\r".to_owned()),
                    keyboard::Key::Named(key::Named::Space) => Some(" ".to_owned()),
                    keyboard::Key::Named(key::Named::Tab) => Some("\t".to_owned()),
                    _ => None,
                };

                self.keys(window, key, modifiers, text)?;

                Ok(json!({ "ok": true }))
            }
            _ => Err(format!("unknown op {op:?}")),
        }
    }

    fn ask(&self, ask: Ask) -> Result<Answer, String> {
        self.send(ask, false)
    }

    fn send(&self, ask: Ask, cleanup: bool) -> Result<Answer, String> {
        // A request the client can no longer get an answer to stops here; only
        // letting go of what an earlier step pressed still goes in.
        if !cleanup {
            if self
                .deadline
                .get()
                .is_some_and(|deadline| Instant::now() > deadline)
            {
                return Err("stopped: the request took longer than it was allowed".into());
            }

            if self.client_gone() {
                return Err("stopped: the client hung up".into());
            }
        }

        let (answer, answered) = mpsc::channel();

        self.jobs
            .send(Job {
                ask,
                answer,
                deadline: Instant::now() + self.limits.answer.saturating_sub(ANSWER_MARGIN),
                cleanup,
            })
            .map_err(|_| "the app has stopped")?;

        (self.wake)();

        match answered.recv_timeout(self.limits.answer) {
            Ok(Answer::Unsupported) => Err("the app's event loop doesn't support this".into()),
            Ok(answer) => Ok(answer),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err("the app has stopped".into()),
            Err(mpsc::RecvTimeoutError::Timeout) => Err("the app did not answer in time".into()),
        }
    }

    /// Whether the client has hung up, without reading anything it sent.
    ///
    /// A client that has only shut its writing side (a one-shot `socat`, say)
    /// is still there to read the answer; only a full close counts.
    fn client_gone(&self) -> bool {
        use rustix::event::{PollFd, PollFlags, poll};

        let Some(peer) = &self.peer else {
            return false;
        };

        let mut fds = [PollFd::new(peer, PollFlags::empty())];

        poll(&mut fds, 0).is_ok() && fds[0].revents().intersects(PollFlags::HUP | PollFlags::ERR)
    }

    fn info(&self) -> Result<Value, String> {
        let Answer::Info(surfaces) = self.ask(Ask::Info)? else {
            return Err("unexpected answer".into());
        };

        Ok(json!({
            "app_id": self.app_id,
            "pid": std::process::id(),
            "windows": surfaces
                .iter()
                .map(|surface| json!({
                    "window": surface.window.to_string(),
                    "popup": surface.popup,
                    "width": surface.size.width,
                    "height": surface.size.height,
                    "scale_factor": surface.scale_factor,
                    "focused": surface.focused,
                }))
                .collect::<Vec<_>>(),
        }))
    }

    fn tree(&self) -> Result<Vec<Node>, String> {
        match self.ask(Ask::Tree)? {
            Answer::Tree(nodes) => Ok(nodes),
            _ => Err("unexpected answer".into()),
        }
    }

    fn idle(&self) -> Result<bool, String> {
        match self.ask(Ask::Idle)? {
            Answer::Idle(idle) => Ok(idle),
            _ => Err("unexpected answer".into()),
        }
    }

    /// Waits a moment for the app to take in what it was just sent, up to the
    /// request's settle time (none at all for `"settle_ms": 0`). Best effort:
    /// a slow app only makes it give up waiting, never stops what follows.
    fn settle(&self) {
        let limit = self.settle.get();

        if limit.is_zero() {
            return;
        }

        let started = Instant::now();

        thread::sleep(POLL.min(limit));

        while started.elapsed() < limit && !self.idle().unwrap_or(true) {
            thread::sleep(POLL);
        }
    }

    fn inject(
        &self,
        window: window::Id,
        cursor: Option<Point>,
        events: Vec<Event>,
    ) -> Result<(), String> {
        self.deliver(window, cursor, events, false)
    }

    /// Like [`Self::inject`], for letting go of something an earlier step pressed.
    fn release(
        &self,
        window: window::Id,
        cursor: Option<Point>,
        events: Vec<Event>,
    ) -> Result<(), String> {
        self.deliver(window, cursor, events, true)
    }

    fn deliver(
        &self,
        window: window::Id,
        cursor: Option<Point>,
        events: Vec<Event>,
        cleanup: bool,
    ) -> Result<(), String> {
        // The pointer moving to another surface leaves the one it was on.
        let leave = if cursor.is_some() {
            self.pointed().filter(|pointed| *pointed != window)
        } else {
            None
        };

        let answer = self.send(
            Ask::Inject {
                window,
                cursor,
                events,
                leave,
            },
            cleanup,
        )?;

        match answer {
            Answer::Injected(true) => {
                if cursor.is_some() {
                    self.point(Some(window));
                }

                self.settle();
                Ok(())
            }
            Answer::Injected(false) => Err(format!("window {window} is gone")),
            _ => Err("unexpected answer".into()),
        }
    }

    /// Moves the pointer to `point` and, for a click, presses and releases the
    /// left button there. A press that went in is always followed by its release.
    fn pointer(&self, window: window::Id, point: Point, click: bool) -> Result<(), String> {
        let moved = Event::Mouse(mouse::Event::CursorMoved { position: point });

        self.inject(window, Some(point), vec![moved])?;

        if click {
            let left = mouse::Button::Left;

            self.inject(
                window,
                Some(point),
                vec![Event::Mouse(mouse::Event::ButtonPressed(left))],
            )?;
            self.pressed(window);
            self.release(
                window,
                Some(point),
                vec![Event::Mouse(mouse::Event::ButtonReleased(left))],
            )?;
        }

        Ok(())
    }

    /// Presses the left button at `from`, moves to `to` in steps, and releases
    /// it there. A press that went in is always released, where the pointer got to.
    fn drag(&self, window: window::Id, from: Point, to: Point) -> Result<(), String> {
        let left = mouse::Button::Left;
        let moved = |position| Event::Mouse(mouse::Event::CursorMoved { position });
        let steps = self.steps.get().max(1);

        self.inject(window, Some(from), vec![moved(from)])?;
        self.inject(
            window,
            Some(from),
            vec![Event::Mouse(mouse::Event::ButtonPressed(left))],
        )?;
        self.pressed(window);

        let mut at = from;
        let mut travelled = Ok(());

        for step in 1..=steps {
            let progress = f32::from(step) / f32::from(steps);
            let point = if step == steps {
                to
            } else {
                Point::new(
                    from.x + (to.x - from.x) * progress,
                    from.y + (to.y - from.y) * progress,
                )
            };

            if let Err(error) = self.inject(window, Some(point), vec![moved(point)]) {
                travelled = Err(error);
                break;
            }

            at = point;
        }

        let released = self.release(
            window,
            Some(at),
            vec![Event::Mouse(mouse::Event::ButtonReleased(left))],
        );

        travelled.and(released)
    }

    fn keys(
        &self,
        window: window::Id,
        key: keyboard::Key,
        modifiers: keyboard::Modifiers,
        text: Option<String>,
    ) -> Result<(), String> {
        let physical_key = key::Physical::Unidentified(key::NativeCode::Unidentified);

        if !modifiers.is_empty() {
            self.inject(
                window,
                None,
                vec![Event::Keyboard(keyboard::Event::ModifiersChanged(
                    modifiers,
                ))],
            )?;
        }

        // Once the modifiers are down, they are let go of whatever happens next.
        let pressed = self
            .inject(
                window,
                None,
                vec![Event::Keyboard(keyboard::Event::KeyPressed {
                    key: key.clone(),
                    modified_key: key.clone(),
                    physical_key,
                    location: keyboard::Location::Standard,
                    modifiers,
                    text: text.map(SmolStr::new),
                    repeat: false,
                })],
            )
            .and_then(|()| {
                self.release(
                    window,
                    None,
                    vec![Event::Keyboard(keyboard::Event::KeyReleased {
                        key: key.clone(),
                        modified_key: key,
                        physical_key,
                        location: keyboard::Location::Standard,
                        modifiers,
                    })],
                )
            });

        let lifted = if modifiers.is_empty() {
            Ok(())
        } else {
            self.release(
                window,
                None,
                vec![Event::Keyboard(keyboard::Event::ModifiersChanged(
                    keyboard::Modifiers::empty(),
                ))],
            )
        };

        pressed.and(lifted)
    }

    /// The surface the door's pointer is on.
    fn pointed(&self) -> Option<window::Id> {
        self.pointed.lock().ok().and_then(|pointed| *pointed)
    }

    fn point(&self, window: Option<window::Id>) {
        if let Ok(mut pointed) = self.pointed.lock() {
            *pointed = window;
        }
    }

    /// Remembers that this client pressed a button on `window`, for `last_press`.
    fn pressed(&self, window: window::Id) {
        let focused = self.focused().ok().flatten();

        self.last_press.set(Some((window, focused)));
    }

    fn focused(&self) -> Result<Option<window::Id>, String> {
        match self.ask(Ask::Focused)? {
            Answer::Focused(window) => Ok(window),
            _ => Err("unexpected answer".into()),
        }
    }

    /// The window named in the request; or else the one this client last
    /// clicked, unless the app's keyboard focus has moved since; or else the
    /// one with keyboard focus.
    fn keyboard_window(&self, request: &Value) -> Result<window::Id, String> {
        if let Some(window) = self.named_window(request)? {
            return Ok(window);
        }

        let focused = self.focused()?;

        if let Some((pressed, focused_then)) = self.last_press.get()
            && focused_then == focused
        {
            let Answer::Info(surfaces) = self.ask(Ask::Info)? else {
                return Err("unexpected answer".into());
            };

            if surfaces.iter().any(|surface| surface.window == pressed) {
                return Ok(pressed);
            }
        }

        focused.ok_or_else(|| "the app has no window".into())
    }

    fn named_window(&self, request: &Value) -> Result<Option<window::Id>, String> {
        let Some(wanted) = window_name(&request["window"]) else {
            return Ok(None);
        };

        let Answer::Info(surfaces) = self.ask(Ask::Info)? else {
            return Err("unexpected answer".into());
        };

        surfaces
            .iter()
            .map(|surface| surface.window)
            .find(|window| window.to_string() == wanted)
            .map(Some)
            .ok_or_else(|| format!("no window {wanted}"))
    }

    /// Where a pointer request should land: on a widget, or at a point.
    fn locate(&self, request: &Value) -> Result<(window::Id, Point), String> {
        if let (Some(x), Some(y)) = (request["x"].as_f64(), request["y"].as_f64()) {
            let window = self.keyboard_window(request)?;

            #[allow(clippy::cast_possible_truncation)]
            return Ok((window, Point::new(x as f32, y as f32)));
        }

        let window = window_name(&request["window"]);
        let nodes = self.tree()?;
        let (wanted, matches): (String, Vec<&Node>) = if let Some(id) = request["id"].as_str() {
            (
                format!("id {id:?}"),
                nodes
                    .iter()
                    .filter(|node| node.id.as_deref() == Some(id))
                    .collect(),
            )
        } else if let Some(text) = request["text"].as_str() {
            (
                format!("text {text:?}"),
                nodes
                    .iter()
                    .filter(|node| node.text.as_deref().map(str::trim) == Some(text.trim()))
                    .collect(),
            )
        } else {
            return Err("give \"id\", \"text\", or \"x\" and \"y\"".into());
        };

        let mut visible: Vec<(&Node, Rectangle)> = matches
            .iter()
            .filter(|node| {
                window
                    .as_deref()
                    .is_none_or(|window| node.window.to_string() == window)
            })
            .filter_map(|node| {
                node.visible
                    .filter(|visible| visible.width >= 1.0 && visible.height >= 1.0)
                    .map(|visible| (*node, visible))
            })
            .collect();

        // One widget can report itself more than once (a text input is also a
        // focusable; a `test_id` wraps a widget with the same bounds). Matches in
        // the same place on the same surface are the same thing.
        let mut seen = Vec::new();
        visible.retain(|(node, _)| {
            let place = (node.window, node.popup, node.bounds);
            let new = !seen.contains(&place);
            seen.push(place);
            new
        });

        let nth = request["nth"].as_u64();

        let (node, visible) = match (visible.as_slice(), nth) {
            ([], _) if matches.is_empty() => return Err(format!("nothing has {wanted}")),
            ([], _) => return Err(format!("{wanted} is not visible")),
            ([one], None) => *one,
            (many, None) => {
                return Err(format!(
                    "{} visible widgets have {wanted}; add \"nth\" (0-based) or \"window\"",
                    many.len()
                ));
            }
            (many, Some(n)) => *usize::try_from(n)
                .ok()
                .and_then(|n| many.get(n))
                .ok_or_else(|| format!("only {} visible widgets have {wanted}", many.len()))?,
        };

        Ok((node.window, visible.center()))
    }
}

fn window_name(value: &Value) -> Option<String> {
    match value {
        Value::String(name) => Some(name.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn modifiers(value: &Value) -> Result<keyboard::Modifiers, String> {
    let mut modifiers = keyboard::Modifiers::empty();

    let Some(names) = value.as_array() else {
        return if value.is_null() {
            Ok(modifiers)
        } else {
            Err("\"modifiers\" must be a list".into())
        };
    };

    for name in names {
        modifiers |= match name.as_str().map(str::to_ascii_lowercase).as_deref() {
            Some("ctrl" | "control") => keyboard::Modifiers::CTRL,
            Some("shift") => keyboard::Modifiers::SHIFT,
            Some("alt") => keyboard::Modifiers::ALT,
            Some("logo" | "super" | "meta") => keyboard::Modifiers::LOGO,
            _ => return Err(format!("unknown modifier {name}")),
        };
    }

    Ok(modifiers)
}

fn parse_key(name: &str) -> Result<keyboard::Key, String> {
    use key::Named;

    let named = match name {
        "Enter" | "Return" => Named::Enter,
        "Escape" | "Esc" => Named::Escape,
        "Tab" => Named::Tab,
        "Space" => Named::Space,
        "Backspace" => Named::Backspace,
        "Delete" => Named::Delete,
        "Insert" => Named::Insert,
        "Home" => Named::Home,
        "End" => Named::End,
        "PageUp" => Named::PageUp,
        "PageDown" => Named::PageDown,
        "ArrowUp" | "Up" => Named::ArrowUp,
        "ArrowDown" | "Down" => Named::ArrowDown,
        "ArrowLeft" | "Left" => Named::ArrowLeft,
        "ArrowRight" | "Right" => Named::ArrowRight,
        "F1" => Named::F1,
        "F2" => Named::F2,
        "F3" => Named::F3,
        "F4" => Named::F4,
        "F5" => Named::F5,
        "F6" => Named::F6,
        "F7" => Named::F7,
        "F8" => Named::F8,
        "F9" => Named::F9,
        "F10" => Named::F10,
        "F11" => Named::F11,
        "F12" => Named::F12,
        _ => {
            let mut chars = name.chars();

            return match (chars.next(), chars.next()) {
                (Some(c), None) => Ok(keyboard::Key::Character(SmolStr::new(
                    c.to_lowercase().to_string(),
                ))),
                _ => Err(format!("unknown key {name:?}")),
            };
        }
    };

    Ok(keyboard::Key::Named(named))
}

fn node_json(node: &Node) -> Value {
    let rectangle = |r: Rectangle| json!({ "x": r.x, "y": r.y, "w": r.width, "h": r.height });

    json!({
        "window": node.window.to_string(),
        "popup": node.popup,
        "kind": node.kind,
        "id": node.id,
        "text": node.text,
        "bounds": rectangle(node.bounds),
        "visible": node.visible.map(rectangle),
        "focused": node.focused,
    })
}

/// Walks a widget tree and keeps what a test driver can find things by: named
/// containers, texts, text inputs, focusables and scrollables, with their
/// surface-relative bounds and the part of them that is visible.
///
/// Run it over a surface with `UserInterface::operate`, then hand
/// [`Collector::into_nodes`] back in [`Answer::Tree`].
///
/// A widget that paints its children moved from their layout can say so: it
/// reports a [`PaintedOffset`] through `custom`, right before walking into
/// them, and they are then placed where it paints them. A widget that keeps
/// children it doesn't show reports [`Hidden`] the same way, and none of them
/// counts as visible.
#[derive(Debug)]
pub struct Collector {
    window: window::Id,
    popup: bool,
    nodes: Vec<Node>,
    /// The viewport, translation and hiddenness to go back to after a walk.
    stack: Vec<(Rectangle, Vector, bool)>,
    viewport: Rectangle,
    translation: Vector,
    /// Whether the walk is inside something kept but not shown.
    hidden: bool,
    /// A painted offset reported through `custom`, for the next `traverse`.
    shift: Option<Vector>,
    /// Whether the next `traverse` walks into something not shown.
    hide: bool,
}

impl Collector {
    /// A collector for a surface of the given logical `size`. Nothing outside
    /// it counts as visible.
    pub fn new(window: window::Id, popup: bool, size: Size) -> Self {
        let viewport = Rectangle::new(Point::ORIGIN, size);

        Self {
            window,
            popup,
            nodes: Vec::new(),
            stack: vec![(viewport, Vector::ZERO, false)],
            viewport,
            translation: Vector::ZERO,
            hidden: false,
            shift: None,
            hide: false,
        }
    }

    /// What was found, in tree order.
    pub fn into_nodes(self) -> Vec<Node> {
        self.nodes
    }

    fn push(
        &mut self,
        kind: &'static str,
        id: Option<&Id>,
        text: Option<&str>,
        bounds: Rectangle,
        focused: bool,
    ) {
        // An offset or a hiding applies to the walk right after it, not to
        // siblings.
        self.shift = None;
        self.hide = false;
        let bounds = bounds + self.translation;

        self.nodes.push(Node {
            window: self.window,
            popup: self.popup,
            kind,
            id: id.and_then(Id::as_str).map(str::to_owned),
            text: text.map(str::to_owned),
            bounds,
            visible: if self.hidden {
                None
            } else {
                self.viewport.intersection(&bounds)
            },
            focused,
        });
    }
}

impl Operation for Collector {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        self.stack
            .push((self.viewport, self.translation, self.hidden));
        if let Some(shift) = self.shift.take() {
            self.translation += shift;
        }
        if std::mem::take(&mut self.hide) {
            self.hidden = true;
        }
        operate(self);
        let _ = self.stack.pop();

        if let Some((viewport, translation, hidden)) = self.stack.last() {
            self.viewport = *viewport;
            self.translation = *translation;
            self.hidden = *hidden;
        }
    }

    fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
        if id.and_then(Id::as_str).is_some() {
            self.push("named", id, None, bounds, false);
        }
    }

    fn focusable(&mut self, id: Option<&Id>, bounds: Rectangle, state: &mut dyn Focusable) {
        let focused = state.is_focused();

        // A text input reports itself, then its focus, in the same place: the
        // input is as focused as its focusable.
        if let Some(input) = self.nodes.last_mut()
            && input.kind == "text_input"
            && input.bounds == bounds + self.translation
        {
            input.focused = focused;
        }

        self.push("focusable", id, None, bounds, focused);
    }

    fn scrollable(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        _content_bounds: Rectangle,
        translation: Vector,
        _state: &mut dyn Scrollable,
    ) {
        self.push("scrollable", id, None, bounds, false);

        let visible = self.viewport.intersection(&(bounds + self.translation));

        self.translation -= translation;
        self.viewport = visible.unwrap_or_default();
    }

    fn text_input(&mut self, id: Option<&Id>, bounds: Rectangle, state: &mut dyn TextInput) {
        self.push("text_input", id, Some(state.text()), bounds, false);
    }

    fn text(&mut self, id: Option<&Id>, bounds: Rectangle, text: &str) {
        self.push("text", id, Some(text), bounds, false);
    }

    fn custom(&mut self, _id: Option<&Id>, _bounds: Rectangle, state: &mut dyn std::any::Any) {
        if let Some(PaintedOffset(offset)) = state.downcast_ref::<PaintedOffset>() {
            self.shift = Some(*offset);
        } else if state.is::<Hidden>() {
            self.hide = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::core::widget::operation::scrollable::{AbsoluteOffset, RelativeOffset};

    /// A scratch folder with a short path (sockets must fit in 108 bytes, and
    /// a nested nix-shell's `TMPDIR` can be 140 characters long on its own).
    fn scratch(name: &str) -> PathBuf {
        let base = if Path::new("/tmp").is_dir() {
            PathBuf::from("/tmp")
        } else {
            std::env::temp_dir()
        };
        let dir = base.join(format!("ia-{name}-{}", std::process::id()));

        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create scratch folder");
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).expect("chmod");

        dir
    }

    /// A private runtime folder in `dir`, as `XDG_RUNTIME_DIR` must be.
    fn runtime(dir: &Path) -> PathBuf {
        let runtime = dir.join("run");
        fs::create_dir_all(&runtime).expect("create runtime folder");
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).expect("chmod");

        runtime
    }

    /// The owners a switch may have in these tests: root, and whoever owns
    /// what a test can't create itself (this user; the owner of `/`, which is
    /// not root in a Nix sandbox; the owner of the scratch base). So a switch a
    /// test makes can be accepted, and every other rule still checked.
    fn trusted() -> Vec<u32> {
        let owner = |path: &str| fs::metadata(path).map_or(0, |metadata| metadata.uid());

        vec![
            0,
            rustix::process::geteuid().as_raw(),
            owner("/"),
            owner("/tmp"),
        ]
    }

    fn noop() -> Wake {
        Arc::new(|| {})
    }

    /// Tests that open a real door take turns: there is one per process.
    static REAL_DOOR: Mutex<()> = Mutex::new(());

    fn real_door() -> std::sync::MutexGuard<'static, ()> {
        REAL_DOOR
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[test]
    fn stays_shut_without_the_switch() {
        let _turn = real_door();
        let dir = scratch("no-switch");
        let runtime = runtime(&dir);

        let opened = open_door(
            Config::new().switch(dir.join("automation-enabled")),
            &trusted(),
            Some(runtime.clone()),
            Some("test"),
            noop(),
        );

        assert!(opened.is_none());
        assert!(
            !runtime.join("iced-automation").exists(),
            "a shut door must leave nothing behind"
        );
        assert!(!is_open());
    }

    #[test]
    fn a_switch_others_can_write_does_not_count() {
        let dir = scratch("writable");
        let switch = dir.join("automation-enabled");
        fs::write(&switch, "").expect("create switch");

        fs::set_permissions(&switch, fs::Permissions::from_mode(0o644)).expect("chmod");
        assert!(
            switch_is_on(&switch, &trusted()),
            "the same file, not writable, counts"
        );

        fs::set_permissions(&switch, fs::Permissions::from_mode(0o666)).expect("chmod");
        assert!(!switch_is_on(&switch, &trusted()));

        fs::set_permissions(&switch, fs::Permissions::from_mode(0o664)).expect("chmod");
        assert!(!switch_is_on(&switch, &trusted()), "group-writable");

        fs::set_permissions(&switch, fs::Permissions::from_mode(0o644)).expect("chmod");
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o775)).expect("chmod");
        assert!(
            !switch_is_on(&switch, &trusted()),
            "in a folder others can write"
        );
    }

    #[test]
    fn a_folder_is_not_a_switch() {
        let dir = scratch("folder");
        let switch = dir.join("automation-enabled");
        fs::create_dir_all(&switch).expect("create folder");
        fs::set_permissions(&switch, fs::Permissions::from_mode(0o755)).expect("chmod");

        assert!(!switch_is_on(&switch, &trusted()));
    }

    #[test]
    fn only_the_trusted_owner_can_turn_it_on() {
        let dir = scratch("owner");
        let switch = dir.join("automation-enabled");
        fs::write(&switch, "").expect("create switch");
        fs::set_permissions(&switch, fs::Permissions::from_mode(0o644)).expect("chmod");

        assert!(switch_is_on(&switch, &trusted()));

        // As it runs for real, trusting only root: a file this user made
        // doesn't count (unless the tests run as root).
        let made_by_root = fs::metadata(&switch).expect("metadata").uid() == 0;
        let tree_is_roots = fs::metadata("/").expect("/").uid() == 0;

        assert_eq!(switch_is_on(&switch, &[0]), made_by_root && tree_is_roots);
    }

    #[test]
    fn a_nixos_etc_link_into_the_store_counts() {
        // /etc/x -> /etc/static/x, /etc/static -> /nix/store/…-etc/etc, with
        // the store sticky, as NixOS's `environment.etc` lays it out.
        let dir = scratch("nixos");
        let store = dir.join("store");
        let generation = store.join("abc-etc").join("etc");
        fs::create_dir_all(&generation).expect("create store");
        fs::set_permissions(&store, fs::Permissions::from_mode(0o1775)).expect("chmod");
        fs::set_permissions(store.join("abc-etc"), fs::Permissions::from_mode(0o555))
            .expect("chmod");
        fs::write(generation.join("automation-enabled"), "").expect("create switch");
        fs::set_permissions(
            generation.join("automation-enabled"),
            fs::Permissions::from_mode(0o444),
        )
        .expect("chmod");
        fs::set_permissions(&generation, fs::Permissions::from_mode(0o555)).expect("chmod");

        let etc = dir.join("etc");
        fs::create_dir_all(&etc).expect("create etc");
        std::os::unix::fs::symlink(&generation, etc.join("static")).expect("link");
        std::os::unix::fs::symlink("static/automation-enabled", etc.join("automation-enabled"))
            .expect("link");

        assert!(switch_is_on(&etc.join("automation-enabled"), &trusted()));

        // A link to nothing keeps the door shut.
        std::os::unix::fs::symlink("static/nothing-here", etc.join("dangling")).expect("link");
        assert!(!switch_is_on(&etc.join("dangling"), &trusted()));

        // Let the scratch folder be removed next time.
        let _ = fs::set_permissions(&generation, fs::Permissions::from_mode(0o755));
        let _ = fs::set_permissions(store.join("abc-etc"), fs::Permissions::from_mode(0o755));
    }

    #[test]
    fn a_link_to_a_file_others_could_replace_does_not_count() {
        let dir = scratch("link");
        let open = dir.join("open");
        fs::create_dir_all(&open).expect("create folder");
        fs::set_permissions(&open, fs::Permissions::from_mode(0o777)).expect("chmod");

        let target = open.join("target");
        fs::write(&target, "").expect("create target");
        fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).expect("chmod");

        let switch = dir.join("automation-enabled");
        std::os::unix::fs::symlink(&target, &switch).expect("link");

        assert!(
            !switch_is_on(&switch, &trusted()),
            "anyone may replace a file in {open:?}"
        );

        // A sticky folder (like /nix/store) is fine: others can't replace root's files.
        fs::set_permissions(&open, fs::Permissions::from_mode(0o1777)).expect("chmod");

        assert!(switch_is_on(&switch, &trusted()));

        // A relative link, through `..`, is walked the same way.
        let relative = dir.join("relative");
        std::os::unix::fs::symlink("../link-placeholder/../open/target", &relative).expect("link");
        assert!(
            !switch_is_on(&relative, &trusted()),
            "a missing step on the way fails"
        );
    }

    #[test]
    fn the_door_only_opens_in_the_users_own_folder() {
        let dir = scratch("not-mine");

        // Only root can hand a folder to someone else; other users skip this.
        if std::os::unix::fs::chown(&dir, Some(65534), Some(65534)).is_err() {
            return;
        }

        assert!(listen(&dir, Some("test")).is_err());
        assert!(!dir.join("iced-automation").exists());
    }

    #[test]
    fn a_runtime_folder_others_can_open_is_refused() {
        let dir = scratch("open-runtime");
        let runtime = runtime(&dir);
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o755)).expect("chmod");

        assert!(
            listen(&runtime, Some("test")).is_err_and(|error| error.to_string().contains("0700"))
        );
        assert!(!runtime.join("iced-automation").exists());
    }

    #[test]
    fn a_long_app_id_still_fits_in_a_socket_path() {
        let dir = runtime(&scratch("long"));
        let long = "org.example.a-very-long-application-identifier-that-goes-on.and-on.and-on";
        let (_listener, path) = listen(&dir, Some(long)).expect("listen");

        assert!(path.as_os_str().len() < 108, "{path:?}");

        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("name");
        assert!(name.starts_with("org.example.a-very-long"), "{name}");
        assert!(name.ends_with(&format!("-{}.sock", std::process::id())));

        // Two long names that differ only at the end don't collide.
        let other = format!("{long}2");
        let (_other_listener, other_path) = listen(&dir, Some(&other)).expect("listen");
        assert_ne!(path, other_path);
    }

    #[test]
    fn the_socket_is_private_to_the_user() {
        let dir = runtime(&scratch("socket"));
        let (_listener, path) = listen(&dir, Some("org.kora/slate")).expect("listen");

        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some(format!("org.kora_slate-{}.sock", std::process::id()).as_str())
        );
        assert_eq!(
            fs::metadata(&path).expect("socket").permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(path.parent().expect("folder"))
                .expect("folder")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }

    #[test]
    fn keys_parse_by_name_or_character() {
        assert_eq!(
            parse_key("Enter"),
            Ok(keyboard::Key::Named(key::Named::Enter))
        );
        assert_eq!(
            parse_key("N"),
            Ok(keyboard::Key::Character(SmolStr::new("n")))
        );
        assert!(parse_key("Hyper").is_err());
        assert_eq!(
            modifiers(&json!(["ctrl", "Shift"])),
            Ok(keyboard::Modifiers::CTRL | keyboard::Modifiers::SHIFT)
        );
        assert!(modifiers(&json!(["hyper"])).is_err());
    }

    #[test]
    fn the_tree_places_widgets_inside_scrolled_areas() {
        let window = window::Id::unique();
        let mut collector = Collector::new(window, false, Size::new(400.0, 300.0));
        let viewport = Rectangle::new(Point::new(0.0, 100.0), Size::new(200.0, 100.0));

        collector.traverse(&mut |operation| {
            struct Still;

            impl Scrollable for Still {
                fn snap_to(&mut self, _offset: RelativeOffset<Option<f32>>) {}
                fn scroll_to(&mut self, _offset: AbsoluteOffset<Option<f32>>) {}
                fn scroll_by(
                    &mut self,
                    _offset: AbsoluteOffset,
                    _bounds: Rectangle,
                    _content_bounds: Rectangle,
                ) {
                }
            }

            operation.scrollable(
                Some(&Id::new("list")),
                viewport,
                Rectangle::new(Point::new(0.0, 100.0), Size::new(200.0, 400.0)),
                Vector::new(0.0, 50.0),
                &mut Still,
            );
            operation.traverse(&mut |operation| {
                operation.text(
                    None,
                    Rectangle::new(Point::new(10.0, 160.0), Size::new(50.0, 20.0)),
                    "Shown",
                );
                operation.text(
                    None,
                    Rectangle::new(Point::new(10.0, 400.0), Size::new(50.0, 20.0)),
                    "Below",
                );
            });
            operation.text(
                None,
                Rectangle::new(Point::new(10.0, 210.0), Size::new(50.0, 20.0)),
                "After",
            );
        });

        let find = |text: &str| {
            collector
                .nodes
                .iter()
                .find(|node| node.text.as_deref() == Some(text))
                .cloned()
                .expect("node")
        };

        assert_eq!(
            find("Shown").visible,
            Some(Rectangle::new(
                Point::new(10.0, 110.0),
                Size::new(50.0, 20.0)
            ))
        );
        assert_eq!(find("Below").visible, None);
        assert_eq!(
            find("After").bounds,
            Rectangle::new(Point::new(10.0, 210.0), Size::new(50.0, 20.0))
        );
        assert_eq!(collector.nodes[0].id.as_deref(), Some("list"));
    }

    /// A stand-in event loop on its own thread: answers the door from `nodes`,
    /// and keeps every injection, with the cursor it placed. Injections whose
    /// events `fails` matches are refused.
    struct FakeLoop {
        door: Door,
        injected: Arc<Mutex<Vec<(Option<Point>, Vec<Event>)>>>,
        /// The window each injection went to, in order.
        targets: Arc<Mutex<Vec<window::Id>>>,
        /// The surface each injection's pointer left first, in order.
        left: Arc<Mutex<Vec<Option<window::Id>>>>,
    }

    fn fake_loop(nodes: Vec<Node>, fails: fn(&[Event]) -> bool) -> FakeLoop {
        fake_loop_focused(nodes, fails, None)
    }

    /// Like [`fake_loop`], with keyboard focus on another window, `focused`.
    fn fake_loop_focused(
        nodes: Vec<Node>,
        fails: fn(&[Event]) -> bool,
        focused: Option<window::Id>,
    ) -> FakeLoop {
        let (jobs, inbox) = mpsc::channel::<Job>();
        let injected = Arc::new(Mutex::new(Vec::new()));
        let window = nodes
            .first()
            .map_or_else(window::Id::unique, |node| node.window);
        let record = injected.clone();
        let targets = Arc::new(Mutex::new(Vec::new()));
        let targeted = targets.clone();
        let left = Arc::new(Mutex::new(Vec::new()));
        let leaving = left.clone();

        let _ = thread::spawn(move || {
            while let Ok(job) = inbox.recv() {
                let answer = match job.ask {
                    Ask::Info => Answer::Info(
                        std::iter::once(window)
                            .chain(focused)
                            .map(|surface| {
                                Surface::new(
                                    surface,
                                    false,
                                    Size::new(800.0, 600.0),
                                    1.0,
                                    Some(surface) == focused.or(Some(window)),
                                )
                            })
                            .collect(),
                    ),
                    Ask::Tree => Answer::Tree(nodes.clone()),
                    Ask::Idle => Answer::Idle(true),
                    Ask::Focused => Answer::Focused(focused.or(Some(window))),
                    Ask::Inject {
                        window: target,
                        cursor,
                        events,
                        leave,
                    } => {
                        let refused = fails(&events);

                        if !refused {
                            record.lock().expect("record").push((cursor, events));
                            targeted.lock().expect("targets").push(target);
                            leaving.lock().expect("left").push(leave);
                        }

                        Answer::Injected(!refused)
                    }
                };

                let _ = job.answer.send(answer);
            }
        });

        FakeLoop {
            door: Door::new(
                jobs,
                noop(),
                "test".into(),
                None,
                DEFAULTS,
                Max::default(),
                Ops::ALL,
                Arc::new(Mutex::new(None)),
            ),
            injected,
            targets,
            left,
        }
    }

    fn node(
        window: window::Id,
        kind: &'static str,
        id: Option<&str>,
        text: Option<&str>,
        bounds: Rectangle,
    ) -> Node {
        Node {
            window,
            popup: false,
            kind,
            id: id.map(str::to_owned),
            text: text.map(str::to_owned),
            bounds,
            visible: Some(bounds),
            focused: false,
        }
    }

    fn pointer_events(fake: &FakeLoop) -> Vec<String> {
        fake.injected
            .lock()
            .expect("record")
            .iter()
            .flat_map(|(_, events)| events.iter())
            .map(|event| match event {
                Event::Mouse(mouse::Event::CursorMoved { position }) => {
                    format!("move {},{}", position.x, position.y)
                }
                Event::Mouse(mouse::Event::ButtonPressed(_)) => "press".into(),
                Event::Mouse(mouse::Event::ButtonReleased(_)) => "release".into(),
                Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                    format!("modifiers {modifiers:?}")
                }
                Event::Keyboard(keyboard::Event::KeyPressed { .. }) => "key down".into(),
                Event::Keyboard(keyboard::Event::KeyReleased { .. }) => "key up".into(),
                other => format!("{other:?}"),
            })
            .collect()
    }

    #[test]
    fn a_widget_reported_twice_in_one_place_is_one_match() {
        let window = window::Id::unique();
        let field = Rectangle::new(Point::new(10.0, 10.0), Size::new(100.0, 20.0));
        let fake = fake_loop(
            vec![
                node(window, "text_input", Some("name"), Some("Name"), field),
                node(window, "focusable", Some("name"), None, field),
            ],
            |_| false,
        );

        let reply = fake.door.handle(&json!({ "op": "click", "id": "name" }));

        assert!(reply.is_ok(), "{reply:?}");
        assert_eq!(pointer_events(&fake), ["move 60,20", "press", "release"]);
    }

    #[test]
    fn a_drag_presses_moves_and_releases() {
        let window = window::Id::unique();
        let fake = fake_loop(
            vec![
                node(
                    window,
                    "text",
                    None,
                    Some("Card"),
                    Rectangle::new(Point::new(0.0, 0.0), Size::new(20.0, 20.0)),
                ),
                node(
                    window,
                    "named",
                    Some("bin"),
                    None,
                    Rectangle::new(Point::new(80.0, 0.0), Size::new(20.0, 20.0)),
                ),
            ],
            |_| false,
        );

        let reply = fake
            .door
            .handle(&json!({ "op": "drag", "text": "Card", "to": { "id": "bin" } }))
            .expect("drag");

        assert_eq!(reply["to"], json!({ "x": 90.0, "y": 10.0 }));

        let events = pointer_events(&fake);

        assert_eq!(events[..2], ["move 10,10", "press"]);
        assert_eq!(events.len(), 2 + usize::from(DEFAULTS.drag_steps) + 1);
        assert_eq!(events[events.len() - 2..], ["move 90,10", "release"]);

        assert!(
            fake.door
                .handle(&json!({ "op": "drag", "text": "Card" }))
                .is_err(),
            "a drag needs somewhere to go"
        );
    }

    #[test]
    fn a_drag_to_a_point_stays_in_the_drags_window() {
        let window = window::Id::unique();
        let elsewhere = window::Id::unique();
        let fake = fake_loop_focused(
            vec![node(
                window,
                "text",
                None,
                Some("Handle"),
                Rectangle::new(Point::new(0.0, 0.0), Size::new(20.0, 20.0)),
            )],
            |_| false,
            Some(elsewhere),
        );

        let reply = fake
            .door
            .handle(&json!({ "op": "drag", "text": "Handle", "to": { "x": 100, "y": 50 } }))
            .expect("drag");

        assert_eq!(reply["window"], json!(window.to_string()));
        assert_eq!(
            fake.injected
                .lock()
                .expect("record")
                .last()
                .and_then(|(cursor, _)| *cursor),
            Some(Point::new(100.0, 50.0)),
            "released exactly where it was asked to go"
        );
        assert!(
            fake.door
                .handle(
                    &json!({ "op": "drag", "text": "Handle", "to": { "x": 1, "y": 1 }, "dx": 5 })
                )
                .is_err()
        );
    }

    #[test]
    fn keys_go_where_the_door_clicked() {
        let window = window::Id::unique();
        let elsewhere = window::Id::unique();
        let field = Rectangle::new(Point::new(10.0, 10.0), Size::new(100.0, 20.0));
        let fake = fake_loop_focused(
            vec![node(
                window,
                "text_input",
                Some("name"),
                Some("Name"),
                field,
            )],
            |_| false,
            Some(elsewhere),
        );

        let _ = fake
            .door
            .handle(&json!({ "op": "click", "id": "name" }))
            .expect("click");
        fake.targets.lock().expect("targets").clear();

        let _ = fake
            .door
            .handle(&json!({ "op": "type", "text": "ab" }))
            .expect("type");

        let targets = fake.targets.lock().expect("targets").clone();
        assert!(!targets.is_empty());
        assert!(
            targets.iter().all(|target| *target == window),
            "keys went to {targets:?}, not the clicked window {window:?}"
        );
    }

    #[test]
    fn a_request_past_its_time_stops_but_still_lets_go() {
        let window = window::Id::unique();
        let fake = fake_loop(
            vec![node(
                window,
                "text",
                None,
                Some("Handle"),
                Rectangle::new(Point::new(0.0, 0.0), Size::new(20.0, 20.0)),
            )],
            |_| false,
        );

        fake.door
            .deadline
            .set(Instant::now().checked_sub(Duration::from_secs(1)));

        let moved = vec![Event::Mouse(mouse::Event::CursorMoved {
            position: Point::ORIGIN,
        })];
        let released = vec![Event::Mouse(mouse::Event::ButtonReleased(
            mouse::Button::Left,
        ))];

        assert!(fake.door.inject(window, None, moved).is_err());
        assert!(fake.door.release(window, None, released).is_ok());
        assert_eq!(pointer_events(&fake), ["release"]);
    }

    #[test]
    fn a_client_that_hung_up_is_noticed() {
        let (ours, theirs) = UnixStream::pair().expect("pair");
        let fake = fake_loop(vec![], |_| false);
        let door = Door {
            peer: Some(ours),
            ..fake.door
        };

        assert!(!door.client_gone());

        drop(theirs);

        assert!(door.client_gone());
        assert!(
            door.inject(window::Id::unique(), None, vec![])
                .is_err_and(|error| error.contains("hung up"))
        );
    }

    #[test]
    fn a_drag_by_an_offset_is_in_pixels() {
        let window = window::Id::unique();
        let fake = fake_loop(
            vec![node(
                window,
                "text",
                None,
                Some("Handle"),
                Rectangle::new(Point::new(0.0, 0.0), Size::new(20.0, 20.0)),
            )],
            |_| false,
        );

        let _ = fake
            .door
            .handle(&json!({ "op": "drag", "text": "Handle", "dy": 40 }))
            .expect("drag");

        assert_eq!(
            pointer_events(&fake).last().map(String::as_str),
            Some("release")
        );
        assert_eq!(
            fake.injected
                .lock()
                .expect("record")
                .last()
                .and_then(|(cursor, _)| *cursor),
            Some(Point::new(10.0, 50.0))
        );
    }

    #[test]
    fn a_failed_drag_still_lets_go() {
        let window = window::Id::unique();
        let fake = fake_loop(
            vec![node(
                window,
                "text",
                None,
                Some("Handle"),
                Rectangle::new(Point::new(0.0, 0.0), Size::new(20.0, 20.0)),
            )],
            // The third move is refused, half way.
            |events| {
                static MOVES: AtomicUsize = AtomicUsize::new(0);

                matches!(events, [Event::Mouse(mouse::Event::CursorMoved { .. })])
                    && MOVES.fetch_add(1, Ordering::SeqCst) == 3
            },
        );

        assert!(
            fake.door
                .handle(&json!({ "op": "drag", "text": "Handle", "dx": 80 }))
                .is_err()
        );
        assert_eq!(
            pointer_events(&fake).last().map(String::as_str),
            Some("release")
        );
    }

    #[test]
    fn modifiers_are_let_go_even_when_the_key_fails() {
        let fake = fake_loop(vec![], |events| {
            matches!(
                events,
                [Event::Keyboard(keyboard::Event::KeyPressed { .. })]
            )
        });

        assert!(
            fake.door
                .handle(&json!({ "op": "key", "key": "a", "modifiers": ["ctrl"] }))
                .is_err()
        );
        assert_eq!(
            pointer_events(&fake),
            [
                format!("modifiers {:?}", keyboard::Modifiers::CTRL),
                format!("modifiers {:?}", keyboard::Modifiers::empty()),
            ]
        );
    }

    #[test]
    fn a_request_the_client_gave_up_on_is_dropped() {
        let (jobs, inbox) = mpsc::channel();
        let (late, _late_answer) = mpsc::channel();
        let (cleanup, _cleanup_answer) = mpsc::channel();
        let past = Instant::now()
            .checked_sub(Duration::from_secs(1))
            .expect("past");

        jobs.send(Job {
            ask: Ask::Idle,
            answer: late,
            deadline: past,
            cleanup: false,
        })
        .expect("send");
        jobs.send(Job {
            ask: Ask::Focused,
            answer: cleanup,
            deadline: past,
            cleanup: true,
        })
        .expect("send");

        let mut asked = Vec::new();

        answer_all(&inbox, &mut |ask| {
            asked.push(format!("{ask:?}"));
            Answer::Focused(None)
        });

        assert_eq!(asked, ["Focused"], "only the late release still goes in");
    }

    #[test]
    fn the_socket_goes_when_the_door_closes() {
        let _turn = real_door();
        let dir = runtime(&scratch("close"));
        let (listener, path) = listen(&dir, Some("test")).expect("listen");
        drop(listener);

        let open = Open { path: path.clone() };
        assert!(path.exists());

        drop(open);
        assert!(!path.exists());
    }

    #[test]
    fn nothing_outside_the_window_is_visible() {
        let mut collector = Collector::new(window::Id::unique(), false, Size::new(400.0, 300.0));

        collector.text(
            None,
            Rectangle::new(Point::new(10.0, 2000.0), Size::new(50.0, 20.0)),
            "Off screen",
        );
        collector.text(
            None,
            Rectangle::new(Point::new(390.0, 10.0), Size::new(50.0, 20.0)),
            "Half in",
        );

        assert_eq!(collector.nodes[0].visible, None);
        assert_eq!(
            collector.nodes[1].visible,
            Some(Rectangle::new(
                Point::new(390.0, 10.0),
                Size::new(10.0, 20.0)
            ))
        );
    }
    #[test]
    fn a_painted_offset_moves_the_walk_after_it_and_nothing_else() {
        let mut collector = Collector::new(window::Id::unique(), false, Size::new(400.0, 300.0));
        let at = |y| Rectangle::new(Point::new(10.0, y), Size::new(50.0, 20.0));

        // A translated widget: its offset, then its children.
        collector.custom(None, at(200.0), &mut PaintedOffset(Vector::new(0.0, -40.0)));
        collector.traverse(&mut |collector| collector.text(None, at(200.0), "Raised"));
        // Its next sibling is where it is laid out.
        collector.text(None, at(240.0), "Below");
        // An offset with no walk after it moves nothing.
        collector.custom(None, at(0.0), &mut PaintedOffset(Vector::new(0.0, 100.0)));
        collector.text(None, at(260.0), "Still here");
        // Other custom reports are ignored, a bare `Vector` included.
        collector.custom(None, at(0.0), &mut 7_u8);
        collector.custom(None, at(0.0), &mut Vector::<f32>::new(0.0, 100.0));
        collector.traverse(&mut |collector| collector.text(None, at(280.0), "Plain"));

        let ys: Vec<f32> = collector.nodes.iter().map(|node| node.bounds.y).collect();
        assert_eq!(ys, [160.0, 240.0, 260.0, 280.0]);
    }

    #[test]
    fn what_is_kept_but_not_shown_is_never_visible() {
        let mut collector = Collector::new(window::Id::unique(), false, Size::new(400.0, 300.0));
        let at = |y| Rectangle::new(Point::new(10.0, y), Size::new(50.0, 20.0));

        // A covered page: hidden, then walked into; a name inside it too.
        collector.custom(None, at(0.0), &mut Hidden);
        collector.traverse(&mut |collector| {
            collector.container(Some(&Id::new("composer.send")), at(10.0));
            collector.traverse(&mut |collector| collector.text(None, at(10.0), "Send"));
        });
        // The page on top is shown.
        collector.traverse(&mut |collector| collector.text(None, at(10.0), "Inbox"));
        // A hiding with no walk after it hides nothing.
        collector.custom(None, at(0.0), &mut Hidden);
        collector.text(None, at(40.0), "Shown");

        let visible: Vec<(Option<&str>, Option<&str>, bool)> = collector
            .nodes
            .iter()
            .map(|node| {
                (
                    node.id.as_deref(),
                    node.text.as_deref(),
                    node.visible.is_some(),
                )
            })
            .collect();

        assert_eq!(
            visible,
            [
                (Some("composer.send"), None, false),
                (None, Some("Send"), false),
                (None, Some("Inbox"), true),
                (None, Some("Shown"), true),
            ]
        );
    }

    #[test]
    fn a_text_input_is_as_focused_as_its_focusable() {
        struct Field(bool);

        impl Focusable for Field {
            fn is_focused(&self) -> bool {
                self.0
            }
            fn focus(&mut self) {}
            fn unfocus(&mut self) {}
        }

        impl TextInput for Field {
            fn text(&self) -> &str {
                "Name"
            }
            fn move_cursor_to_front(&mut self) {}
            fn move_cursor_to_end(&mut self) {}
            fn move_cursor_to(&mut self, _position: usize) {}
            fn select_all(&mut self) {}
            fn select_range(&mut self, _start: usize, _end: usize) {}
        }

        let mut collector = Collector::new(window::Id::unique(), false, Size::new(400.0, 300.0));
        let at = |y| Rectangle::new(Point::new(10.0, y), Size::new(100.0, 20.0));

        // As iced's text input reports itself: the input, then its focus.
        collector.text_input(None, at(10.0), &mut Field(true));
        collector.focusable(None, at(10.0), &mut Field(true));
        collector.text_input(None, at(40.0), &mut Field(false));
        collector.focusable(None, at(40.0), &mut Field(false));

        let inputs: Vec<bool> = collector
            .nodes
            .iter()
            .filter(|node| node.kind == "text_input")
            .map(|node| node.focused)
            .collect();

        assert_eq!(inputs, [true, false]);
    }

    #[test]
    fn a_switch_file_is_empty_or_json_the_door_knows() {
        assert_eq!(parse_policy(""), Ok(Policy::default()));
        assert_eq!(parse_policy("  \n"), Ok(Policy::default()));

        let policy = parse_policy(
            r#"{ "max": { "settle_ms": 100, "steps": 4, "clients": 2 },
                 "ops": ["read", "pointer"],
                 "apps": ["org.example.app"] }"#,
        )
        .expect("policy");

        assert_eq!(policy.max.settle, Some(Duration::from_millis(100)));
        assert_eq!(policy.max.drag_steps, Some(4));
        assert_eq!(policy.max.clients, Some(2));
        assert_eq!(policy.max.request, None);
        let mut ops = Ops::ALL;
        ops.keyboard = false;
        assert_eq!(policy.ops, ops);
        assert!(policy.allows("org.example.app"));
        assert!(!policy.allows("org.example.other"));

        // Anything else keeps the door shut: a typo must not open it wider.
        for bad in [
            "yes",
            "{",
            "[]",
            r#"{ "maximum": {} }"#,
            r#"{ "max": { "settle": 100 } }"#,
            r#"{ "max": { "settle_ms": -1 } }"#,
            r#"{ "ops": ["everything"] }"#,
            r#"{ "apps": "org.example.app" }"#,
        ] {
            assert!(parse_policy(bad).is_err(), "{bad} was accepted");
        }
    }

    #[test]
    fn numbers_stay_within_the_bounds_the_app_and_the_admin_set() {
        // The app asks for more than the hard ceiling, and less than the floor.
        let config = Config::new()
            .request_timeout(Duration::from_secs(100_000))
            .settle_timeout(Duration::from_millis(250))
            .max_clients(0);
        let limits = Limits::resolve(&config, &Max::default());

        assert_eq!(limits.request, HARD_MAX.request);
        assert_eq!(limits.settle, Duration::from_millis(250));
        assert_eq!(limits.clients, HARD_MIN.clients);
        assert_eq!(limits.answer, DEFAULTS.answer);

        // The administrator's caps win over the app's wishes.
        let max = Max {
            settle: Some(Duration::from_millis(100)),
            clients: Some(2),
            ..Max::default()
        };
        let limits = Limits::resolve(&Config::new().max_clients(16), &max);

        assert_eq!(limits.settle, Duration::from_millis(100));
        assert_eq!(limits.clients, 2);

        // And over a request's.
        let fake = fake_loop(vec![], |_| false);
        let door = Door { max, ..fake.door };
        let _ = door
            .handle(&json!({ "op": "idle", "settle_ms": 5000, "steps": 1000 }))
            .expect("idle");

        assert_eq!(door.settle.get(), Duration::from_millis(100));
        assert_eq!(door.steps.get(), HARD_MAX.drag_steps);
        assert!(
            door.handle(&json!({ "op": "idle", "timeout_ms": "soon" }))
                .is_err()
        );
    }

    #[test]
    fn a_drag_takes_the_steps_it_asks_for() {
        let window = window::Id::unique();
        let fake = fake_loop(
            vec![node(
                window,
                "text",
                None,
                Some("Handle"),
                Rectangle::new(Point::new(0.0, 0.0), Size::new(20.0, 20.0)),
            )],
            |_| false,
        );

        let _ = fake
            .door
            .handle(
                &json!({ "op": "drag", "text": "Handle", "dx": 30, "steps": 3, "settle_ms": 0 }),
            )
            .expect("drag");

        // Move there, press, three moves, release.
        assert_eq!(pointer_events(&fake).len(), 2 + 3 + 1);
    }

    #[test]
    fn ops_the_device_leaves_out_are_refused() {
        let window = window::Id::unique();
        let fake = fake_loop(
            vec![node(
                window,
                "text",
                None,
                Some("Save"),
                Rectangle::new(Point::new(0.0, 0.0), Size::new(20.0, 20.0)),
            )],
            |_| false,
        );
        let door = Door {
            ops: Ops::READ,
            ..fake.door
        };

        assert!(door.handle(&json!({ "op": "tree" })).is_ok());
        assert!(
            door.handle(&json!({ "op": "click", "text": "Save" }))
                .is_err_and(|error| error.contains("isn't allowed"))
        );
        assert!(
            door.handle(&json!({ "op": "type", "text": "rm -rf ~" }))
                .is_err()
        );
        assert!(fake.injected.lock().expect("record").is_empty());
    }

    #[test]
    fn typing_is_one_batch_of_presses_and_releases() {
        let window = window::Id::unique();
        let fake = fake_loop(
            vec![node(window, "focusable", None, None, Rectangle::default())],
            |_| false,
        );

        let _ = fake
            .door
            .handle(&json!({ "op": "type", "text": "ab c" }))
            .expect("type");

        let injected = fake.injected.lock().expect("record");
        assert_eq!(injected.len(), 1, "one injection for the whole string");

        let kinds: Vec<&str> = injected[0]
            .1
            .iter()
            .map(|event| match event {
                Event::Keyboard(keyboard::Event::KeyPressed { .. }) => "down",
                Event::Keyboard(keyboard::Event::KeyReleased { .. }) => "up",
                _ => "other",
            })
            .collect();

        assert_eq!(
            kinds,
            ["down", "up", "down", "up", "down", "up", "down", "up"]
        );
    }

    #[test]
    fn each_client_keeps_its_own_click_for_keys() {
        let window = window::Id::unique();
        let elsewhere = window::Id::unique();
        let field = Rectangle::new(Point::new(10.0, 10.0), Size::new(100.0, 20.0));
        let first = fake_loop_focused(
            vec![node(
                window,
                "text_input",
                Some("name"),
                Some("Name"),
                field,
            )],
            |_| false,
            Some(elsewhere),
        );
        let second = fake_loop_focused(
            vec![node(
                window,
                "text_input",
                Some("name"),
                Some("Name"),
                field,
            )],
            |_| false,
            Some(elsewhere),
        );

        // The first client clicks; the second, which didn't, types.
        let _ = first
            .door
            .handle(&json!({ "op": "click", "id": "name" }))
            .expect("click");
        let _ = second
            .door
            .handle(&json!({ "op": "type", "text": "x" }))
            .expect("type");

        assert_eq!(
            second.targets.lock().expect("targets").as_slice(),
            [elsewhere],
            "keys follow the app's focus, not another client's click"
        );
    }

    #[test]
    fn the_pointer_leaves_one_surface_for_another() {
        let window = window::Id::unique();
        let popup = window::Id::unique();
        let at = Rectangle::new(Point::new(0.0, 0.0), Size::new(20.0, 20.0));
        let fake = fake_loop(
            vec![
                node(window, "text", None, Some("Open"), at),
                node(popup, "text", None, Some("Item"), at),
            ],
            |_| false,
        );

        let _ = fake
            .door
            .handle(&json!({ "op": "hover", "text": "Open" }))
            .expect("hover");
        let _ = fake
            .door
            .handle(&json!({ "op": "hover", "text": "Item" }))
            .expect("hover");
        let _ = fake.door.handle(&json!({ "op": "leave" })).expect("leave");
        let after = fake.door.handle(&json!({ "op": "leave" })).expect("leave");

        assert_eq!(
            fake.left.lock().expect("left").as_slice(),
            [None, Some(window), Some(popup)],
            "first nothing to leave, then the window for the popup, then the popup"
        );
        assert_eq!(after["left"], Value::Null, "nothing left to leave");
    }

    #[test]
    fn leaving_another_surface_keeps_the_one_pointed_at() {
        let window = window::Id::unique();
        let popup = window::Id::unique();
        let at = Rectangle::new(Point::new(0.0, 0.0), Size::new(20.0, 20.0));
        let fake = fake_loop(
            vec![
                node(window, "text", None, Some("Open"), at),
                node(popup, "text", None, Some("Item"), at),
            ],
            |_| false,
        );

        let _ = fake
            .door
            .handle(&json!({ "op": "hover", "text": "Item" }))
            .expect("hover");
        let _ = fake
            .door
            .handle(&json!({ "op": "leave", "window": window.to_string() }))
            .expect("leave");
        let _ = fake
            .door
            .handle(&json!({ "op": "hover", "text": "Open" }))
            .expect("hover");

        // The popup is still pointed at after the window was left, so moving
        // onto the window leaves the popup.
        let left = fake.left.lock().expect("left").clone();
        assert_eq!(left.last(), Some(&Some(popup)), "{left:?}");
    }

    #[test]
    fn another_client_can_move_the_pointer_off() {
        let window = window::Id::unique();
        let at = Rectangle::new(Point::new(0.0, 0.0), Size::new(20.0, 20.0));
        let fake = fake_loop(vec![node(window, "text", None, Some("Tip"), at)], |_| false);

        // A test driver connects once per request, as door.py and koraqa do.
        let next = Door::new(
            fake.door.jobs.clone(),
            noop(),
            "test".into(),
            None,
            DEFAULTS,
            Max::default(),
            Ops::ALL,
            fake.door.pointed.clone(),
        );

        let _ = fake
            .door
            .handle(&json!({ "op": "hover", "text": "Tip" }))
            .expect("hover");
        let left = next.handle(&json!({ "op": "leave" })).expect("leave");

        assert_eq!(left["left"], json!(window.to_string()));
        assert_eq!(fake.left.lock().expect("left").last(), Some(&Some(window)));
    }

    #[test]
    fn a_client_that_only_stopped_writing_still_gets_its_answer() {
        let (ours, theirs) = UnixStream::pair().expect("pair");
        let fake = fake_loop(vec![], |_| false);
        let door = Door {
            peer: Some(ours),
            ..fake.door
        };

        // `echo '{"op":"info"}' | socat - UNIX-CONNECT:…` does this.
        theirs
            .shutdown(std::net::Shutdown::Write)
            .expect("shutdown");

        assert!(!door.client_gone());
        assert!(door.handle(&json!({ "op": "info" })).is_ok());

        drop(theirs);
        assert!(door.client_gone());
    }

    /// Answers the door from a thread until `stop`, like an event loop would.
    fn serve_until(stop: Arc<AtomicBool>) -> thread::JoinHandle<()> {
        thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                serve(|ask| match ask {
                    Ask::Info => Answer::Info(Vec::new()),
                    Ask::Idle => Answer::Idle(true),
                    _ => Answer::Unsupported,
                });
                thread::sleep(Duration::from_millis(5));
            }
        })
    }

    fn ask_door(path: &Path, request: &str) -> Value {
        let mut stream = UnixStream::connect(path).expect("connect");
        let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
        // A refused client may find the door already closed for writing.
        let _ = writeln!(stream, "{request}");

        let mut line = String::new();
        let _ = BufReader::new(stream).read_line(&mut line).expect("read");

        serde_json::from_str(&line).expect("JSON")
    }

    #[test]
    fn the_switch_is_read_again_for_every_client() {
        let _turn = real_door();
        let dir = scratch("recheck");
        let runtime = runtime(&dir);
        let switch = dir.join("automation-enabled");
        fs::write(&switch, "").expect("create switch");
        fs::set_permissions(&switch, fs::Permissions::from_mode(0o644)).expect("chmod");

        let open = open_door(
            Config::new()
                .switch(&switch)
                .idle_client_timeout(Duration::from_secs(1)),
            &trusted(),
            Some(runtime),
            Some("org.example.app"),
            noop(),
        )
        .expect("the door opens");
        let stop = Arc::new(AtomicBool::new(false));
        let looping = serve_until(stop.clone());

        assert_eq!(
            ask_door(open.path(), r#"{"op":"info"}"#)["app_id"],
            "org.example.app"
        );

        // An administrator narrows it to another app: new clients are turned away.
        fs::set_permissions(&switch, fs::Permissions::from_mode(0o644)).expect("chmod");
        fs::write(&switch, r#"{"apps":["org.example.other"]}"#).expect("write switch");
        assert!(
            ask_door(open.path(), r#"{"op":"info"}"#)["error"]
                .as_str()
                .is_some_and(|error| error.contains("doesn't list this app"))
        );

        // ...and removes it: the door is shut, without restarting the app.
        fs::remove_file(&switch).expect("remove switch");
        assert!(
            ask_door(open.path(), r#"{"op":"info"}"#)["error"]
                .as_str()
                .is_some_and(|error| error.contains("is off"))
        );

        // A client that sends nothing gives its place up.
        fs::write(&switch, "").expect("create switch");
        let mut quiet = UnixStream::connect(open.path()).expect("connect");
        let _ = quiet.set_read_timeout(Some(Duration::from_secs(5)));
        let mut line = String::new();
        let _ = BufReader::new(quiet.try_clone().expect("clone"))
            .read_line(&mut line)
            .expect("read");
        assert!(line.contains("nothing sent"), "{line}");
        let _ = quiet.flush();

        // Once the loop stops and drops the door, waiting clients are told.
        let mut waiting = UnixStream::connect(open.path()).expect("connect");
        let _ = waiting.set_read_timeout(Some(Duration::from_secs(10)));
        stop.store(true, Ordering::SeqCst);
        looping.join().expect("loop");
        drop(open);
        assert!(!is_open());

        writeln!(waiting, r#"{{"op":"info"}}"#).expect("send");
        let mut line = String::new();
        let _ = BufReader::new(waiting).read_line(&mut line).expect("read");
        assert!(line.contains("the app has stopped"), "{line}");
    }
}
