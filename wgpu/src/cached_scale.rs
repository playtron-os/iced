//! Cached scale rendering support.
//!
//! Renders content to an offscreen texture at `render_scale` resolution, then
//! composites it as a GPU-scaled quad at `display_scale`. By rendering at the
//! maximum expected scale, the GPU only ever downscales from a higher-resolution
//! texture during animation, preserving quality while avoiding per-frame
//! re-rasterization of SVGs and text.
//!
//! The implementation mirrors the gradient_fade pattern:
//! 1. `start()` — flush layers, record starting layer count
//! 2. Content is drawn with a `render_scale` transformation for higher-res rasterization
//! 3. `end()` — record ending layer count, store the region
//! 4. During rendering, layers in the region are rendered to an offscreen texture
//!    then composited back as a scaled quad with bilinear filtering

use crate::core::{Rectangle, Size};
use crate::graphics::Viewport;
use std::borrow::Cow;
use wgpu::util::DeviceExt;

/// A cached scale region with layer indices and transform parameters.
#[derive(Debug, Clone)]
pub struct CachedScaleRegion {
    /// Logical bounds of the content (unscaled, in screen space)
    pub bounds: Rectangle,
    /// Expanded clip bounds (includes shadow padding, in screen space)
    pub clip_bounds: Rectangle,
    /// The scale at which content was rasterized in the offscreen texture
    pub render_scale: f32,
    /// The scale at which to composite the quad on screen
    pub display_scale: f32,
    /// The starting layer index (inclusive)
    pub start_layer: usize,
    /// The ending layer index (exclusive)
    pub end_layer: usize,
    /// Content-blur radius in logical pixels; 0.0 for no blur.
    ///
    /// This is a CSS `filter: blur()` — it softens the captured content
    /// itself. The backdrop blur in `blur.rs` is the other thing entirely.
    pub blur_radius: f32,
}

/// Sampling stops this many standard deviations out.
///
/// A Gaussian is past 99.7% of its mass by 3σ, so tails beyond this are
/// invisible; the kernel is renormalised by its own sum, which absorbs the
/// truncation exactly.
const BLUR_SIGMA_CUTOFF: f32 = 3.0;

/// Ceiling on taps per side, per pass.
///
/// Purely a guard against a pathological radius turning one frame into a
/// several-thousand-tap loop; real radii are single-digit logical pixels and
/// land far below it.
const MAX_BLUR_TAPS: f32 = 48.0;

/// Uniform data for the cached scale shader.
#[derive(Debug, Clone, Copy, bytemuck::Zeroable, bytemuck::Pod)]
#[repr(C)]
struct CachedScaleUniforms {
    /// Source region in normalized texture coordinates (x, y, width, height)
    src_rect: [f32; 4],
    /// Destination quad in NDC (x, y, width, height)
    dst_rect: [f32; 4],
    /// (step_u, step_v, sigma_texels, taps_per_side)
    blur: [f32; 4],
}

/// Pipeline for compositing a cached scale layer.
#[derive(Debug, Clone)]
pub struct Pipeline {
    pipeline: wgpu::RenderPipeline,
    /// The horizontal blur pass.
    ///
    /// Identical to `pipeline` but writing with `REPLACE` — it renders into a
    /// freshly cleared intermediate, so blending against it would mix the
    /// pass with garbage rather than compositing onto a scene.
    blur_pass: wgpu::RenderPipeline,
    constant_layout: wgpu::BindGroupLayout,
    texture_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

impl Pipeline {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("iced_wgpu.cached_scale.sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..wgpu::SamplerDescriptor::default()
        });

