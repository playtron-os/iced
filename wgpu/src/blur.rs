//! Backdrop blur rendering support.
//!
//! This module provides the ability to blur content behind a widget,
//! similar to CSS backdrop-filter: blur().
//!
//! The implementation uses a two-pass Gaussian blur (horizontal + vertical)
//! for efficient O(n) blur instead of O(n²).

use crate::core::{Rectangle, Size};
use crate::graphics::Viewport;
use std::borrow::Cow;
use wgpu::util::DeviceExt;

/// Configuration for a backdrop blur effect.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BackdropBlur {
    /// The bounds where the blur applies
    pub bounds: Rectangle,
    /// Blur radius in logical pixels
    pub radius: f32,
    /// Border radius [top_left, top_right, bottom_right, bottom_left] in logical pixels
    pub border_radius: [f32; 4],
    /// Vertical fade start as fraction (0.0–1.0) of bounds height.
    /// Full blur above this point, linearly fading to 0 at the bottom.
    /// 0.0 = fade across entire height, 1.0 = no fade (default).
    pub fade_start: f32,
    /// CSS `saturate()` amount applied to the blurred backdrop.
    /// 1.0 = unchanged (default), 0.0 = greyscale, above 1.0 = more saturated.
    /// Negative amounts are not legal in CSS and are clamped to 0.
    pub saturation: f32,
    /// Strength of the whole filter, 0.0–1.0.
    ///
    /// Read from the renderer's opacity stack, so a region inside a
    /// `with_opacity` group fades its backdrop blur along with everything else
    /// instead of popping in at full strength. 1.0 = full (default).
    pub alpha: f32,
}

/// A backdrop blur region with layer indices for tracking which layers to render.
#[derive(Debug, Clone)]
pub struct BlurRegion {
    /// The blur configuration
    pub blur: BackdropBlur,
    /// The starting layer index (content BEFORE this is what gets blurred)
    pub layer_index: usize,
}

impl BackdropBlur {
    /// Creates a new backdrop blur with the given bounds and radius.
    pub fn new(bounds: Rectangle, radius: f32) -> Self {
        Self {
            bounds,
            radius: radius.max(0.0),
            border_radius: [0.0; 4],
            fade_start: 1.0,
            saturation: 1.0,
            alpha: 1.0,
        }
    }

    /// Creates a new backdrop blur with the given bounds, radius, and border radius.
    pub fn with_border_radius(
        bounds: Rectangle,
        radius: f32,
        border_radius: [f32; 4],
        fade_start: f32,
        saturation: f32,
    ) -> Self {
        Self {
            bounds,
            radius: radius.max(0.0),
            border_radius,
            fade_start: fade_start.clamp(0.0, 1.0),
            saturation: saturation.max(0.0),
            alpha: 1.0,
        }
    }

    /// Scales the strength of the whole filter — see [`Self::alpha`].
    #[must_use]
    pub fn with_alpha(mut self, alpha: f32) -> Self {
        self.alpha = alpha.clamp(0.0, 1.0);
        self
    }
}

/// Uniform data for the blur shader.
#[derive(Debug, Clone, Copy, bytemuck::Zeroable, bytemuck::Pod)]
#[repr(C)]
struct BlurUniforms {
    /// Quad bounds in normalized device coordinates (x, y, width, height) - expanded for blur sampling
    quad_bounds: [f32; 4],
    /// Clip bounds in normalized device coordinates (x, y, width, height) - original widget bounds for SDF
    clip_bounds: [f32; 4],
    /// params.x = blur_radius, params.y = direction (0=horizontal, 1=vertical)
    /// params.z = texture_width, params.w = texture_height
    params: [f32; 4],
    /// Border radius [top_left, top_right, bottom_right, bottom_left] in pixels
    border_radius: [f32; 4],
    /// fade_params.x = fade_start (0.0–1.0 fraction of bounds height)
    /// fade_params.y = 1.0 on the restore pass (emit the complementary weight)
    /// fade_params.z = region alpha (1.0 = full strength). Must be 1.0 on any
    ///                 pass that is not part of the erase/restore/blur crossfade,
    ///                 or that pass renders nothing.
    /// fade_params.w = reserved
    fade_params: [f32; 4],
    /// filter_params.x = CSS `saturate()` amount (1.0 = identity)
    /// filter_params.y = 1.0 when the render target format is `*Srgb`
    /// filter_params.z/w = reserved
    ///
    /// A whole `vec4` rather than a bare `f32`: WGSL rounds a uniform struct up
    /// to a 16-byte multiple, so a lone trailing scalar would make the Rust size
    /// 84 where the shader reads 96. `min_binding_size` below is derived from
    /// the Rust size, so that mismatch is a wgpu validation panic on the first
    /// blurred frame rather than anything the compiler would catch.
    filter_params: [f32; 4],
}

