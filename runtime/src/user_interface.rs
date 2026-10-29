//! Implement your own event loop to drive a user interface.
use crate::core::event::{self, Event};
use crate::core::keyboard;
use crate::core::layout;
use crate::core::mouse;
use crate::core::overlay;
use crate::core::renderer;
use crate::core::text;
use crate::core::widget;
use crate::core::widget::operation::{self, Operation};
use crate::core::window;
use crate::core::{Clipboard, Element, InputMethod, Layout, Point, Rectangle, Shell, Size, Vector};

/// A set of interactive graphical elements with a specific [`Layout`].
///
/// It can be updated and drawn.
///
/// Iced tries to avoid dictating how to write your event loop. You are in
/// charge of using this type in your system in any way you want.
///
/// # Example
/// The [`integration`] example uses a [`UserInterface`] to integrate Iced in an
/// existing graphical application.
///
/// [`integration`]: https://github.com/iced-rs/iced/tree/master/examples/integration
pub struct UserInterface<'a, Message, Theme, Renderer> {
    root: Element<'a, Message, Theme, Renderer>,
    base: layout::Node,
    state: widget::Tree,
    overlay: Option<Overlay>,
    bounds: Size,
    selection: Selection,
}

struct Overlay {
    layout: layout::Node,
    interaction: mouse::Interaction,
}

