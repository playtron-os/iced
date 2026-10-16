#![allow(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]
pub mod window;

mod backdrop;
mod engine;
mod layer;
mod primitive;
mod settings;
mod shadow;
mod text;

#[cfg(feature = "image")]
mod raster;

#[cfg(feature = "svg")]
mod vector;

#[cfg(feature = "geometry")]
pub mod geometry;

use iced_debug as debug;
pub use iced_graphics as graphics;
pub use iced_graphics::core;

pub use layer::Layer;
pub use primitive::Primitive;
pub use settings::Settings;

#[cfg(feature = "geometry")]
pub use geometry::Geometry;

use crate::core::renderer;
use crate::core::{Background, Color, Font, Pixels, Point, Rectangle, Size, Transformation};
use crate::engine::Engine;
use crate::graphics::Viewport;
use crate::graphics::compositor;
use crate::graphics::text::{Editor, Paragraph};

/// A [`tiny-skia`] graphics renderer for [`iced`].
///
/// [`tiny-skia`]: https://github.com/RazrFalcon/tiny-skia
/// [`iced`]: https://github.com/iced-rs/iced
#[derive(Debug)]
pub struct Renderer {
    default_font: Font,
    default_text_size: Pixels,
    layers: layer::Stack,
    engine: Engine, // TODO: Shared engine
    groups: graphics::opacity::Groups,
    /// Offscreen pixmaps for composited opacity groups, one per nesting depth.
    group_pixmaps: Vec<tiny_skia::Pixmap>,
    pending_backdrop: bool,
    backdrop_layers: Vec<bool>,
    /// The scale factor last hinted, so text lays out for the pixels it lands on.
    scale_factor: Option<f32>,
}

impl Renderer {
    pub fn new(default_font: Font, default_text_size: Pixels) -> Self {
        Self {
            default_font,
            default_text_size,
            layers: layer::Stack::new(),
            engine: Engine::new(),
            groups: graphics::opacity::Groups::new(),
            group_pixmaps: Vec::new(),
            pending_backdrop: false,
            backdrop_layers: Vec::new(),
            scale_factor: None,
        }
    }

    /// Scales each primitive of a group that held a backdrop and so could not
    /// be composited.
    fn scale_group(&mut self, scaled: graphics::opacity::Scaled) {
        let layers = self.layers.as_mut_slice();
        let end = scaled.end.min(layers.len());
        let start = scaled.start.min(end);

        for layer in &mut layers[start..end] {
            for (quad, background) in &mut layer.quads {
                let (scaled_background, scaled_quad) =
                    apply_opacity(scaled.opacity, *background, *quad);
                *background = scaled_background;
                *quad = scaled_quad;
            }
            for item in &mut layer.text {
                if let layer::Item::Live(text) = item {
                    text.scale_opacity(scaled.opacity);
                }
            }
            for image in &mut layer.images {
                image.scale_opacity(scaled.opacity);
            }
            if let Some((_, opacity)) = &mut layer.backdrop {
                *opacity *= scaled.opacity;
            }
        }
    }

    fn split_layer(&mut self) {
        self.layers.split();
    }

    pub fn layers(&mut self) -> &[Layer] {
        self.layers.flush();
        self.layers.as_slice()
    }