/// Everything about a blur region's placement that every pass shares.
#[derive(Debug, Clone, Copy)]
struct PassGeometry {
    /// Region expanded by the sampling radius, normalized.
    quad_bounds: [f32; 4],
    /// The widget's own bounds, normalized — what the SDF clips to.
    clip_bounds: [f32; 4],
    /// Corner radii already scaled to physical pixels.
    border_radius: [f32; 4],
    tex_width: f32,
    tex_height: f32,
    /// Sampling radius in physical pixels.
    total_radius: f32,
}

/// Whether a region needs the restore pass.
///
/// The erase pass always clears the whole region, and only the final pass puts
/// anything back. Any time that final pass writes at less than full weight —
/// a vertical fade, a partial alpha, or both — the difference has to be made up
/// by additively restoring the original scene, or the region is a hole.
fn restores(fade: f32, alpha: f32) -> bool {
    fade < 1.0 || alpha < 1.0
}

/// Builds every uniform block for one blur region, in execution order.
///
/// Pure, and lifted out of [`Pipeline::render`] for one reason: the property
/// that makes saturation correct — that a non-identity amount appears on the
/// final block and on no other — is otherwise only observable on a GPU. The
/// five ping-pong passes feed each other, so an amount leaking into them would
/// compound to its sixth power, and that is a silent, plausible-looking bug.
fn build_uniforms(
    geo: &PassGeometry,
    fade: f32,
    alpha: f32,
    saturation: f32,
    srgb: f32,
) -> Vec<BlurUniforms> {
    let PassGeometry {
        quad_bounds,
        clip_bounds,
        border_radius,
        tex_width,
        tex_height,
        total_radius,
    } = *geo;

    // Saturation rides on the final pass alone, so identity is what every other
    // block carries.
    let identity = [1.0, srgb, 0.0, 0.0];

    let mut all = Vec::with_capacity(8);

    // Pass 0: Erase.
    all.push(BlurUniforms {
        quad_bounds: clip_bounds,
        clip_bounds,
        params: [-1.0, 0.0, tex_width, tex_height],
        border_radius,
        fade_params: [1.0, 0.0, 1.0, 0.0],
        filter_params: identity,
    });

    // Pass 1: Restore.
    if restores(fade, alpha) {
        all.push(BlurUniforms {
            quad_bounds: clip_bounds,
            clip_bounds,
            params: [0.0, 0.0, tex_width, tex_height],
            border_radius,
            // Carries the unfiltered original, so it stays at identity
            // saturation: saturating it would put saturated-but-unblurred
            // pixels in the fade region and seam against the scene outside.
            fade_params: [fade, 1.0, alpha, 0.0], // invert_fade
            filter_params: identity,
        });
    }

    // Passes 2-6: H/V/H/V/H intermediate blur (ping-pong).
    for dir in [0.0f32, 1.0, 0.0, 1.0, 0.0] {
        all.push(BlurUniforms {
            quad_bounds,
            clip_bounds: quad_bounds,
            params: [total_radius, dir, tex_width, tex_height],
            border_radius: [0.0; 4],
            fade_params: [1.0, 0.0, 1.0, 0.0],
            filter_params: identity,
        });
    }

    // Pass 7: Final V blur → target (additive). The only pass that writes
    // filtered colour where anyone can see it, and so the only one that
    // saturates.
    all.push(BlurUniforms {
        quad_bounds,
        clip_bounds,
        params: [total_radius, 1.0, tex_width, tex_height],
        border_radius,
        fade_params: [fade, 0.0, alpha, 0.0],
        filter_params: [saturation, srgb, 0.0, 0.0],
    });

    all
}

/// Which pipeline/blend mode to use for a blur pass.
#[derive(Debug, Clone, Copy)]
enum BlurPassMode {
    /// No blending — intermediate ping-pong passes and blits
    Intermediate,
    /// Destination-out blending — erase pass before final draw
    Erase,
    /// Additive blending (One + One) — for crossfade restore and final blur draw
    Additive,
}

/// Pipeline for rendering blur effects.
#[derive(Debug, Clone)]
pub struct Pipeline {
    /// Pipeline without blending (for intermediate passes)
    pipeline: wgpu::RenderPipeline,
    /// Pipeline with destination-out blending (erases target content using src alpha)
    pipeline_erase: wgpu::RenderPipeline,
    /// Pipeline with additive blending (src + dst) for crossfade compositing
    pipeline_additive: wgpu::RenderPipeline,
    constant_layout: wgpu::BindGroupLayout,
    texture_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform_alignment: u32,
    /// Whether the render target decodes sRGB in hardware.
    ///
    /// Decides which space the shader sees: with an `*Srgb` format the sample
    /// arrives as linear light, otherwise it is still sRGB-encoded. CSS filter
    /// functions are defined on sRGB-encoded values, so saturation needs the
    /// transfer in the first case and must not apply it in the second.
    srgb_target: bool,
}