        let constant_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("iced_wgpu.cached_scale.constant_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("iced_wgpu.cached_scale.texture_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("iced_wgpu.cached_scale.pipeline_layout"),
            bind_group_layouts: &[&constant_layout, &texture_layout],
            immediate_size: 0,
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("iced_wgpu.cached_scale.shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!(
                "shader/cached_scale.wgsl"
            ))),
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("iced_wgpu.cached_scale.pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Cw,
                ..wgpu::PrimitiveState::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let blur_pass = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("iced_wgpu.cached_scale.blur_pass_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Cw,
                ..wgpu::PrimitiveState::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        Self {
            pipeline,
            blur_pass,
            constant_layout,
            texture_layout,
            sampler,
        }
    }

    /// Composites the offscreen texture onto the target with a scale transform.
    ///
    /// `bounds` is the logical content bounds, `scale` is the desired scale factor,
    /// and the content is centered within the scaled region.
    pub fn render(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source_texture: &wgpu::TextureView,
        intermediate_texture: Option<&wgpu::TextureView>,
        target: &wgpu::TextureView,
        region: &CachedScaleRegion,
        viewport: &Viewport,
    ) {
        let physical_size = viewport.physical_size();
        let sf = viewport.scale_factor();

        // The content in the offscreen texture was rasterized at render_scale.
        // To display at display_scale, we scale the quad by display_scale / render_scale.
        // When render_scale == display_scale, effective_scale is 1.0 (no GPU scaling).
        // When display_scale < render_scale, we downscale (always looks good).
        let effective_scale = region.display_scale / region.render_scale;

        // A blur pulls colour outward past the content's edges, so the region
        // sampled and composited has to grow with it or the softened halo is
        // sliced off square — which reads as a hard edge on an otherwise soft
        // element. The offscreen texture is transparent out there, so the extra
        // area composites to nothing when there is no blur to catch.
        let sigma = region.blur_radius * sf;
        let blur_padding = if sigma > 0.0 {
            sigma * BLUR_SIGMA_CUTOFF
        } else {
            0.0
        };

        // Use clip_bounds (expanded) for sampling the full area including shadows
        let phys_clip = (region.clip_bounds * sf).expand(blur_padding);
        // Original content bounds for centering
        let phys_bounds = region.bounds * sf;

        // Source rect: the expanded clip region in the offscreen texture
        let src_rect = [
            phys_clip.x / physical_size.width as f32,
            phys_clip.y / physical_size.height as f32,
            phys_clip.width / physical_size.width as f32,
            phys_clip.height / physical_size.height as f32,
        ];

        // Destination: scale the clip region around the original content center
        // by the effective scale (display_scale / render_scale)
        let center_x = phys_bounds.x + phys_bounds.width / 2.0;
        let center_y = phys_bounds.y + phys_bounds.height / 2.0;
        let dst_w = phys_clip.width * effective_scale;
        let dst_h = phys_clip.height * effective_scale;
        let dst_x = center_x + (phys_clip.x - center_x) * effective_scale;
        let dst_y = center_y + (phys_clip.y - center_y) * effective_scale;

        // Convert to NDC (-1..1, with Y flipped)
        let ndc_x = (dst_x / physical_size.width as f32) * 2.0 - 1.0;
        let ndc_y = 1.0 - (dst_y / physical_size.height as f32) * 2.0;
        let ndc_w = (dst_w / physical_size.width as f32) * 2.0;
        let ndc_h = -(dst_h / physical_size.height as f32) * 2.0; // negative for Y-down

        let taps = if sigma > 0.0 {
            (sigma * BLUR_SIGMA_CUTOFF).ceil().min(MAX_BLUR_TAPS)
        } else {
            0.0
        };
        let blurring = taps > 0.0;

        // The horizontal half of the separable Gaussian, into the
        // intermediate; the vertical half rides along with the composite
        // below. Without an intermediate to land in there is nowhere to put
        // the first pass, so the blur is skipped rather than half-applied.
        let source_texture = match (blurring, intermediate_texture) {
            (true, Some(intermediate)) => {
                self.draw(
                    device,
                    encoder,
                    source_texture,
                    intermediate,
                    &CachedScaleUniforms {
                        src_rect,
                        // 1:1 into the intermediate — only the second pass
                        // applies the scale transform.
                        dst_rect: [
                            (phys_clip.x / physical_size.width as f32) * 2.0 - 1.0,
                            1.0 - (phys_clip.y / physical_size.height as f32) * 2.0,
                            (phys_clip.width / physical_size.width as f32) * 2.0,
                            -(phys_clip.height / physical_size.height as f32) * 2.0,
                        ],
                        blur: [1.0 / physical_size.width as f32, 0.0, sigma, taps],
                    },
                    &self.blur_pass,
                    wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    "iced_wgpu.cached_scale.blur_h_pass",
                );
                intermediate
            }
            _ => source_texture,
        };

        let uniforms = CachedScaleUniforms {
            src_rect,
            dst_rect: [ndc_x, ndc_y, ndc_w, ndc_h],
            blur: if blurring && intermediate_texture.is_some() {
                [0.0, 1.0 / physical_size.height as f32, sigma, taps]
            } else {
                [0.0, 0.0, 0.0, 0.0]
            },
        };

        self.draw(
            device,
            encoder,
            source_texture,
            target,
            &uniforms,
            &self.pipeline,
            wgpu::LoadOp::Load,
            "iced_wgpu.cached_scale.render_pass",
        );
    }

    /// One full-quad pass: bind the uniforms and the source, draw the quad.
    ///
    /// Shared by the composite and the horizontal blur pass, which differ only
    /// in pipeline, target and load op.
    #[allow(clippy::too_many_arguments)]
    fn draw(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source_texture: &wgpu::TextureView,
        target: &wgpu::TextureView,
        uniforms: &CachedScaleUniforms,
        pipeline: &wgpu::RenderPipeline,
        load: wgpu::LoadOp<wgpu::Color>,
        label: &str,
    ) {
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("iced_wgpu.cached_scale.uniform_buffer"),
            contents: bytemuck::bytes_of(uniforms),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        let constant_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("iced_wgpu.cached_scale.constant_bind_group"),
            layout: &self.constant_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: uniform_buffer.as_entire_binding(),
                },
            ],
        });

        let texture_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("iced_wgpu.cached_scale.texture_bind_group"),
            layout: &self.texture_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(source_texture),
            }],
        });

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });

        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &constant_bind_group, &[]);
        pass.set_bind_group(1, &texture_bind_group, &[]);
        pass.draw(0..6, 0..1);
    }
}

