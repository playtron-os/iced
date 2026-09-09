//! Buttons allow your users to perform actions by pressing them.
//!
//! # Example
//! ```no_run
//! # mod iced { pub mod widget { pub use iced_widget::*; } }
//! # pub type State = ();
//! # pub type Element<'a, Message> = iced_widget::core::Element<'a, Message, iced_widget::Theme, iced_widget::Renderer>;
//! use iced::widget::button;
//!
//! #[derive(Clone)]
//! enum Message {
//!     ButtonPressed,
//! }
//!
//! fn view(state: &State) -> Element<'_, Message> {
//!     button("Press me!").on_press(Message::ButtonPressed).into()
//! }
//! ```
use crate::core::animation::Easing;
use crate::core::border::{self, Border};
use crate::core::keyboard;
use crate::core::keyboard::key;
use crate::core::layout;
use crate::core::mouse;
use crate::core::overlay;
use crate::core::renderer;
use crate::core::theme::palette;
use crate::core::time::{Duration, Instant};
use crate::core::touch;
use crate::core::widget::Id;
use crate::core::widget::operation::{self, Operation};
use crate::core::widget::tree::{self, Tree};
use crate::core::window;
use crate::core::{
    Background, Color, Element, Event, Layout, Length, Padding, Point, Rectangle, Shadow, Shell,
    Size, Theme, Vector, Widget,
};

use std::any::Any;

/// Distance (in logical pixels) a finger may travel after pressing a button
/// before the touch is treated as a scroll/drag rather than a tap. Kept in sync
/// with the drag threshold used by scrollable containers so a swipe that begins
/// on a button scrolls instead of activating it.
const TOUCH_DRAG_SLOP: f32 = 8.0;

/// A generic widget that produces a message when pressed.
///
/// # Example
/// ```no_run
/// # mod iced { pub mod widget { pub use iced_widget::*; } }
/// # pub type State = ();
/// # pub type Element<'a, Message> = iced_widget::core::Element<'a, Message, iced_widget::Theme, iced_widget::Renderer>;
/// use iced::widget::button;
///
/// #[derive(Clone)]
/// enum Message {
///     ButtonPressed,
/// }
///
/// fn view(state: &State) -> Element<'_, Message> {
///     button("Press me!").on_press(Message::ButtonPressed).into()
/// }
/// ```
///
/// If a [`Button::on_press`] handler is not set, the resulting [`Button`] will
/// be disabled:
///
/// ```no_run
/// # mod iced { pub mod widget { pub use iced_widget::*; } }
/// # pub type State = ();
/// # pub type Element<'a, Message> = iced_widget::core::Element<'a, Message, iced_widget::Theme, iced_widget::Renderer>;
/// use iced::widget::button;
///
/// #[derive(Clone)]
/// enum Message {
///     ButtonPressed,
/// }
///
/// fn view(state: &State) -> Element<'_, Message> {
///     button("I am disabled!").into()
/// }
/// ```
pub struct Button<'a, Message, Theme = crate::Theme, Renderer = crate::Renderer>
where
    Renderer: crate::core::Renderer,
    Theme: Catalog,
{
    content: Element<'a, Message, Theme, Renderer>,
    on_press: Option<OnPress<'a, Message>>,
    width: Length,
    height: Length,
    padding: Padding,
    clip: bool,
    class: Theme::Class<'a>,
    status: Option<Status>,
    animate_background: Option<(Duration, Easing)>,
    snap_fill: bool,
}

enum OnPress<'a, Message> {
    Direct(Message),
    Closure(Box<dyn Fn() -> Message + 'a>),
}

impl<Message: Clone> OnPress<'_, Message> {
    fn get(&self) -> Message {
        match self {
            OnPress::Direct(message) => message.clone(),
            OnPress::Closure(f) => f(),
        }
    }
}