impl Pipeline {
    /// Creates a new blur pipeline.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("iced_wgpu.blur.sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..wgpu::SamplerDescriptor::default()
        });

        let constant_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("iced_wgpu.blur.constant_layout"),
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
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(
                            std::mem::size_of::<BlurUniforms>() as u64,
                        ),
                    },
                    count: None,
                },
            ],
        });

        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("iced_wgpu.blur.texture_layout"),
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
            label: Some("iced_wgpu.blur.pipeline_layout"),
            bind_group_layouts: &[&constant_layout, &texture_layout],
            immediate_size: 0,
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("iced_wgpu.blur.shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("shader/blur.wgsl"))),
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("iced_wgpu.blur.pipeline"),
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
                    blend: None, // No blending for intermediate passes
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

        // Pipeline with destination-out blending for erasing the target region.
        // Blend: src * 0 + dst * (1 - src_alpha)
        // This erases the target content within the SDF-clipped region,
        // so the subsequent blur pass can write without sharp content bleeding through.
        let pipeline_erase = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("iced_wgpu.blur.pipeline_erase"),
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
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::Zero,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::Zero,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
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

        // Pipeline with additive blending (src + dst) for crossfade compositing.
        // Used for the restore pass (original * (1-fade) * sdf) and the final
        // blur draw pass (blurred * fade * sdf). The sum gives a perfect crossfade
        // without the alpha deficit that premultiplied blending causes on
        // semi-transparent content.
        let pipeline_additive = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("iced_wgpu.blur.pipeline_additive"),
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
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
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

        let uniform_alignment = device.limits().min_uniform_buffer_offset_alignment;

        Self {
            pipeline,
            pipeline_erase,
            pipeline_additive,
            constant_layout,
            texture_layout,
            sampler,
            uniform_alignment,
            srgb_target: format.is_srgb(),
        }
    }

    /// `filter_params.y` for this target — see [`Self::srgb_target`].
    fn srgb_flag(&self) -> f32 {
        if self.srgb_target { 1.0 } else { 0.0 }
    }

    /// Performs a single blur pass (horizontal or vertical).
    fn blur_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        constant_bind_group: &wgpu::BindGroup,
        dynamic_offset: u32,
        source_bind_group: &wgpu::BindGroup,
        target: &wgpu::TextureView,
        target_width: f32,
        target_height: f32,
        mode: BlurPassMode,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("iced_wgpu.blur.render_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });

        // Set viewport to match actual content size (textures may be larger due to grow-only policy)
        pass.set_viewport(0.0, 0.0, target_width, target_height, 0.0, 1.0);

        // Select pipeline based on mode
        let pipeline = match mode {
            BlurPassMode::Intermediate => &self.pipeline,
            BlurPassMode::Erase => &self.pipeline_erase,
            BlurPassMode::Additive => &self.pipeline_additive,
        };
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, constant_bind_group, &[dynamic_offset]);
        pass.set_bind_group(1, source_bind_group, &[]);
        pass.draw(0..6, 0..1); // 6 vertices for bounds quad
    }

    /// Creates a texture bind group for the given texture view.
    pub fn create_texture_bind_group(
        &self,
        device: &wgpu::Device,
        texture_view: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("iced_wgpu.blur.texture_bind_group"),
            layout: &self.texture_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(texture_view),
            }],
        })
    }

    /// Packs multiple [`BlurUniforms`] into a single GPU buffer aligned to
    /// `minUniformBufferOffsetAlignment` and creates one constant bind group
    /// that uses dynamic offsets to index into it.
    ///
    /// Returns `(bind_group, aligned_stride)` where each pass uses
    /// `dynamic_offset = pass_index * aligned_stride`.
    fn create_packed_uniforms(
        &self,
        device: &wgpu::Device,
        uniforms: &[BlurUniforms],
    ) -> (wgpu::BindGroup, u32) {
        let block_size = std::mem::size_of::<BlurUniforms>();
        let alignment = self.uniform_alignment as usize;
        let aligned_stride = block_size.div_ceil(alignment) * alignment;
        let total_size = aligned_stride * uniforms.len();

        let mut buffer_data = vec![0u8; total_size];
        for (i, u) in uniforms.iter().enumerate() {
            let offset = i * aligned_stride;
            buffer_data[offset..offset + block_size].copy_from_slice(bytemuck::bytes_of(u));
        }

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("iced_wgpu.blur.uniform_buffer"),
            contents: &buffer_data,
            usage: wgpu::BufferUsages::UNIFORM,
        });

        let constant_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("iced_wgpu.blur.constant_bind_group"),
            layout: &self.constant_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &uniform_buffer,
                        offset: 0,
                        size: wgpu::BufferSize::new(block_size as u64),
                    }),
                },
            ],
        });

        (constant_bind_group, aligned_stride as u32)
    }

    /// Renders the blur effect using two passes (horizontal + vertical).
    ///
    /// Requires an intermediate texture for the two-pass blur.
    /// Uses iterative passes for large blur radii to achieve smooth results.
    pub fn render(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source_texture: &wgpu::TextureView,
        intermediate_texture: &wgpu::TextureView,
        target: &wgpu::TextureView,
        blur: &BackdropBlur,
        viewport: &Viewport,
    ) {
        let physical_size = viewport.physical_size();
        let scale_factor = viewport.scale_factor();

        // Calculate normalized clip bounds (original widget bounds for SDF)
        let bounds = blur.bounds * scale_factor;
        let clip_bounds = [
            bounds.x / physical_size.width as f32,
            bounds.y / physical_size.height as f32,
            bounds.width / physical_size.width as f32,
            bounds.height / physical_size.height as f32,
        ];

        let total_radius = blur.radius * scale_factor;
        let tex_width = physical_size.width as f32;
        let tex_height = physical_size.height as f32;

        // Calculate expanded quad bounds (add padding to sample beyond edges)
        // W3C box blur formula: d = floor(sigma * 1.88 + 0.5)
        // Each box blur pass samples ±(d-1)/2 ≈ ±sigma pixels
        // After 3 passes, the effective sampling range compounds slightly
        // Using 3*sigma as padding provides good coverage
        let padding = total_radius * 3.0;
        let expanded_x = (bounds.x - padding).max(0.0);
        let expanded_y = (bounds.y - padding).max(0.0);
        let expanded_right = (bounds.x + bounds.width + padding).min(tex_width);
        let expanded_bottom = (bounds.y + bounds.height + padding).min(tex_height);
        let quad_bounds = [
            expanded_x / tex_width,
            expanded_y / tex_height,
            (expanded_right - expanded_x) / tex_width,
            (expanded_bottom - expanded_y) / tex_height,
        ];

        log::trace!(
            "blur render: logical_bounds={:?}, physical_bounds=({:.1},{:.1},{:.1},{:.1}), \
             expanded_quad=({:.1},{:.1},{:.1},{:.1}), radius={:.1}",
            blur.bounds,
            bounds.x,
            bounds.y,
            bounds.width,
            bounds.height,
            expanded_x,
            expanded_y,
            expanded_right - expanded_x,
            expanded_bottom - expanded_y,
            total_radius
        );

        // Scale border radius by scale factor
        let scaled_border_radius = [
            blur.border_radius[0] * scale_factor,
            blur.border_radius[1] * scale_factor,
            blur.border_radius[2] * scale_factor,
            blur.border_radius[3] * scale_factor,
        ];

        // Additive crossfade approach for semi-transparent windows:
        //
        // The problem: premultiplied alpha blending can't perfectly crossfade
        // between two semi-transparent contents. With window bg alpha=0.6,
        // erase + premultiplied-blend creates an alpha deficit (dark band).
        //
        // Solution: erase to transparent, then ADDITIVELY blend both the
        // original (with inverted fade) and the blurred content (with fade).
        // The sum original*(1-fade)*sdf + blurred*fade*sdf gives a perfect
        // crossfade with no alpha loss.
        //
        // Pass order:
        //   0. Erase target region to transparent
        //   1. Restore original scene additively with (1-weight)*sdf
        //      (must happen before blur passes destroy source_texture)
        //   2-6. Blur passes: ping-pong source ↔ intermediate
        //   7. Final V blur intermediate → target additively with weight*sdf
        //
        // `weight` is the vertical fade scaled by the region's alpha, so the
        // same crossfade that handles `fade_start` also handles a region inside
        // a `with_opacity` group: at alpha 0 the restore pass puts back exactly
        // what the erase took away and the region is a pixel-exact no-op.

        let fade = blur.fade_start;
        let alpha = blur.alpha.clamp(0.0, 1.0);
        let saturation = blur.saturation.max(0.0);

        // Create texture bind groups once for all passes in this render call.
        let source_bg = self.create_texture_bind_group(device, source_texture);
        let intermediate_bg = self.create_texture_bind_group(device, intermediate_texture);

        let all_uniforms = build_uniforms(
            &PassGeometry {
                quad_bounds,
                clip_bounds,
                border_radius: scaled_border_radius,
                tex_width,
                tex_height,
                total_radius,
            },
            fade,
            alpha,
            saturation,
            self.srgb_flag(),
        );
        let has_restore = restores(fade, alpha);

        let (constant_bg, stride) = self.create_packed_uniforms(device, &all_uniforms);

        // Execute passes — each uses a dynamic offset into the packed buffer.
        let mut idx: u32 = 0;

        // Erase pass
        self.blur_pass(
            encoder,
            &constant_bg,
            idx * stride,
            &intermediate_bg,
            target,
            tex_width,
            tex_height,
            BlurPassMode::Erase,
        );
        idx += 1;

        // Restore pass
        if has_restore {
            self.blur_pass(
                encoder,
                &constant_bg,
                idx * stride,
                &source_bg,
                target,
                tex_width,
                tex_height,
                BlurPassMode::Additive,
            );
            idx += 1;
        }

        // Blur passes 1-5: ping-pong between source and intermediate
        // H: source→intermediate, V: inter→source, repeated, final H: source→inter
        let ping_pong: [(&wgpu::BindGroup, &wgpu::TextureView); 5] = [
            (&source_bg, intermediate_texture),
            (&intermediate_bg, source_texture),
            (&source_bg, intermediate_texture),
            (&intermediate_bg, source_texture),
            (&source_bg, intermediate_texture),
        ];
        for (src_bg, dst) in &ping_pong {
            self.blur_pass(
                encoder,
                &constant_bg,
                idx * stride,
                src_bg,
                dst,
                tex_width,
                tex_height,
                BlurPassMode::Intermediate,
            );
            idx += 1;
        }

        // Final V blur → target (additive)
        self.blur_pass(
            encoder,
            &constant_bg,
            idx * stride,
            &intermediate_bg,
            target,
            tex_width,
            tex_height,
            BlurPassMode::Additive,
        );
    }

    /// Copies (blits) content from source to destination using viewport.
    /// Uses the blur shader with radius=0 which acts as a simple copy.
    pub fn blit(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source_texture: &wgpu::TextureView,
        target: &wgpu::TextureView,
        viewport: &Viewport,
    ) {
        let physical_size = viewport.physical_size();
        self.blit_full(device, encoder, source_texture, target, physical_size);
    }

    /// Copies (blits) full content from source to destination.
    /// Uses the blur shader with radius=0 which acts as a simple copy.
    pub fn blit_full(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source_texture: &wgpu::TextureView,
        target: &wgpu::TextureView,
        physical_size: Size<u32>,
    ) {
        let tex_width = physical_size.width as f32;
        let tex_height = physical_size.height as f32;

        // Full screen bounds (normalized)
        let full_bounds = [0.0, 0.0, 1.0, 1.0];

        // A plain copy of the whole window: full weight, no filter. Both of
        // those are load-bearing — an alpha of 0 here would blit nothing, and a
        // saturation other than 1 would tint the entire frame.
        let uniforms = BlurUniforms {
            quad_bounds: full_bounds,
            clip_bounds: full_bounds,
            params: [0.0, 0.0, tex_width, tex_height],
            border_radius: [0.0; 4],
            fade_params: [1.0, 0.0, 1.0, 0.0],
            filter_params: [1.0, self.srgb_flag(), 0.0, 0.0],
        };

        let (constant_bg, _) = self.create_packed_uniforms(device, &[uniforms]);
        let source_bg = self.create_texture_bind_group(device, source_texture);

        self.blur_pass(
            encoder,
            &constant_bg,
            0,
            &source_bg,
            target,
            tex_width,
            tex_height,
            BlurPassMode::Intermediate,
        );
    }

    /// Copies (blits) a specific region from source to destination.
    /// Uses the blur shader with radius=0 which acts as a simple copy.
    pub fn blit_region(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source_texture: &wgpu::TextureView,
        target: &wgpu::TextureView,
        bounds: &Rectangle,
        viewport: &Viewport,
    ) {
        let physical_size = viewport.physical_size();
        let scale_factor = viewport.scale_factor();

        // Calculate normalized bounds
        let scaled_bounds = *bounds * scale_factor;
        let normalized_bounds = [
            scaled_bounds.x / physical_size.width as f32,
            scaled_bounds.y / physical_size.height as f32,
            scaled_bounds.width / physical_size.width as f32,
            scaled_bounds.height / physical_size.height as f32,
        ];

        let tex_width = physical_size.width as f32;
        let tex_height = physical_size.height as f32;

        log::debug!(
            "blit_region: bounds=({:.2}, {:.2}, {:.2}, {:.2})",
            normalized_bounds[0],
            normalized_bounds[1],
            normalized_bounds[2],
            normalized_bounds[3]
        );

        // Use blur pass with radius=0 to just copy the region
        let uniforms = BlurUniforms {
            quad_bounds: normalized_bounds,
            clip_bounds: normalized_bounds,
            params: [0.0, 0.0, tex_width, tex_height],
            border_radius: [0.0; 4],
            fade_params: [1.0, 0.0, 1.0, 0.0],
            filter_params: [1.0, self.srgb_flag(), 0.0, 0.0],
        };

        let (constant_bg, _) = self.create_packed_uniforms(device, &[uniforms]);
        let source_bg = self.create_texture_bind_group(device, source_texture);

        self.blur_pass(
            encoder,
            &constant_bg,
            0,
            &source_bg,
            target,
            tex_width,
            tex_height,
            BlurPassMode::Intermediate,
        );
    }
}