    pub fn draw(
        &mut self,
        pixels: &mut tiny_skia::PixmapMut<'_>,
        clip_mask: &mut tiny_skia::Mask,
        viewport: &Viewport,
        damage: &[Rectangle],
        background_color: Color,
    ) {
        let scale_factor = viewport.scale_factor();
        self.layers.flush();

        // A partial redraw can sample last frame's foreground through the blur.
        let full = [Rectangle::with_size(viewport.logical_size())];
        let damage = if self.layers.iter().any(|layer| layer.backdrop.is_some()) {
            &full[..]
        } else {
            damage
        };
        let walk = graphics::opacity::Walk::new(self.groups.regions());
        let mut pixmaps = std::mem::take(&mut self.group_pixmaps);
        let size = Size::new(pixels.width(), pixels.height());

        for &damage_bounds in damage {
            let damage_bounds = damage_bounds * scale_factor;

            let path = tiny_skia::PathBuilder::from_rect(
                tiny_skia::Rect::from_xywh(
                    damage_bounds.x,
                    damage_bounds.y,
                    damage_bounds.width,
                    damage_bounds.height,
                )
                .expect("Create damage rectangle"),
            );

            pixels.fill_path(
                &path,
                &tiny_skia::Paint {
                    shader: tiny_skia::Shader::SolidColor(engine::into_color(background_color)),
                    anti_alias: false,
                    blend_mode: tiny_skia::BlendMode::Source,
                    ..Default::default()
                },
                tiny_skia::FillRule::default(),
                tiny_skia::Transform::identity(),
                None,
            );

            let layers = self.layers.as_slice();
            // Composited groups open here: (region, the physical area to
            // clear and composite, if any of it is damaged).
            let mut open: Vec<(graphics::opacity::Region, Option<tiny_skia::Rect>)> = Vec::new();

            for (index, layer) in layers.iter().enumerate() {
                for region in walk.opening(index) {
                    let depth = open.len();
                    let area = group_area(layers, region, scale_factor, damage_bounds, size);
                    if pixmaps.len() <= depth {
                        pixmaps.push(group_pixmap(size));
                    } else if pixmaps[depth].width() != size.width
                        || pixmaps[depth].height() != size.height
                    {
                        pixmaps[depth] = group_pixmap(size);
                    }
                    if let Some(area) = area {
                        clear(&mut pixmaps[depth].as_mut(), area);
                    }
                    open.push((*region, area));
                }

                match open.len() {
                    0 => draw_layer(
                        &mut self.engine,
                        layer,
                        pixels,
                        clip_mask,
                        damage_bounds,
                        scale_factor,
                    ),
                    depth => {
                        if open.iter().all(|(_, area)| area.is_some()) {
                            draw_layer(
                                &mut self.engine,
                                layer,
                                &mut pixmaps[depth - 1].as_mut(),
                                clip_mask,
                                damage_bounds,
                                scale_factor,
                            );
                        }
                    }
                }

                while let Some(&(region, area)) = open.last() {
                    if region.end > index + 1 && index + 1 < layers.len() {
                        break;
                    }
                    let _ = open.pop();
                    let depth = open.len();
                    let Some(area) = area else {
                        continue;
                    };
                    let (below, above) = pixmaps.split_at_mut(depth);
                    let source = above[0].as_ref();
                    if depth == 0 {
                        composite(pixels, source, area, region.opacity);
                    } else {
                        composite(&mut below[depth - 1].as_mut(), source, area, region.opacity);
                    }
                }
            }
        }

        self.group_pixmaps = pixmaps;
        self.engine.trim();
    }
}

