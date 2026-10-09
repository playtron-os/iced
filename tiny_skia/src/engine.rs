use crate::Primitive;
use crate::core::renderer::Quad;
use crate::core::{Background, Color, Gradient, Point, Rectangle, Size, Transformation, Vector};
use crate::graphics::{Image, Text};
use crate::shadow;
use crate::text;

#[derive(Debug)]
pub struct Engine {
    text_pipeline: text::Pipeline,

    #[cfg(feature = "image")]
    pub(crate) raster_pipeline: crate::raster::Pipeline,
    #[cfg(feature = "svg")]
    pub(crate) vector_pipeline: crate::vector::Pipeline,
}

impl Engine {
    pub fn new() -> Self {
        Self {
            text_pipeline: text::Pipeline::new(),
            #[cfg(feature = "image")]
            raster_pipeline: crate::raster::Pipeline::new(),
            #[cfg(feature = "svg")]
            vector_pipeline: crate::vector::Pipeline::new(),
        }
    }

    pub fn draw_quad(
        &mut self,
        quad: &Quad,
        background: &Background,
        transformation: Transformation,
        pixels: &mut tiny_skia::PixmapMut<'_>,
        clip_mask: &mut tiny_skia::Mask,
        clip_bounds: Rectangle,
        force_clip: bool,
    ) {
        let physical_bounds = quad.bounds * transformation;
        let shadow_bounds =
            (quad.shadow.color.a > 0.0).then(|| shadow::bounds(quad) * transformation);
        let fill_damaged = clip_bounds.intersects(&physical_bounds);

        // A shadow reaches past its quad, so damage on the shadow alone must still
        // repaint it; the damaged area was cleared to the background.
        if !fill_damaged && !shadow_bounds.is_some_and(|bounds| clip_bounds.intersects(&bounds)) {
            return;
        }

        let transform = into_transform(transformation);

        // Resolve per-side border widths
        let border_widths = quad
            .border
            .widths()
            .map(|width| snap_border_width(width, transformation.scale_factor()));
        let max_border_width = border_widths[0]
            .max(border_widths[1])
            .max(border_widths[2])
            .max(border_widths[3])
            .min(quad.bounds.width / 2.0)
            .min(quad.bounds.height / 2.0);

        // For fill/radius calculations, use max border width
        let border_width = max_border_width;

        let mut fill_border_radius = <[f32; 4]>::from(quad.border.radius);

        for radius in &mut fill_border_radius {
            *radius = (*radius)
                .min(quad.bounds.width / 2.0)
                .min(quad.bounds.height / 2.0);
        }

        let path = rounded_rectangle(quad.bounds, fill_border_radius);

        let shadow = |pixels: &mut tiny_skia::PixmapMut<'_>, mask: &tiny_skia::Mask| {
            if let Some(bounds) = shadow_bounds
                && let Some((x, y, pixmap)) = shadow::pixmap(
                    quad,
                    bounds,
                    fill_border_radius,
                    transformation,
                    clip_bounds,
                )
            {
                draw_translated(pixels, x, y, &pixmap, mask);
            }
        };

        // An outset shadow goes under the quad, an inset one over its background
        // and under its border.
        if !quad.shadow.inset {
            shadow(pixels, clip_mask);
        }

        if !fill_damaged {
            return;
        }

        let mask: &tiny_skia::Mask = clip_mask;
        let clip_mask = (force_clip || !physical_bounds.is_within(&clip_bounds)).then_some(mask);

        let paint = tiny_skia::Paint {
            shader: match background {
                Background::Color(color) => tiny_skia::Shader::SolidColor(into_color(*color)),
                Background::Gradient(Gradient::Linear(linear)) => {
                    let (start, end) = linear.angle.to_distance(&quad.bounds);

                    let stops: Vec<tiny_skia::GradientStop> = linear
                        .stops
                        .into_iter()
                        .flatten()
                        .map(|stop| {
                            tiny_skia::GradientStop::new(
                                stop.offset,
                                tiny_skia::Color::from_rgba(
                                    stop.color.b,
                                    stop.color.g,
                                    stop.color.r,
                                    stop.color.a,
                                )
                                .expect("Create color"),
                            )
                        })
                        .collect();

                    tiny_skia::LinearGradient::new(
                        tiny_skia::Point {
                            x: start.x,
                            y: start.y,
                        },
                        tiny_skia::Point { x: end.x, y: end.y },
                        if stops.is_empty() {
                            vec![tiny_skia::GradientStop::new(0.0, tiny_skia::Color::BLACK)]
                        } else {
                            stops
                        },
                        tiny_skia::SpreadMode::Pad,
                        tiny_skia::Transform::identity(),
                    )
                    .expect("Create linear gradient")
                }
                Background::Gradient(Gradient::Radial(radial)) => elliptical_gradient(
                    Point::new(
                        quad.bounds.x + radial.center.x * quad.bounds.width,
                        quad.bounds.y + radial.center.y * quad.bounds.height,
                    ),
                    Vector::new(
                        radial.radius_x * quad.bounds.width,
                        radial.radius_y * quad.bounds.height,
                    ),
                    radial
                        .stops
                        .into_iter()
                        .flatten()
                        .map(|stop| (stop.offset, stop.color)),
                ),
                Background::Gradient(Gradient::Conic(conic)) => {
                    // tiny_skia doesn't support conic gradients natively.
                    // Fall back to a solid color from the first stop.
                    let color = conic
                        .stops
                        .iter()
                        .flatten()
                        .next()
                        .map(|stop| stop.color)
                        .unwrap_or(Color::TRANSPARENT);

                    tiny_skia::Shader::SolidColor(into_color(color))
                }
            },
            anti_alias: true,
            ..tiny_skia::Paint::default()
        };

        // A shadow is often carried by a quad with nothing to fill, and blending a
        // transparent fill over the whole shape changes no pixel.
        let invisible = matches!(background, Background::Color(color) if color.a == 0.0);

        // A rounded clip layer's mask spans the whole layer, so only a rect mask
        // can stand in for the damage the path would otherwise be filled across.
        let part = (!invisible && !force_clip && clip_mask.is_some())
            .then(|| damaged_part(quad.bounds, fill_border_radius, transformation, clip_bounds))
            .flatten();

        if !invisible {
            pixels.fill_path(
                part.as_ref().unwrap_or(&path),
                &paint,
                tiny_skia::FillRule::EvenOdd,
                transform,
                clip_mask,
            );
        }

        if quad.shadow.inset {
            shadow(pixels, mask);
        }

