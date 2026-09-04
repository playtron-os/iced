//! Ask the X server which gpu it renders on.
use std::{
    fs,
    io::{BufRead, BufReader},
    path::Path,
};

use crate::graphics::compositor::Window;

use as_raw_xcb_connection::AsRawXcbConnection;
use raw_window_handle::{RawDisplayHandle, XcbDisplayHandle, XlibDisplayHandle};
use rustix::fs::{fstat, stat};
use tiny_xlib::Display;
use x11rb::{
    connection::{Connection, RequestConnection},
    protocol::{
        dri3::{ConnectionExt as _, X11_EXTENSION_NAME as DRI3_NAME},
        randr::{ConnectionExt as _, ProviderCapability, X11_EXTENSION_NAME as RANDR_NAME},
    },
    xcb_ffi::XCBConnection,
};

/// The pci ids of the gpu backing this X display.
pub fn get_x11_device_ids<W: Window>(window: &W) -> Option<(u16, u16)> {
    x11rb::xcb_ffi::load_libxcb().ok()?;

    let (conn, screen) = match window.display_handle().map(|handle| handle.as_raw()) {
        #[allow(unsafe_code)]
        Ok(RawDisplayHandle::Xlib(XlibDisplayHandle {
            display, screen, ..
        })) => match display {
            // Intentionally leaks the display: closing the connection is not ours to do.
            Some(ptr) => unsafe {
                let xlib_display = Display::from_ptr(ptr.as_ptr());
                let conn = XCBConnection::from_raw_xcb_connection(
                    xlib_display.as_raw_xcb_connection().cast(),
                    false,
                )
                .ok();

                (conn?, screen)
            },
            None => (XCBConnection::connect(None).ok()?.0, screen),
        },
        Ok(RawDisplayHandle::Xcb(XcbDisplayHandle {
            connection, screen, ..
        })) => match connection {
            #[allow(unsafe_code)]
            Some(ptr) => (
                unsafe { XCBConnection::from_raw_xcb_connection(ptr.as_ptr(), false).ok()? },
                screen,
            ),
            None => (XCBConnection::connect(None).ok()?.0, screen),
        },
        _ => return None,
    };
    let root = conn.setup().roots[screen as usize].root;

    // The proprietary nvidia driver advertises DRI2 and DRI3 but returns nothing useful
    // from either, and an X11 EGL display gives back no EGLDevice. Hence the detour below.
    let _ = conn.extension_information(RANDR_NAME).ok()??;
    let version = conn.randr_query_version(1, 4).ok()?.reply().ok()?;
    if version.major_version < 1 || (version.major_version == 1 && version.minor_version < 4) {
        return None;
    }

    // The first Source Output provider is the device actually driving the outputs.
    let randr = conn.randr_get_providers(root).ok()?.reply().ok()?;
    let mut name = None;
    for provider in randr.providers {
        let info = conn
            .randr_get_provider_info(provider, randr.timestamp)
            .ok()?
            .reply()
            .ok()?;
        if info
            .capabilities
            .contains(ProviderCapability::SOURCE_OUTPUT)
            || name.is_none()
        {
            name = std::str::from_utf8(&info.name)
                .ok()
                .map(ToString::to_string);
        }
    }

    // A provider named `NVIDIA-x` gives x = the /dev/nvidiaX minor, which we can walk back
    // to a /dev/dri node through sysfs.
    let Some(number) =
        name.and_then(|name| name.trim().strip_prefix("NVIDIA-")?.parse::<u32>().ok())
    else {
        let _ = conn.extension_information(DRI3_NAME).ok()??;
        // NONE tells the X server to use the RandR provider.
        let dri3 = conn.dri3_open(root, x11rb::NONE).ok()?.reply().ok()?;
        return super::ids_from_dev(fstat(dri3.device_fd).ok()?.st_rdev);
    };

    for busid in fs::read_dir("/proc/driver/nvidia/gpus").ok()?.flatten() {
        for line in BufReader::new(fs::File::open(busid.path().join("information")).ok()?)
            .lines()
            .map_while(Result::ok)
        {
            let Some(minor) = line
                .strip_prefix("Device Minor")
                .and_then(|rest| rest.split_once(':'))
                .and_then(|(_, num)| num.trim().parse::<u32>().ok())
            else {
                continue;
            };
            if minor != number {
                continue;
            }
            let drm = Path::new("/sys/module/nvidia/drivers/pci:nvidia/")
                .join(busid.file_name())
                .join("drm");
            for device in fs::read_dir(drm).ok()?.flatten() {
                let device = device.file_name();
                let name = device.to_string_lossy();
                if name.starts_with("card") || name.starts_with("render") {
                    let stat = stat(Path::new("/dev/dri").join(&device)).ok()?;
                    return super::ids_from_dev(stat.st_rdev);
                }
            }
        }
    }

    None
}