/// Represents content that should be rendered after blur effects.
#[derive(Debug, Clone)]
pub struct PostBlurContent {
    /// The bounds of this post-blur layer
    pub bounds: Rectangle,
    /// The layer index where this content starts
    pub start_layer: usize,
    /// The layer index where this content ends (exclusive)
    pub end_layer: Option<usize>,
}

/// State for managing blur rendering.
#[derive(Debug, Default)]
pub struct State {
    /// Pending blur regions to process
    regions: Vec<BlurRegion>,
    /// Content that should be rendered after blur
    post_blur_content: Vec<PostBlurContent>,
    /// Stack of in-progress post-blur regions, each `(bounds, start_layer)`.
    current_post_blur: Vec<(Rectangle, usize)>,
}

impl State {
    /// Creates a new blur state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a blur region.
    pub fn add_region(&mut self, blur: BackdropBlur, layer_index: usize) {
        self.regions.push(BlurRegion { blur, layer_index });
    }

    /// Takes all pending regions, clearing the state.
    pub fn take_regions(&mut self) -> Vec<BlurRegion> {
        std::mem::take(&mut self.regions)
    }

    /// Returns true if there are pending blur regions.
    pub fn has_regions(&self) -> bool {
        !self.regions.is_empty()
    }

    /// Clears all pending blur state (called at the start of each frame).
    pub fn clear(&mut self) {
        self.regions.clear();
        self.post_blur_content.clear();
        self.current_post_blur.clear();
    }