        if border_width > 0.0 {
            let border_paint = tiny_skia::Paint {
                shader: tiny_skia::Shader::SolidColor(into_color(quad.border.color)),
                anti_alias: true,
                ..tiny_skia::Paint::default()
            };

            let all_equal = border_widths[0] == border_widths[1]
                && border_widths[1] == border_widths[2]
                && border_widths[2] == border_widths[3];

            // Border path is offset by half the border width
            let border_bounds = Rectangle {
                x: quad.bounds.x + border_width / 2.0,
                y: quad.bounds.y + border_width / 2.0,
                width: quad.bounds.width - border_width,
                height: quad.bounds.height - border_width,
            };

            // A stroke along that path follows the outline exactly when every
            // corner is square or rounder than half the width.
            let mut border_radius = <[f32; 4]>::from(quad.border.radius);
            let mut strokes_exactly = all_equal;

            for radius in &mut border_radius {
                *radius = if *radius == 0.0 {
                    0.0
                } else if *radius > border_width / 2.0 {
                    *radius - border_width / 2.0
                } else {
                    strokes_exactly = false;
                    0.0
                }
                .min(border_bounds.width / 2.0)
                .min(border_bounds.height / 2.0);
            }

            if strokes_exactly {
                let border_path = rounded_rectangle(border_bounds, border_radius);

                pixels.stroke_path(
                    &border_path,
                    &border_paint,
                    &tiny_skia::Stroke {
                        width: border_width,
                        dash: stroke_dash(&quad.border, quad.bounds.size(), border_width),
                        ..tiny_skia::Stroke::default()
                    },
                    transform,
                    clip_mask,
                );
            } else if all_equal
                && let Some(dash) = stroke_dash(&quad.border, quad.bounds.size(), border_width)
            {
                // A dash has to follow a stroke: square its tight corners and
                // trim them to the rounded outline.
                let outline = match clip_mask {
                    Some(clip_mask) => {
                        let mut mask = clip_mask.clone();
                        mask.intersect_path(&path, tiny_skia::FillRule::EvenOdd, true, transform);
                        mask
                    }
                    None => {
                        let mut mask = tiny_skia::Mask::new(pixels.width(), pixels.height())
                            .expect("Create border mask");
                        mask.fill_path(&path, tiny_skia::FillRule::EvenOdd, true, transform);
                        mask
                    }
                };

                pixels.stroke_path(
                    &rounded_rectangle(border_bounds, border_radius),
                    &border_paint,
                    &tiny_skia::Stroke {
                        width: border_width,
                        dash: Some(dash),
                        ..tiny_skia::Stroke::default()
                    },
                    transform,
                    Some(&outline),
                );
            } else {
                // What CSS paints: the outline minus the padding edge, whose
                // corners shrink by the width of the side they meet on each axis.
                pixels.fill_path(
                    &border_ring(quad.bounds, fill_border_radius, border_widths),
                    &border_paint,
                    tiny_skia::FillRule::EvenOdd,
                    transform,
                    clip_mask,
                );
            }
        }
    }

    pub fn draw_text(
        &mut self,
        text: &Text,
        transformation: Transformation,
        pixels: &mut tiny_skia::PixmapMut<'_>,
        clip_mask: &mut tiny_skia::Mask,
        clip_bounds: Rectangle,
    ) {
        match text {
            Text::Paragraph {
                paragraph,
                position,
                color,
                clip_bounds: local_clip_bounds,
                transformation: local_transformation,
            } => {
                let transformation = transformation * *local_transformation;
                let Some(clip_bounds) =
                    clip_bounds.intersection(&(*local_clip_bounds * transformation))
                else {
                    return;
                };

                let physical_bounds =
                    Rectangle::new(*position, paragraph.min_bounds) * transformation;

                if !clip_bounds.intersects(&physical_bounds) {
                    return;
                }

                let clip_mask = match physical_bounds.is_within(&clip_bounds) {
                    true => None,
                    false => {
                        adjust_clip_mask(clip_mask, clip_bounds);
                        Some(clip_mask as &_)
                    }
                };

                self.text_pipeline.draw_paragraph(
                    paragraph,
                    *position,
                    *color,
                    pixels,
                    clip_mask,
                    transformation,
                );
            }
            Text::Editor {
                editor,
                position,
                color,
                clip_bounds: local_clip_bounds,
                transformation: local_transformation,
            } => {
                let transformation = transformation * *local_transformation;

                let Some(clip_bounds) =
                    clip_bounds.intersection(&(*local_clip_bounds * transformation))
                else {
                    return;
                };

                adjust_clip_mask(clip_mask, clip_bounds);

                self.text_pipeline.draw_editor(
                    editor,
                    *position,
                    *color,
                    pixels,
                    Some(clip_mask),
                    transformation,
                );
            }
            Text::Cached {
                content,
                bounds,
                color,
                size,
                line_height,
                font,
                align_x,
                align_y,
                shaping,
                wrapping,
                ellipsis,
                letter_spacing,
                clip_bounds: local_clip_bounds,
            } => {
                let physical_bounds = *local_clip_bounds * transformation;

                if !clip_bounds.intersects(&physical_bounds) {
                    return;
                }

                let clip_mask = match physical_bounds.is_within(&clip_bounds) {
                    true => None,
                    false => {
                        adjust_clip_mask(clip_mask, clip_bounds);
                        Some(clip_mask as &_)
                    }
                };

                self.text_pipeline.draw_cached(
                    content,
                    *bounds,
                    *color,
                    *size,
                    *line_height,
                    *font,
                    *align_x,
                    *align_y,
                    *shaping,
                    *wrapping,
                    *ellipsis,
                    *letter_spacing,
                    pixels,
                    clip_mask,
                    transformation,
                );
            }
            Text::Raw {
                raw,
                transformation: local_transformation,
            } => {
                let Some(buffer) = raw.buffer.upgrade() else {
                    return;
                };

                let transformation = transformation * *local_transformation;
                let (width, height) = buffer.size();

                let physical_bounds = Rectangle::new(
                    raw.position,
                    Size::new(
                        width.unwrap_or(clip_bounds.width),
                        height.unwrap_or(clip_bounds.height),
                    ),
                ) * transformation;

                if !clip_bounds.intersects(&physical_bounds) {
                    return;
                }

                let clip_mask =
                    (!physical_bounds.is_within(&clip_bounds)).then_some(clip_mask as &_);

                self.text_pipeline.draw_raw(
                    &buffer,
                    raw.position,
                    raw.color,
                    pixels,
                    clip_mask,
                    transformation,
                );
            }
        }
    }

    pub fn draw_primitive(
        &mut self,
        primitive: &Primitive,
        transformation: Transformation,
        pixels: &mut tiny_skia::PixmapMut<'_>,
        clip_mask: &mut tiny_skia::Mask,
        clip_bounds: Rectangle,
    ) {
        match primitive {
            Primitive::Fill { path, paint, rule } => {
                let physical_bounds = {
                    let bounds = path.bounds();

                    Rectangle {
                        x: bounds.x(),
                        y: bounds.y(),
                        width: bounds.width(),
                        height: bounds.height(),
                    } * transformation
                };

                if !clip_bounds.intersects(&physical_bounds) {
                    return;
                }

                let clip_mask =
                    (!physical_bounds.is_within(&clip_bounds)).then_some(clip_mask as &_);

                pixels.fill_path(
                    path,
                    paint,
                    *rule,
                    into_transform(transformation),
                    clip_mask,
                );
            }
            Primitive::Stroke {
                path,
                paint,
                stroke,
            } => {
                let physical_bounds = {
                    let bounds = path.bounds();

                    Rectangle {
                        x: bounds.x() - stroke.width / 2.0,
                        y: bounds.y() - stroke.width / 2.0,
                        width: bounds.width() + stroke.width,
                        height: bounds.height() + stroke.width,
                    } * transformation
                };

                if !clip_bounds.intersects(&physical_bounds) {
                    return;
                }

                let clip_mask =
                    (!physical_bounds.is_within(&clip_bounds)).then_some(clip_mask as &_);

                pixels.stroke_path(
                    path,
                    paint,
                    stroke,
                    into_transform(transformation),
                    clip_mask,
                );
            }
        }
    }

    pub fn draw_image(
        &mut self,
        image: &Image,
        _transformation: Transformation,
        _pixels: &mut tiny_skia::PixmapMut<'_>,
        _clip_mask: &mut tiny_skia::Mask,
        _clip_bounds: Rectangle,
    ) {
        match image {
            #[cfg(feature = "image")]
            Image::Raster {
                image,
                bounds,
                clip_bounds: local_clip_bounds,
            } => {
                let physical_bounds = *local_clip_bounds * _transformation;

                let Some(clip_bounds) = physical_bounds.intersection(&_clip_bounds) else {
                    return;
                };

                // TODO: Border radius
                adjust_clip_mask(_clip_mask, clip_bounds);

                let center = physical_bounds.center();
                let radians = f32::from(image.rotation);

                let transform = into_transform(_transformation).post_rotate_at(
                    radians.to_degrees(),
                    center.x,
                    center.y,
                );

                self.raster_pipeline.draw(
                    &image.handle,
                    image.filter_method,
                    *bounds,
                    image.opacity,
                    _pixels,
                    transform,
                    Some(_clip_mask),
                );
            }
            #[cfg(feature = "svg")]
            Image::Vector { svg, bounds, .. } => {
                let physical_bounds = *bounds * _transformation;

                if !_clip_bounds.intersects(&physical_bounds) {
                    return;
                }

                let clip_mask =
                    (!physical_bounds.is_within(&_clip_bounds)).then_some(_clip_mask as &_);

                let center = physical_bounds.center();
                let radians = f32::from(svg.rotation);

                let transform = into_transform(_transformation).post_rotate_at(
                    radians.to_degrees(),
                    center.x,
                    center.y,
                );

                self.vector_pipeline.draw(
                    &svg.handle,
                    svg.color,
                    *bounds,
                    svg.rasterize_size,
                    svg.opacity,
                    _pixels,
                    transform,
                    clip_mask,
                );
            }
            #[cfg(not(feature = "image"))]
            Image::Raster { .. } => {
                log::warn!("Unsupported primitive in `iced_tiny_skia`: {image:?}",);
            }
            #[cfg(not(feature = "svg"))]
            Image::Vector { .. } => {
                log::warn!("Unsupported primitive in `iced_tiny_skia`: {image:?}",);
            }
        }
    }

    pub fn trim(&mut self) {
        self.text_pipeline.trim_cache();

        #[cfg(feature = "image")]
        self.raster_pipeline.trim_cache();

        #[cfg(feature = "svg")]
        self.vector_pipeline.trim_cache();
    }
}

