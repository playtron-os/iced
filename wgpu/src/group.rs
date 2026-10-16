//! Composited opacity groups: drawn into a texture of their own, then
//! composited once at the group's opacity (see [`crate::graphics::opacity`]).
use crate::cached_draw;
use crate::core::{Rectangle, Size};
use crate::graphics::Viewport;
use crate::graphics::opacity::{Region, Walk};

/// The textures of the groups open at once, reused from frame to frame.
#[derive(Debug, Default)]
pub struct Pool {
    targets: Vec<Target>,
    in_use: usize,
}

#[derive(Debug)]
struct Target {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    size: Size<u32>,
}

impl Target {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat, size: Size<u32>) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("iced_wgpu.opacity_group.texture"),
            size: wgpu::Extent3d {
                width: size.width.max(1),
                height: size.height.max(1),
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
            _texture: texture,
            view,
            size,
        }
    }
}

impl Pool {
    /// A cleared texture the size of the viewport, held until [`Pool::release`].
    fn acquire(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        encoder: &mut wgpu::CommandEncoder,
        size: Size<u32>,
    ) -> wgpu::TextureView {
        if self.targets.len() <= self.in_use {
            self.targets.push(Target::new(device, format, size));
        } else if self.targets[self.in_use].size != size {
            self.targets[self.in_use] = Target::new(device, format, size);
        }

        let view = self.targets[self.in_use].view.clone();
        self.in_use += 1;

        let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("iced_wgpu.opacity_group.clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });

        view
    }

    fn release(&mut self) {
        self.in_use = self.in_use.saturating_sub(1);
    }
}

/// What composites a group: the device, its pipeline and the frame's size.
#[derive(Clone, Copy)]
pub struct Compositor<'a> {
    pub device: &'a wgpu::Device,
    pub format: wgpu::TextureFormat,
    pub pipeline: &'a cached_draw::Pipeline,
    pub viewport: &'a Viewport,
}

/// The groups open in one walk over the layers, drawn onto one target. The
/// main pass and every offscreen effect pass keep one each, so a group is
/// composited by whichever pass draws its layers.
#[derive(Debug)]
pub struct Pass {
    walk: Walk,
    base: wgpu::TextureView,
    open: Vec<(Region, wgpu::TextureView)>,
}

impl Pass {
    /// A walk over `regions`, drawing onto `base` outside any group.
    pub fn new(regions: &[Region], base: &wgpu::TextureView) -> Self {
        Self {
            walk: Walk::new(regions),
            base: base.clone(),
            open: Vec::new(),
        }
    }

    /// Where a layer draws: the innermost open group, or the base.
    pub fn target(&self) -> &wgpu::TextureView {
        self.open.last().map_or(&self.base, |(_, view)| view)
    }

    /// Whether a group ends by `layer`, or one starts at it.
    pub fn changes_at(&self, layer: usize) -> bool {
        self.closes_at(layer) || self.opens_at(layer)
    }

    /// Whether a group starts at `layer`.
    pub fn opens_at(&self, layer: usize) -> bool {
        self.walk.opening(layer).next().is_some()
    }

    /// Whether a group ends by `layer`.
    pub fn closes_at(&self, layer: usize) -> bool {
        self.open
            .last()
            .is_some_and(|(region, _)| region.end <= layer)
    }

    /// Composites every group that ends by `layer` onto the target below it.
    pub fn close(
        &mut self,
        layer: usize,
        pool: &mut Pool,
        compositor: Compositor<'_>,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        while self.closes_at(layer) {
            let Some((region, view)) = self.open.pop() else {
                break;
            };

            compositor.pipeline.render(
                compositor.device,
                encoder,
                &view,
                self.target(),
                Rectangle::with_size(compositor.viewport.logical_size()),
                region.opacity,
                compositor.viewport,
            );
            pool.release();
        }
    }

    /// Opens the groups that start at `layer`, outermost first.
    pub fn open(
        &mut self,
        layer: usize,
        pool: &mut Pool,
        compositor: Compositor<'_>,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let opening: Vec<Region> = self.walk.opening(layer).copied().collect();

        for region in opening {
            let view = pool.acquire(
                compositor.device,
                compositor.format,
                encoder,
                compositor.viewport.physical_size(),
            );
            self.open.push((region, view));
        }
    }

    /// Closes the groups at `layer`, then opens those starting there.
    pub fn step(
        &mut self,
        layer: usize,
        pool: &mut Pool,
        compositor: Compositor<'_>,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        self.close(layer, pool, compositor, encoder);
        self.open(layer, pool, compositor, encoder);
    }

    /// Composites every group still open.
    pub fn finish(
        &mut self,
        pool: &mut Pool,
        compositor: Compositor<'_>,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        self.close(usize::MAX, pool, compositor, encoder);
    }
}

/// Begins a render pass on `view` that keeps what is already there.
pub fn load_pass<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("iced_wgpu render pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
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
    })
}