impl<'a, Message, Theme, Renderer> Button<'a, Message, Theme, Renderer>
where
    Renderer: crate::core::Renderer,
    Theme: Catalog,
{
    /// Creates a new [`Button`] with the given content.
    pub fn new(content: impl Into<Element<'a, Message, Theme, Renderer>>) -> Self {
        let content = content.into();
        let size = content.as_widget().size_hint();

        Button {
            content,
            on_press: None,
            width: size.width.fluid(),
            height: size.height.fluid(),
            padding: DEFAULT_PADDING,
            clip: false,
            class: Theme::default(),
            status: None,
            animate_background: None,
            snap_fill: false,
        }
    }

    /// Sets the width of the [`Button`].
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    /// Sets the height of the [`Button`].
    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.height = height.into();
        self
    }

    /// Sets the [`Padding`] of the [`Button`].
    pub fn padding<P: Into<Padding>>(mut self, padding: P) -> Self {
        self.padding = padding.into();
        self
    }

    /// Sets the message that will be produced when the [`Button`] is pressed.
    ///
    /// Unless `on_press` is called, the [`Button`] will be disabled.
    pub fn on_press(mut self, on_press: Message) -> Self {
        self.on_press = Some(OnPress::Direct(on_press));
        self
    }

    /// Sets the message that will be produced when the [`Button`] is pressed.
    ///
    /// This is analogous to [`Button::on_press`], but using a closure to produce
    /// the message.
    ///
    /// This closure will only be called when the [`Button`] is actually pressed and,
    /// therefore, this method is useful to reduce overhead if creating the resulting
    /// message is slow.
    pub fn on_press_with(mut self, on_press: impl Fn() -> Message + 'a) -> Self {
        self.on_press = Some(OnPress::Closure(Box::new(on_press)));
        self
    }

    /// Sets the message that will be produced when the [`Button`] is pressed,
    /// if `Some`.
    ///
    /// If `None`, the [`Button`] will be disabled.
    pub fn on_press_maybe(mut self, on_press: Option<Message>) -> Self {
        self.on_press = on_press.map(OnPress::Direct);
        self
    }

    /// Sets whether the contents of the [`Button`] should be clipped on
    /// overflow.
    pub fn clip(mut self, clip: bool) -> Self {
        self.clip = clip;
        self
    }

    /// Sets the style of the [`Button`].
    #[must_use]
    pub fn style(mut self, style: impl Fn(&Theme, Status) -> Style + 'a) -> Self
    where
        Theme::Class<'a>: From<StyleFn<'a, Theme>>,
    {
        self.class = (Box::new(style) as StyleFn<'a, Theme>).into();
        self
    }

    /// Sets the style class of the [`Button`].
    #[cfg(feature = "advanced")]
    #[must_use]
    pub fn class(mut self, class: impl Into<Theme::Class<'a>>) -> Self {
        self.class = class.into();
        self
    }

    /// Enables smooth background color animation over the given duration.
    ///
    /// When enabled, background color changes (e.g. on hover/press) will
    /// smoothly interpolate instead of snapping instantly.
    ///
    /// Uses [`Easing::EaseOutCubic`] by default.
    #[must_use]
    pub fn animate_background(mut self, duration: Duration) -> Self {
        self.animate_background = Some((duration, Easing::EaseOutCubic));
        self
    }

    /// Enables smooth background color animation with a custom [`Easing`].
    #[must_use]
    pub fn animate_background_with_easing(mut self, duration: Duration, easing: Easing) -> Self {
        self.animate_background = Some((duration, easing));
        self
    }

    /// Takes the background straight on a status change, leaving only the
    /// border and shadow to tween — Material animates its shape, not its fill.
    #[must_use]
    pub fn snap_fill(mut self, snap: bool) -> Self {
        self.snap_fill = snap;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct State {
    is_pressed: bool,
    /// Finger position where a touch press landed, for the tap-vs-drag slop.
    press_origin: Point,
    is_focused: bool,
    /// Painted as focused on a parent's behalf (e.g. an enclosing focus ring),
    /// without becoming a focus target: styling only, never key activation.
    style_focused: bool,
    /// The status last seen by `update`, kept here rather than on the widget so
    /// it survives the tree rebuild that follows every message.
    last_status: Option<Status>,
    /// The previous status before the last transition (for animation).
    previous_status: Option<Status>,
    /// When the last status transition started.
    transition_start: Option<Instant>,
}

impl operation::Focusable for State {
    fn is_focused(&self) -> bool {
        self.is_focused
    }

    fn focus(&mut self) {
        self.is_focused = true;
    }

    fn unfocus(&mut self) {
        self.is_focused = false;
    }
}

impl<'a, Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for Button<'a, Message, Theme, Renderer>
where
    Message: 'a + Clone,
    Renderer: 'a + crate::core::Renderer,
    Theme: Catalog,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: self.width,
            height: self.height,
        }
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::padded(limits, self.width, self.height, self.padding, |limits| {
            self.content
                .as_widget_mut()
                .layout(&mut tree.children[0], renderer, limits)
        })
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        let state = tree.state.downcast_mut::<State>();

        operation.focusable(None, layout.bounds(), state);
        operation.custom(None, layout.bounds(), state);
        operation.container(None, layout.bounds());
        operation.traverse(&mut |operation| {
            self.content.as_widget_mut().operate(
                &mut tree.children[0],
                layout.children().next().unwrap(),
                renderer,
                operation,
            );
        });
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout.children().next().unwrap(),
            cursor,
            renderer,
            shell,
            viewport,
        );

        if shell.is_event_captured() {
            return;
        }

        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                if self.on_press.is_some() =>
            {
                let bounds = layout.bounds();

                if cursor.is_over(bounds) {
                    let state = tree.state.downcast_mut::<State>();

                    state.is_pressed = true;

                    shell.capture_event();
                }
            }
            // A finger press only *arms* the button; unlike the mouse it does not
            // capture the event, so an enclosing scrollable can still turn the
            // same press into a drag-scroll. The tap fires on lift (below) and is
            // cancelled by `FingerMoved` once the finger travels past the slop.
            Event::Touch(touch::Event::FingerPressed { position, .. })
                if self.on_press.is_some() =>
            {
                let bounds = layout.bounds();

                if cursor.is_over(bounds) {
                    let state = tree.state.downcast_mut::<State>();

                    state.is_pressed = true;
                    state.press_origin = *position;
                }
            }
            Event::Touch(touch::Event::FingerMoved { position, .. }) => {
                let state = tree.state.downcast_mut::<State>();

                if state.is_pressed && state.press_origin.distance(*position) > TOUCH_DRAG_SLOP {
                    // The gesture became a scroll/drag — no longer a tap.
                    state.is_pressed = false;
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
            | Event::Touch(touch::Event::FingerLifted { .. }) => {
                if let Some(on_press) = &self.on_press {
                    let state = tree.state.downcast_mut::<State>();

                    if state.is_pressed {
                        state.is_pressed = false;

                        let bounds = layout.bounds();

                        if cursor.is_over(bounds) {
                            shell.publish(on_press.get());
                        }

                        shell.capture_event();
                    }
                }
            }
            Event::Touch(touch::Event::FingerLost { .. }) => {
                let state = tree.state.downcast_mut::<State>();

                state.is_pressed = false;
            }
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(key::Named::Enter | key::Named::Space),
                ..
            }) => {
                let state = tree.state.downcast_ref::<State>();

                if let Some(on_press) = self.on_press.as_ref().filter(|_| state.is_focused) {
                    shell.publish(on_press.get());
                    shell.capture_event();
                }
            }
            _ => {}
        }

        let current_status = if self.on_press.is_none() {
            Status::Disabled
        } else if cursor.is_over(layout.bounds()) {
            let state = tree.state.downcast_ref::<State>();

            if state.is_pressed {
                Status::Pressed
            } else {
                Status::Hovered
            }
        } else {
            let state = tree.state.downcast_ref::<State>();

            if state.is_focused || state.style_focused {
                Status::Focused
            } else {
                Status::Active
            }
        };

        let state = tree.state.downcast_mut::<State>();
        let status_changed = state.last_status != Some(current_status);

        // Recorded on any status change, redraws included: focus arrives through an
        // operation, so the first update to see it is usually the next redraw.
        if status_changed {
            if self.animate_background.is_some() && state.last_status.is_some() {
                state.previous_status = state.last_status;
                state.transition_start = Some(Instant::now());
            }

            state.last_status = Some(current_status);
        }

        if let Event::Window(window::Event::RedrawRequested(_now)) = event {
            // Keep the frames coming while a transition is in flight.
            if let Some((duration, _)) = self.animate_background
                && let Some(start) = state.transition_start
                && start.elapsed() < duration
            {
                shell.request_redraw();
            }

            self.status = Some(current_status);
        } else if status_changed {
            shell.request_redraw();
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let content_layout = layout.children().next().unwrap();
        let current_status = self.status.unwrap_or(Status::Disabled);
        let mut style = theme.style(&self.class, current_status);

        // Border and shadow always tween; the background joins them unless
        // `snap_fill` takes it straight, as Material animates shape, not fill.
        if let Some((duration, easing)) = self.animate_background {
            let state = tree.state.downcast_ref::<State>();

            if let (Some(start), Some(prev_status)) =
                (state.transition_start, state.previous_status)
                && start.elapsed() < duration
            {
                let progress = (start.elapsed().as_secs_f32() / duration.as_secs_f32()).min(1.0);
                let eased = easing.value(progress);

                let prev_style = theme.style(&self.class, prev_status);

                if !self.snap_fill {
                    style.background =
                        Background::lerp_maybe(prev_style.background, style.background, eased);
                }

                style.border = prev_style.border.lerp(style.border, eased);
                style.shadow = prev_style.shadow.lerp(style.shadow, eased);
            }
        }

        if style.background.is_some() || style.border.width > 0.0 || style.shadow.color.a > 0.0 {
            renderer.fill_quad(
                renderer::Quad {
                    bounds,
                    border: style.border,
                    shadow: style.shadow,
                    snap: style.snap,
                    border_only: false,
                },
                style
                    .background
                    .unwrap_or(Background::Color(Color::TRANSPARENT)),
            );
        }

        let viewport = if self.clip {
            bounds.intersection(viewport).unwrap_or(*viewport)
        } else {
            *viewport
        };

        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            &renderer::Style {
                text_color: style.text_color,
            },
            content_layout,
            cursor,
            &viewport,
        );
    }

    fn mouse_interaction(
        &self,
        _tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        let is_mouse_over = cursor.is_over(layout.bounds());

        if is_mouse_over && self.on_press.is_some() {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout.children().next().unwrap(),
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, Message, Theme, Renderer> From<Button<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: Clone + 'a,
    Theme: Catalog + 'a,
    Renderer: crate::core::Renderer + 'a,
{
    fn from(button: Button<'a, Message, Theme, Renderer>) -> Self {
        Self::new(button)
    }
}

/// Produces an [`Operation`] that paints every [`Button`] in the subtree it runs over as
/// [`Status::Focused`] — styling only, so `Enter`/`Space` still need real iced focus.
pub fn style_focus<T>(focused: bool) -> impl Operation<T> {
    struct StyleFocus {
        focused: bool,
    }

    impl<T> Operation<T> for StyleFocus {
        fn custom(&mut self, _id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Any) {
            if let Some(state) = state.downcast_mut::<State>() {
                state.style_focused = self.focused;
            }
        }

        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<T>)) {
            operate(self);
        }
    }

    StyleFocus { focused }
}