pub fn into_color(color: Color) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba(color.b, color.g, color.r, color.a)
        .expect("Convert color from iced to tiny_skia")
}

/// A radial gradient over the ellipse with `radii` around `center`, as CSS's
/// `radial-gradient(ellipse …)` draws it: a unit circle's gradient stretched
/// onto the ellipse, since tiny-skia only draws circles. A flat ellipse paints
/// its last stop.
pub(crate) fn elliptical_gradient(
    center: Point,
    radii: Vector,
    stops: impl IntoIterator<Item = (f32, Color)>,
) -> tiny_skia::Shader<'static> {
    let stops: Vec<(f32, Color)> = stops.into_iter().collect();

    let Some(&(_, last)) = stops.last() else {
        return tiny_skia::Shader::SolidColor(tiny_skia::Color::BLACK);
    };

    if !(radii.x > 0.0 && radii.y > 0.0) {
        return tiny_skia::Shader::SolidColor(into_color(last));
    }

    tiny_skia::RadialGradient::new(
        tiny_skia::Point::from_xy(0.0, 0.0),
        tiny_skia::Point::from_xy(0.0, 0.0),
        1.0,
        stops
            .iter()
            .map(|(offset, color)| tiny_skia::GradientStop::new(*offset, into_color(*color)))
            .collect(),
        tiny_skia::SpreadMode::Pad,
        tiny_skia::Transform::from_row(radii.x, 0.0, 0.0, radii.y, center.x, center.y),
    )
    .unwrap_or(tiny_skia::Shader::SolidColor(into_color(last)))
}

fn into_transform(transformation: Transformation) -> tiny_skia::Transform {
    let translation = transformation.translation();

    tiny_skia::Transform {
        sx: transformation.scale_factor(),
        kx: 0.0,
        ky: 0.0,
        sy: transformation.scale_factor(),
        tx: translation.x,
        ty: translation.y,
    }
}

/// A border `width` snapped as CSS snaps a border width to device pixels at
/// `scale`: down to a whole pixel, but a border thinner than a pixel is one
/// pixel wide rather than a faint anti-aliased line.
fn snap_border_width(width: f32, scale: f32) -> f32 {
    let physical = width * scale;

    if physical > 0.0 && physical < 1.0 {
        1.0 / scale
    } else {
        (physical + 1.0e-5).floor() / scale
    }
}

