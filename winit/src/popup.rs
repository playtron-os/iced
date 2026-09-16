//! Popup surface management for Wayland xdg_popup.
//!
//! This module provides infrastructure for rendering content to popup surfaces
//! that can extend outside their parent window bounds.

use crate::core::Size;
use crate::core::theme;
use crate::core::window;
use crate::graphics::{Compositor, Viewport};
use crate::program::Program;

use std::collections::{BTreeMap, BTreeSet};
use std::ptr::NonNull;

use winit::raw_window_handle;

/// Unique ID for a popup surface within iced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PopupId(pub u64);

impl PopupId {
    /// Generate a new unique popup ID.
    #[allow(dead_code)]
    pub fn unique() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        Self(COUNTER.fetch_add(1, Ordering::Relaxed))
    }
}

/// Raw window handles for a Wayland popup surface.
/// Implements HasWindowHandle and HasDisplayHandle for wgpu surface creation.
#[derive(Clone)]
pub struct PopupSurface {
    surface_ptr: NonNull<std::ffi::c_void>,
    display_ptr: NonNull<std::ffi::c_void>,
}

impl PopupSurface {
    /// Create a new popup surface wrapper from raw pointers.
    ///
    /// # Safety
    /// The pointers must be valid wl_surface and wl_display pointers.
    pub fn new(
        surface_ptr: NonNull<std::ffi::c_void>,
        display_ptr: NonNull<std::ffi::c_void>,
    ) -> Self {
        Self {
            surface_ptr,
            display_ptr,
        }
    }
}

// Safety: The pointers are from the Wayland event loop which is Send
#[allow(unsafe_code)]
unsafe impl Send for PopupSurface {}
// Safety: The pointers are from the Wayland event loop which is Sync
#[allow(unsafe_code)]
unsafe impl Sync for PopupSurface {}

#[allow(unsafe_code)]
impl raw_window_handle::HasWindowHandle for PopupSurface {
    fn window_handle(
        &self,
    ) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        let handle = raw_window_handle::WaylandWindowHandle::new(self.surface_ptr);
        // Safety: The surface pointer is valid for the lifetime of self
        Ok(unsafe { raw_window_handle::WindowHandle::borrow_raw(handle.into()) })
    }
}

#[allow(unsafe_code)]
impl raw_window_handle::HasDisplayHandle for PopupSurface {
    fn display_handle(
        &self,
    ) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
        let handle = raw_window_handle::WaylandDisplayHandle::new(self.display_ptr);
        // Safety: The display pointer is valid for the lifetime of self
        Ok(unsafe { raw_window_handle::DisplayHandle::borrow_raw(handle.into()) })
    }
}

/// State for a popup being managed by iced.
pub struct Popup<C>
where
    C: Compositor,
{
    /// The popup's iced ID.
    ///
    /// Also the manager's key, and `PopupId(winit_popup_id)` -- so this is the
    /// one field that identifies the underlying xdg_popup from the moment it is
    /// inserted. Destroy against this, not [`Popup::winit_popup_id`].
    pub id: PopupId,
    /// The popup's iced window ID (used for view lookup).
    pub iced_id: window::Id,
    /// Direct parent's ID: a toplevel window, or the popup in `parent_popup`.
    pub parent_id: window::Id,
    /// The toplevel this popup's tree hangs off; it renders, scales and redraws the popup.
    pub root_window: window::Id,
    /// The popup this one is parented to, if not a toplevel.
    pub parent_popup: Option<PopupId>,
    /// The winit popup ID, once the compositor has configured the popup.
    ///
    /// `None` between `PopupCreated` and `PopupConfigured`, when the xdg_popup
    /// already exists -- so it is NOT a safe "does this have a surface to
    /// destroy?" test. Use [`Popup::id`] for that.
    pub winit_popup_id: Option<u64>,
    /// Size of the popup.
    pub size: Size<u32>,
    /// Scale factor (inherited from the root window).
    pub scale_factor: f32,
    /// Viewport for rendering.
    pub viewport: Option<Viewport>,
    /// Compositor surface for rendering.
    pub surface: Option<C::Surface>,
    /// Renderer for this popup.
    pub renderer: Option<C::Renderer>,
    /// Whether the popup has been configured by the compositor.
    pub configured: bool,
    /// Whether a frame has been presented, which maps the popup.
    pub presented: bool,
}

