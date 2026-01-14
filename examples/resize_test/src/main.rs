//! Test example to verify programmatic window resizing works on Wayland/COSMIC
//!
//! Click "Grow" to increase window size, "Shrink" to decrease.
//! "Animated Grow/Shrink" uses the COSMIC animated resize protocol.

use iced::widget::{button, center, column, row, text};
use iced::window;
use iced::{Element, Size, Subscription, Task};
use tracing_subscriber::EnvFilter;

pub fn main() -> iced::Result {
    // Initialize tracing with RUST_LOG env var
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    iced::application(App::default, App::update, App::view)
        .subscription(App::subscription)
        .window_size(Size::new(400.0, 300.0))
        .title("Resize Test")
        .run()
}

#[derive(Debug, Default)]
struct App {
    current_size: Size,
}

#[derive(Debug, Clone)]
enum Message {
    Grow,
    Shrink,
    AnimatedGrow,
    AnimatedShrink,
    WindowResized(Size),
}

impl App {
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::WindowResized(size) => {
                println!("Window resized to: {:?}", size);
                self.current_size = size;
                Task::none()
            }
            Message::Grow => {
                let new_size = Size::new(
                    self.current_size.width + 100.0,
                    self.current_size.height + 100.0,
                );
                println!(
                    "Requesting resize: {:?} -> {:?}",
                    self.current_size, new_size
                );
                window::oldest().and_then(move |id| window::resize(id, new_size))
            }
            Message::Shrink => {
                let new_size = Size::new(
                    (self.current_size.width - 100.0).max(200.0),
                    (self.current_size.height - 100.0).max(200.0),
                );
                println!(
                    "Requesting resize: {:?} -> {:?}",
                    self.current_size, new_size
                );
                window::oldest().and_then(move |id| window::resize(id, new_size))
            }
            Message::AnimatedGrow => {
                let new_width = (self.current_size.width + 200.0) as u32;
                let new_height = (self.current_size.height + 200.0) as u32;
                println!(
                    "Requesting ANIMATED resize: {:?} -> {}x{} (300ms)",
                    self.current_size, new_width, new_height
                );
                window::oldest()
                    .and_then(move |id| window::animated_resize(id, new_width, new_height, 300))
            }
            Message::AnimatedShrink => {
                let new_width = ((self.current_size.width - 200.0).max(200.0)) as u32;
                let new_height = ((self.current_size.height - 200.0).max(200.0)) as u32;
                println!(
                    "Requesting ANIMATED resize: {:?} -> {}x{} (300ms)",
                    self.current_size, new_width, new_height
                );
                window::oldest()
                    .and_then(move |id| window::animated_resize(id, new_width, new_height, 300))
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        center(
            column![
                text(format!(
                    "Current size: {}x{}",
                    self.current_size.width, self.current_size.height
                ))
                .size(24),
                text("Instant resize:").size(16),
                row![
                    button("Grow (+100)").on_press(Message::Grow).padding(10),
                    button("Shrink (-100)")
                        .on_press(Message::Shrink)
                        .padding(10),
                ]
                .spacing(20),
                text("Animated resize (COSMIC protocol):").size(16),
                row![
                    button("Animated Grow (+200)")
                        .on_press(Message::AnimatedGrow)
                        .padding(10),
                    button("Animated Shrink (-200)")
                        .on_press(Message::AnimatedShrink)
                        .padding(10),
                ]
                .spacing(20)
            ]
            .spacing(10),
        )
        .into()
    }

    fn subscription(&self) -> Subscription<Message> {
        window::resize_events().map(|(_id, size)| Message::WindowResized(size))
    }
}