/// The border's dash pattern as a stroke dash, when it has one, fitted round
/// the centre line of a `width` border on a box of `size`.
fn stroke_dash(
    border: &crate::core::Border,
    size: Size,
    width: f32,
) -> Option<tiny_skia::StrokeDash> {
    let dash = border.dash?.around(size, border.radius.into(), width);
    tiny_skia::StrokeDash::new(vec![dash.on, dash.off], 0.0)
}

pub(crate) fn rounded_rectangle(bounds: Rectangle, border_radius: [f32; 4]) -> tiny_skia::Path {
    let [top_left, top_right, bottom_right, bottom_left] = border_radius;

    if top_left == 0.0 && top_right == 0.0 && bottom_right == 0.0 && bottom_left == 0.0 {
        return tiny_skia::PathBuilder::from_rect(
            tiny_skia::Rect::from_xywh(bounds.x, bounds.y, bounds.width, bounds.height)
                .expect("Build quad rectangle"),
        );
    }

    if top_left == top_right
        && top_left == bottom_right
        && top_left == bottom_left
        && top_left == bounds.width / 2.0
        && top_left == bounds.height / 2.0
    {
        return tiny_skia::PathBuilder::from_circle(
            bounds.x + bounds.width / 2.0,
            bounds.y + bounds.height / 2.0,
            top_left,
        )
        .expect("Build circle path");
    }

    let mut builder = tiny_skia::PathBuilder::new();

    builder.move_to(bounds.x + top_left, bounds.y);
    builder.line_to(bounds.x + bounds.width - top_right, bounds.y);

    if top_right > 0.0 {
        arc_to(
            &mut builder,
            bounds.x + bounds.width - top_right,
            bounds.y,
            bounds.x + bounds.width,
            bounds.y + top_right,
            top_right,
        );
    }

    maybe_line_to(
        &mut builder,
        bounds.x + bounds.width,
        bounds.y + bounds.height - bottom_right,
    );

    if bottom_right > 0.0 {
        arc_to(
            &mut builder,
            bounds.x + bounds.width,
            bounds.y + bounds.height - bottom_right,
            bounds.x + bounds.width - bottom_right,
            bounds.y + bounds.height,
            bottom_right,
        );
    }

    maybe_line_to(
        &mut builder,
        bounds.x + bottom_left,
        bounds.y + bounds.height,
    );

    if bottom_left > 0.0 {
        arc_to(
            &mut builder,
            bounds.x + bottom_left,
            bounds.y + bounds.height,
            bounds.x,
            bounds.y + bounds.height - bottom_left,
            bottom_left,
        );
    }

    maybe_line_to(&mut builder, bounds.x, bounds.y + top_left);

    if top_left > 0.0 {
        arc_to(
            &mut builder,
            bounds.x,
            bounds.y + top_left,
            bounds.x + top_left,
            bounds.y,
            top_left,
        );
    }

    builder.finish().expect("Build rounded rectangle path")
}

/// The area CSS paints a border in: the rounded outline minus the padding edge,
/// to fill with [`tiny_skia::FillRule::EvenOdd`].
///
/// The padding edge is inset by each side's width, and each of its corners
/// shrinks by the width of the side it meets on that axis, so a corner between
/// sides of different widths is a quarter ellipse.
fn border_ring(bounds: Rectangle, radii: [f32; 4], widths: [f32; 4]) -> tiny_skia::Path {
    let [top, right, bottom, left] = widths.map(|width| width.max(0.0));
    let [top_left, top_right, bottom_right, bottom_left] = radii;

    let mut builder = tiny_skia::PathBuilder::new();
    builder.push_path(&rounded_rectangle(bounds, radii));

    let inner = Rectangle {
        x: bounds.x + left,
        y: bounds.y + top,
        width: bounds.width - left - right,
        height: bounds.height - top - bottom,
    };

    if inner.width > 0.0 && inner.height > 0.0 {
        let corner = |radius: f32, horizontal: f32, vertical: f32| {
            let corner = ((radius - horizontal).max(0.0), (radius - vertical).max(0.0));

            if corner.0 > 0.0 && corner.1 > 0.0 {
                corner
            } else {
                (0.0, 0.0)
            }
        };

        builder.push_path(&elliptical_rectangle(
            inner,
            [
                corner(top_left, left, top),
                corner(top_right, right, top),
                corner(bottom_right, right, bottom),
                corner(bottom_left, left, bottom),
            ],
        ));
    }

    builder.finish().expect("Build border path")
}

/// A rectangle with quarter-ellipse corners, given as (horizontal, vertical)
/// radii from the top-left corner clockwise.
fn elliptical_rectangle(bounds: Rectangle, radii: [(f32, f32); 4]) -> tiny_skia::Path {
    let [top_left, top_right, bottom_right, bottom_left] = radii;
    let (left, top) = (bounds.x, bounds.y);
    let (right, bottom) = (bounds.x + bounds.width, bounds.y + bounds.height);

    let mut builder = tiny_skia::PathBuilder::new();

    builder.move_to(left + top_left.0, top);
    maybe_line_to(&mut builder, right - top_right.0, top);
    elliptical_arc_to(&mut builder, right, top + top_right.1, top_right);
    maybe_line_to(&mut builder, right, bottom - bottom_right.1);
    elliptical_arc_to(&mut builder, right - bottom_right.0, bottom, bottom_right);
    maybe_line_to(&mut builder, left + bottom_left.0, bottom);
    elliptical_arc_to(&mut builder, left, bottom - bottom_left.1, bottom_left);
    maybe_line_to(&mut builder, left, top + top_left.1);
    elliptical_arc_to(&mut builder, left + top_left.0, top, top_left);
    builder.close();

    builder.finish().expect("Build elliptical rectangle path")
}

/// A clockwise quarter ellipse from the builder's last point to (`x`, `y`).
fn elliptical_arc_to(builder: &mut tiny_skia::PathBuilder, x: f32, y: f32, radii: (f32, f32)) {
    let Some(from) = builder.last_point() else {
        return;
    };

    if radii.0 <= 0.0 || radii.1 <= 0.0 {
        maybe_line_to(builder, x, y);
        return;
    }

    let arc = kurbo::SvgArc {
        from: kurbo::Point::new(f64::from(from.x), f64::from(from.y)),
        to: kurbo::Point::new(f64::from(x), f64::from(y)),
        radii: kurbo::Vec2::new(f64::from(radii.0), f64::from(radii.1)),
        x_rotation: 0.0,
        large_arc: false,
        sweep: true,
    };

    match kurbo::Arc::from_svg_arc(&arc) {
        Some(arc) => {
            arc.to_cubic_beziers(0.1, |p1, p2, p| {
                builder.cubic_to(
                    p1.x as f32,
                    p1.y as f32,
                    p2.x as f32,
                    p2.y as f32,
                    p.x as f32,
                    p.y as f32,
                );
            });
        }
        None => builder.line_to(x, y),
    }
}