    /// Begins recording a (possibly nested) post-blur region.
    pub fn start_post_blur(&mut self, bounds: Rectangle, layer_index: usize) {
        log::trace!(
            "start_post_blur: bounds={:?}, layer_index={}, depth={}",
            bounds,
            layer_index,
            self.current_post_blur.len() + 1
        );
        self.current_post_blur.push((bounds, layer_index));
    }

    /// Ends the innermost in-progress post-blur region, recording it.
    pub fn end_post_blur(&mut self, end_layer: usize) {
        log::trace!("end_post_blur: end_layer={}", end_layer);
        if let Some((bounds, start_layer)) = self.current_post_blur.pop() {
            log::trace!(
                "Recording post-blur content: start_layer={}, end_layer={}",
                start_layer,
                end_layer
            );
            self.post_blur_content.push(PostBlurContent {
                bounds,
                start_layer,
                end_layer: Some(end_layer),
            });
        }
    }

    /// Checks if a layer index is within any post-blur content region.
    /// Used to skip these layers in the first render pass.
    pub fn is_layer_in_post_blur(&self, layer_index: usize) -> bool {
        self.post_blur_content.iter().any(|content| {
            let end = content.end_layer.unwrap_or(usize::MAX);
            layer_index >= content.start_layer && layer_index < end
        }) || self
            .current_post_blur
            .iter()
            .any(|(_, start)| layer_index >= *start)
    }

