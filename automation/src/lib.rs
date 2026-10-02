//! A door that lets a test driver on the same device read and press the widgets
//! of a running app, the way a person would.
//!
//! The door stays shut unless an administrator has switched it on for the whole
//! device, by creating [`SWITCH`] owned by root and writable by no one else.
//! Nothing the user's own processes can set (an environment variable, a
//! setting, a file in their home) opens it: on Wayland one app may not press
//! buttons in another, and the door must not become a way around that.
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
//!
//! Clicks and keys go in as input events, into the same queue a person's input
//! reaches the widgets through, never by calling the app's handlers. They skip
//! the windowing layer itself, so, for instance, a click outside a Wayland popup
//! doesn't dismiss it. Positions are surface-relative logical pixels. Pointer requests also take `"x"` and
//! `"y"`, and any request can name a `"window"` from `info`.
//!
//! # For event loops
//!
//! This crate owns the switch, the socket and the protocol; it knows nothing
//! about any one event loop. A loop opens the door once with [`start`], keeping
//! the [`Open`] it returns for as long as it runs, and calls [`serve`] once per
//! pass, before it hands out its pending events. [`serve`] gives it each
//! request as an [`Ask`]; it answers with an [`Answer`], reading its widget
//! trees with a [`Collector`].
#![cfg(target_os = "linux")]

use iced_core as core;

use crate::core::keyboard::{self, key};
use crate::core::mouse;
use crate::core::widget::operation::{Focusable, Scrollable, TextInput};
use crate::core::widget::{Id, Operation};
use crate::core::window;
use crate::core::{Event, Point, Rectangle, Size, SmolStr, Vector};

use serde_json::{Value, json};

use std::cell::Cell;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::thread;
use std::time::{Duration, Instant};

/// The file an administrator creates to open the door on a device.
pub const SWITCH: &str = "/etc/kora/automation-enabled";

/// How long a request waits for the app to answer.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(5);

/// How long input waits for the app to settle between steps.
const SETTLE_TIMEOUT: Duration = Duration::from_millis(500);

/// The longest request line read, in bytes.
const MAX_REQUEST: u64 = 64 * 1024;

/// How many clients may be connected at once.
const MAX_CLIENTS: usize = 8;

/// How long before a client gives up the event loop stops taking its request,
/// so that a request the client was told timed out never runs later.
const ANSWER_MARGIN: Duration = Duration::from_millis(250);

/// How long one request may take in all, across its steps (a long `type`, say).
/// After that, or once the client has hung up, the rest is not sent.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// How many pointer moves a drag is split into.
const DRAG_STEPS: u16 = 8;

/// Requests waiting for the event loop, set once the door opens.
static INBOX: OnceLock<Mutex<mpsc::Receiver<Job>>> = OnceLock::new();

/// The surface the door last pressed a button on, with the one the app said had
/// keyboard focus at that moment. Keys go to the pressed surface, as they would
/// after a person's click, until the app's focus moves somewhere else.
static LAST_PRESS: Mutex<Option<(window::Id, Option<window::Id>)>> = Mutex::new(None);

type Wake = Arc<dyn Fn() + Send + Sync>;

/// An open door. Dropping it removes the socket; keep it while the loop runs.
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
        let _ = fs::remove_file(&self.path);
    }
}