fn maybe_line_to(path: &mut tiny_skia::PathBuilder, x: f32, y: f32) {
    if path.last_point() != Some(tiny_skia::Point { x, y }) {
        path.line_to(x, y);
    }
}

fn arc_to(
    path: &mut tiny_skia::PathBuilder,
    x_from: f32,
    y_from: f32,
    x_to: f32,
    y_to: f32,
    radius: f32,
) {
    let svg_arc = kurbo::SvgArc {
        from: kurbo::Point::new(f64::from(x_from), f64::from(y_from)),
        to: kurbo::Point::new(f64::from(x_to), f64::from(y_to)),
        radii: kurbo::Vec2::new(f64::from(radius), f64::from(radius)),
        x_rotation: 0.0,
        large_arc: false,
        sweep: true,
    };

    match kurbo::Arc::from_svg_arc(&svg_arc) {
        Some(arc) => {
            arc.to_cubic_beziers(0.1, |p1, p2, p| {
                path.cubic_to(
                    p1.x as f32,
                    p1.y as f32,
                    p2.x as f32,
                    p2.y as f32,
                    p.x as f32,
                    p.y as f32,
                );
            });
        }
        None => {
            path.line_to(x_to, y_to);
        }
    }
}

/// The part of a rounded rectangle's fill that `clip_bounds` sees, as a smaller
/// path that rasterises identically there, or `None` to fill the whole shape.
///
/// Filling the whole shape costs its whole area however small the damage. The
/// part is cut a whole pixel outside the damage, where the clip mask discards it,
/// and keeps the shape's own straight edges and arcs, built from the same values
/// as `rounded_rectangle`, so every pixel the mask lets through is unchanged.
fn damaged_part(
    bounds: Rectangle,
    radii: [f32; 4],
    transformation: Transformation,
    clip_bounds: Rectangle,
) -> Option<tiny_skia::Path> {
    let [top_left, top_right, bottom_right, bottom_left] = radii;
    // `rounded_rectangle` draws this one as a circle, whose arcs differ.
    if radii.iter().all(|radius| *radius == top_left)
        && top_left == bounds.width / 2.0
        && top_left == bounds.height / 2.0
    {
        return None;
    }

    let scale = transformation.scale_factor();
    let physical = bounds * transformation;
    let visible = physical.intersection(&clip_bounds)?;
    let (physical_right, physical_bottom) =
        (physical.x + physical.width, physical.y + physical.height);

    // Each corner's arc, and a pixel around it, in physical pixels.
    let corner = |radius: f32, x: f32, y: f32| {
        let radius = radius * scale;
        radius > 0.0
            && visible.intersects(&Rectangle {
                x,
                y,
                width: radius + 1.0,
                height: radius + 1.0,
            })
    };
    let top = corner(top_left, physical.x, physical.y)
        || corner(
            top_right,
            physical_right - top_right * scale - 1.0,
            physical.y,
        );
    let bottom = corner(
        bottom_right,
        physical_right - bottom_right * scale - 1.0,
        physical_bottom - bottom_right * scale - 1.0,
    ) || corner(
        bottom_left,
        physical.x,
        physical_bottom - bottom_left * scale - 1.0,
    );

    // A cut a whole pixel beyond the damage, so its own edge is never let through.
    let clip_left = clip_bounds.x.floor() - 1.0;
    let clip_top = clip_bounds.y.floor() - 1.0;
    let clip_right = (clip_bounds.x + clip_bounds.width).ceil() + 1.0;
    let clip_bottom = (clip_bounds.y + clip_bounds.height).ceil() + 1.0;
    let (right, bottom_edge) = (bounds.x + bounds.width, bounds.y + bounds.height);
    let mut path = tiny_skia::PathBuilder::new();

    match (top, bottom) {
        (false, false) => {
            // Nearer than a pixel inside the shape, a cut would cover its edge.
            let left = if clip_left > physical.x {
                clip_left / scale
            } else {
                bounds.x
            };
            let top = if clip_top > physical.y {
                clip_top / scale
            } else {
                bounds.y
            };
            let right = if clip_right < physical_right {
                clip_right / scale
            } else {
                right
            };
            let bottom = if clip_bottom < physical_bottom {
                clip_bottom / scale
            } else {
                bottom_edge
            };
            return Some(tiny_skia::PathBuilder::from_rect(
                tiny_skia::Rect::from_ltrb(left, top, right, bottom)?,
            ));
        }
        (false, true) => {
            // From no lower than where the bottom arcs start, so the path never
            // doubles back; it must stay below the top corners.
            let cut =
                clip_top.min((physical_bottom - bottom_left.max(bottom_right) * scale).floor());
            if cut < physical.y + top_left.max(top_right) * scale + 1.0 {
                return None;
            }
            let cut = cut / scale;

            path.move_to(bounds.x, cut);
            path.line_to(right, cut);
            maybe_line_to(&mut path, right, bottom_edge - bottom_right);
            if bottom_right > 0.0 {
                arc_to(
                    &mut path,
                    right,
                    bottom_edge - bottom_right,
                    right - bottom_right,
                    bottom_edge,
                    bottom_right,
                );
            }
            maybe_line_to(&mut path, bounds.x + bottom_left, bottom_edge);
            if bottom_left > 0.0 {
                arc_to(
                    &mut path,
                    bounds.x + bottom_left,
                    bottom_edge,
                    bounds.x,
                    bottom_edge - bottom_left,
                    bottom_left,
                );
            }
        }
        (true, false) => {
            // Down to no higher than where the top arcs end, and above the
            // bottom corners.
            let cut = clip_bottom.max((physical.y + top_left.max(top_right) * scale).ceil());
            if cut > physical_bottom - bottom_left.max(bottom_right) * scale - 1.0 {
                return None;
            }
            let cut = cut / scale;

            path.move_to(bounds.x + top_left, bounds.y);
            path.line_to(right - top_right, bounds.y);
            if top_right > 0.0 {
                arc_to(
                    &mut path,
                    right - top_right,
                    bounds.y,
                    right,
                    bounds.y + top_right,
                    top_right,
                );
            }
            path.line_to(right, cut);
            path.line_to(bounds.x, cut);
            maybe_line_to(&mut path, bounds.x, bounds.y + top_left);
            if top_left > 0.0 {
                arc_to(
                    &mut path,
                    bounds.x,
                    bounds.y + top_left,
                    bounds.x + top_left,
                    bounds.y,
                    top_left,
                );
            }
        }
        (true, true) => return None,
    }

    path.close();
    path.finish()
}

