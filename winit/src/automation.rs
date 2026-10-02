//! Connects this event loop to the automation door in [`iced_automation`].
//!
//! The door, its switch and its protocol live there; this module only answers
//! the door's requests from the windows, popups and pending events of this loop.
use crate::core::window;
use crate::core::{Event, Point, Size, theme};
use crate::graphics::{Compositor, Viewport};
use crate::popup::PopupManager;
use crate::program::{self, Program};
use crate::runtime::user_interface::{self, UserInterface};
use crate::window::WindowManager;

use iced_automation::{Answer, Ask, Collector, Surface};
use rustc_hash::FxHashMap;

use std::time::Instant;

pub use iced_automation::start;

/// Answers the door's requests. Called by the loop before it hands out its
/// pending events, so anything injected here is processed in the same pass,
/// exactly as if the compositor had sent it.
#[allow(clippy::too_many_arguments)]
pub fn serve<P, C>(
    program: &program::Instance<P>,
    window_manager: &mut WindowManager<P, C>,
    user_interfaces: &mut FxHashMap<
        window::Id,
        UserInterface<'_, P::Message, P::Theme, P::Renderer>,
    >,
    popup_manager: &mut PopupManager<P, C>,
    popup_cursor_position: &mut FxHashMap<window::Id, Point>,
    ui_caches: &mut FxHashMap<window::Id, user_interface::Cache>,
    events: &mut Vec<(window::Id, Event)>,
    messages: &[P::Message],
) where
    P: Program,
    C: Compositor<Renderer = P::Renderer>,
    P::Theme: theme::Base,
{
    if !iced_automation::is_open() {
        return;
    }

    let now = Instant::now();
    let mut idle = events.is_empty()
        && messages.is_empty()
        && window_manager.iter_mut().all(|(_, window)| {
            !iced_automation::frame_due(window.redraw_requested_at, window.redraw_at, now)
        });

    iced_automation::serve(|ask| match ask {
        Ask::Idle => Answer::Idle(idle),
        Ask::Info => {
            let mut surfaces: Vec<Surface> = window_manager
                .iter_mut()
                .map(|(id, window)| Surface {
                    window: id,
                    popup: false,
                    size: window.state.logical_size(),
                    scale_factor: window.state.scale_factor(),
                    focused: window.raw.has_focus(),
                })
                .collect();

            surfaces.extend(
                popup_manager
                    .iter()
                    .filter(|(_, popup)| popup.configured)
                    .map(|(_, popup)| Surface {
                        window: popup.iced_id,
                        popup: true,
                        size: popup
                            .viewport
                            .as_ref()
                            .map(Viewport::logical_size)
                            .unwrap_or(Size::ZERO),
                        scale_factor: popup.scale_factor,
                        focused: false,
                    }),
            );

            Answer::Info(surfaces)
        }
        Ask::Focused => {
            let mut windows = window_manager
                .iter_mut()
                .map(|(id, window)| (id, window.raw.has_focus()));
            let first = windows.next();

            Answer::Focused(
                first
                    .into_iter()
                    .chain(windows)
                    .find(|(_, focused)| *focused)
                    .or(first)
                    .map(|(id, _)| id),
            )
        }
        Ask::Tree => {
            let mut nodes = Vec::new();

            for (id, window) in window_manager.iter_mut() {
                if let Some(ui) = user_interfaces.get_mut(&id) {
                    let mut collector = Collector::new(id, false, window.state.logical_size());

                    ui.operate(&window.renderer, &mut collector);
                    nodes.extend(collector.into_nodes());
                }
            }

            for (_, popup) in popup_manager.iter_mut() {
                let (true, Some(renderer), Some(viewport)) = (
                    popup.configured,
                    popup.renderer.as_mut(),
                    popup.viewport.as_ref(),
                ) else {
                    continue;
                };

                let size = viewport.logical_size();
                let cache = ui_caches.remove(&popup.iced_id).unwrap_or_default();
                let mut ui = UserInterface::build(
                    program.view(popup.iced_id),
                    Size::new(size.width, size.height),
                    cache,
                    renderer,
                );
                let mut collector = Collector::new(popup.iced_id, true, size);

                ui.operate(renderer, &mut collector);
                nodes.extend(collector.into_nodes());

                let _ = ui_caches.insert(popup.iced_id, ui.into_cache());
            }

            Answer::Tree(nodes)
        }
        Ask::Inject {
            window,
            cursor,
            events: injected,
        } => {
            let found = if let Some(toplevel) = window_manager.get_mut(window) {
                if let Some(cursor) = cursor {
                    toplevel.state.place_cursor(cursor);
                }

                true
            } else if popup_manager.find_by_iced_id(window).is_some() {
                if let Some(cursor) = cursor {
                    let _ = popup_cursor_position.insert(window, cursor);
                }

                true
            } else {
                false
            };

            if found {
                events.extend(injected.into_iter().map(|event| (window, event)));
                idle = false;
            }

            Answer::Injected(found)
        }
    });
}
