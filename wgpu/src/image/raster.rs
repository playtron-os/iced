use crate::core::Size;
use crate::core::image;
use crate::graphics;
use crate::image::atlas::{self, Atlas};
use crate::image::grace::Idle;

use rustc_hash::{FxHashMap, FxHashSet};
use std::sync::{Arc, Weak};

pub type Image = graphics::image::Buffer;

/// Entry in cache corresponding to an image handle
#[derive(Debug)]
pub enum Memory {
    /// Image data on host
    Host(Image),
    /// Storage entry
    Device {
        entry: atlas::Entry,
        bind_group: Option<Arc<wgpu::BindGroup>>,
        allocation: Option<Weak<image::Memory>>,
        /// The image's own size; `entry` holds it halved `levels` times.
        size: Size<u32>,
        levels: u32,
    },
    Error(image::Error),
}

impl Memory {
    pub fn load(handle: &image::Handle) -> Self {
        match graphics::image::load(handle) {
            Ok(image) => Self::Host(image),
            Err(error) => Self::Error(error),
        }
    }

    pub fn dimensions(&self) -> Size<u32> {
        match self {
            Memory::Host(image) => {
                let (width, height) = image.dimensions();

                Size::new(width, height)
            }
            Memory::Device { size, .. } => *size,
            Memory::Error(_) => Size::new(1, 1),
        }
    }
}

#[derive(Debug, Default)]
pub struct Cache {
    map: FxHashMap<image::Id, Memory>,
    hits: FxHashSet<image::Id>,
    idle: Idle<image::Id>,
    should_trim: bool,
}

impl Cache {
    pub fn get_mut(&mut self, handle: &image::Handle) -> Option<&mut Memory> {
        let _ = self.hits.insert(handle.id());

        self.map.get_mut(&handle.id())
    }

    /// Caches `memory` for `handle`, returning what it replaces.
    pub fn insert(&mut self, handle: &image::Handle, memory: Memory) -> Option<Memory> {
        let _ = self.hits.insert(handle.id());

        self.should_trim = true;

        self.map.insert(handle.id(), memory)
    }

    pub fn contains(&self, handle: &image::Handle) -> bool {
        self.map.contains_key(&handle.id())
    }

    pub fn trim(&mut self, atlas: &mut Atlas, on_drop: impl Fn(Arc<wgpu::BindGroup>)) {
        // Only trim if new entries have landed in the `Cache`
        if !self.should_trim {
            return;
        }

        let hits = &self.hits;
        let idle = &mut self.idle;

        self.map.retain(|id, memory| {
            // Retain active allocations
            if let Memory::Device { allocation, .. } = memory
                && allocation
                    .as_ref()
                    .is_some_and(|allocation| allocation.strong_count() > 0)
            {
                idle.touch(id);
                return true;
            }

            if hits.contains(id) {
                idle.touch(id);
                return true;
            }

            // Unused this pass, but within its grace: a cover one frame past
            // the edge of a shelf is about to scroll back in, and the upload
            // it would cost is the whole texture.
            let retain = !idle.expired(id);

            if !retain {
                log::debug!("Dropping image allocation: {id:?}");

                if let Memory::Device {
                    entry, bind_group, ..
                } = memory
                {
                    if let Some(bind_group) = bind_group.take() {
                        on_drop(bind_group);
                    } else {
                        atlas.remove(entry);
                    }
                }
            }

            retain
        });

        self.hits.clear();
        self.should_trim = false;
    }
}
