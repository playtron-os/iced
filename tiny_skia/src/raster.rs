use crate::core::image as raster;
use crate::core::{Rectangle, Size};
use crate::graphics;

use rustc_hash::{FxHashMap, FxHashSet};
use std::cell::RefCell;

#[derive(Debug)]
pub struct Pipeline {
    cache: RefCell<Cache>,
}

impl Pipeline {
    pub fn new() -> Self {
        Self {
            cache: RefCell::new(Cache::default()),
        }
    }

    pub fn load(&self, handle: &raster::Handle) -> Result<raster::Allocation, raster::Error> {
        let size = self.cache.borrow_mut().entry(handle)?.size;

        #[allow(unsafe_code)]
        Ok(unsafe { raster::allocate(handle, size) })
    }

    pub fn dimensions(&self, handle: &raster::Handle) -> Option<Size<u32>> {
        Some(self.cache.borrow_mut().entry(handle).ok()?.size)
    }

    pub fn draw(
        &mut self,
        handle: &raster::Handle,
        filter_method: raster::FilterMethod,
        bounds: Rectangle,
        opacity: f32,
        pixels: &mut tiny_skia::PixmapMut<'_>,
        transform: tiny_skia::Transform,
        clip_mask: Option<&tiny_skia::Mask>,
    ) {
        let mut cache = self.cache.borrow_mut();

        let Ok(image) = cache.pixmap(handle, bounds.size()) else {
            return;
        };

        let width_scale = bounds.width / image.width() as f32;
        let height_scale = bounds.height / image.height() as f32;

        // Placed by the transform, not by whole source pixels, so the image
        // starts exactly at `bounds`.
        let transform = transform
            .pre_translate(bounds.x, bounds.y)
            .pre_scale(width_scale, height_scale);

        let quality = match filter_method {
            raster::FilterMethod::Linear => tiny_skia::FilterQuality::Bilinear,
            raster::FilterMethod::Nearest => tiny_skia::FilterQuality::Nearest,
        };

        pixels.draw_pixmap(
            0,
            0,
            image,
            &tiny_skia::PixmapPaint {
                quality,
                opacity,
                ..Default::default()
            },
            transform,
            clip_mask,
        );
    }

    pub fn trim_cache(&mut self) {
        self.cache.borrow_mut().trim();
    }
}

#[derive(Debug, Default)]
struct Cache {
    entries: FxHashMap<raster::Id, Result<Entry, raster::Error>>,
    hits: FxHashSet<raster::Id>,
}

impl Cache {
    fn entry(&mut self, handle: &raster::Handle) -> Result<&mut Entry, raster::Error> {
        let id = handle.id();
        let _ = self.hits.insert(id);

        self.entries
            .entry(id)
            .or_insert_with(|| Entry::load(handle))
            .as_mut()
            .map_err(|error| error.clone())
    }

    /// The pixels that draw `handle` over `drawn` device pixels.
    fn pixmap(
        &mut self,
        handle: &raster::Handle,
        drawn: Size<f32>,
    ) -> Result<tiny_skia::PixmapRef<'_>, raster::Error> {
        let entry = self.entry(handle)?;
        let levels = graphics::image::reduction(entry.size, drawn);

        // Only ever finer, so an image drawn at two sizes is not reduced
        // again every frame.
        if let Some(source) = &entry.source
            && entry
                .pixels
                .as_ref()
                .is_none_or(|pixels| levels < pixels.levels)
        {
            entry.pixels = Some(if levels > 0 {
                Pixels::new(&graphics::image::reduce(source, levels), levels)
            } else {
                Pixels::new(source, 0)
            });

            if levels == 0 {
                entry.source = None;
            }
        }

        let pixels = entry.pixels.as_ref().ok_or(raster::Error::Empty)?;

        Ok(tiny_skia::PixmapRef::from_bytes(
            bytemuck::cast_slice(&pixels.data),
            pixels.width,
            pixels.height,
        )
        .expect("Build pixmap from image bytes"))
    }

    fn trim(&mut self) {
        self.entries.retain(|key, _| self.hits.contains(key));
        self.hits.clear();
    }
}

#[derive(Debug)]
struct Entry {
    /// The image's own size.
    size: Size<u32>,
    /// The image's own pixels, kept while a larger draw could still need
    /// more of them than `pixels` has.
    source: Option<graphics::image::Buffer>,
    pixels: Option<Pixels>,
}

impl Entry {
    fn load(handle: &raster::Handle) -> Result<Self, raster::Error> {
        let image = graphics::image::load(handle)?;

        if image.width() == 0 || image.height() == 0 {
            return Err(raster::Error::Empty);
        }

        let size = Size::new(image.width(), image.height());

        // A small image is never reduced: convert it now and drop the source.
        if image.len() < graphics::image::LARGE {
            return Ok(Self {
                size,
                source: None,
                pixels: Some(Pixels::new(&image, 0)),
            });
        }

        Ok(Self {
            size,
            source: Some(image),
            pixels: None,
        })
    }
}

#[derive(Debug)]
struct Pixels {
    /// How many times the image was halved.
    levels: u32,
    width: u32,
    height: u32,
    /// Premultiplied BGRA, as tiny-skia draws it.
    data: Vec<u32>,
}

impl Pixels {
    fn new(image: &graphics::image::Buffer, levels: u32) -> Self {
        let data = image
            .pixels()
            .map(|pixel| {
                let [r, g, b, a] = pixel.0;

                bytemuck::cast(tiny_skia::ColorU8::from_rgba(b, g, r, a).premultiply())
            })
            .collect();

        Self {
            levels,
            width: image.width(),
            height: image.height(),
            data,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // An image that fails to load is measured again in the same frame (layout,
    // then draw); the second look must fail too, not panic.
    #[test]
    fn a_failed_image_fails_every_time() {
        let mut pipeline = Pipeline::new();
        let mut canvas = tiny_skia::Pixmap::new(8, 8).expect("canvas");
        let failed = [
            raster::Handle::from_bytes(&b"not an image"[..]),
            raster::Handle::from_path("/nonexistent/icon.png"),
            raster::Handle::from_rgba(0, 0, Vec::new()),
        ];

        for handle in &failed {
            for _ in 0..2 {
                assert_eq!(pipeline.dimensions(handle), None);
                assert!(pipeline.load(handle).is_err());
                pipeline.draw(
                    handle,
                    raster::FilterMethod::Linear,
                    Rectangle::new([0.0, 0.0].into(), Size::new(8.0, 8.0)),
                    1.0,
                    &mut canvas.as_mut(),
                    tiny_skia::Transform::identity(),
                    None,
                );
            }
        }

        assert!(canvas.pixels().iter().all(|pixel| pixel.alpha() == 0));
    }
}