    /// Takes all post-blur content, clearing the state.
    pub fn take_post_blur_content(&mut self) -> Vec<PostBlurContent> {
        std::mem::take(&mut self.post_blur_content)
    }

    /// Returns true if there is post-blur content to render.
    pub fn has_post_blur_content(&self) -> bool {
        !self.post_blur_content.is_empty()
    }

    /// Returns the post-blur content without taking ownership.
    pub fn post_blur_content(&self) -> &[PostBlurContent] {
        &self.post_blur_content
    }
}

/// Texture cache for blur operations.
///
/// Maintains intermediate textures needed for two-pass blur.
#[derive(Debug)]
pub struct TextureCache {
    /// Intermediate texture for blur passes
    intermediate: Option<(wgpu::Texture, wgpu::TextureView, Size<u32>)>,
    /// Copy of the scene before blur regions
    scene_copy: Option<(wgpu::Texture, wgpu::TextureView, Size<u32>)>,
}

impl TextureCache {
    /// Creates a new texture cache.
    pub fn new() -> Self {
        Self {
            intermediate: None,
            scene_copy: None,
        }
    }

    /// Gets or creates the intermediate texture for blur passes.
    /// Texture is recreated if size doesn't match exactly (blur requires 1:1 pixel mapping).
    pub fn get_intermediate(
        &mut self,
        device: &wgpu::Device,
        size: Size<u32>,
        format: wgpu::TextureFormat,
    ) -> &wgpu::TextureView {
        // Blur requires exact 1:1 pixel mapping - recreate if size doesn't match
        let needs_resize = self
            .intermediate
            .as_ref()
            .is_none_or(|(_, _, s)| s.width != size.width || s.height != size.height);

        if needs_resize {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("iced_wgpu.blur.intermediate_texture"),
                size: wgpu::Extent3d {
                    width: size.width,
                    height: size.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });

            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.intermediate = Some((texture, view, size));
        }