impl<'a, Message, Theme, Renderer> UserInterface<'a, Message, Theme, Renderer>
where
    Renderer: crate::core::Renderer,
{
    /// Builds a user interface for an [`Element`].
    ///
    /// It is able to avoid expensive computations when using a [`Cache`]
    /// obtained from a previous instance of a [`UserInterface`].
    ///
    /// # Example
    /// Imagine we want to build a [`UserInterface`] for
    /// [the counter example that we previously wrote](index.html#usage). Here
    /// is naive way to set up our application loop:
    ///
    /// ```no_run
    /// # mod iced_wgpu {
    /// #     pub type Renderer = ();
    /// # }
    /// #
    /// # pub struct Counter;
    /// #
    /// # impl Counter {
    /// #     pub fn new() -> Self { Counter }
    /// #     pub fn view(&self) -> iced_core::Element<(), (), Renderer> { unimplemented!() }
    /// #     pub fn update(&mut self, _: ()) {}
    /// # }
    /// use iced_runtime::core::Size;
    /// use iced_runtime::user_interface::{self, UserInterface};
    /// use iced_wgpu::Renderer;
    ///
    /// // Initialization
    /// let mut counter = Counter::new();
    /// let mut cache = user_interface::Cache::new();
    /// let mut renderer = Renderer::default();
    /// let mut window_size = Size::new(1024.0, 768.0);
    ///
    /// // Application loop
    /// loop {
    ///     // Process system events here...
    ///
    ///     // Build the user interface
    ///     let user_interface = UserInterface::build(
    ///         counter.view(),
    ///         window_size,
    ///         cache,
    ///         &mut renderer,
    ///     );
    ///
    ///     // Update and draw the user interface here...
    ///     // ...
    ///
    ///     // Obtain the cache for the next iteration
    ///     cache = user_interface.into_cache();
    /// }
    /// ```
    pub fn build<E: Into<Element<'a, Message, Theme, Renderer>>>(
        root: E,
        bounds: Size,
        cache: Cache,
        renderer: &mut Renderer,
    ) -> Self {
        let mut root = root.into();

        let Cache {
            mut state,
            selection,
        } = cache;
        state.diff(root.as_widget());

        let base = root.as_widget_mut().layout(
            &mut state,
            renderer,
            &layout::Limits::new(Size::ZERO, bounds),
        );

        UserInterface {
            root,
            base,
            state,
            overlay: None,
            bounds,
            selection,
        }
    }

    /// Updates the [`UserInterface`] by processing each provided [`Event`].
    ///
    /// It returns __messages__ that may have been produced as a result of user
    /// interactions. You should feed these to your __update logic__.
    ///
    /// # Example
    /// Let's allow our [counter](index.html#usage) to change state by
    /// completing [the previous example](#example):
    ///
    /// ```no_run
    /// # mod iced_wgpu {
    /// #     pub type Renderer = ();
    /// # }
    /// #
    /// # pub struct Counter;
    /// #
    /// # impl Counter {
    /// #     pub fn new() -> Self { Counter }
    /// #     pub fn view(&self) -> iced_core::Element<(), (), Renderer> { unimplemented!() }
    /// #     pub fn update(&mut self, _: ()) {}
    /// # }
    /// use iced_runtime::core::mouse;
    /// use iced_runtime::core::Size;
    /// use iced_runtime::user_interface::{self, UserInterface};
    /// use iced_wgpu::Renderer;
    ///
    /// let mut counter = Counter::new();
    /// let mut cache = user_interface::Cache::new();
    /// let mut renderer = Renderer::default();
    /// let mut window_size = Size::new(1024.0, 768.0);
    /// let mut cursor = mouse::Cursor::default();
    ///
    /// // Initialize our event storage
    /// let mut events = Vec::new();
    /// let mut messages = Vec::new();
    ///
    /// loop {
    ///     // Obtain system events...
    ///
    ///     let mut user_interface = UserInterface::build(
    ///         counter.view(),
    ///         window_size,
    ///         cache,
    ///         &mut renderer,
    ///     );
    ///
    ///     // Update the user interface
    ///     let (state, event_statuses) = user_interface.update(
    ///         &events,
    ///         cursor,
    ///         &mut renderer,
    ///         &mut messages
    ///     );
    ///
    ///     cache = user_interface.into_cache();
    ///
    ///     // Process the produced messages
    ///     for message in messages.drain(..) {
    ///         counter.update(message);
    ///     }
    /// }
    /// ```
    pub fn update(
        &mut self,
        events: &[Event],
        cursor: mouse::Cursor,
        renderer: &mut Renderer,
        messages: &mut Vec<Message>,
    ) -> (State, Vec<event::Status>) {
        let mut outdated = false;
        let mut redraw_request = window::RedrawRequest::Wait;
        let mut input_method = InputMethod::Disabled;
        let mut clipboard = Clipboard::new();
        let mut has_layout_changed = false;
        let mut auto_focus_pending = false;
        let viewport = Rectangle::with_size(self.bounds);

        let mut maybe_overlay = self
            .root
            .as_widget_mut()
            .overlay(
                &mut self.state,
                Layout::new(&self.base),
                renderer,
                &viewport,
                Vector::ZERO,
            )
            .map(overlay::Nested::new);

        let (base_cursor, overlay_statuses, overlay_interaction) = if maybe_overlay.is_some() {
            let bounds = self.bounds;

            let mut overlay = maybe_overlay.as_mut().unwrap();
            let mut layout = overlay.layout(renderer, bounds);
            let mut event_statuses = Vec::new();

            for event in events {
                let mut shell = Shell::new(messages);

                overlay.update(event, Layout::new(&layout), cursor, renderer, &mut shell);

                event_statuses.push(shell.event_status());
                redraw_request = redraw_request.min(shell.redraw_request());
                input_method.merge(shell.input_method());
                clipboard.merge(shell.clipboard_mut());

                if shell.is_layout_invalid() {
                    drop(maybe_overlay);

                    self.base = self.root.as_widget_mut().layout(
                        &mut self.state,
                        renderer,
                        &layout::Limits::new(Size::ZERO, self.bounds),
                    );

                    maybe_overlay = self
                        .root
                        .as_widget_mut()
                        .overlay(
                            &mut self.state,
                            Layout::new(&self.base),
                            renderer,
                            &viewport,
                            Vector::ZERO,
                        )
                        .map(overlay::Nested::new);

                    if maybe_overlay.is_none() {
                        break;
                    }

                    overlay = maybe_overlay.as_mut().unwrap();

                    shell.revalidate_layout(|| {
                        layout = overlay.layout(renderer, bounds);
                        has_layout_changed = true;
                    });
                }

                if shell.are_widgets_invalid() {
                    outdated = true;
                }

                if shell.is_auto_focus_pending() {
                    auto_focus_pending = true;
                }
            }

            let (base_cursor, interaction) = if let Some(overlay) = maybe_overlay.as_mut() {
                let interaction = cursor
                    .position()
                    .map(|cursor_position| {
                        overlay.mouse_interaction(
                            Layout::new(&layout),
                            mouse::Cursor::Available(cursor_position),
                            renderer,
                        )
                    })
                    .unwrap_or_default();

                if interaction == mouse::Interaction::None {
                    (cursor, mouse::Interaction::None)
                } else {
                    (mouse::Cursor::Unavailable, interaction)
                }
            } else {
                (cursor, mouse::Interaction::None)
            };

            self.overlay = Some(Overlay {
                layout,
                interaction,
            });

            (base_cursor, event_statuses, interaction)
        } else {
            (
                cursor,
                vec![event::Status::Ignored; events.len()],
                mouse::Interaction::None,
            )
        };

        drop(maybe_overlay);

        // The latest drag to apply, kept apart from `self.selection.drag` so a
        // release in the same batch as the last move doesn't drop it.
        let mut reselect = None;
        let mut select_all = false;
        let mut publish_primary = false;

        let event_statuses = events
            .iter()
            .zip(overlay_statuses)
            .map(|(event, overlay_status)| {
                if matches!(overlay_status, event::Status::Captured) {
                    return overlay_status;
                }

                let mut shell = Shell::new(messages);

                self.root.as_widget_mut().update(
                    &mut self.state,
                    event,
                    Layout::new(&self.base),
                    base_cursor,
                    renderer,
                    &mut shell,
                    &viewport,
                );

                if shell.event_status() == event::Status::Captured {
                    self.overlay = None;
                }

                redraw_request = redraw_request.min(shell.redraw_request());
                input_method.merge(shell.input_method());
                {
                    let shell_cb = shell.clipboard_mut();
                    clipboard.merge(shell_cb);
                }

                shell.revalidate_layout(|| {
                    has_layout_changed = true;

                    self.base = self.root.as_widget_mut().layout(
                        &mut self.state,
                        renderer,
                        &layout::Limits::new(Size::ZERO, self.bounds),
                    );

                    if let Some(mut overlay) = self
                        .root
                        .as_widget_mut()
                        .overlay(
                            &mut self.state,
                            Layout::new(&self.base),
                            renderer,
                            &viewport,
                            Vector::ZERO,
                        )
                        .map(overlay::Nested::new)
                    {
                        let layout = overlay.layout(renderer, self.bounds);
                        let interaction =
                            overlay.mouse_interaction(Layout::new(&layout), cursor, renderer);

                        self.overlay = Some(Overlay {
                            layout,
                            interaction,
                        });
                    }
                });

                if shell.are_widgets_invalid() {
                    outdated = true;
                }

                if shell.is_auto_focus_pending() {
                    auto_focus_pending = true;
                }

                let status = shell.event_status().merge(overlay_status);

                // Text selection belongs to the runtime: it starts wherever no
                // widget took the press, and crosses the selectable widgets in
                // the scrollable it started in.
                match event {
                    Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                        self.selection.modifiers = *modifiers;
                    }
                    Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                        if status == event::Status::Ignored =>
                    {
                        if let Some(position) = base_cursor.position() {
                            let click = mouse::Click::new(
                                position,
                                mouse::Button::Left,
                                self.selection.last_click,
                            );

                            let start = self
                                .selection
                                .anchor
                                .filter(|_| {
                                    self.selection.modifiers.shift()
                                        && click.kind() == mouse::click::Kind::Single
                                })
                                .unwrap_or(position);

                            self.selection.last_click = Some(click);
                            self.selection.anchor = Some(start);
                            self.selection.drag = Some((start, position));
                            self.selection.target = match click.kind() {
                                mouse::click::Kind::Single => text::Target::Character,
                                mouse::click::Kind::Double => text::Target::Word,
                                mouse::click::Kind::Triple => text::Target::Line,
                            };
                            reselect = self.selection.drag;
                        }
                    }
                    Event::Mouse(mouse::Event::CursorMoved { position }) => {
                        if let Some((_, end)) = self.selection.drag.as_mut()
                            && end != position
                        {
                            *end = *position;
                            reselect = self.selection.drag;
                        }
                    }
                    Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                        if self.selection.drag.take().is_some() =>
                    {
                        publish_primary = true;
                    }
                    Event::Keyboard(keyboard::Event::KeyPressed {
                        key: keyboard::Key::Character(c),
                        modifiers,
                        repeat: false,
                        ..
                    }) if status == event::Status::Ignored
                        && modifiers.command()
                        && !modifiers.alt()
                        && !modifiers.shift() =>
                    {
                        match c.as_str() {
                            // A widget's own copy, such as an app-managed
                            // selection's, wins.
                            "c" if clipboard.write.is_none() => {
                                if let Some(text) = self.copy_selection(renderer) {
                                    clipboard.write =
                                        Some(crate::core::clipboard::Content::Text(text));
                                }
                            }
                            // Only once the selection has been used, so the
                            // shortcut doesn't paint a window that never asked.
                            "a" if self.selection.anchor.is_some() => select_all = true,
                            _ => {}
                        }
                    }
                    _ => {}
                }

                status
            })
            .collect();

        if let Some((start, end)) = reselect {
            let scope = self.selection_scope(renderer, start);
            let mut select = operation::selection::select(start, end, self.selection.target, scope);
            self.operate(renderer, &mut select);
            redraw_request = window::RedrawRequest::NextFrame;
        }

        if select_all && let Some(anchor) = self.selection.anchor {
            let scope = self.selection_scope(renderer, anchor);
            self.operate(renderer, &mut operation::selection::select_all(scope));
            redraw_request = window::RedrawRequest::NextFrame;
        }

        if (publish_primary || select_all)
            && let Some(text) = self.copy_selection(renderer)
        {
            clipboard.primary_write = Some(text);
        }

        let mouse_interaction = if overlay_interaction == mouse::Interaction::None {
            self.root.as_widget().mouse_interaction(
                &self.state,
                Layout::new(&self.base),
                base_cursor,
                &viewport,
                renderer,
            )
        } else {
            overlay_interaction
        };

        (
            if outdated {
                State::Outdated {
                    clipboard,
                    auto_focus_pending,
                }
            } else {
                State::Updated {
                    mouse_interaction,
                    redraw_request,
                    input_method,
                    clipboard,
                    has_layout_changed,
                    auto_focus_pending,
                }
            },
            event_statuses,
        )
    }

    /// Draws the [`UserInterface`] with the provided [`Renderer`].
    ///
    /// It returns the current [`mouse::Interaction`]. You should update the
    /// icon of the mouse cursor accordingly in your system.
    ///
    /// [`Renderer`]: crate::core::Renderer
    ///
    /// # Example
    /// We can finally draw our [counter](index.html#usage) by
    /// [completing the last example](#example-1):
    ///
    /// ```no_run
    /// # mod iced_wgpu {
    /// #     pub type Renderer = ();
    /// #     pub type Theme = ();
    /// # }
    /// #
    /// # pub struct Counter;
    /// #
    /// # impl Counter {
    /// #     pub fn new() -> Self { Counter }
    /// #     pub fn view(&self) -> Element<(), (), Renderer> { unimplemented!() }
    /// #     pub fn update(&mut self, _: ()) {}
    /// # }
    /// use iced_runtime::core::mouse;
    /// use iced_runtime::core::renderer;
    /// use iced_runtime::core::{Element, Size};
    /// use iced_runtime::user_interface::{self, UserInterface};
    /// use iced_wgpu::{Renderer, Theme};
    ///
    /// let mut counter = Counter::new();
    /// let mut cache = user_interface::Cache::new();
    /// let mut renderer = Renderer::default();
    /// let mut window_size = Size::new(1024.0, 768.0);
    /// let mut cursor = mouse::Cursor::default();
    /// let mut events = Vec::new();
    /// let mut messages = Vec::new();
    /// let mut theme = Theme::default();
    ///
    /// loop {
    ///     // Obtain system events...
    ///
    ///     let mut user_interface = UserInterface::build(
    ///         counter.view(),
    ///         window_size,
    ///         cache,
    ///         &mut renderer,
    ///     );
    ///
    ///     // Update the user interface
    ///     let event_statuses = user_interface.update(
    ///         &events,
    ///         cursor,
    ///         &mut renderer,
    ///         &mut messages
    ///     );
    ///
    ///     // Draw the user interface
    ///     let mouse_interaction = user_interface.draw(&mut renderer, &theme, &renderer::Style::default(), cursor);
    ///
    ///     cache = user_interface.into_cache();
    ///
    ///     for message in messages.drain(..) {
    ///         counter.update(message);
    ///     }
    ///
    ///     // Update mouse cursor icon...
    ///     // Flush rendering operations...
    /// }
    /// ```
    pub fn draw(
        &mut self,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        cursor: mouse::Cursor,
    ) {
        let viewport = Rectangle::with_size(self.bounds);
        renderer.reset(viewport);

        let base_cursor = match &self.overlay {
            None
            | Some(Overlay {
                interaction: mouse::Interaction::None,
                ..
            }) => cursor,
            _ => mouse::Cursor::Unavailable,
        };

        self.root.as_widget().draw(
            &self.state,
            renderer,
            theme,
            style,
            Layout::new(&self.base),
            base_cursor,
            &viewport,
        );

        let Self {
            overlay,
            root,
            base,
            ..
        } = self;

        let Some(Overlay { layout, .. }) = overlay.as_ref() else {
            return;
        };

        let overlay = root
            .as_widget_mut()
            .overlay(
                &mut self.state,
                Layout::new(base),
                renderer,
                &viewport,
                Vector::ZERO,
            )
            .map(overlay::Nested::new);

        if let Some(mut overlay) = overlay {
            // Above everything, including anything the base tree is holding
            // back for the pass that follows the backdrop blur — which is
            // drawn last and so would otherwise cover it. A dialog on a pane
            // of frosted glass is exactly that, and a menu opened inside one
            // was being painted over by the card it belonged to: built, sized,
            // taking clicks where it should be, and invisible.
            //
            // Only when there is such a pass to join. Deferring content is
            // what makes the renderer copy the scene and composite it, and an
            // interface with no blur in it should not start paying for that
            // the moment a tooltip appears.
            if renderer.has_post_blur_content() {
                renderer.with_post_blur_layer(viewport, |renderer| {
                    overlay.draw(renderer, theme, style, Layout::new(layout), cursor);
                });
            } else {
                overlay.draw(renderer, theme, style, Layout::new(layout), cursor);
            }
        }
    }

    /// Applies a [`widget::Operation`] to the [`UserInterface`].
    pub fn operate(&mut self, renderer: &Renderer, operation: &mut dyn widget::Operation) {
        let viewport = Rectangle::with_size(self.bounds);

        self.root.as_widget_mut().operate(
            &mut self.state,
            Layout::new(&self.base),
            renderer,
            operation,
        );

        if let Some(mut overlay) = self
            .root
            .as_widget_mut()
            .overlay(
                &mut self.state,
                Layout::new(&self.base),
                renderer,
                &viewport,
                Vector::ZERO,
            )
            .map(overlay::Nested::new)
        {
            if self.overlay.is_none() {
                self.overlay = Some(Overlay {
                    layout: overlay.layout(renderer, self.bounds),
                    interaction: mouse::Interaction::None,
                });
            }

            overlay.operate(
                Layout::new(&self.overlay.as_ref().unwrap().layout),
                renderer,
                operation,
            );
        }
    }

    /// Where a selection that starts at `start` may reach.
    fn selection_scope(
        &mut self,
        renderer: &Renderer,
        start: Point,
    ) -> operation::selection::Scope {
        let mut scope = operation::selection::scope(start);
        self.operate(renderer, &mut operation::black_box(&mut scope));

        match scope.finish() {
            operation::Outcome::Some(scope) => scope,
            _ => operation::selection::Scope::default(),
        }
    }

    /// The text selected across the selectable widgets, one line per widget.
    fn copy_selection(&mut self, renderer: &Renderer) -> Option<String> {
        let mut copy = operation::selection::copy();
        self.operate(renderer, &mut operation::black_box(&mut copy));

        match copy.finish() {
            operation::Outcome::Some(text) => Some(text),
            _ => None,
        }
    }

    /// Relayouts and returns a new  [`UserInterface`] using the provided
    /// bounds.
    pub fn relayout(self, bounds: Size, renderer: &mut Renderer) -> Self {
        Self::build(
            self.root,
            bounds,
            Cache {
                state: self.state,
                selection: self.selection,
            },
            renderer,
        )
    }

    /// Returns the size of the root content after layout.
    ///
    /// This is the actual size that the content wants to be, which may be
    /// smaller than the bounds passed to `build` or `relayout`.
    pub fn content_size(&self) -> Size {
        self.base.size()
    }

    /// Extract the [`Cache`] of the [`UserInterface`], consuming it in the
    /// process.
    pub fn into_cache(self) -> Cache {
        Cache {
            state: self.state,
            selection: self.selection,
        }
    }
}