/// Draws one layer onto `pixels`, within the damaged bounds.
fn draw_layer(
    engine: &mut Engine,
    layer: &Layer,
    pixels: &mut tiny_skia::PixmapMut<'_>,
    clip_mask: &mut tiny_skia::Mask,
    damage_bounds: Rectangle,
    scale_factor: f32,
) {
    let physical_bounds = layer.bounds * scale_factor;
    let Some(layer_bounds) = damage_bounds.intersection(&physical_bounds) else {
        return;
    };

    // A rounded clip layer trims its corners; its quads must always
    // consult the mask (the rect `is_within` shortcut would skip it).
    // Build the mask from the *full* bounds so the corners land in the
    // right place; draws are still culled to the damaged `layer_bounds`.
    let rounded_clip = layer.bounds_radius;
    if let Some(radius) = rounded_clip {
        engine::adjust_clip_mask_rounded(clip_mask, physical_bounds, radius * scale_factor);
    } else {
        engine::adjust_clip_mask(clip_mask, layer_bounds);
    }

    if let Some((filter, opacity)) = &layer.backdrop {
        backdrop::draw(
            pixels,
            filter,
            *opacity,
            scale_factor,
            layer_bounds,
            clip_mask,
        );
    }

    if !layer.quads.is_empty() {
        let render_span = debug::render(debug::Primitive::Quad);
        for (quad, background) in &layer.quads {
            engine.draw_quad(
                quad,
                background,
                Transformation::scale(scale_factor),
                pixels,
                clip_mask,
                layer_bounds,
                rounded_clip.is_some(),
            );
        }
        render_span.finish();
    }

    if !layer.primitives.is_empty() {
        let render_span = debug::render(debug::Primitive::Triangle);

        for group in &layer.primitives {
            let Some(group_bounds) =
                (group.clip_bounds() * scale_factor).intersection(&layer_bounds)
            else {
                continue;
            };

            engine::adjust_clip_mask(clip_mask, group_bounds);

            for primitive in group.as_slice() {
                engine.draw_primitive(
                    primitive,
                    Transformation::scale(scale_factor) * group.transformation(),
                    pixels,
                    clip_mask,
                    group_bounds,
                );
            }

            engine::adjust_clip_mask(clip_mask, layer_bounds);
        }

        render_span.finish();
    }

    if !layer.images.is_empty() {
        let render_span = debug::render(debug::Primitive::Image);

        for image in &layer.images {
            engine.draw_image(
                image,
                Transformation::scale(scale_factor),
                pixels,
                clip_mask,
                layer_bounds,
            );
        }

        render_span.finish();
    }

    if !layer.text.is_empty() {
        let render_span = debug::render(debug::Primitive::Image);

        for group in &layer.text {
            for text in group.as_slice() {
                engine.draw_text(
                    text,
                    Transformation::scale(scale_factor) * group.transformation(),
                    pixels,
                    clip_mask,
                    layer_bounds,
                );
            }
        }

        render_span.finish();
    }
}

/// The whole physical pixels a composited group draws on within the damaged
/// bounds, or `None` when it draws nothing there.
fn group_area(
    layers: &[Layer],
    region: &graphics::opacity::Region,
    scale_factor: f32,
    damage_bounds: Rectangle,
    size: Size<u32>,
) -> Option<tiny_skia::Rect> {
    let end = region.end.min(layers.len());
    let content = layers[region.start.min(end)..end]
        .iter()
        .filter_map(Layer::content_bounds)
        .reduce(|a, b| a.union(&b))?;
    let area = (content * scale_factor)
        .expand(1.0)
        .intersection(&damage_bounds)?
        .intersection(&Rectangle::with_size(Size::new(
            size.width as f32,
            size.height as f32,
        )))?;

    let left = area.x.floor();
    let top = area.y.floor();
    let right = (area.x + area.width).ceil();
    let bottom = (area.y + area.height).ceil();
    tiny_skia::Rect::from_ltrb(left, top, right, bottom)
}

fn group_pixmap(size: Size<u32>) -> tiny_skia::Pixmap {
    tiny_skia::Pixmap::new(size.width.max(1), size.height.max(1)).expect("Create group pixmap")
}

/// Makes `area` of a group pixmap transparent before the group draws into it.
fn clear(pixels: &mut tiny_skia::PixmapMut<'_>, area: tiny_skia::Rect) {
    pixels.fill_rect(
        area,
        &tiny_skia::Paint {
            shader: tiny_skia::Shader::SolidColor(tiny_skia::Color::TRANSPARENT),
            anti_alias: false,
            blend_mode: tiny_skia::BlendMode::Source,
            ..Default::default()
        },
        tiny_skia::Transform::identity(),
        None,
    );
}