        &self.intermediate.as_ref().unwrap().1
    }

    /// Gets or creates a texture to copy the scene for blurring.
    /// Texture is recreated if size doesn't match exactly (blur requires 1:1 pixel mapping).
    pub fn get_scene_copy(
        &mut self,
        device: &wgpu::Device,
        size: Size<u32>,
        format: wgpu::TextureFormat,
    ) -> &wgpu::TextureView {
        // Blur requires exact 1:1 pixel mapping - recreate if size doesn't match
        let needs_resize = self
            .scene_copy
            .as_ref()
            .is_none_or(|(_, _, s)| s.width != size.width || s.height != size.height);

        if needs_resize {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("iced_wgpu.blur.scene_copy_texture"),
                size: wgpu::Extent3d {
                    width: size.width,
                    height: size.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });

            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.scene_copy = Some((texture, view, size));
        }

        &self.scene_copy.as_ref().unwrap().1
    }

    /// Gets the raw scene copy texture for copy operations.
    pub fn get_scene_copy_texture(&self) -> Option<&wgpu::Texture> {
        self.scene_copy.as_ref().map(|(t, _, _)| t)
    }

    /// Gets or creates both intermediate and scene copy textures, returning their views.
    ///
    /// This method ensures both textures exist and returns their views together,
    /// avoiding borrow checker issues when both are needed simultaneously.
    pub fn get_blur_textures(
        &mut self,
        device: &wgpu::Device,
        size: Size<u32>,
        format: wgpu::TextureFormat,
    ) -> (&wgpu::TextureView, &wgpu::TextureView) {
        // Ensure intermediate texture exists (exact match required for blur)
        let needs_intermediate_resize = self
            .intermediate
            .as_ref()
            .is_none_or(|(_, _, s)| s.width != size.width || s.height != size.height);

        if needs_intermediate_resize {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("iced_wgpu.blur.intermediate_texture"),
                size: wgpu::Extent3d {
                    width: size.width,
                    height: size.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });

            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.intermediate = Some((texture, view, size));
        }

        // Ensure scene_copy texture exists (exact match required for blur)
        let needs_scene_copy_resize = self
            .scene_copy
            .as_ref()
            .is_none_or(|(_, _, s)| s.width != size.width || s.height != size.height);

        if needs_scene_copy_resize {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("iced_wgpu.blur.scene_copy_texture"),
                size: wgpu::Extent3d {
                    width: size.width,
                    height: size.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });

            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.scene_copy = Some((texture, view, size));
        }

        // Now both are guaranteed to exist, return references
        (
            &self.scene_copy.as_ref().unwrap().1,
            &self.intermediate.as_ref().unwrap().1,
        )
    }
}