/// Where a popup hangs in its tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PopupParent {
    /// The direct parent's ID: a toplevel window, or the popup in `popup`.
    pub id: window::Id,
    /// The toplevel at the root of the tree.
    pub root_window: window::Id,
    /// The parent popup, if `id` isn't a toplevel.
    pub popup: Option<PopupId>,
}

impl PopupParent {
    /// A toplevel parent, which is its own root.
    pub fn window(id: window::Id) -> Self {
        Self {
            id,
            root_window: id,
            popup: None,
        }
    }
}

/// Why a popup can't be parented to another popup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParentError {
    /// No popup has that ID, nor is one being created with it.
    NotFound,
    /// Not drawn yet, so not mapped; xdg-shell needs a mapped parent.
    NotMapped,
}

/// Resolves popup `iced_id` as a parent: `found` is its `(key, root window, presented)` when the
/// manager has it, `requested` whether its creation is still in flight.
fn resolve_parent(
    iced_id: window::Id,
    found: Option<(PopupId, window::Id, bool)>,
    requested: bool,
) -> Result<PopupParent, ParentError> {
    match found {
        Some((popup, root_window, true)) => Ok(PopupParent {
            id: iced_id,
            root_window,
            popup: Some(popup),
        }),
        Some(_) => Err(ParentError::NotMapped),
        None if requested => Err(ParentError::NotMapped),
        None => Err(ParentError::NotFound),
    }
}

/// `id` and every popup below it, each child ahead of its parent: the order xdg-shell requires
/// popups be destroyed in. Siblings are sorted by id only to keep the order deterministic.
///
/// `links` pairs each popup with the popup it is parented to.
fn subtree_leaf_first(links: &[(PopupId, Option<PopupId>)], id: PopupId) -> Vec<PopupId> {
    // Breadth-first from `id`, so the reversal puts every child ahead of its parent.
    let mut order = vec![id];
    let mut next = 0;
    while let Some(&parent) = order.get(next) {
        // `order.contains` also stops a malformed cycle.
        let mut children: Vec<_> = links
            .iter()
            .filter(|(child, link)| *link == Some(parent) && !order.contains(child))
            .map(|(child, _)| *child)
            .collect();
        children.sort_unstable();
        order.extend(children);
        next += 1;
    }
    order.reverse();
    order
}

/// Manages popup surfaces and their rendering state.
pub struct PopupManager<P, C>
where
    P: Program,
    C: Compositor<Renderer = P::Renderer>,
    P::Theme: theme::Base,
{
    entries: BTreeMap<PopupId, Popup<C>>,
    /// Popups asked of winit and not reported back yet.
    requested: BTreeSet<window::Id>,
    _marker: std::marker::PhantomData<P>,
}