/// Draws `area` of a finished group over the target, once, at `opacity`.
fn composite(
    pixels: &mut tiny_skia::PixmapMut<'_>,
    group: tiny_skia::PixmapRef<'_>,
    area: tiny_skia::Rect,
    opacity: f32,
) {
    pixels.fill_rect(
        area,
        &tiny_skia::Paint {
            shader: tiny_skia::Pattern::new(
                group,
                tiny_skia::SpreadMode::Pad,
                tiny_skia::FilterQuality::Nearest,
                opacity,
                tiny_skia::Transform::identity(),
            ),
            anti_alias: false,
            blend_mode: tiny_skia::BlendMode::SourceOver,
            ..Default::default()
        },
        tiny_skia::Transform::identity(),
        None,
    );
}

/// Applies opacity to a background and quad border, returning the modified values.
#[inline]
fn apply_opacity(
    opacity: f32,
    background: impl Into<Background>,
    quad: renderer::Quad,
) -> (Background, renderer::Quad) {
    if opacity >= 1.0 {
        return (background.into(), quad);
    }

    let background = match background.into() {
        Background::Color(mut color) => {
            color.a *= opacity;
            Background::Color(color)
        }
        Background::Gradient(mut gradient) => {
            match &mut gradient {
                core::Gradient::Linear(linear) => {
                    for color_stop in linear.stops.iter_mut().flatten() {
                        color_stop.color.a *= opacity;
                    }
                }
                core::Gradient::Radial(radial) => {
                    for color_stop in radial.stops.iter_mut().flatten() {
                        color_stop.color.a *= opacity;
                    }
                }
                core::Gradient::Conic(conic) => {
                    for color_stop in conic.stops.iter_mut().flatten() {
                        color_stop.color.a *= opacity;
                    }
                }
            }
            Background::Gradient(gradient)
        }
    };

    let mut border = quad.border;
    border.color.a *= opacity;
    let quad = renderer::Quad { border, ..quad };

    (background, quad)
}

impl core::Renderer for Renderer {
    fn draw_backdrop_filter(&mut self, mut filter: renderer::BackdropFilter) {
        self.groups.mark_backdrop();
        if !filter.software {
            return;
        }
        let transformation = self.layers.transformation();
        let scale = transformation.scale_factor();
        filter.bounds = filter.bounds * transformation;
        filter.radius *= scale;
        filter.border_radius = filter.border_radius.map(|radius| radius * scale);
        self.split_layer();
        self.layers.current_mut().0.backdrop = Some((filter, 1.0));
        self.split_layer();
        self.pending_backdrop = true;
    }

    fn draw_backdrop_blur(
        &mut self,
        _bounds: Rectangle,
        _radius: f32,
        _border_radius: [f32; 4],
        _fade_start: f32,
        _saturation: f32,
    ) {
        self.groups.mark_backdrop();
    }

    fn start_post_blur_layer(&mut self, _bounds: Rectangle) {
        self.groups.mark_backdrop();
        let active = std::mem::take(&mut self.pending_backdrop);
        self.backdrop_layers.push(active);
    }

    fn end_post_blur_layer(&mut self) {
        if self.backdrop_layers.pop().unwrap_or(false) {
            self.split_layer();
        }
    }

    fn start_layer(&mut self, bounds: Rectangle) {
        self.layers.push_clip(bounds);
    }

    fn start_layer_rounded(&mut self, bounds: Rectangle, radius: crate::core::border::Radius) {
        self.layers.push_clip_rounded(bounds, radius);
    }

    fn end_layer(&mut self) {
        self.layers.pop_clip();
    }

    fn start_transformation(&mut self, transformation: Transformation) {
        self.layers.push_transformation(transformation);
    }

    fn end_transformation(&mut self) {
        self.layers.pop_transformation();
    }

    fn start_opacity(&mut self, _bounds: Rectangle, opacity: f32) {
        match self.groups.start(opacity) {
            graphics::opacity::Start::Pass => {}
            graphics::opacity::Start::Composite => {
                self.layers.split();
                self.groups.open(self.layers.active_count() - 1);
            }
        }
    }