/// The default [`Padding`] of a [`Button`].
pub const DEFAULT_PADDING: Padding = Padding {
    top: 5.0,
    bottom: 5.0,
    right: 10.0,
    left: 10.0,
};

/// The possible status of a [`Button`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// The [`Button`] can be pressed.
    Active,
    /// The [`Button`] can be pressed and it is being hovered.
    Hovered,
    /// The [`Button`] is being pressed.
    Pressed,
    /// The [`Button`] cannot be pressed.
    Disabled,
    /// The [`Button`] is focused via keyboard/gamepad navigation.
    Focused,
}

/// The style of a button.
///
/// If not specified with [`Button::style`]
/// the theme will provide the style.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    /// The [`Background`] of the button.
    pub background: Option<Background>,
    /// The text [`Color`] of the button.
    pub text_color: Color,
    /// The [`Border`] of the button.
    pub border: Border,
    /// The [`Shadow`] of the button.
    pub shadow: Shadow,
    /// Whether the button should be snapped to the pixel grid.
    pub snap: bool,
}

impl Style {
    /// Updates the [`Style`] with the given [`Background`].
    pub fn with_background(self, background: impl Into<Background>) -> Self {
        Self {
            background: Some(background.into()),
            ..self
        }
    }
}

impl Default for Style {
    fn default() -> Self {
        Self {
            background: None,
            text_color: Color::BLACK,
            border: Border::default(),
            shadow: Shadow::default(),
            snap: renderer::CRISP,
        }
    }
}

