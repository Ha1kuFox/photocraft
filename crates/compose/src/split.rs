//! Split reduced composite: a live preview that changes one clip group per frame composites
//! the static part once and only the changed group fresh. Preview-only; the release commit
//! always renders the exact path.

use crate::psblend as blend;
use crate::{Buffer, render_reduced};
use photocraft_color::BlendMode;
use photocraft_doc::{Document, LayerContent, LayerId};

/// Whether the stack splits exactly at `k` (top = `layers[k..]`): the boundary blends Normal
/// and is no group, no knockout anywhere (deep knockouts reveal the document outside the
/// split), the top needs no knockout floor, and the top holds no adjustment layers.
fn split_valid(doc: &Document, k: usize) -> bool {
    if k >= doc.layers.len() {
        return false;
    }
    if doc.layers.iter().any(|l| l.advanced.knockout != photocraft_doc::Knockout::None) {
        return false;
    }
    let boundary = &doc.layers[k];
    if boundary.blend != BlendMode::Normal || matches!(boundary.content, LayerContent::Group(_)) {
        return false;
    }
    let top = &doc.layers[k..];
    if crate::advanced::needs_floor(top) {
        return false;
    }
    if top.iter().any(|l| matches!(l.content, LayerContent::Adjustment(_))) {
        return false;
    }
    true
}

/// Where a live preview that changes one clip group per frame (the Gradient tool's drag) may
/// split the stack: the caller composites `layers[..below_end]` once and `layers[below_end..]`
/// fresh every frame (see [`render_reduced_split`]). `layer` is the changing layer.
/// Returns `None` (render everything) when [`split_valid`] fails; nested layers never split
/// (only top-level sibling lists do). Clipped layers are fine: the split expands down to the
/// clip group's base, which renders fresh with the group.
pub fn preview_split(doc: &Document, layer: LayerId) -> Option<usize> {
    let idx = doc.layers.iter().position(|l| l.id == layer)?;
    // Expand down over a clipped run to its base: the group renders atomically.
    let mut k = idx;
    while k > 0 && doc.layers[k].clipped {
        k -= 1;
    }
    split_valid(doc, k).then_some(k)
}

/// [`render_reduced`] split at `below_end` (see [`preview_split`]): `base` is the cached
/// composite of `layers[..below_end]`, or `None` to render it now. Returns `(full, base)`;
/// the caller caches `base` while the revision stands and re-renders only the top part per
/// frame. Bit-identical to [`render_reduced`] up to float reassociation across the split
/// (plain alpha-`over` of a Normal boundary), verified by the split oracle tests.
pub fn render_reduced_split(doc: &Document, below_end: usize, w: u32, h: u32, base: Option<Buffer>) -> (Buffer, Buffer) {
    let k = below_end.min(doc.layers.len());
    if !split_valid(doc, k) {
        // Not splittable after all: the exact full path (base unusable, report empty).
        let full = render_reduced(doc, w, h);
        let rect = full.rect;
        return (full, Buffer::transparent(rect));
    }
    let base = base.unwrap_or_else(|| {
        let mut below_doc = (*doc).clone();
        below_doc.layers.truncate(k);
        render_reduced(&below_doc, w, h)
    });
    let mut top_doc = (*doc).clone();
    top_doc.layers = top_doc.layers.split_off(k).into();
    let top = render_reduced(&top_doc, w, h);
    (blend_over(&top, &base), base)
}

/// Live-drag preview split: like [`render_reduced_split`], but the top part renders
/// reduced-space ([`crate::preview::render_top_reduced`]) instead of full-res-then-averaged.
/// Returns `None` when the top is not [`crate::preview::top_eligible`] (caller renders
/// everything). Approximate by design and preview-only (see [`crate::preview`]); the release
/// commit always renders exact. Returns `(full, base)` like [`render_reduced_split`].
pub fn render_reduced_preview(doc: &Document, below_end: usize, w: u32, h: u32, base: Option<Buffer>) -> Option<(Buffer, Buffer)> {
    use crate::preview::top_eligible;
    let k = below_end.min(doc.layers.len());
    if !split_valid(doc, k) || !top_eligible(doc, k) {
        return None;
    }
    let base = base.unwrap_or_else(|| {
        let mut below_doc = (*doc).clone();
        below_doc.layers.truncate(k);
        render_reduced(&below_doc, w, h)
    });
    let top = crate::preview::render_top_reduced(doc, k, w, h)?;
    Some((blend_over(&top, &base), base))
}

/// Alpha-composite `top` over `base` (same rect): the split-preview join. Same per-pixel math
/// the compositor uses for a Normal boundary (`psblend::composite(Normal, …, 1)`).
pub fn blend_over(top: &Buffer, base: &Buffer) -> Buffer {
    debug_assert_eq!(top.rect, base.rect);
    let n = top.px.len().min(base.px.len());
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        let px: Vec<[f32; 4]> = top.px.par_iter().zip(base.px.par_iter()).take(n).map(|(f, b)| blend::composite(BlendMode::Normal, *b, *f, 1.0)).collect();
        Buffer { rect: top.rect, px }
    }
    #[cfg(target_arch = "wasm32")]
    {
        let px: Vec<[f32; 4]> = top.px.iter().zip(base.px.iter()).take(n).map(|(f, b)| blend::composite(BlendMode::Normal, *b, *f, 1.0)).collect();
        Buffer { rect: top.rect, px }
    }
}
