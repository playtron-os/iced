//! Ask the compositor which gpu it scans out from.
use crate::graphics::compositor::Window;

use raw_window_handle::{RawDisplayHandle, WaylandDisplayHandle};
use smithay_client_toolkit::{
    delegate_dmabuf, delegate_registry,
    dmabuf::{DmabufFeedback, DmabufHandler, DmabufState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
};
use wayland_client::{
    Connection, QueueHandle, backend::Backend, globals::registry_queue_init, protocol::wl_buffer,
};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_buffer_params_v1, zwp_linux_dmabuf_feedback_v1,
};

struct AppData {
    registry_state: RegistryState,
    dmabuf_state: DmabufState,
    feedback: Option<DmabufFeedback>,
}

impl DmabufHandler for AppData {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.dmabuf_state
    }

    fn dmabuf_feedback(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _proxy: &zwp_linux_dmabuf_feedback_v1::ZwpLinuxDmabufFeedbackV1,
        feedback: DmabufFeedback,
    ) {
        self.feedback = Some(feedback);
    }

    fn created(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _params: &zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1,
        _buffer: wl_buffer::WlBuffer,
    ) {
    }

    fn failed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _params: &zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1,
    ) {
    }

    fn released(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _buffer: &wl_buffer::WlBuffer,
    ) {
    }
}

impl ProvidesRegistryState for AppData {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![,];
}

/// The pci ids of the gpu the compositor renders the default feedback on, if it
/// advertises dmabuf feedback (v4+).
pub fn get_wayland_device_ids<W: Window>(window: &W) -> Option<(u16, u16)> {
    if !wayland_sys::client::is_lib_available() {
        return None;
    }

    let conn = match window.display_handle().map(|handle| handle.as_raw()) {
        #[allow(unsafe_code)]
        Ok(RawDisplayHandle::Wayland(WaylandDisplayHandle { display, .. })) => {
            Connection::from_backend(unsafe {
                Backend::from_foreign_display(display.as_ptr().cast())
            })
        }
        _ => return None,
    };

    let (globals, mut event_queue) = registry_queue_init(&conn).ok()?;
    let qh = event_queue.handle();

    let mut app_data = AppData {
        registry_state: RegistryState::new(&globals),
        dmabuf_state: DmabufState::new(&globals, &qh),
        feedback: None,
    };

    match app_data.dmabuf_state.version() {
        Some(4..) => {
            let _ = app_data.dmabuf_state.get_default_feedback(&qh).ok()?;

            let feedback = loop {
                let _ = event_queue.blocking_dispatch(&mut app_data).ok()?;
                if let Some(feedback) = app_data.feedback.as_ref() {
                    break feedback;
                }
            };

            super::ids_from_dev(feedback.main_device())
        }
        _ => None,
    }
}

delegate_dmabuf!(AppData);
delegate_registry!(AppData);