/// The theme catalog of a [`Button`].
///
/// All themes that can be used with [`Button`]
/// must implement this trait.
///
/// # Example
/// ```no_run
/// # use iced_widget::core::{Color, Background};
/// # use iced_widget::button::{Catalog, Status, Style};
/// # struct MyTheme;
/// #[derive(Debug, Default)]
/// pub enum ButtonClass {
///     #[default]
///     Primary,
///     Secondary,
///     Danger
/// }
///
/// impl Catalog for MyTheme {
///     type Class<'a> = ButtonClass;
///     
///     fn default<'a>() -> Self::Class<'a> {
///         ButtonClass::default()
///     }
///     
///
///     fn style(&self, class: &Self::Class<'_>, status: Status) -> Style {
///         let mut style = Style::default();
///
///         match class {
///             ButtonClass::Primary => {
///                 style.background = Some(Background::Color(Color::from_rgb(0.529, 0.808, 0.921)));
///             },
///             ButtonClass::Secondary => {
///                 style.background = Some(Background::Color(Color::WHITE));
///             },
///             ButtonClass::Danger => {
///                 style.background = Some(Background::Color(Color::from_rgb(0.941, 0.502, 0.502)));
///             },
///         }
///
///         style
///     }
/// }
/// ```
///
/// Although, in order to use [`Button::style`]
/// with `MyTheme`, [`Catalog::Class`] must implement
/// `From<StyleFn<'_, MyTheme>>`.
pub trait Catalog {
    /// The item class of the [`Catalog`].
    type Class<'a>;