/// State for managing cached scale rendering.
#[derive(Debug)]
pub struct State {
    /// Completed cached scale regions
    completed_regions: Vec<CachedScaleRegion>,
    /// Active (not yet closed) cached scale
    active: Option<ActiveCachedScale>,
    /// Offscreen texture for rendering content
    offscreen_texture: Option<OffscreenTexture>,
    /// Scratch target for the horizontal half of a content blur.
    ///
    /// Only allocated once something actually asks for a blur — most frames
    /// never do, and this is a full-screen texture.
    blur_intermediate: Option<OffscreenTexture>,
}

#[derive(Debug, Clone)]
struct ActiveCachedScale {
    bounds: Rectangle,
    clip_bounds: Rectangle,
    render_scale: f32,
    display_scale: f32,
    start_layer: usize,
    blur_radius: f32,
}

#[derive(Debug)]
struct OffscreenTexture {
    #[allow(dead_code)]
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    size: Size<u32>,
}

impl OffscreenTexture {
    fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        size: Size<u32>,
        label: &str,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size.width,
                height: size.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        Self {
            texture,
            view,
            size,
        }
    }
}

impl State {
    pub fn new() -> Self {
        Self {
            completed_regions: Vec::new(),
            active: None,
            offscreen_texture: None,
            blur_intermediate: None,
        }
    }

    /// Starts a new cached scale region.
    pub fn start(
        &mut self,
        bounds: Rectangle,
        clip_bounds: Rectangle,
        render_scale: f32,
        display_scale: f32,
        blur_radius: f32,
        current_layer_count: usize,
    ) {
        self.active = Some(ActiveCachedScale {
            bounds,
            clip_bounds,
            render_scale,
            display_scale,
            start_layer: current_layer_count,
            blur_radius,
        });
    }

