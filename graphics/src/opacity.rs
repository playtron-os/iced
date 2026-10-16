//! Opacity groups, kept the same way by every renderer.
//!
//! A group fades its contents as one layer, the way CSS `opacity` does: the
//! contents are drawn offscreen at full strength and composited once. So a
//! faded label over a faded fill shows the label, not the fill through it.
//!
//! A group that holds or sits inside an offscreen effect (a backdrop blur, a
//! gradient fade, a cached region) is not composited; its contents are scaled
//! one by one instead, as before. Both renderers decide this here, so they
//! draw the same picture.

/// A group composited offscreen: its layers, end exclusive, and its opacity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Region {
    /// The first layer of the group.
    pub start: usize,
    /// One past the last layer of the group.
    pub end: usize,
    /// The opacity the group is composited with.
    pub opacity: f32,
}

/// What a renderer does when a group starts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Start {
    /// Fully opaque: nothing to do.
    Pass,
    /// Scale each primitive by this opacity as it is recorded.
    Scale(f32),
    /// Start a fresh layer and report its index with [`Groups::open`].
    Composite,
}

/// What a renderer does when a group ends.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum End {
    /// Nothing to undo.
    Pass,
    /// Undo the matching [`Start::Scale`].
    Unscale,
    /// The group's layers end here; report the end with [`Groups::close`].
    Close,
}

#[derive(Debug, Clone, Copy)]
enum Open {
    Pass,
    Scale,
    Composite {
        start: usize,
        opacity: f32,
        effect: bool,
    },
}

/// The opacity groups of one frame.
#[derive(Debug, Default)]
pub struct Groups {
    open: Vec<Open>,
    effects: usize,
    regions: Vec<Region>,
}

/// A group that holds an offscreen effect, so its recorded layers are scaled
/// by its opacity instead of composited.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scaled {
    /// The first layer of the group.
    pub start: usize,
    /// One past the last layer of the group.
    pub end: usize,
    /// The opacity to scale the layers by.
    pub opacity: f32,
}

impl Groups {
    /// Creates an empty set of groups.
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts a group with the given opacity.
    pub fn start(&mut self, opacity: f32) -> Start {
        let opacity = opacity.clamp(0.0, 1.0);

        if opacity >= 1.0 {
            self.open.push(Open::Pass);
            return Start::Pass;
        }

        if self.effects > 0 {
            self.open.push(Open::Scale);
            return Start::Scale(opacity);
        }

        self.open.push(Open::Composite {
            start: usize::MAX,
            opacity,
            effect: false,
        });

        Start::Composite
    }

    /// Records the first layer of the group just started with [`Start::Composite`].
    pub fn open(&mut self, start: usize) {
        if let Some(Open::Composite { start: first, .. }) = self.open.last_mut() {
            *first = start;
        }
    }

    /// Ends the innermost group.
    pub fn end(&mut self) -> End {
        match self.open.last() {
            None | Some(Open::Pass) => {
                let _ = self.open.pop();
                End::Pass
            }
            Some(Open::Scale) => {
                let _ = self.open.pop();
                End::Unscale
            }
            Some(Open::Composite { .. }) => End::Close,
        }
    }

    /// Closes the group [`End::Close`] asked for, at the given end layer.
    ///
    /// Returns the layers to scale when the group held an offscreen effect.
    pub fn close(&mut self, end: usize) -> Option<Scaled> {
        let Some(Open::Composite {
            start,
            opacity,
            effect,
        }) = self.open.pop()
        else {
            return None;
        };

        if effect {
            return Some(Scaled {
                start,
                end,
                opacity,
            });
        }

        if end > start {
            self.regions.push(Region {
                start,
                end,
                opacity,
            });
        }

        None
    }

    /// Starts an offscreen effect: a gradient fade, a cached region or a
    /// layer drawn over a backdrop blur.
    pub fn start_effect(&mut self) {
        self.effects += 1;
        self.mark_effect();
    }

    /// Ends the offscreen effect started last.
    pub fn end_effect(&mut self) {
        self.effects = self.effects.saturating_sub(1);
    }