/// `pixmap` drawn over `pixels` at (`x`, `y`) through `mask`, exactly as
/// `draw_pixmap` draws it.
///
/// `draw_pixmap` samples through a pattern shader in tiny-skia's float pipeline,
/// most of a shadow's cost. A translated copy needs no sampling, only that
/// pipeline's blend, repeated here operation for operation.
fn draw_translated(
    pixels: &mut tiny_skia::PixmapMut<'_>,
    x: i32,
    y: i32,
    pixmap: &tiny_skia::Pixmap,
    mask: &tiny_skia::Mask,
) {
    let (width, height) = (pixels.width() as i32, pixels.height() as i32);
    // `draw_pixmap` paints an extra row and column of edge pixels past a pixmap
    // hanging off the top or left; a shadow never does, so leave that to it.
    if x < 0 || y < 0 || (mask.width() as i32, mask.height() as i32) != (width, height) {
        pixels.draw_pixmap(
            x,
            y,
            pixmap.as_ref(),
            &tiny_skia::PixmapPaint::default(),
            tiny_skia::Transform::default(),
            Some(mask),
        );
        return;
    }

    let source_width = pixmap.width() as i32;
    let (left, top) = (x.max(0), y.max(0));
    let right = (x + source_width).min(width);
    let bottom = (y + pixmap.height() as i32).min(height);
    let source = pixmap.pixels();
    let mask = mask.data();
    let target = pixels.pixels_mut();

    for row in top..bottom {
        for column in left..right {
            let at = (row * width + column) as usize;
            let coverage = mask[at];
            let from = source[((row - y) * source_width + column - x) as usize];
            // Either way the pipeline would store the pixel it loaded.
            if coverage == 0 || from.alpha() == 0 {
                continue;
            }
            // Damage is cleared before it is drawn, and fully covering nothing
            // stores the source unchanged.
            if coverage == u8::MAX && bytemuck::cast::<_, u32>(target[at]) == 0 {
                target[at] = from;
                continue;
            }
            target[at] = source_over(from, target[at], coverage);
        }
    }
}

/// tiny-skia's float source-over of `source` onto `target` under `coverage`: load
/// as `n × (1/255)`, scale the source by `coverage / 255`, `target × (1 − alpha)
/// + source`, clamp, and round half to even as its SIMD conversion does.
fn source_over(
    source: tiny_skia::PremultipliedColorU8,
    target: tiny_skia::PremultipliedColorU8,
    coverage: u8,
) -> tiny_skia::PremultipliedColorU8 {
    const FACTOR: f32 = 1.0 / 255.0;
    let coverage = f32::from(coverage) / 255.0;
    let inverse = 1.0 - f32::from(source.alpha()) * FACTOR * coverage;
    let channel = |source: u8, target: u8| {
        let value = f32::from(target) * FACTOR * inverse + f32::from(source) * FACTOR * coverage;
        // Adding 2^23 leaves no fraction bits, so the default rounding mode (half to
        // even) does the rounding; `round_ties_even` is a libm call without SSE4.1.
        const ROUND: f32 = 8_388_608.0;
        ((value.clamp(0.0, 1.0) * 255.0 + ROUND) - ROUND) as u8
    };

    // Stored unchecked, as the pipeline stores it.
    bytemuck::cast([
        channel(source.red(), target.red()),
        channel(source.green(), target.green()),
        channel(source.blue(), target.blue()),
        channel(source.alpha(), target.alpha()),
    ])
}

pub fn adjust_clip_mask(clip_mask: &mut tiny_skia::Mask, bounds: Rectangle) {
    clip_mask.clear();

    let path = tiny_skia::PathBuilder::from_rect(
        tiny_skia::Rect::from_xywh(bounds.x, bounds.y, bounds.width, bounds.height)
            .expect("Create clip rectangle"),
    );

    clip_mask.fill_path(
        &path,
        tiny_skia::FillRule::EvenOdd,
        false,
        tiny_skia::Transform::default(),
    );
}