/// Reusable data of a specific [`UserInterface`].
#[derive(Debug)]
pub struct Cache {
    state: widget::Tree,
    selection: Selection,
}

/// The text selection the runtime drives across selectable widgets.
#[derive(Debug, Clone, Copy, Default)]
struct Selection {
    last_click: Option<mouse::Click>,
    /// Where the latest selection started, so a shift-click can extend it.
    anchor: Option<Point>,
    /// The drag in progress: where it started and where the cursor is now.
    drag: Option<(Point, Point)>,
    target: text::Target,
    modifiers: keyboard::Modifiers,
}

impl Cache {
    /// Creates an empty [`Cache`].
    ///
    /// You should use this to initialize a [`Cache`] before building your first
    /// [`UserInterface`].
    pub fn new() -> Cache {
        Cache {
            state: widget::Tree::empty(),
            selection: Selection::default(),
        }
    }
}

impl Default for Cache {
    fn default() -> Cache {
        Cache::new()
    }
}

/// The resulting state after updating a [`UserInterface`].
#[derive(Debug)]
pub enum State {
    /// The [`UserInterface`] is outdated and needs to be rebuilt.
    ///
    /// Clipboard requests from the update cycle are still preserved so they
    /// can be processed even when the widget tree needs rebuilding.
    Outdated {
        /// The set of [`Clipboard`] requests that the user interface has produced.
        clipboard: Clipboard,
        /// Whether an auto-focus operation should be run after the tree is rebuilt.
        auto_focus_pending: bool,
    },

