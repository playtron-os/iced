//! Display rendering results on windows.
pub mod compositor;

#[cfg(all(
    unix,
    feature = "wayland",
    not(target_os = "macos"),
    not(target_os = "redox")
))]
mod wayland;

#[cfg(all(
    unix,
    feature = "x11",
    not(target_os = "macos"),
    not(target_os = "redox")
))]
mod x11;

pub use compositor::Compositor;
pub use wgpu::Surface;

/// The pci ids of the gpu the display server presents from, if it will tell us.
#[cfg(all(
    unix,
    any(feature = "wayland", feature = "x11"),
    not(target_os = "macos"),
    not(target_os = "redox")
))]
pub(crate) fn scanout_device_ids<W: crate::graphics::compositor::Window>(
    window: &W,
) -> Option<(u16, u16)> {
    #[cfg(feature = "wayland")]
    if let Some(ids) = wayland::get_wayland_device_ids(window) {
        return Some(ids);
    }

    #[cfg(feature = "x11")]
    if let Some(ids) = x11::get_x11_device_ids(window) {
        return Some(ids);
    }

    None
}

/// Resolve a drm device number to its pci `(vendor, device)` ids.
#[cfg(all(
    unix,
    any(feature = "wayland", feature = "x11"),
    not(target_os = "macos"),
    not(target_os = "redox")
))]
fn ids_from_dev(dev: u64) -> Option<(u16, u16)> {
    use rustix::fs::{major, minor};

    let path = std::path::PathBuf::from(format!(
        "/sys/dev/char/{}:{}/device",
        major(dev),
        minor(dev)
    ));

    let read_id = |name: &str| {
        let contents = std::fs::read_to_string(path.join(name)).ok()?;
        u16::from_str_radix(contents.trim().trim_start_matches("0x"), 16).ok()
    };

    Some((read_id("vendor")?, read_id("device")?))
}