/// Opens the door if the device's switch is on. Otherwise does nothing at all,
/// and returns `None`.
///
/// The socket is named after `app_id` (the Wayland app id, when the app sets
/// one), or else the name the program was started as. `wake` must make the
/// event loop run a pass soon, so that it calls [`serve`]; it is called from
/// the door's own threads.
#[must_use = "dropping the door removes its socket at once"]
pub fn start(app_id: Option<&str>, wake: impl Fn() + Send + Sync + 'static) -> Option<Open> {
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    let program = std::env::args_os()
        .next()
        .and_then(|arg| {
            Path::new(&arg)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .filter(|name| !name.is_empty());

    start_with(
        Path::new(SWITCH),
        runtime_dir,
        app_id.or(program.as_deref()),
        Arc::new(wake),
    )
}

/// Whether the door is open in this process. Cheap: one atomic load.
pub fn is_open() -> bool {
    INBOX.get().is_some()
}

/// Answers the requests waiting for the event loop, by calling `answer` for
/// each. A loop calls it once per pass, before it hands out its pending events,
/// so anything injected is processed in the same pass, exactly as if the
/// compositor had sent it. Does nothing when the door is shut.
///
/// Requests the client has already given up on are dropped unasked, except
/// ones that let go of a key or button an earlier request pressed.
pub fn serve(mut answer: impl FnMut(Ask) -> Answer) {
    let Some(inbox) = INBOX.get() else {
        return;
    };

    let Ok(inbox) = inbox.lock() else {
        return;
    };

    answer_all(&inbox, &mut answer);
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
/// A frame asked for long ago and never drawn means the compositor isn't
/// drawing the surface (it is hidden), and a timed redraw further off (a text
/// cursor blinking) is not work in progress, so neither keeps the app busy.
pub fn frame_due(requested_at: Option<Instant>, redraw_at: Option<Instant>, now: Instant) -> bool {
    const FRAME: Duration = Duration::from_millis(50);
    const UNDRAWN: Duration = Duration::from_millis(250);

    requested_at.is_some_and(|requested| now.saturating_duration_since(requested) < UNDRAWN)
        || redraw_at.is_some_and(|at| at <= now + FRAME)
}

fn start_with(
    switch: &Path,
    runtime_dir: Option<PathBuf>,
    app_id: Option<&str>,
    wake: Wake,
) -> Option<Open> {
    if !switch_is_on(switch) {
        return None;
    }

    // Before anything touches the socket folder: a second door would bind,
    // and then remove, the first one's socket.
    if INBOX.get().is_some() {
        log::warn!("automation: the door is already open in this process");
        return None;
    }

    let Some(runtime_dir) = runtime_dir else {
        log::warn!("automation: {SWITCH} is on, but XDG_RUNTIME_DIR is unset; door stays shut");
        return None;
    };

    let (listener, path) = match listen(&runtime_dir, app_id) {
        Ok(opened) => opened,
        Err(error) => {
            log::warn!("automation: {SWITCH} is on, but the door could not open: {error}");
            return None;
        }
    };

    let open = Open { path };
    let (jobs, inbox) = mpsc::channel();

    if INBOX.set(Mutex::new(inbox)).is_err() {
        log::warn!("automation: the door is already open in this process");
        return None;
    }

    let app_id = app_id.unwrap_or("app").to_owned();

    let _ = thread::Builder::new()
        .name("iced-automation".into())
        .spawn(move || accept(listener, jobs, wake, app_id))
        .ok()?;

    log::warn!(
        "automation: door open at {} because {SWITCH} is on",
        open.path.display()
    );

    Some(open)
}

/// Whether `switch` is a file that only root could have put there.
///
/// The path is walked one step at a time, following symlinks by hand. Every
/// folder, symlink and the file itself must belong to root, the file must not
/// be writable by anyone else, and no folder on the way may be writable by
/// anyone else unless it is sticky (as `/nix/store` is), where others cannot
/// replace what root put there. So a NixOS `environment.etc` link into the
/// store counts, and a link to a file a user could recreate does not.
fn switch_is_on(switch: &Path) -> bool {
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
            && metadata.uid() == 0
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

        if metadata.uid() != 0 {
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
    if fs::symlink_metadata(runtime_dir)?.uid() != user {
        return Err(io::Error::other(format!(
            "{} does not belong to the user the app runs as",
            runtime_dir.display()
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

    let path = dir.join(format!("{name}-{}.sock", std::process::id()));

    match fs::remove_file(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    let listener = UnixListener::bind(&path)?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;

    Ok((listener, path))
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

fn accept(listener: UnixListener, jobs: mpsc::Sender<Job>, wake: Wake, app_id: String) {
    let clients = Arc::new(AtomicUsize::new(0));

    for stream in listener.incoming() {
        let Ok(mut stream) = stream else {
            continue;
        };

        if clients.fetch_add(1, Ordering::SeqCst) >= MAX_CLIENTS {
            let _ = clients.fetch_sub(1, Ordering::SeqCst);
            let _ = writeln!(stream, "{}", json!({ "error": "too many clients" }));
            continue;
        }

        let door = Door {
            jobs: jobs.clone(),
            wake: wake.clone(),
            app_id: app_id.clone(),
            peer: stream.try_clone().ok(),
            deadline: Cell::new(None),
        };
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

/// What the door asks the event loop.
#[derive(Debug)]
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
    Inject {
        /// The window or popup the events are for.
        window: window::Id,
        /// Where its cursor goes, in surface-relative logical pixels.
        cursor: Option<Point>,
        /// The events, in order.
        events: Vec<Event>,
    },
}

/// The event loop's answer to an [`Ask`].
#[derive(Debug)]
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
}

/// A window or popup, as [`Answer::Info`] lists it.
#[derive(Debug, Clone, PartialEq)]
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
    /// When the request being handled must be done by.
    deadline: Cell<Option<Instant>>,
}

impl Door {
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
                    if error.kind() == io::ErrorKind::InvalidData {
                        let _ = writeln!(writer, "{}", json!({ "error": "request is not UTF-8" }));
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

    fn handle(&self, request: &Value) -> Result<Value, String> {
        let op = request["op"].as_str().ok_or("missing \"op\"")?;

        self.deadline.set(Some(Instant::now() + REQUEST_TIMEOUT));

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

                for c in text.chars() {
                    let (key, text) = match c {
                        ' ' => (keyboard::Key::Named(key::Named::Space), " ".to_owned()),
                        '\n' | '\r' => (keyboard::Key::Named(key::Named::Enter), "\r".to_owned()),
                        '\t' => (keyboard::Key::Named(key::Named::Tab), "\t".to_owned()),
                        c => (
                            keyboard::Key::Character(SmolStr::new(c.to_string())),
                            c.to_string(),
                        ),
                    };

                    self.keys(window, key, keyboard::Modifiers::empty(), Some(text))?;
                }

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
                return Err(format!(
                    "stopped: the request took longer than {} s",
                    REQUEST_TIMEOUT.as_secs()
                ));
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
                deadline: Instant::now() + ANSWER_TIMEOUT - ANSWER_MARGIN,
                cleanup,
            })
            .map_err(|_| "the app has stopped")?;

        (self.wake)();

        answered
            .recv_timeout(ANSWER_TIMEOUT)
            .map_err(|_| "the app did not answer in time".to_owned())
    }

    /// Whether the client has closed its end, without reading anything it sent.
    fn client_gone(&self) -> bool {
        use rustix::net::{RecvFlags, recv};

        let Some(peer) = &self.peer else {
            return false;
        };

        let mut byte = [0; 1];

        matches!(
            recv(peer, &mut byte, RecvFlags::PEEK | RecvFlags::DONTWAIT),
            Ok(0)
        )
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

    /// Waits a moment for the app to take in what it was just sent. Best effort:
    /// a slow app only makes it give up waiting, never stops what follows.
    fn settle(&self) {
        let started = Instant::now();

        thread::sleep(Duration::from_millis(16));

        while started.elapsed() < SETTLE_TIMEOUT && !self.idle().unwrap_or(true) {
            thread::sleep(Duration::from_millis(16));
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
        let answer = self.send(
            Ask::Inject {
                window,
                cursor,
                events,
            },
            cleanup,
        )?;

        match answer {
            Answer::Injected(true) => {
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

        self.inject(window, Some(from), vec![moved(from)])?;
        self.inject(
            window,
            Some(from),
            vec![Event::Mouse(mouse::Event::ButtonPressed(left))],
        )?;
        self.pressed(window);

        let mut at = from;
        let mut travelled = Ok(());

        for step in 1..=DRAG_STEPS {
            let progress = f32::from(step) / f32::from(DRAG_STEPS);
            let point = if step == DRAG_STEPS {
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

    /// Remembers that the door pressed a button on `window`, for [`LAST_PRESS`].
    fn pressed(&self, window: window::Id) {
        let focused = self.focused().ok().flatten();

        if let Ok(mut last) = LAST_PRESS.lock() {
            *last = Some((window, focused));
        }
    }

    fn focused(&self) -> Result<Option<window::Id>, String> {
        match self.ask(Ask::Focused)? {
            Answer::Focused(window) => Ok(window),
            _ => Err("unexpected answer".into()),
        }
    }

    /// The window named in the request; or else the one the door last clicked,
    /// unless the app's keyboard focus has moved since; or else the one with
    /// keyboard focus.
    fn keyboard_window(&self, request: &Value) -> Result<window::Id, String> {
        if let Some(window) = self.named_window(request)? {
            return Ok(window);
        }

        let focused = self.focused()?;
        let last = LAST_PRESS.lock().ok().and_then(|last| *last);

        if let Some((pressed, focused_then)) = last
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
/// reports the offset as a [`Vector`] through `custom`, right before walking
/// into them, and they are then placed where it paints them. (icetron's
/// `AnimatedTranslate` does this.)
#[derive(Debug)]
pub struct Collector {
    window: window::Id,
    popup: bool,
    nodes: Vec<Node>,
    stack: Vec<(Rectangle, Vector)>,
    viewport: Rectangle,
    translation: Vector,
    /// A painted offset reported through `custom`, for the next `traverse`.
    shift: Option<Vector>,
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
            stack: vec![(viewport, Vector::ZERO)],
            viewport,
            translation: Vector::ZERO,
            shift: None,
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
        // An offset applies to the walk right after it, not to siblings.
        self.shift = None;
        let bounds = bounds + self.translation;

        self.nodes.push(Node {
            window: self.window,
            popup: self.popup,
            kind,
            id: id.and_then(Id::as_str).map(str::to_owned),
            text: text.map(str::to_owned),
            bounds,
            visible: self.viewport.intersection(&bounds),
            focused,
        });
    }
}

impl Operation for Collector {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        self.stack.push((self.viewport, self.translation));
        if let Some(shift) = self.shift.take() {
            self.translation += shift;
        }
        operate(self);
        let _ = self.stack.pop();

        if let Some((viewport, translation)) = self.stack.last() {
            self.viewport = *viewport;
            self.translation = *translation;
        }
    }

    fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
        if id.and_then(Id::as_str).is_some() {
            self.push("named", id, None, bounds, false);
        }
    }

    fn focusable(&mut self, id: Option<&Id>, bounds: Rectangle, state: &mut dyn Focusable) {
        self.push("focusable", id, None, bounds, state.is_focused());
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
        self.shift = state.downcast_ref::<Vector>().copied();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::core::widget::operation::scrollable::{AbsoluteOffset, RelativeOffset};

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "iced-automation-test-{name}-{}",
            std::process::id()
        ));

        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create scratch folder");
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).expect("chmod");

        dir
    }

    fn noop() -> Wake {
        Arc::new(|| {})
    }

    #[test]
    fn stays_shut_without_the_switch() {
        let dir = scratch("no-switch");
        let runtime = dir.join("runtime");
        fs::create_dir_all(&runtime).expect("create runtime folder");

        let opened = start_with(
            &dir.join("automation-enabled"),
            Some(runtime.clone()),
            Some("test"),
            noop(),
        );

        assert!(opened.is_none());
        assert!(
            !runtime.join("iced-automation").exists(),
            "a shut door must leave nothing behind"
        );
        assert!(INBOX.get().is_none());
    }

    #[test]
    fn a_switch_others_can_write_does_not_count() {
        let dir = scratch("writable");
        let switch = dir.join("automation-enabled");
        fs::write(&switch, "").expect("create switch");

        fs::set_permissions(&switch, fs::Permissions::from_mode(0o666)).expect("chmod");
        assert!(!switch_is_on(&switch));

        fs::set_permissions(&switch, fs::Permissions::from_mode(0o644)).expect("chmod");
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o775)).expect("chmod");
        assert!(!switch_is_on(&switch));
    }

    #[test]
    fn a_folder_is_not_a_switch() {
        let dir = scratch("folder");
        let switch = dir.join("automation-enabled");
        fs::create_dir_all(&switch).expect("create folder");
        fs::set_permissions(&switch, fs::Permissions::from_mode(0o755)).expect("chmod");

        assert!(!switch_is_on(&switch));
    }

    #[test]
    fn only_root_can_turn_it_on() {
        let dir = scratch("owner");
        let switch = dir.join("automation-enabled");
        fs::write(&switch, "").expect("create switch");
        fs::set_permissions(&switch, fs::Permissions::from_mode(0o644)).expect("chmod");

        let made_by_root = fs::metadata(&switch).expect("metadata").uid() == 0;

        assert_eq!(switch_is_on(&switch), made_by_root);
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
            !switch_is_on(&switch),
            "anyone may replace a file in {open:?}"
        );

        // A sticky folder (like /nix/store) is fine: others can't replace root's files.
        fs::set_permissions(&open, fs::Permissions::from_mode(0o1777)).expect("chmod");

        let made_by_root = fs::metadata(&target).expect("metadata").uid() == 0;

        assert_eq!(switch_is_on(&switch), made_by_root);

        // A relative link, through `..`, is walked the same way.
        let relative = dir.join("relative");
        std::os::unix::fs::symlink("../link-placeholder/../open/target", &relative).expect("link");
        assert!(!switch_is_on(&relative), "a missing step on the way fails");
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
    fn the_socket_is_private_to_the_user() {
        let dir = scratch("socket");
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

        let _ = thread::spawn(move || {
            while let Ok(job) = inbox.recv() {
                let answer = match job.ask {
                    Ask::Info => Answer::Info(
                        std::iter::once(window)
                            .chain(focused)
                            .map(|surface| Surface {
                                window: surface,
                                popup: false,
                                size: Size::new(800.0, 600.0),
                                scale_factor: 1.0,
                                focused: Some(surface) == focused.or(Some(window)),
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
                    } => {
                        let refused = fails(&events);

                        if !refused {
                            record.lock().expect("record").push((cursor, events));
                            targeted.lock().expect("targets").push(target);
                        }

                        Answer::Injected(!refused)
                    }
                };

                let _ = job.answer.send(answer);
            }
        });

        FakeLoop {
            door: Door {
                jobs,
                wake: noop(),
                app_id: "test".into(),
                peer: None,
                deadline: Cell::new(None),
            },
            injected,
            targets,
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
        assert_eq!(events.len(), 2 + usize::from(DRAG_STEPS) + 1);
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
        let dir = scratch("close");
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
        collector.custom(None, at(200.0), &mut Vector::<f32>::new(0.0, -40.0));
        collector.traverse(&mut |collector| collector.text(None, at(200.0), "Raised"));
        // Its next sibling is where it is laid out.
        collector.text(None, at(240.0), "Below");
        // An offset with no walk after it moves nothing.
        collector.custom(None, at(0.0), &mut Vector::<f32>::new(0.0, 100.0));
        collector.text(None, at(260.0), "Still here");
        // Other custom reports are ignored.
        collector.custom(None, at(0.0), &mut 7_u8);
        collector.traverse(&mut |collector| collector.text(None, at(280.0), "Plain"));

        let ys: Vec<f32> = collector.nodes.iter().map(|node| node.bounds.y).collect();
        assert_eq!(ys, [160.0, 240.0, 260.0, 280.0]);
    }
}