    /// The default class produced by the [`Catalog`].
    fn default<'a>() -> Self::Class<'a>;

    /// The [`Style`] of a class with the given status.
    fn style(&self, class: &Self::Class<'_>, status: Status) -> Style;
}

/// A styling function for a [`Button`].
pub type StyleFn<'a, Theme> = Box<dyn Fn(&Theme, Status) -> Style + 'a>;

impl Catalog for Theme {
    type Class<'a> = StyleFn<'a, Self>;

    fn default<'a>() -> Self::Class<'a> {
        Box::new(primary)
    }

    fn style(&self, class: &Self::Class<'_>, status: Status) -> Style {
        class(self, status)
    }
}

/// A primary button; denoting a main action.
pub fn primary(theme: &Theme, status: Status) -> Style {
    let palette = theme.extended_palette();
    let base = styled(palette.primary.base);

    match status {
        Status::Active | Status::Pressed => base,
        Status::Hovered | Status::Focused => Style {
            background: Some(Background::Color(palette.primary.strong.color)),
            ..base
        },
        Status::Disabled => disabled(base),
    }
}

/// A secondary button; denoting a complementary action.
pub fn secondary(theme: &Theme, status: Status) -> Style {
    let palette = theme.extended_palette();
    let base = styled(palette.secondary.base);

    match status {
        Status::Active | Status::Pressed => base,
        Status::Hovered | Status::Focused => Style {
            background: Some(Background::Color(palette.secondary.strong.color)),
            ..base
        },
        Status::Disabled => disabled(base),
    }
}

/// A success button; denoting a good outcome.
pub fn success(theme: &Theme, status: Status) -> Style {
    let palette = theme.extended_palette();
    let base = styled(palette.success.base);

    match status {
        Status::Active | Status::Pressed => base,
        Status::Hovered | Status::Focused => Style {
            background: Some(Background::Color(palette.success.strong.color)),
            ..base
        },
        Status::Disabled => disabled(base),
    }
}

/// A warning button; denoting a risky action.
pub fn warning(theme: &Theme, status: Status) -> Style {
    let palette = theme.extended_palette();
    let base = styled(palette.warning.base);

    match status {
        Status::Active | Status::Pressed => base,
        Status::Hovered | Status::Focused => Style {
            background: Some(Background::Color(palette.warning.strong.color)),
            ..base
        },
        Status::Disabled => disabled(base),
    }
}

/// A danger button; denoting a destructive action.
pub fn danger(theme: &Theme, status: Status) -> Style {
    let palette = theme.extended_palette();
    let base = styled(palette.danger.base);

    match status {
        Status::Active | Status::Pressed => base,
        Status::Hovered | Status::Focused => Style {
            background: Some(Background::Color(palette.danger.strong.color)),
            ..base
        },
        Status::Disabled => disabled(base),
    }
}

/// A text button; useful for links.
pub fn text(theme: &Theme, status: Status) -> Style {
    let palette = theme.extended_palette();

    let base = Style {
        text_color: palette.background.base.text,
        ..Style::default()
    };

    match status {
        Status::Active | Status::Pressed => base,
        Status::Hovered | Status::Focused => Style {
            text_color: palette.background.base.text.scale_alpha(0.8),
            ..base
        },
        Status::Disabled => disabled(base),
    }
}

/// A button using background shades.
pub fn background(theme: &Theme, status: Status) -> Style {
    let palette = theme.extended_palette();
    let base = styled(palette.background.base);

    match status {
        Status::Active => base,
        Status::Pressed => Style {
            background: Some(Background::Color(palette.background.strong.color)),
            ..base
        },
        Status::Hovered | Status::Focused => Style {
            background: Some(Background::Color(palette.background.weak.color)),
            ..base
        },
        Status::Disabled => disabled(base),
    }
}

/// A subtle button using weak background shades.
pub fn subtle(theme: &Theme, status: Status) -> Style {
    let palette = theme.extended_palette();
    let base = styled(palette.background.weakest);

    match status {
        Status::Active => base,
        Status::Pressed => Style {
            background: Some(Background::Color(palette.background.strong.color)),
            ..base
        },
        Status::Hovered | Status::Focused => Style {
            background: Some(Background::Color(palette.background.weaker.color)),
            ..base
        },
        Status::Disabled => disabled(base),
    }
}

fn styled(pair: palette::Pair) -> Style {
    Style {
        background: Some(Background::Color(pair.color)),
        text_color: pair.text,
        border: border::rounded(2),
        ..Style::default()
    }
}

fn disabled(style: Style) -> Style {
    Style {
        background: style
            .background
            .map(|background| background.scale_alpha(0.5)),
        text_color: style.text_color.scale_alpha(0.5),
        ..style
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::Space;
    use crate::core::image;
    use crate::core::widget::operation;
    use crate::core::{Transformation, border};

    #[derive(Debug, Clone, PartialEq)]
    struct Pressed;

    /// A renderer that only remembers the quads it was asked to fill, and the
    /// background each was filled with.
    #[derive(Default)]
    struct Recorder {
        quads: Vec<renderer::Quad>,
        backgrounds: Vec<Background>,
    }

    impl crate::core::Renderer for Recorder {
        fn start_layer(&mut self, _bounds: Rectangle) {}

        fn end_layer(&mut self) {}

        fn start_transformation(&mut self, _transformation: Transformation) {}

        fn end_transformation(&mut self) {}

        fn fill_quad(&mut self, quad: renderer::Quad, background: impl Into<Background>) {
            self.quads.push(quad);
            self.backgrounds.push(background.into());
        }

        fn allocate_image(
            &mut self,
            _handle: &image::Handle,
            _callback: impl FnOnce(Result<image::Allocation, image::Error>) + Send + 'static,
        ) {
        }

        fn hint(&mut self, _scale_factor: f32) {}

        fn scale_factor(&self) -> Option<f32> {
            None
        }

        fn reset(&mut self, _new_bounds: Rectangle) {}
    }

    type TestButton<'a> = Button<'a, Pressed, Theme, Recorder>;

    /// A button whose focused style is the only one with a border and the only
    /// white fill, so a stepped border or fill is told apart from a tweened one.
    fn button() -> TestButton<'static> {
        Button::new(Space::new())
            .on_press(Pressed)
            .animate_background(Duration::from_millis(200))
            .style(|_theme, status| {
                let focused = status == Status::Focused;

                Style {
                    background: Some(Background::Color(if focused {
                        Color::WHITE
                    } else {
                        Color::BLACK
                    })),
                    border: if focused {
                        border::rounded(0).width(4)
                    } else {
                        Border::default()
                    },
                    ..Style::default()
                }
            })
    }

    fn tree(button: &TestButton<'_>) -> Tree {
        Tree::new(button as &dyn Widget<Pressed, Theme, Recorder>)
    }

    fn node(button: &mut TestButton<'_>, tree: &mut Tree) -> layout::Node {
        button.layout(
            tree,
            &Recorder::default(),
            &layout::Limits::new(Size::ZERO, Size::new(200.0, 50.0)),
        )
    }

    fn redraw(button: &mut TestButton<'_>, tree: &mut Tree, node: &layout::Node) {
        let mut messages = Vec::new();
        let mut shell = Shell::new(&mut messages);

        button.update(
            tree,
            &Event::Window(window::Event::RedrawRequested(Instant::now())),
            Layout::new(node),
            mouse::Cursor::Unavailable,
            &Recorder::default(),
            &mut shell,
            &Rectangle::with_size(Size::new(200.0, 50.0)),
        );
    }

    /// Draws once and returns the fill and border of the button's own quad.
    fn paint(button: &TestButton<'_>, tree: &Tree, node: &layout::Node) -> (Background, Border) {
        let mut renderer = Recorder::default();

        button.draw(
            tree,
            &mut renderer,
            &Theme::Dark,
            &renderer::Style::default(),
            Layout::new(node),
            mouse::Cursor::Unavailable,
            &Rectangle::with_size(Size::new(200.0, 50.0)),
        );

        (
            *renderer
                .backgrounds
                .first()
                .expect("the button fills a quad"),
            renderer
                .quads
                .first()
                .expect("the button fills a quad")
                .border,
        )
    }

    /// Backdates a running transition, so a frame is read from the middle of the
    /// tween instead of the hair's width the clock has actually moved.
    fn wind_back(tree: &mut Tree, elapsed: Duration) {
        let state = tree.state.downcast_mut::<State>();

        state.transition_start = state
            .transition_start
            .and_then(|start| start.checked_sub(elapsed));
    }

    fn style_focus_button(button: &mut TestButton<'_>, tree: &mut Tree, node: &layout::Node) {
        let mut operation = style_focus::<()>(true);

        button.operate(
            tree,
            Layout::new(node),
            &Recorder::default(),
            &mut operation as &mut dyn Operation,
        );
    }

    #[test]
    fn style_focus_paints_the_button_as_focused() {
        let mut button = button();
        let mut tree = tree(&button);
        let node = node(&mut button, &mut tree);

        redraw(&mut button, &mut tree, &node);
        assert_eq!(button.status, Some(Status::Active));

        style_focus_button(&mut button, &mut tree, &node);
        redraw(&mut button, &mut tree, &node);

        assert_eq!(button.status, Some(Status::Focused));
    }

    #[test]
    fn style_focus_is_not_keyboard_focus() {
        // The ring owns activation; routing this through `is_focused` would fire
        // both the ring's submit and the button's `on_press`.
        let mut button = button();
        let mut tree = tree(&button);
        let node = node(&mut button, &mut tree);

        style_focus_button(&mut button, &mut tree, &node);

        let state = tree.state.downcast_ref::<State>();
        assert!(state.style_focused);
        assert!(!operation::Focusable::is_focused(state));

        let mut messages = Vec::new();
        let mut shell = Shell::new(&mut messages);

        button.update(
            &mut tree,
            &Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(key::Named::Enter),
                modified_key: keyboard::Key::Named(key::Named::Enter),
                physical_key: keyboard::key::Physical::Unidentified(
                    keyboard::key::NativeCode::Unidentified,
                ),
                location: keyboard::Location::Standard,
                modifiers: keyboard::Modifiers::empty(),
                text: None,
                repeat: false,
            }),
            Layout::new(&node),
            mouse::Cursor::Unavailable,
            &Recorder::default(),
            &mut shell,
            &Rectangle::with_size(Size::new(200.0, 50.0)),
        );

