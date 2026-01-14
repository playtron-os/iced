//! Test example for maximized window → unmaximize → animated resize flow
//!
//! This mimics the chat-ui pattern:
//! 1. Window starts maximized with no decorations
//! 2. Click "Unmaximize" to restore to 500x600
//! 3. Use animated grow/shrink buttons to test resize control

use iced::widget::{button, column, container, row, scrollable, text};
use iced::window;
use iced::{Color, Element, Size, Subscription, Task};
use tracing_subscriber::EnvFilter;

/// Initial windowed size (stored by compositor when maximizing)
const WINDOWED_SIZE: Size = Size::new(400.0, 400.0);

pub fn main() -> iced::Result {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    iced::application(App::default, App::update, App::view)
        .subscription(App::subscription)
        .title("Maximize Test")
        .window(window::Settings {
            size: WINDOWED_SIZE,
            maximized: true,
            decorations: false,
            transparent: true,
            ..Default::default()
        })
        .run()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowState {
    Maximized,
    Windowed,
}

#[derive(Debug)]
struct App {
    current_size: Size,
    /// Size when maximized (used to calculate positions)
    maximized_size: Size,
    window_state: WindowState,
}

impl Default for App {
    fn default() -> Self {
        Self {
            current_size: Size::new(0.0, 0.0), // Will be set by WindowResized event
            maximized_size: Size::new(1920.0, 1080.0), // Default, updated when maximized
            window_state: WindowState::Maximized,
        }
    }
}

#[derive(Debug, Clone)]
enum Message {
    WindowResized(Size),
    Unmaximize,
    Maximize,
    AnimatedGrow,
    AnimatedShrink,
    SetSize(u32, u32),
    /// Set position and size (works while maximized too - stores as hint)
    SetPositionAndSize(i32, i32, u32, u32),
}

impl App {
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::WindowResized(size) => {
                println!("Window resized to: {:?}", size);
                self.current_size = size;
                // Track maximized size for position calculations
                if self.window_state == WindowState::Maximized {
                    self.maximized_size = size;
                }
                Task::none()
            }

            Message::Unmaximize => {
                println!("Unmaximizing window to: {:?}", WINDOWED_SIZE);
                self.window_state = WindowState::Windowed;
                window::oldest().and_then(move |id| {
                    // Set the target size first, then unmaximize
                    // This allows the compositor to restore to the correct size centered
                    window::resize(id, WINDOWED_SIZE).chain(window::maximize(id, false))
                })
            }

            Message::Maximize => {
                println!("Maximizing window");
                self.window_state = WindowState::Maximized;
                window::oldest().and_then(|id| window::maximize(id, true))
            }

            Message::AnimatedGrow => {
                let new_width = (self.current_size.width + 100.0) as u32;
                let new_height = (self.current_size.height + 100.0) as u32;
                println!(
                    "Animated grow: {:?} -> {}x{} (300ms)",
                    self.current_size, new_width, new_height
                );
                window::oldest()
                    .and_then(move |id| window::animated_resize(id, new_width, new_height, 300))
            }

            Message::AnimatedShrink => {
                let new_width = ((self.current_size.width - 100.0).max(300.0)) as u32;
                let new_height = ((self.current_size.height - 100.0).max(300.0)) as u32;
                println!(
                    "Animated shrink: {:?} -> {}x{} (300ms)",
                    self.current_size, new_width, new_height
                );
                window::oldest()
                    .and_then(move |id| window::animated_resize(id, new_width, new_height, 300))
            }

            Message::SetSize(width, height) => {
                println!("Animated resize to: {}x{}", width, height);
                window::oldest().and_then(move |id| window::animated_resize(id, width, height, 300))
            }

            Message::SetPositionAndSize(x, y, width, height) => {
                println!(
                    "Set position and size: ({}, {}) {}x{} (works while maximized!)",
                    x, y, width, height
                );
                window::oldest().and_then(move |id| {
                    window::animated_resize_with_position(id, x, y, width, height, 300)
                })
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let state_text = match self.window_state {
            WindowState::Maximized => "MAXIMIZED",
            WindowState::Windowed => "WINDOWED",
        };

        let content = column![
            text(format!("Window State: {}", state_text)).size(28),
            text(format!(
                "Current Size: {:.0}x{:.0}",
                self.current_size.width, self.current_size.height
            ))
            .size(20),
            // Maximize/Unmaximize controls
            text("Window Mode:").size(16),
            row![
                button("Maximize")
                    .on_press_maybe(match self.window_state {
                        WindowState::Windowed => Some(Message::Maximize),
                        WindowState::Maximized => None,
                    })
                    .padding(10),
                button("Unmaximize → 500x600")
                    .on_press_maybe(match self.window_state {
                        WindowState::Maximized => Some(Message::Unmaximize),
                        WindowState::Windowed => None,
                    })
                    .padding(10),
            ]
            .spacing(10),
            // Quick size presets (only when windowed)
            text("Quick Size Presets:").size(16),
            row![
                button("400x400")
                    .on_press_maybe(match self.window_state {
                        WindowState::Windowed => Some(Message::SetSize(400, 400)),
                        WindowState::Maximized => None,
                    })
                    .padding(8),
                button("500x600")
                    .on_press_maybe(match self.window_state {
                        WindowState::Windowed => Some(Message::SetSize(500, 600)),
                        WindowState::Maximized => None,
                    })
                    .padding(8),
                button("800x600")
                    .on_press_maybe(match self.window_state {
                        WindowState::Windowed => Some(Message::SetSize(800, 600)),
                        WindowState::Maximized => None,
                    })
                    .padding(8),
            ]
            .spacing(10),
            // Animated resize (only when windowed)
            text("Animated Resize (300ms):").size(16),
            row![
                button("Animated Grow (+100)")
                    .on_press_maybe(match self.window_state {
                        WindowState::Windowed => Some(Message::AnimatedGrow),
                        WindowState::Maximized => None,
                    })
                    .padding(10),
                button("Animated Shrink (-100)")
                    .on_press_maybe(match self.window_state {
                        WindowState::Windowed => Some(Message::AnimatedShrink),
                        WindowState::Maximized => None,
                    })
                    .padding(10),
            ]
            .spacing(10),
            // Position hints (works while MAXIMIZED - sets restore position)
            text("Position + Size Hints (works while maximized!):").size(16),
            text(format!(
                "Display size: {:.0}x{:.0}",
                self.maximized_size.width, self.maximized_size.height
            ))
            .size(12),
            row![
                button("Top-Left 400x400")
                    .on_press(Message::SetPositionAndSize(50, 50, 400, 400))
                    .padding(8),
                button("Center 500x600")
                    .on_press(Message::SetPositionAndSize(
                        ((self.maximized_size.width - 500.0) / 2.0) as i32,
                        ((self.maximized_size.height - 600.0) / 2.0) as i32,
                        500,
                        600
                    ))
                    .padding(8),
                button("Bottom-Right 600x400")
                    .on_press(Message::SetPositionAndSize(
                        (self.maximized_size.width - 600.0 - 50.0) as i32,
                        (self.maximized_size.height - 400.0 - 50.0) as i32,
                        600,
                        400
                    ))
                    .padding(8),
            ]
            .spacing(10),
            text("^ These set the restore position when unmaximizing").size(12),
        ]
        .spacing(15)
        .padding(20);

        container(scrollable(content).height(iced::Length::Fill))
            .style(|_| container::Style {
                background: Some(Color::from_rgb(0.1, 0.1, 0.15).into()),
                ..Default::default()
            })
            .width(iced::Length::Fill)
            .height(iced::Length::Fill)
            .padding(20)
            .into()
    }

    fn subscription(&self) -> Subscription<Message> {
        window::resize_events().map(|(_id, size)| Message::WindowResized(size))
    }
}