    /// Returns the render_scale of the currently active cached scale, if any.
    pub fn active_render_scale(&self) -> Option<f32> {
        self.active.as_ref().map(|a| a.render_scale)
    }

    /// Ends the current cached scale and records the layer range.
    pub fn end(&mut self, current_layer_count: usize) -> Option<CachedScaleRegion> {
        if let Some(active) = self.active.take() {
            let region = CachedScaleRegion {
                bounds: active.bounds,
                clip_bounds: active.clip_bounds,
                render_scale: active.render_scale,
                display_scale: active.display_scale,
                start_layer: active.start_layer,
                end_layer: current_layer_count,
                blur_radius: active.blur_radius,
            };
            self.completed_regions.push(region.clone());
            Some(region)
        } else {
            None
        }
    }

    /// Returns completed regions and clears the list.
    pub fn take_regions(&mut self) -> Vec<CachedScaleRegion> {
        std::mem::take(&mut self.completed_regions)
    }

    /// Returns true if a layer index is within any cached scale region.
    pub fn is_layer_in_region(&self, layer_index: usize) -> bool {
        self.completed_regions
            .iter()
            .any(|r| layer_index >= r.start_layer && layer_index < r.end_layer)
    }

    /// Clears all pending state (called at the start of a new frame).
    pub fn clear(&mut self) {
        self.completed_regions.clear();
        self.active = None;
    }

    /// Gets or creates an offscreen texture of the given size.
    pub fn get_or_create_texture(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        size: Size<u32>,
    ) -> &wgpu::TextureView {
        let needs_recreate = self
            .offscreen_texture
            .as_ref()
            .map(|t| t.size != size)
            .unwrap_or(true);

        if needs_recreate {
            self.offscreen_texture = Some(OffscreenTexture::new(
                device,
                format,
                size,
                "iced_wgpu.cached_scale.offscreen_texture",
            ));
        }

        &self.offscreen_texture.as_ref().unwrap().view
    }

    /// Returns the current offscreen texture view if available.
    pub fn texture_view(&self) -> Option<&wgpu::TextureView> {
        self.offscreen_texture.as_ref().map(|t| &t.view)
    }

    /// Gets or creates the scratch texture the horizontal blur pass lands in.
    pub fn get_or_create_blur_intermediate(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        size: Size<u32>,
    ) -> &wgpu::TextureView {
        if self
            .blur_intermediate
            .as_ref()
            .is_none_or(|t| t.size != size)
        {
            self.blur_intermediate = Some(OffscreenTexture::new(
                device,
                format,
                size,
                "iced_wgpu.cached_scale.blur_intermediate",
            ));
        }

        &self.blur_intermediate.as_ref().unwrap().view
    }

    /// Returns the blur intermediate texture view if one has been created.
    pub fn blur_intermediate_view(&self) -> Option<&wgpu::TextureView> {
        self.blur_intermediate.as_ref().map(|t| &t.view)
    }

    /// Whether any completed region this frame asks for a content blur.
    pub fn has_blur(&self) -> bool {
        self.completed_regions.iter().any(|r| r.blur_radius > 0.0)
    }
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    /// The shader is compiled by the driver at first paint, so a WGSL mistake
    /// would surface as content that silently stops drawing rather than as a
    /// build failure. Running it through the same naga that will compile it for
    /// real catches that here instead.
    #[test]
    fn shader_parses_and_validates() {
        use wgpu::naga::{front::wgsl, valid};

        let source = include_str!("shader/cached_scale.wgsl");

        let module = wgsl::parse_str(source).unwrap_or_else(|err| {
            panic!(
                "cached_scale.wgsl does not parse:\n{}",
                err.emit_to_string(source)
            )
        });

        let _ = valid::Validator::new(valid::ValidationFlags::all(), valid::Capabilities::empty())
            .validate(&module)
            .unwrap_or_else(|err| panic!("cached_scale.wgsl does not validate: {err:?}"));
    }
}
