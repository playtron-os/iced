//! This example demonstrates conic (angular/sweep) gradients in iced.
//!
//! A conic gradient interpolates colors around a center point, like a color wheel.
//! The angle determines where the gradient starts, and color stops represent
//! positions around the full rotation (0.0 to 1.0).

use iced::gradient;
use iced::theme;
use iced::widget::{checkbox, column, container, row, slider, space, text};
use iced::{Center, Color, Element, Fill, Point, Radians, Theme, color};

pub fn main() -> iced::Result {
    #[cfg(target_arch = "wasm32")]
    console_error_panic_hook::set_once();

    #[cfg(not(target_arch = "wasm32"))]
    tracing_subscriber::fmt::init();

    iced::application(
        ConicGradient::default,
        ConicGradient::update,
        ConicGradient::view,
    )
    .style(ConicGradient::style)
    .transparent(true)
    .run()
}

#[derive(Debug, Clone, Copy)]
struct ConicGradient {
    start_color: Color,
    mid_color: Color,
    end_color: Color,
    center_x: f32,
    center_y: f32,
    start_angle: Radians,
    transparent: bool,
    show_color_wheel: bool,
}

#[derive(Debug, Clone, Copy)]
enum Message {
    StartColorChanged(Color),
    MidColorChanged(Color),
    EndColorChanged(Color),
    CenterXChanged(f32),
    CenterYChanged(f32),
    StartAngleChanged(Radians),
    TransparentToggled(bool),
    ColorWheelToggled(bool),
}

impl ConicGradient {
    fn new() -> Self {
        Self {
            start_color: color!(0xff0000), // Red
            mid_color: color!(0x00ff00),   // Green
            end_color: color!(0x0000ff),   // Blue
            center_x: 0.5,
            center_y: 0.5,
            start_angle: Radians(0.0),
            transparent: false,
            show_color_wheel: false,
        }
    }

    fn update(&mut self, message: Message) {
        match message {
            Message::StartColorChanged(color) => self.start_color = color,
            Message::MidColorChanged(color) => self.mid_color = color,
            Message::EndColorChanged(color) => self.end_color = color,
            Message::CenterXChanged(x) => self.center_x = x,
            Message::CenterYChanged(y) => self.center_y = y,
            Message::StartAngleChanged(angle) => self.start_angle = angle,
            Message::TransparentToggled(transparent) => self.transparent = transparent,
            Message::ColorWheelToggled(show) => self.show_color_wheel = show,
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let Self {
            start_color,
            mid_color,
            end_color,
            center_x,
            center_y,
            start_angle,
            transparent,
            show_color_wheel,
        } = *self;

        // Main conic gradient display
        let gradient_box = container(space())
            .style(move |_theme| {
                if show_color_wheel {
                    // Show a full color wheel (rainbow)
                    gradient::Conic::new(Point::new(center_x, center_y), start_angle)
                        .add_stop(0.0, color!(0xff0000)) // Red
                        .add_stop(0.167, color!(0xffff00)) // Yellow
                        .add_stop(0.333, color!(0x00ff00)) // Green
                        .add_stop(0.5, color!(0x00ffff)) // Cyan
                        .add_stop(0.667, color!(0x0000ff)) // Blue
                        .add_stop(0.833, color!(0xff00ff)) // Magenta
                        .add_stop(1.0, color!(0xff0000)) // Back to Red
                        .into()
                } else {
                    // Custom 3-color gradient
                    gradient::Conic::new(Point::new(center_x, center_y), start_angle)
                        .add_stop(0.0, start_color)
                        .add_stop(0.5, mid_color)
                        .add_stop(1.0, end_color)
                        .into()
                }
            })
            .width(Fill)
            .height(Fill);

        // Controls
        let center_x_picker = row![
            text("Center X").width(80),
            slider(0.0..=1.0, center_x, Message::CenterXChanged).step(0.01),
            text(format!("{:.2}", center_x)).width(40),
        ]
        .spacing(8)
        .padding(8)
        .align_y(Center);

        let center_y_picker = row![
            text("Center Y").width(80),
            slider(0.0..=1.0, center_y, Message::CenterYChanged).step(0.01),
            text(format!("{:.2}", center_y)).width(40),
        ]
        .spacing(8)
        .padding(8)
        .align_y(Center);

        let angle_picker = row![
            text("Start Angle").width(80),
            slider(Radians::RANGE, start_angle, Message::StartAngleChanged).step(0.01),
            text(format!("{:.2}°", start_angle.0.to_degrees())).width(60),
        ]
        .spacing(8)
        .padding(8)
        .align_y(Center);

        let toggles = row![
            checkbox(transparent)
                .label("Transparent window")
                .on_toggle(Message::TransparentToggled),
            checkbox(show_color_wheel)
                .label("Show color wheel")
                .on_toggle(Message::ColorWheelToggled),
        ]
        .spacing(16)
        .padding(8);

        let mut content = column![
            text("Conic Gradient Example").size(24),
            text("A conic gradient sweeps colors around a center point").size(14),
            center_x_picker,
            center_y_picker,
            angle_picker,
            toggles,
        ];

        // Only show color pickers when not in color wheel mode
        if !show_color_wheel {
            content = content
                .push(color_picker("Start", start_color).map(Message::StartColorChanged))
                .push(color_picker("Mid", mid_color).map(Message::MidColorChanged))
                .push(color_picker("End", end_color).map(Message::EndColorChanged));
        }

        content = content.push(gradient_box);

        content.into()
    }

    fn style(&self, theme: &Theme) -> theme::Style {
        if self.transparent {
            theme::Style {
                background_color: Color::TRANSPARENT,
                text_color: theme.palette().text,
            }
        } else {
            theme::default(theme)
        }
    }
}

impl Default for ConicGradient {
    fn default() -> Self {
        Self::new()
    }
}

fn color_picker(label: &str, color: Color) -> Element<'_, Color> {
    row![
        text(label).width(64),
        slider(0.0..=1.0, color.r, move |r| { Color { r, ..color } }).step(0.01),
        slider(0.0..=1.0, color.g, move |g| { Color { g, ..color } }).step(0.01),
        slider(0.0..=1.0, color.b, move |b| { Color { b, ..color } }).step(0.01),
        slider(0.0..=1.0, color.a, move |a| { Color { a, ..color } }).step(0.01),
    ]
    .spacing(8)
    .padding(8)
    .align_y(Center)
    .into()
}