impl<P, C> PopupManager<P, C>
where
    P: Program,
    C: Compositor<Renderer = P::Renderer>,
    P::Theme: theme::Base,
{
    /// Create a new popup manager.
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            requested: BTreeSet::new(),
            _marker: std::marker::PhantomData,
        }
    }

    /// Note that popup `iced_id` was asked of winit, until it is inserted or its creation fails.
    pub fn request(&mut self, iced_id: window::Id) {
        let _ = self.requested.insert(iced_id);
    }

    /// Forget a requested popup winit couldn't create.
    pub fn creation_failed(&mut self, iced_id: window::Id) {
        let _ = self.requested.remove(&iced_id);
    }

    /// Insert a new popup (before it's configured).
    pub fn insert(
        &mut self,
        id: PopupId,
        iced_id: window::Id,
        parent: PopupParent,
        size: Size<u32>,
        scale_factor: f32,
    ) {
        let _ = self.requested.remove(&iced_id);
        let _ = self.entries.insert(
            id,
            Popup {
                id,
                iced_id,
                parent_id: parent.id,
                root_window: parent.root_window,
                parent_popup: parent.popup,
                winit_popup_id: None,
                size,
                scale_factor,
                viewport: None,
                surface: None,
                renderer: None,
                configured: false,
                presented: false,
            },
        );
    }

    /// Mark a popup as configured and set up rendering surfaces.
    pub fn configure(
        &mut self,
        id: PopupId,
        winit_popup_id: u64,
        width: u32,
        height: u32,
        popup_surface: PopupSurface,
        compositor: &mut C,
    ) -> bool {
        if let Some(popup) = self.entries.get_mut(&id) {
            popup.winit_popup_id = Some(winit_popup_id);
            popup.configured = true;

            // width/height from Wayland are in logical coordinates
            // Convert to physical size for rendering
            let physical_width = (width as f32 * popup.scale_factor).ceil() as u32;
            let physical_height = (height as f32 * popup.scale_factor).ceil() as u32;

            popup.size = Size::new(physical_width, physical_height);

            // Create viewport for rendering using physical size
            popup.viewport = Some(Viewport::with_physical_size(
                Size::new(physical_width, physical_height),
                popup.scale_factor,
            ));

            // Create compositor surface for rendering at physical size
            let surface = compositor.create_surface(popup_surface, physical_width, physical_height);
            let renderer = compositor.create_renderer();

            popup.surface = Some(surface);
            popup.renderer = Some(renderer);

            true
        } else {
            false
        }
    }

    /// Get a popup by ID.
    #[allow(dead_code)]
    pub fn get(&self, id: PopupId) -> Option<&Popup<C>> {
        self.entries.get(&id)
    }

    /// Get a mutable reference to a popup.
    #[allow(dead_code)]
    pub fn get_mut(&mut self, id: PopupId) -> Option<&mut Popup<C>> {
        self.entries.get_mut(&id)
    }

    /// Remove a popup.
    pub fn remove(&mut self, id: PopupId) -> Option<Popup<C>> {
        self.entries.remove(&id)
    }

    /// Find a popup by its iced window ID.
    pub fn find_by_iced_id(&self, iced_id: window::Id) -> Option<&Popup<C>> {
        self.entries.values().find(|p| p.iced_id == iced_id)
    }

    /// The popup `iced_id` as a parent for a new popup.
    pub fn parent_popup(&self, iced_id: window::Id) -> Result<PopupParent, ParentError> {
        resolve_parent(
            iced_id,
            self.find_by_iced_id(iced_id)
                .map(|p| (p.id, p.root_window, p.presented)),
            self.requested.contains(&iced_id),
        )
    }

    /// Each of `tops` with its subtree, children ahead of parents, as `(key, iced ID)`.
    /// Unknown IDs are skipped.
    pub fn subtrees_leaf_first(
        &self,
        tops: impl IntoIterator<Item = PopupId>,
    ) -> Vec<(PopupId, window::Id)> {
        let links: Vec<_> = self
            .entries
            .values()
            .map(|p| (p.id, p.parent_popup))
            .collect();
        tops.into_iter()
            .filter(|id| self.entries.contains_key(id))
            .flat_map(|id| subtree_leaf_first(&links, id))
            .filter_map(|id| self.entries.get(&id).map(|p| (id, p.iced_id)))
            .collect()
    }

    /// Find a popup by its winit popup ID.
    pub fn find_by_winit_id(&self, winit_popup_id: u64) -> Option<&Popup<C>> {
        self.entries
            .values()
            .find(|p| p.winit_popup_id == Some(winit_popup_id))
    }

    /// Resize a popup by its iced window ID.
    ///
    /// Updates the popup's size, viewport, and reconfigures the compositor
    /// surface. Returns the winit popup ID and root window ID if successful.
    pub fn resize(
        &mut self,
        iced_id: window::Id,
        width: u32,
        height: u32,
        compositor: &mut C,
    ) -> Option<(u64, window::Id)> {
        let popup = self.entries.values_mut().find(|p| p.iced_id == iced_id)?;

        let scale = popup.scale_factor;
        let physical_w = (width as f32 * scale).ceil() as u32;
        let physical_h = (height as f32 * scale).ceil() as u32;

        popup.size = Size::new(physical_w, physical_h);
        popup.viewport = Some(Viewport::with_physical_size(
            Size::new(physical_w, physical_h),
            scale,
        ));

        if let Some(ref mut surface) = popup.surface {
            compositor.configure_surface(surface, physical_w, physical_h);
        }

        let winit_id = popup.winit_popup_id?;
        Some((winit_id, popup.root_window))
    }

    /// Check if manager is empty.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate over all popups.
    pub fn iter(&self) -> impl Iterator<Item = (&PopupId, &Popup<C>)> {
        self.entries.iter()
    }

    /// Iterate mutably over all popups.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&PopupId, &mut Popup<C>)> {
        self.entries.iter_mut()
    }

    /// Get all configured popups that are ready for rendering.
    #[allow(dead_code)]
    pub fn configured_popups(&mut self) -> impl Iterator<Item = &mut Popup<C>> {
        self.entries.values_mut().filter(|p| p.configured)
    }
}