    fn end_opacity(&mut self) {
        match self.groups.end() {
            graphics::opacity::End::Pass => {}
            graphics::opacity::End::Close => {
                self.layers.flush();
                let composited = self.groups.regions().len();
                if let Some(scaled) = self.groups.close(self.layers.active_count()) {
                    self.scale_group(scaled);
                } else if let Some(region) = self.groups.regions().get(composited).copied() {
                    let layers = self.layers.as_mut_slice();
                    let end = region.end.min(layers.len());
                    for layer in &mut layers[region.start.min(end)..end] {
                        layer.group_opacity *= region.opacity;
                    }
                }
                self.layers.split();
            }
        }
    }

    // Gradient fade in tiny_skia falls back to rendering content at full opacity.
    // The gradient fade effect requires offscreen rendering and compositing which
    // is not practical in tiny_skia's immediate-mode software rendering approach.
    // Content will render normally without the gradient transparency effect.
    fn start_gradient_fade(
        &mut self,
        _bounds: Rectangle,
        _direction: u8,
        _fade_start: f32,
        _fade_end: f32,
        _overflow_margin: f32,
    ) {
        // No-op: content renders at full opacity
    }

    fn end_gradient_fade(&mut self) {
        // No-op
    }

    fn fill_quad(&mut self, quad: renderer::Quad, background: impl Into<Background>) {
        let (layer, transformation) = self.layers.current_mut();
        layer.draw_quad(quad, background.into(), transformation);
    }

    fn allocate_image(
        &mut self,
        _handle: &core::image::Handle,
        callback: impl FnOnce(Result<core::image::Allocation, core::image::Error>) + Send + 'static,
    ) {
        #[cfg(feature = "image")]
        #[allow(unsafe_code)]
        // TODO: Concurrency
        callback(self.engine.raster_pipeline.load(_handle));

        #[cfg(not(feature = "image"))]
        callback(Err(core::image::Error::Unsupported));
    }

    fn hint(&mut self, scale_factor: f32) {
        // No glyph hinting yet, but text still lays its baseline out for this scale.
        self.scale_factor = Some(scale_factor);
    }

    fn scale_factor(&self) -> Option<f32> {
        self.scale_factor
    }

    fn reset(&mut self, new_bounds: Rectangle) {
        self.layers.reset(new_bounds);
        self.groups.clear();
        self.pending_backdrop = false;
        self.backdrop_layers.clear();
    }
}

impl core::text::Renderer for Renderer {
    type Font = Font;
    type Paragraph = Paragraph;
    type Editor = Editor;

    const ICON_FONT: Font = Font::with_name("Iced-Icons");
    const CHECKMARK_ICON: char = '\u{f00c}';
    const ARROW_DOWN_ICON: char = '\u{e800}';
    const ICED_LOGO: char = '\u{e801}';
    const SCROLL_UP_ICON: char = '\u{e802}';
    const SCROLL_DOWN_ICON: char = '\u{e803}';
    const SCROLL_LEFT_ICON: char = '\u{e804}';
    const SCROLL_RIGHT_ICON: char = '\u{e805}';

    fn default_font(&self) -> Self::Font {
        self.default_font
    }

    fn default_size(&self) -> Pixels {
        self.default_text_size
    }

    fn fill_paragraph(
        &mut self,
        text: &Self::Paragraph,
        position: Point,
        color: Color,
        clip_bounds: Rectangle,
    ) {
        let (layer, transformation) = self.layers.current_mut();

        layer.draw_paragraph(text, position, color, clip_bounds, transformation);
    }

    fn fill_editor(
        &mut self,
        editor: &Self::Editor,
        position: Point,
        color: Color,
        clip_bounds: Rectangle,
    ) {
        let (layer, transformation) = self.layers.current_mut();
        layer.draw_editor(editor, position, color, clip_bounds, transformation);
    }

    fn fill_text(
        &mut self,
        text: core::Text,
        position: Point,
        color: Color,
        clip_bounds: Rectangle,
    ) {
        let (layer, transformation) = self.layers.current_mut();
        layer.draw_text(text, position, color, clip_bounds, transformation);
    }
}