    /// Records a one-off offscreen effect, such as a backdrop blur.
    pub fn mark_effect(&mut self) {
        for open in &mut self.open {
            if let Open::Composite { effect, .. } = open {
                *effect = true;
            }
        }
    }

    /// Returns the composited groups of the frame, innermost first within
    /// each nest.
    pub fn regions(&self) -> &[Region] {
        &self.regions
    }

    /// Whether any group refers to layer indices.
    pub fn tracks_layers(&self) -> bool {
        !self.regions.is_empty()
            || self
                .open
                .iter()
                .any(|open| matches!(open, Open::Composite { .. }))
    }

    /// Forgets every group, for a new frame.
    pub fn clear(&mut self) {
        self.open.clear();
        self.effects = 0;
        self.regions.clear();
    }
}

/// Pairs each composited region with the groups opening and closing at each
/// layer, for a renderer walking its layers in order.
#[derive(Debug, Default)]
pub struct Walk {
    regions: Vec<Region>,
}

impl Walk {
    /// Orders the regions so that, at a shared layer, outer groups open first
    /// and inner groups close first.
    pub fn new(regions: &[Region]) -> Self {
        let mut regions = regions.to_vec();
        regions.sort_by(|a, b| a.start.cmp(&b.start).then(b.end.cmp(&a.end)));
        Self { regions }
    }

    /// The groups opening at `layer`, outermost first.
    pub fn opening(&self, layer: usize) -> impl Iterator<Item = &Region> {
        self.regions
            .iter()
            .filter(move |region| region.start == layer)
    }

    /// Whether any group is composited this frame.
    pub fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_opaque_group_does_nothing() {
        let mut groups = Groups::new();
        assert_eq!(groups.start(1.0), Start::Pass);
        assert_eq!(groups.end(), End::Pass);
        assert!(groups.regions().is_empty());
        assert!(!groups.tracks_layers());
    }

    #[test]
    fn a_faded_group_is_composited() {
        let mut groups = Groups::new();
        assert_eq!(groups.start(0.4), Start::Composite);
        groups.open(3);
        assert_eq!(groups.end(), End::Close);
        assert_eq!(groups.close(6), None);
        assert_eq!(
            groups.regions(),
            &[Region {
                start: 3,
                end: 6,
                opacity: 0.4
            }]
        );
    }

    #[test]
    fn a_group_holding_a_blur_is_scaled_instead() {
        let mut groups = Groups::new();
        assert_eq!(groups.start(0.5), Start::Composite);
        groups.open(2);
        groups.mark_effect();
        assert_eq!(groups.end(), End::Close);
        assert_eq!(
            groups.close(4),
            Some(Scaled {
                start: 2,
                end: 4,
                opacity: 0.5
            })
        );
        assert!(groups.regions().is_empty());
    }

    #[test]
    fn a_group_inside_an_effect_is_scaled_as_it_is_recorded() {
        let mut groups = Groups::new();
        groups.start_effect();
        assert_eq!(groups.start(0.5), Start::Scale(0.5));
        assert_eq!(groups.end(), End::Unscale);
        groups.end_effect();
        assert_eq!(groups.start(0.5), Start::Composite);
    }

    #[test]
    fn an_effect_inside_a_nested_group_scales_every_open_group() {
        let mut groups = Groups::new();
        assert_eq!(groups.start(0.5), Start::Composite);
        groups.open(1);
        assert_eq!(groups.start(0.5), Start::Composite);
        groups.open(2);
        groups.start_effect();
        groups.end_effect();
        assert_eq!(groups.end(), End::Close);
        assert!(groups.close(3).is_some());
        assert_eq!(groups.end(), End::Close);
        assert!(groups.close(4).is_some());
    }

    #[test]
    fn outer_groups_open_first_at_a_shared_layer() {
        let walk = Walk::new(&[
            Region {
                start: 2,
                end: 3,
                opacity: 0.5,
            },
            Region {
                start: 2,
                end: 5,
                opacity: 0.5,
            },
        ]);
        let ends: Vec<usize> = walk.opening(2).map(|region| region.end).collect();
        assert_eq!(ends, [5, 3]);
    }
}