impl<P, C> Default for PopupManager<P, C>
where
    P: Program,
    C: Compositor<Renderer = P::Renderer>,
    P::Theme: theme::Base,
{
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{ParentError, PopupId, PopupParent, resolve_parent, subtree_leaf_first};
    use crate::core::window;

    #[test]
    fn unknown_parent_is_not_found() {
        let id = window::Id::unique();
        assert_eq!(resolve_parent(id, None, false), Err(ParentError::NotFound));
    }

    #[test]
    fn parent_in_flight_or_undrawn_is_not_mapped() {
        let (id, root) = (window::Id::unique(), window::Id::unique());
        assert_eq!(resolve_parent(id, None, true), Err(ParentError::NotMapped));
        assert_eq!(
            resolve_parent(id, Some((PopupId(7), root, false)), false),
            Err(ParentError::NotMapped)
        );
    }

    #[test]
    fn drawn_parent_nests_under_its_root() {
        let (id, root) = (window::Id::unique(), window::Id::unique());
        assert_eq!(
            resolve_parent(id, Some((PopupId(7), root, true)), false),
            Ok(PopupParent {
                id,
                root_window: root,
                popup: Some(PopupId(7)),
            })
        );
    }

    fn ids(raw: &[u64]) -> Vec<PopupId> {
        raw.iter().copied().map(PopupId).collect()
    }

    fn links(raw: &[(u64, Option<u64>)]) -> Vec<(PopupId, Option<PopupId>)> {
        raw.iter()
            .map(|&(id, parent)| (PopupId(id), parent.map(PopupId)))
            .collect()
    }

    #[test]
    fn toplevel_popups_are_only_themselves() {
        let links = links(&[(1, None), (2, None), (3, None)]);
        for id in 1..=3 {
            assert_eq!(subtree_leaf_first(&links, PopupId(id)), ids(&[id]));
        }
    }

    #[test]
    fn window_parent_is_its_own_root() {
        let id = window::Id::unique();
        let parent = PopupParent::window(id);
        assert_eq!(parent.root_window, id);
        assert_eq!(parent.popup, None);
    }

    #[test]
    fn chain_destroys_deepest_first() {
        let links = links(&[(1, None), (2, Some(1)), (3, Some(2))]);
        assert_eq!(subtree_leaf_first(&links, PopupId(1)), ids(&[3, 2, 1]));
    }

    #[test]
    fn branches_put_children_before_parents() {
        // 1 -> {2 -> 3, 4}; input order must not matter.
        let links = links(&[(4, Some(1)), (3, Some(2)), (1, None), (2, Some(1))]);
        assert_eq!(subtree_leaf_first(&links, PopupId(1)), ids(&[3, 4, 2, 1]));
    }

    #[test]
    fn subtree_excludes_parent_and_siblings() {
        let links = links(&[
            (1, None),
            (2, Some(1)),
            (3, Some(2)),
            (4, Some(1)),
            (5, None),
        ]);
        assert_eq!(subtree_leaf_first(&links, PopupId(2)), ids(&[3, 2]));
    }

    #[test]
    fn orphan_is_only_itself() {
        // A child whose parent iced already dropped, awaiting winit's `Done`.
        let links = links(&[(2, Some(1)), (3, None)]);
        assert_eq!(subtree_leaf_first(&links, PopupId(2)), ids(&[2]));
    }

    #[test]
    fn cycle_terminates() {
        let links = links(&[(1, Some(2)), (2, Some(1))]);
        assert_eq!(subtree_leaf_first(&links, PopupId(1)), ids(&[2, 1]));
    }
}