impl graphics::text::Renderer for Renderer {
    fn fill_raw(&mut self, raw: graphics::text::Raw) {
        let (layer, transformation) = self.layers.current_mut();
        layer.draw_text_raw(raw, transformation);
    }
}

#[cfg(feature = "geometry")]
impl graphics::geometry::Renderer for Renderer {
    type Geometry = Geometry;
    type Frame = geometry::Frame;

    fn new_frame(&self, bounds: Rectangle) -> Self::Frame {
        geometry::Frame::new(bounds)
    }

    fn draw_geometry(&mut self, geometry: Self::Geometry) {
        let (layer, transformation) = self.layers.current_mut();

        match geometry {
            Geometry::Live {
                primitives,
                images,
                text,
                clip_bounds,
            } => {
                layer.draw_primitive_group(primitives, clip_bounds, transformation);

                for image in images {
                    layer.draw_image(image, transformation);
                }

                layer.draw_text_group(text, clip_bounds, transformation);
            }
            Geometry::Cache(cache) => {
                layer.draw_primitive_cache(cache.primitives, cache.clip_bounds, transformation);

                for image in cache.images.iter() {
                    layer.draw_image(image.clone(), transformation);
                }

                layer.draw_text_cache(cache.text, cache.clip_bounds, transformation);
            }
        }
    }
}

impl graphics::mesh::Renderer for Renderer {
    fn draw_mesh(&mut self, _mesh: graphics::Mesh) {
        log::warn!("iced_tiny_skia does not support drawing meshes");
    }

    fn draw_mesh_cache(&mut self, _cache: iced_graphics::mesh::Cache) {
        log::warn!("iced_tiny_skia does not support drawing meshes");
    }
}

#[cfg(feature = "image")]
impl core::image::Renderer for Renderer {
    type Handle = core::image::Handle;

    fn load_image(
        &self,
        handle: &Self::Handle,
    ) -> Result<core::image::Allocation, core::image::Error> {
        self.engine.raster_pipeline.load(handle)
    }

    fn measure_image(&self, handle: &Self::Handle) -> Option<crate::core::Size<u32>> {
        self.engine.raster_pipeline.dimensions(handle)
    }

    fn draw_image(&mut self, image: core::Image, bounds: Rectangle, clip_bounds: Rectangle) {
        let (layer, transformation) = self.layers.current_mut();
        layer.draw_raster(image, bounds, clip_bounds, transformation);
    }
}

#[cfg(feature = "svg")]
impl core::svg::Renderer for Renderer {
    fn measure_svg(&self, handle: &core::svg::Handle) -> crate::core::Size<u32> {
        self.engine.vector_pipeline.viewport_dimensions(handle)
    }

    fn draw_svg(&mut self, svg: core::Svg, bounds: Rectangle, clip_bounds: Rectangle) {
        let (layer, transformation) = self.layers.current_mut();
        layer.draw_svg(svg, bounds, clip_bounds, transformation);
    }
}

impl compositor::Default for Renderer {
    type Compositor = window::Compositor;
}

impl renderer::Headless for Renderer {
    async fn new(
        default_font: Font,
        default_text_size: Pixels,
        backend: Option<&str>,
    ) -> Option<Self> {
        if backend.is_some_and(|backend| !["tiny-skia", "tiny_skia"].contains(&backend)) {
            return None;
        }

        Some(Self::new(default_font, default_text_size))
    }

    fn name(&self) -> String {
        "tiny-skia".to_owned()
    }

    fn screenshot(
        &mut self,
        size: Size<u32>,
        scale_factor: f32,
        background_color: Color,
    ) -> Vec<u8> {
        let viewport = Viewport::with_physical_size(size, scale_factor);

        window::compositor::screenshot(self, &viewport, background_color)
    }
}

#[cfg(test)]
#[path = "opacity_tests.rs"]
mod opacity_tests;