    /// The [`UserInterface`] is up-to-date and can be reused without
    /// rebuilding.
    Updated {
        /// The current [`mouse::Interaction`] of the user interface.
        mouse_interaction: mouse::Interaction,
        /// The [`window::RedrawRequest`] describing when a redraw should be performed.
        redraw_request: window::RedrawRequest,
        /// The current [`InputMethod`] strategy of the user interface.
        input_method: InputMethod,
        /// The set of [`Clipboard`] requests that the user interface has produced.
        clipboard: Clipboard,
        /// Whether the layout of the [`UserInterface`] has changed.
        has_layout_changed: bool,
        /// Whether an auto-focus operation should be run after the tree is rebuilt.
        auto_focus_pending: bool,
    },
}

impl State {
    /// Returns whether the layout of the [`UserInterface`] has changed.
    pub fn has_layout_changed(&self) -> bool {
        match self {
            State::Outdated { .. } => true,
            State::Updated {
                has_layout_changed, ..
            } => *has_layout_changed,
        }
    }

    /// Returns whether an auto-focus operation is pending.
    pub fn is_auto_focus_pending(&self) -> bool {
        match self {
            State::Outdated {
                auto_focus_pending, ..
            }
            | State::Updated {
                auto_focus_pending, ..
            } => *auto_focus_pending,
        }
    }
}
