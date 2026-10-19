//! A bunch of backend-agnostic types that can be leveraged to build a renderer
//! for [`iced`].
//!
//! ![The native path of the Iced ecosystem](https://github.com/iced-rs/iced/blob/0525d76ff94e828b7b21634fa94a747022001c83/docs/graphs/native.png?raw=true)
//!
//! [`iced`]: https://github.com/iced-rs/iced
#![doc(
    html_logo_url = "https://raw.githubusercontent.com/iced-rs/iced/9ab6923e943f784985e9ef9ca28b10278297225d/docs/logo.svg"
)]
#![cfg_attr(docsrs, feature(doc_cfg))]
mod antialiasing;
mod settings;
mod viewport;

pub mod cache;
pub mod color;
pub mod compositor;
pub mod damage;
pub mod error;
pub mod gradient;
pub mod image;
pub mod layer;
pub mod mesh;
pub mod opacity;
pub mod shell;
pub mod text;

#[cfg(feature = "geometry")]
pub mod geometry;

pub use antialiasing::Antialiasing;
pub use cache::Cache;
pub use compositor::Compositor;
pub use error::Error;
pub use gradient::Gradient;
pub use image::Image;
pub use layer::Layer;
pub use mesh::Mesh;
pub use settings::Settings;
pub use shell::Shell;
pub use text::Text;
pub use viewport::Viewport;

pub use iced_core as core;
pub use iced_futures as futures;

/// `bounds` in device pixels, with each edge rounded to a whole pixel as a snapped quad's
/// are, so surfaces snapped apart still line up.
pub fn snap_to_pixels(bounds: core::Rectangle) -> core::Rectangle {
    let edge = |value: f32| (value + 0.001).round();
    let (x, y) = (edge(bounds.x), edge(bounds.y));

    core::Rectangle {
        x,
        y,
        width: edge(bounds.x + bounds.width) - x,
        height: edge(bounds.y + bounds.height) - y,
    }
}