/// Resets `clip_mask` to a *rounded* rectangle, so primitives drawn against it are
/// clipped to the rounded corners (like CSS `overflow: hidden` on a rounded box).
///
/// `radius` is in the same physical-pixel space as `bounds`.
pub fn adjust_clip_mask_rounded(
    clip_mask: &mut tiny_skia::Mask,
    bounds: Rectangle,
    radius: crate::core::border::Radius,
) {
    clip_mask.clear();

    let mut radii = <[f32; 4]>::from(radius);
    for r in &mut radii {
        *r = (*r).min(bounds.width / 2.0).min(bounds.height / 2.0);
    }

    let path = rounded_rectangle(bounds, radii);

    // Anti-alias so the rounded corners are smooth, matching the card border.
    clip_mask.fill_path(
        &path,
        tiny_skia::FillRule::EvenOdd,
        true,
        tiny_skia::Transform::default(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Border, Shadow, Vector};

    fn card() -> Quad {
        Quad {
            bounds: Rectangle {
                x: 20.0,
                y: 16.0,
                width: 120.0,
                height: 80.0,
            },
            border: Border {
                radius: 12.0.into(),
                ..Border::default()
            },
            shadow: Shadow {
                color: Color::from_rgba(0.0, 0.0, 0.0, 0.4),
                offset: Vector::new(0.0, 6.0),
                blur_radius: 18.0,
                spread_radius: 2.0,
                ..Shadow::default()
            },
            ..Quad::default()
        }
    }

    const EVERYTHING: Rectangle = Rectangle {
        x: 0.0,
        y: 0.0,
        width: 1000.0,
        height: 1000.0,
    };

    /// Damage shades only its strip, and that strip is the whole shadow's.
    #[test]
    fn a_damaged_strip_is_that_strip_of_the_whole_shadow() {
        let quad = card();
        let transformation = Transformation::scale(2.0);
        let bounds = shadow::bounds(&quad) * transformation;
        let radii = <[f32; 4]>::from(quad.border.radius);
        let (wx, wy, whole) =
            shadow::pixmap(&quad, bounds, radii, transformation, EVERYTHING).unwrap();
        let strip = Rectangle {
            x: 60.5,
            y: 90.0,
            width: 170.0,
            height: 13.2,
        };
        let (sx, sy, part) = shadow::pixmap(&quad, bounds, radii, transformation, strip).unwrap();

        assert_eq!((part.width(), part.height()), (171, 14));
        for y in 0..part.height() {
            for x in 0..part.width() {
                let at = whole.pixel(x + (sx - wx) as u32, y + (sy - wy) as u32);
                assert_eq!(part.pixel(x, y), at, "({x}, {y})");
            }
        }
        let outside = Rectangle { x: 900.0, ..strip };
        assert!(shadow::pixmap(&quad, bounds, radii, transformation, outside).is_none());
    }

    /// Filling only what the damage sees must paint exactly what filling the whole
    /// shape under the mask did, including anti-aliased edges and gradients.
    #[test]
    fn a_damaged_part_of_a_fill_matches_the_whole_fill_under_the_mask() {
        let gradient = Background::Gradient(Gradient::Linear(
            crate::core::gradient::Linear::new(0.7)
                .add_stop(0.0, Color::from_rgba(0.9, 0.2, 0.1, 0.8))
                .add_stop(1.0, Color::from_rgba(0.1, 0.3, 0.9, 0.6)),
        ));
        let solid = Background::Color(Color::from_rgba(0.3, 0.6, 0.2, 0.7));
        let quad = Quad {
            bounds: Rectangle {
                x: 20.3,
                y: 15.6,
                width: 180.4,
                height: 120.7,
            },
            border: Border {
                radius: crate::core::border::Radius {
                    top_left: 0.0,
                    top_right: 10.0,
                    bottom_right: 24.0,
                    bottom_left: 6.0,
                },
                ..Border::default()
            },
            ..Quad::default()
        };
        let bordered = Quad {
            border: Border {
                color: Color::from_rgba(1.0, 1.0, 1.0, 0.5),
                width: 1.5,
                ..quad.border
            },
            ..card()
        };

        let mut cut = 0;
        for (quad, background) in [(quad, gradient), (quad, solid), (bordered, solid)] {
            for scale in [1.0, 1.5, 2.0] {
                let transformation = Transformation::scale(scale);
                let physical = quad.bounds * transformation;
                for clip in [
                    // Inside, clear of every corner.
                    Rectangle::new(
                        crate::core::Point::new(physical.x + 40.0, physical.y + 50.0),
                        Size::new(90.0, 13.0),
                    ),
                    // Across the left edge.
                    Rectangle::new(
                        crate::core::Point::new(physical.x - 7.0, physical.y + 60.0),
                        Size::new(60.0, 20.5),
                    ),
                    // Along the top, between the corners.
                    Rectangle::new(
                        crate::core::Point::new(physical.x + 60.0, physical.y - 3.0),
                        Size::new(70.0, 9.0),
                    ),
                    // Half a pixel inside the right edge.
                    Rectangle::new(
                        crate::core::Point::new(physical.x + 50.0, physical.y + 55.0),
                        Size::new(physical.width - 50.5, 11.0),
                    ),
                    // The bottom rows, full width, through both bottom corners.
                    Rectangle::new(
                        crate::core::Point::new(
                            physical.x - 10.0,
                            physical.y + physical.height - 18.0,
                        ),
                        Size::new(physical.width + 20.0, 25.0),
                    ),
                    // Across the top-left corner.
                    Rectangle::new(
                        crate::core::Point::new(physical.x - 5.0, physical.y - 5.0),
                        Size::new(60.0, 20.0),
                    ),
                    // Down the whole height, which the whole shape must fill.
                    Rectangle::new(
                        crate::core::Point::new(physical.x + 40.0, physical.y - 5.0),
                        Size::new(30.0, physical.height + 10.0),
                    ),
                    // Over the bottom-right corner.
                    Rectangle::new(
                        crate::core::Point::new(
                            physical.x + physical.width - 20.0,
                            physical.y + physical.height - 20.0,
                        ),
                        Size::new(30.0, 30.0),
                    ),
                ] {
                    let radii = <[f32; 4]>::from(quad.border.radius);
                    cut += usize::from(
                        damaged_part(quad.bounds, radii, transformation, clip).is_some(),
                    );

                    let draw = |force_clip| {
                        let mut pixmap = tiny_skia::Pixmap::new(480, 360).unwrap();
                        let mut mask = tiny_skia::Mask::new(480, 360).unwrap();
                        adjust_clip_mask(&mut mask, clip);
                        Engine::new().draw_quad(
                            &quad,
                            &background,
                            transformation,
                            &mut pixmap.as_mut(),
                            &mut mask,
                            clip,
                            force_clip,
                        );
                        pixmap
                    };

                    assert!(
                        draw(false).data() == draw(true).data(),
                        "scale {scale}, clip {clip:?}"
                    );
                }
            }
        }
        assert!(cut >= 20, "a smaller part was filled only {cut} times");
    }

    /// Damage reaching one edge's corners is filled as a band with only those
    /// corners; reaching both edges, or a circle, still fills the whole shape.
    #[test]
    fn a_damaged_part_is_as_small_as_its_corners_allow() {
        let bounds = Rectangle {
            x: 10.0,
            y: 10.0,
            width: 200.0,
            height: 120.0,
        };
        let part = |radii, clip: (f32, f32, f32, f32)| {
            let clip = Rectangle {
                x: clip.0,
                y: clip.1,
                width: clip.2,
                height: clip.3,
            };
            damaged_part(bounds, radii, Transformation::scale(2.0), clip)
                .map(|path| (path.bounds().height() * 2.0).round())
        };
        let rounded = [16.0; 4];

        // The middle rows are a rectangle.
        assert_eq!(part(rounded, (0.0, 100.0, 500.0, 20.0)), Some(22.0));
        // The bottom rows keep the bottom arcs, from where they start.
        assert_eq!(part(rounded, (0.0, 240.0, 500.0, 40.0)), Some(32.0));
        // Above the arcs, the cut follows the damage.
        assert_eq!(part(rounded, (0.0, 200.0, 500.0, 80.0)), Some(61.0));
        // The top rows likewise.
        assert_eq!(part(rounded, (0.0, 0.0, 500.0, 40.0)), Some(32.0));
        // Top and bottom corners at once: the whole shape.
        assert_eq!(part(rounded, (0.0, 0.0, 500.0, 300.0)), None);
        // A circle is built differently, so it is never cut.
        let circle = Rectangle {
            width: 120.0,
            ..bounds
        };
        assert!(
            damaged_part(
                circle,
                [60.0; 4],
                Transformation::scale(2.0),
                Rectangle {
                    x: 0.0,
                    y: 100.0,
                    width: 500.0,
                    height: 20.0,
                },
            )
            .is_none()
        );
    }

    /// Blending a shadow directly must store exactly what `draw_pixmap` stores.
    #[test]
    fn a_translated_draw_matches_draw_pixmap() {
        let mut seed = 0x2545_f491_u32;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed >> 8) as u8
        };
        let mut color = || {
            let alpha = match next() % 4 {
                0 => 0,
                1 => 255,
                _ => next(),
            };
            tiny_skia::ColorU8::from_rgba(next(), next(), next(), alpha).premultiply()
        };

        let mut target = tiny_skia::Pixmap::new(64, 56).unwrap();
        for pixel in target.pixels_mut() {
            *pixel = color();
        }
        let mut shadow = tiny_skia::Pixmap::new(20, 15).unwrap();
        for pixel in shadow.pixels_mut() {
            *pixel = color();
        }
        let mut mask = tiny_skia::Mask::new(64, 56).unwrap();
        for (at, coverage) in mask.data_mut().iter_mut().enumerate() {
            *coverage = match at % 5 {
                0 => 0,
                1 | 2 => 255,
                _ => (at * 37 % 256) as u8,
            };
        }

        for (x, y) in [(0, 0), (5, 7), (50, 45), (70, 20), (-3, -4)] {
            let mut expected = target.clone();
            expected.draw_pixmap(
                x,
                y,
                shadow.as_ref(),
                &tiny_skia::PixmapPaint::default(),
                tiny_skia::Transform::default(),
                Some(&mask),
            );
            let mut drawn = target.clone();
            draw_translated(&mut drawn.as_mut(), x, y, &shadow, &mask);

            assert!(drawn.data() == expected.data(), "at ({x}, {y})");
        }
    }

    /// A radial gradient reaches the same stop at the same fraction of each
    /// radius of its ellipse, not of a circle.
    #[test]
    fn a_radial_gradient_follows_its_ellipse() {
        let shader = elliptical_gradient(
            Point::new(100.0, 60.0),
            Vector::new(80.0, 20.0),
            [(0.0, Color::WHITE), (1.0, Color::BLACK)],
        );
        let mut pixmap = tiny_skia::Pixmap::new(200, 120).unwrap();
        pixmap.fill_rect(
            tiny_skia::Rect::from_xywh(0.0, 0.0, 200.0, 120.0).unwrap(),
            &tiny_skia::Paint {
                shader,
                ..tiny_skia::Paint::default()
            },
            tiny_skia::Transform::identity(),
            None,
        );

        for (x, y) in [
            (139, 60),
            (60, 60),
            (100, 69),
            (100, 50),
            (130, 72),
            (185, 60),
        ] {
            let t = ((x as f32 + 0.5 - 100.0) / 80.0).hypot((y as f32 + 0.5 - 60.0) / 20.0);
            let expected = (255.0 * (1.0 - t.min(1.0))).round();
            let shade = f32::from(pixmap.pixel(x, y).unwrap().red());

            assert!(
                (shade - expected).abs() <= 2.0,
                "({x}, {y}): {shade}, not {expected}"
            );
        }
    }

    /// Border widths snap to device pixels as Chrome snaps them: a hairline is
    /// one pixel, anything wider is floored to whole pixels.
    #[test]
    fn border_widths_snap_to_device_pixels() {
        for (width, scale, pixels) in [
            (0.5, 1.0, 1.0),
            (0.25, 2.0, 1.0),
            (0.5, 1.5, 1.0),
            (1.0, 1.5, 1.0),
            (1.5, 1.0, 1.0),
            (1.5, 1.5, 2.0),
            (1.5, 2.0, 3.0),
            (1.3333, 1.5, 1.0),
            (2.5, 1.0, 2.0),
            (0.0, 2.0, 0.0),
        ] {
            let snapped = snap_border_width(width, scale) * scale;

            assert!(
                (snapped - pixels).abs() < 1e-4,
                "{width}px at {scale}x is {snapped} device pixels, not {pixels}"
            );
        }
    }

    /// Borders a single stroke cannot follow land on the quad's edges at any
    /// scale, not just at 1x.
    #[test]
    fn a_border_off_the_stroke_path_lands_on_the_edges_at_2x() {
        let bounds = Rectangle {
            x: 20.0,
            y: 16.0,
            width: 100.0,
            height: 60.0,
        };
        let divider = Border::default().bottom(1.0).rounded(0.0);
        let tight = Border::default().width(6.0).rounded(2.0);
        let dashed = tight.dashed(8.0, 4.0);

        for border in [divider, tight, dashed] {
            let mut pixmap = tiny_skia::Pixmap::new(300, 200).unwrap();
            let mut mask = tiny_skia::Mask::new(300, 200).unwrap();
            Engine::new().draw_quad(
                &Quad {
                    bounds,
                    border: border.color(Color::WHITE),
                    ..Quad::default()
                },
                &Background::Color(Color::TRANSPARENT),
                Transformation::scale(2.0),
                &mut pixmap.as_mut(),
                &mut mask,
                EVERYTHING,
                false,
            );
            let opaque = |x: u32, y: u32| pixmap.pixel(x, y).unwrap().alpha() == 255;
            let blank = |x: u32, y: u32| pixmap.pixel(x, y).unwrap().alpha() == 0;
            let (right, bottom) = (120 * 2, 76 * 2);

            // Its last row and column are painted...
            assert!((40..right).any(|x| opaque(x, bottom - 1)), "{border:?}");
            assert!((32..bottom).any(|y| opaque(right - 1, y)), "{border:?}");
            // ...and nothing past them.
            assert!((0..300).all(|x| blank(x, bottom + 1)), "{border:?}");
            assert!((0..200).all(|y| blank(right + 1, y)), "{border:?}");
        }
    }

    /// The damaged area is cleared first, so a strip of shadow the quad does not
    /// reach must still be shaded back in.
    #[test]
    fn damage_on_the_shadow_alone_repaints_it() {
        let quad = card();
        let mut pixmap = tiny_skia::Pixmap::new(200, 160).unwrap();
        let mut mask = tiny_skia::Mask::new(200, 160).unwrap();
        // Just below the card, inside its shadow.
        let strip = Rectangle {
            x: 30.0,
            y: 100.0,
            width: 100.0,
            height: 8.0,
        };
        adjust_clip_mask(&mut mask, strip);

        Engine::new().draw_quad(
            &quad,
            &Background::Color(Color::WHITE),
            Transformation::IDENTITY,
            &mut pixmap.as_mut(),
            &mut mask,
            strip,
            false,
        );

        assert!(pixmap.pixel(80, 101).unwrap().alpha() > 0);
        // Nothing outside the damage.
        assert_eq!(pixmap.pixel(80, 98).unwrap().alpha(), 0);
    }
}