        assert!(messages.is_empty());
    }

    #[test]
    fn a_status_change_first_seen_on_a_redraw_starts_a_transition() {
        let mut button = button();
        let mut tree = tree(&button);
        let node = node(&mut button, &mut tree);

        redraw(&mut button, &mut tree, &node);
        assert_eq!(tree.state.downcast_ref::<State>().previous_status, None);

        style_focus_button(&mut button, &mut tree, &node);
        redraw(&mut button, &mut tree, &node);

        let state = tree.state.downcast_ref::<State>();
        assert_eq!(state.previous_status, Some(Status::Active));
        assert!(state.transition_start.is_some());
    }

    #[test]
    fn the_border_tweens_with_the_background() {
        let mut button = button();
        let mut tree = tree(&button);
        let node = node(&mut button, &mut tree);

        redraw(&mut button, &mut tree, &node);
        style_focus_button(&mut button, &mut tree, &node);
        redraw(&mut button, &mut tree, &node);

        let (_background, border) = paint(&button, &tree, &node);

        // Barely into a 200ms transition, so the focused border is still growing
        // rather than already at its full 4px.
        assert!(border.width > 0.0, "the border is on its way in");
        assert!(border.width < 1.0, "the border did not step to its target");
    }

    #[test]
    fn the_background_tweens_with_the_border_by_default() {
        let mut button = button();
        let mut tree = tree(&button);
        let node = node(&mut button, &mut tree);

        redraw(&mut button, &mut tree, &node);
        style_focus_button(&mut button, &mut tree, &node);
        redraw(&mut button, &mut tree, &node);
        wind_back(&mut tree, Duration::from_millis(100));

        let (background, border) = paint(&button, &tree, &node);

        let Background::Color(fill) = background else {
            panic!("the fill is a solid color");
        };

        assert!(fill.r > 0.0, "the black fill is on its way to white");
        assert!(fill.r < 1.0, "the fill did not step to its target");
        assert!(border.width > 0.0 && border.width < 4.0, "mid-tween");
    }

    #[test]
    fn snap_fill_takes_the_background_straight() {
        let mut button = button().snap_fill(true);
        let mut tree = tree(&button);
        let node = node(&mut button, &mut tree);

        redraw(&mut button, &mut tree, &node);
        style_focus_button(&mut button, &mut tree, &node);
        redraw(&mut button, &mut tree, &node);
        wind_back(&mut tree, Duration::from_millis(100));

        let (background, border) = paint(&button, &tree, &node);

        // The focused fill lands whole while the focused border is still growing.
        assert_eq!(background, Background::Color(Color::WHITE));
        assert!(border.width > 0.0 && border.width < 4.0, "mid-tween");
    }
}