impl Default for TextureCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{BackdropBlur, BlurUniforms, PassGeometry, build_uniforms, restores};
    use crate::core::Rectangle;

    fn geometry() -> PassGeometry {
        PassGeometry {
            quad_bounds: [0.1, 0.1, 0.4, 0.4],
            clip_bounds: [0.2, 0.2, 0.2, 0.2],
            border_radius: [8.0; 4],
            tex_width: 1920.0,
            tex_height: 1080.0,
            total_radius: 40.0,
        }
    }

    fn bounds() -> Rectangle {
        Rectangle {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
        }
    }

    #[test]
    fn the_uniform_block_is_the_size_the_shader_reads() {
        // WGSL rounds a uniform struct up to a multiple of 16, and
        // `min_binding_size` is derived from the Rust size. A mismatch is not a
        // compile error — it is a wgpu validation panic on the first blurred
        // frame, which is why this is asserted rather than trusted.
        assert_eq!(std::mem::size_of::<BlurUniforms>(), 96);
        assert_eq!(std::mem::size_of::<BlurUniforms>() % 16, 0);
    }

    #[test]
    fn only_the_final_pass_saturates() {
        // The property the whole design rests on. The five ping-pong passes
        // feed each other, so an amount leaking into them compounds: 1.8 would
        // arrive as 1.8^6 ≈ 34. Every block but the last must be exactly 1.0.
        let uniforms = build_uniforms(&geometry(), 1.0, 1.0, 1.8, 0.0);

        let (last, rest) = uniforms.split_last().expect("at least one pass");

        assert!(
            (last.filter_params[0] - 1.8).abs() < f32::EPSILON,
            "the final pass should carry the requested amount"
        );
        for (index, pass) in rest.iter().enumerate() {
            assert!(
                (pass.filter_params[0] - 1.0).abs() < f32::EPSILON,
                "pass {index} saturates, and it must not — it feeds a later pass"
            );
        }
    }

    #[test]
    fn an_untouched_saturation_leaves_every_pass_at_identity() {
        // The compatibility guarantee: with saturation defaulted, the uniform
        // stream is what it always was, so the frame is unchanged.
        let uniforms = build_uniforms(&geometry(), 1.0, 1.0, 1.0, 0.0);

        assert!(
            uniforms
                .iter()
                .all(|pass| (pass.filter_params[0] - 1.0).abs() < f32::EPSILON),
            "a default saturation must not perturb any pass"
        );
    }

    #[test]
    fn the_srgb_flag_reaches_every_pass() {
        // Whichever pass ends up filtering, it has to know which space it is
        // working in — so the flag is not something only the last block gets.
        let uniforms = build_uniforms(&geometry(), 1.0, 1.0, 1.0, 1.0);

        assert!(
            uniforms
                .iter()
                .all(|pass| (pass.filter_params[1] - 1.0).abs() < f32::EPSILON)
        );
    }

    #[test]
    fn a_fully_faded_region_still_restores_what_the_erase_took() {
        // Without the alpha term in `restores`, a region at partial opacity
        // with no vertical fade would erase itself and put nothing back — a
        // transparent hole punched in the window, which is much worse than the
        // popping this fixes.
        assert!(restores(1.0, 0.5), "a partial alpha needs the restore pass");
        assert!(restores(0.5, 1.0), "a vertical fade needs the restore pass");
        assert!(
            !restores(1.0, 1.0),
            "a full-strength region covers its own erase"
        );

        let faded = build_uniforms(&geometry(), 1.0, 0.5, 1.0, 0.0);
        let full = build_uniforms(&geometry(), 1.0, 1.0, 1.0, 0.0);
        assert_eq!(faded.len(), full.len() + 1);
    }

    #[test]
    fn the_region_alpha_rides_on_the_crossfade_pair_only() {
        // The erase and the ping-pong passes are not part of the crossfade;
        // they must stay at full weight or they render nothing at all.
        let uniforms = build_uniforms(&geometry(), 1.0, 0.25, 1.0, 0.0);

        let restore = &uniforms[1];
        let final_pass = uniforms.last().expect("at least one pass");

        assert!((restore.fade_params[2] - 0.25).abs() < f32::EPSILON);
        assert!(
            (restore.fade_params[1] - 1.0).abs() < f32::EPSILON,
            "inverted"
        );
        assert!((final_pass.fade_params[2] - 0.25).abs() < f32::EPSILON);

        assert!((uniforms[0].fade_params[2] - 1.0).abs() < f32::EPSILON);
        for pass in &uniforms[2..uniforms.len() - 1] {
            assert!(
                (pass.fade_params[2] - 1.0).abs() < f32::EPSILON,
                "an intermediate pass at partial weight renders nothing"
            );
        }
    }

    #[test]
    fn out_of_range_inputs_are_brought_into_range() {
        let blur = BackdropBlur::with_border_radius(bounds(), 10.0, [0.0; 4], 2.0, -1.0);
        assert!((blur.fade_start - 1.0).abs() < f32::EPSILON);
        assert!(
            blur.saturation.abs() < f32::EPSILON,
            "CSS has no negative saturation"
        );
        assert!((blur.alpha - 1.0).abs() < f32::EPSILON, "opaque by default");

        assert!((blur.with_alpha(2.0).alpha - 1.0).abs() < f32::EPSILON);
        assert!(blur.with_alpha(-1.0).alpha.abs() < f32::EPSILON);
    }
}

#[cfg(test)]
mod shader {
    /// The blur shader compiles.
    ///
    /// `cargo check` never looks at the WGSL — it is handed to
    /// `create_shader_module` at runtime, so a typo in it is a panic on the
    /// first blurred frame on a real GPU, which is the slowest possible place
    /// to find one. Parsing and validating it here moves that to `cargo test`.
    #[test]
    fn the_blur_shader_parses_and_validates() {
        use wgpu::naga::{front::wgsl, valid};

        let source = include_str!("shader/blur.wgsl");
        let module = wgsl::parse_str(source).expect("the shader parses");

        let _info =
            valid::Validator::new(valid::ValidationFlags::all(), valid::Capabilities::empty())
                .validate(&module)
                .expect("the shader validates");
    }
}
