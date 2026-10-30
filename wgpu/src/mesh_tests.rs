//! These draw on a GPU, so they are ignored by default; run them with
//! `cargo test -p iced_wgpu -- --ignored mesh_tests` on a machine with one.
use crate::Renderer;
use crate::core::renderer::Headless;
use crate::core::{Color, Font, Pixels, Point, Rectangle, Renderer as _, Size, Transformation};
use crate::graphics::Viewport;
use crate::graphics::color;
use crate::graphics::mesh::{Indexed, Mesh, Renderer as _, SolidVertex2D};

const SIDE: u32 = 20;

/// A red square over the whole screen, drawn with the given indices.
fn square(indices: Vec<u32>, clip_bounds: Rectangle) -> Mesh {
    let side = SIDE as f32;
    let red = color::pack(Color::from_rgb(1.0, 0.0, 0.0));

    Mesh::Solid {
        buffers: Indexed {
            vertices: [[0.0, 0.0], [side, 0.0], [side, side], [0.0, side]]
                .into_iter()
                .map(|position| SolidVertex2D {
                    position,
                    color: red,
                })
                .collect(),
            indices,
        },
        transformation: Transformation::IDENTITY,
        clip_bounds,
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn a_mesh_after_a_clipped_one_draws_its_own_triangles() {
    let screen = Rectangle::with_size(Size::new(SIDE as f32, SIDE as f32));

    let mut renderer = futures::executor::block_on(<Renderer as Headless>::new(
        Font::DEFAULT,
        Pixels(16.0),
        None,
    ))
    .expect("a GPU adapter");
    renderer.reset(screen);

    // Out of view, so it is skipped. Its indices would draw nothing.
    renderer.draw_mesh(square(
        vec![0, 0, 0, 0, 0, 0],
        Rectangle::new(Point::new(100.0, 100.0), Size::new(10.0, 10.0)),
    ));
    renderer.draw_mesh(square(vec![0, 1, 2, 0, 2, 3], screen));

    let pixels = renderer.screenshot(
        &Viewport::with_physical_size(Size::new(SIDE, SIDE), 1.0),
        Color::TRANSPARENT,
    );

    let center = ((SIDE / 2 * SIDE + SIDE / 2) * 4) as usize;
    assert_eq!(pixels[center..center + 4], [255, 0, 0, 255]);
}
