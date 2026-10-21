//! Reduced-space live-preview rendering for drags that change one clip group per frame
//! (the Gradient tool). Approximate by design and preview-only: layers render bilinearly
//! resampled (gradients analytic) at output size instead of full-res-then-averaged, and joins
//! use plain alpha-`over`. Release commits through the exact path, so finals never change.
//!
//! [`top_eligible`] falls back to the full path for anything this renderer does not model
//! (adjustments, effects, groups, text/shapes/vectors, patterns, clipped layers, exotic
//! boundary blends, knockouts, feathered masks, vector masks, non-RGB8 documents).

use photocraft_color::BlendMode;
use photocraft_doc::{Fill, Layer, LayerContent};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use crate::psblend as blend;

use crate::{Buffer, blend_over};
use crate::effects::{gradient_t_eval, gradient_t_prepare};
use crate::gradient_fill::Ramp;

/// The top slice must be plain paint to render reduced: raster/gradient/solid layers, all
/// Normal blends, no adjustments, effects, groups, text, shapes, vectors or patterns, masks
/// without feather, vector masks absent, and no clipped layers (the split point owns the
/// clip group's base, so a clipped layer above would need its base rendered twice).
fn plain_layer(layer: &Layer) -> bool {
    // Invisible layers contribute nothing (the full path skips them, group and all).
    if layer.blend != BlendMode::Normal {
        return false;
    }
    if matches!(
        layer.content,
        LayerContent::Adjustment(_)
            | LayerContent::Group(_)
            | LayerContent::Text(_)
            | LayerContent::Shape(_)

    ) {
        return false;
    }
    if matches!(layer.content, LayerContent::Fill(Fill::Pattern { .. })) {
        return false;
    }
    if crate::effects::has_effects(layer) {
        return false;
    }
    if layer.vector_mask.is_some() {
        return false;
    }
    if layer.advanced.knockout != photocraft_doc::Knockout::None {
        return false;
    }
    if layer.mask.as_ref().is_some_and(|m| !m.enabled || m.feather != 0.0) {
        return false;
    }
    true
}

/// Whether `layers[start..]` renders reduced exactly as the full path would (up to resampling).
/// `start` comes from [`crate::preview_split`] (a clip-group boundary or the gradient itself).
/// Clipped layers are allowed only directly after their base (which `preview_split` includes);
/// a clipped first layer would need its base from the cached part.
pub fn top_eligible(doc: &photocraft_doc::Document, start: usize) -> bool {
    let layers = &doc.layers;
    if start >= layers.len() {
        return false;
    }
    if doc.mode != photocraft_color::ColorMode::Rgb || doc.depth != photocraft_color::SampleType::U8 {
        return false;
    }
    if layers[start].clipped {
        return false;
    }
    for layer in &layers[start..] {
        if !plain_layer(layer) {
            return false;
        }
        if crate::channel_weights(layer, doc.mode).is_some() || crate::blend_if_active(layer, doc.mode) {
            return false;
        }
    }
    // Blend Clipped Layers as Group off takes a different code path: any base with visible
    // clipped followers falls back.
    for (i, layer) in layers[start..].iter().enumerate() {
        if !layer.clipped
            && !layer.advanced.blend_clipped
            && layers[start + i + 1..].iter().take_while(|c| c.clipped).any(|c| c.visible)
        {
            return false;
        }
    }
    true
}

/// Composite `doc.layers[start..]` at `w`×`h` over transparent, reduced-space. Returns `None`
/// when [`top_eligible`] fails (caller renders everything).
pub fn render_top_reduced(doc: &photocraft_doc::Document, start: usize, w: u32, h: u32) -> Option<Buffer> {
    if !top_eligible(doc, start) {
        return None;
    }
    let b = doc.bounds();
    let (fw, fh) = (b.width().max(1) as f64, b.height().max(1) as f64);
    let (w, h) = (w.max(1), h.max(1));
    let out = Rect::from_xywh(0, 0, w, h);
    let mut px = vec![[0.0f32; 4]; (w * h) as usize];
    // Output pixel centres mapped to document coords.
    let map = |t: u32, f: f64, n: u32| b.x0 as f64 + (f64::from(t) + 0.5) * f / f64::from(n);
    let frame = b;
    let layers = &doc.layers[start..];
    // Fill geometry and ramps hoisted out of the pixel loop (a fresh `Ramp::new` per
    // pixel would dominate the render).
    let fills: Vec<Option<Prepared>> = layers
        .iter()
        .map(|l| match &l.content {
            LayerContent::Fill(f) => prepare_fill(f, frame),
            _ => None,
        })
        .collect();
    let render_row = |j: usize, row: &mut [[f32; 4]]| {
        for (i, p) in row.iter_mut().enumerate() {
            let (x, y) = (map(i as u32, fw, w), map(j as u32, fh, h));
            let mut acc = [0.0f32; 4];
            let mut k = 0;
            while k < layers.len() {
                // A clip group composites atomically: the base, then each follower masked
                // by the base alpha, then the group at the base's blend and opacity.
                let mut j = k + 1;
                while j < layers.len() && layers[j].clipped {
                    j += 1;
                }
                if j == k + 1 {
                    let layer = &layers[k];
                    if !layer.visible {
                        k += 1;
                        continue;
                    }
                    let fg = content_sample(layer, fills[k].as_ref(), x, y);
                    if fg[3] > 0.0 {
                        acc = blend::composite(BlendMode::Normal, acc, fg, layer.opacity * layer.fill_opacity);
                    }
                    k += 1;
                    continue;
                }
                // An invisible base hides its clipped followers, like the full path.
                if !layers[k].visible {
                    k = j;
                    continue;
                }
                let mut group = content_sample(&layers[k], fills[k].as_ref(), x, y);
                for (t, c) in layers[k + 1..j].iter().enumerate() {
                    if !c.visible {
                        continue;
                    }
                    if group[3] <= 0.0 {
                        break;
                    }
                    let s = content_sample(c, fills[k + 1 + t].as_ref(), x, y);
                    if s[3].is_nan() || s[3] <= 0.0 {
                        continue;
                    }
                    // Blend as if the base were opaque, then keep the base's alpha.
                    let r = blend::composite_gamma(
                        BlendMode::Normal,
                        [group[0], group[1], group[2], 1.0],
                        s,
                        c.opacity * c.fill_opacity,
                        1.0,
                    );
                    group = [r[0], r[1], r[2], group[3]];
                }
                let base = &layers[k];
                if group[3] > 0.0 {
                    acc = blend::composite(BlendMode::Normal, acc, group, base.opacity * base.fill_opacity);
                }
                k = j;
            }
            *p = acc;
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        px.par_chunks_mut(w as usize).enumerate().for_each(|(j, row)| render_row(j, row));
    }
    #[cfg(target_arch = "wasm32")]
    for (j, row) in px.chunks_mut(w as usize).enumerate() {
        render_row(j, row);
    }
    Some(Buffer { rect: out, px })
}

/// Layer content at document position with the mask in alpha (like `render_content`),
/// straight (non-premultiplied) RGBA.
fn content_sample(layer: &Layer, fill: Option<&Prepared>, x: f64, y: f64) -> [f32; 4] {
    let mut s = match &layer.content {
        LayerContent::Raster(surf) => sample_bilinear(surf, x, y),
        LayerContent::Fill(_) => fill.map_or([0.0; 4], |f| sample_prepared(f, x, y)),
        // Eligibility excludes the rest.
        _ => [0.0; 4],
    };
    s[3] *= mask_at(layer, x, y);
    s
}

/// Per-layer prepared content: gradient geometry and ramps hoisted out of the pixel loop.
enum Prepared {
    Solid([f32; 4]),
    Gradient { prep: crate::effects::PreparedGradientT, ramp: Ramp },
}

/// Gradient fill of `f` prepared for sampling, or `None` for other fills.
fn prepare_fill(f: &Fill, frame: Rect) -> Option<Prepared> {
    match f {
        Fill::Solid(c) => {
            let rgb = c.to_rgb();
            Some(Prepared::Solid([rgb[0], rgb[1], rgb[2], c.alpha]))
        }
        Fill::Gradient { angle, scale, style, reverse, offset, .. } => {
            let prep = gradient_t_prepare(*style, *angle, *scale, *reverse, *offset, frame);
            Ramp::new(f).map(|ramp| Prepared::Gradient { prep, ramp })
        }
        _ => None,
    }
}

fn sample_prepared(p: &Prepared, x: f64, y: f64) -> [f32; 4] {
    match p {
        Prepared::Solid(c) => *c,
        Prepared::Gradient { prep, ramp } => ramp.sample(gradient_t_eval(prep, x as f32 + 0.5, y as f32 + 0.5)),
    }
}

/// Pixel mask value at document position, bilinearly resampled. Effective value follows
/// `LayerMask::values_into` (`1 - density * (1 - v)`), not a plain product.
fn mask_at(layer: &Layer, x: f64, y: f64) -> f32 {
    let Some(m) = layer.mask.as_ref().filter(|m| m.enabled) else { return 1.0 };
    let v = sample_bilinear(&m.surface, x, y)[0];
    (1.0 - m.density * (1.0 - v)).clamp(0.0, 1.0)
}

/// Bilinear resample of a surface at document coords; outside reads transparent.
fn sample_bilinear(surf: &Surface, x: f64, y: f64) -> [f32; 4] {
    if !x.is_finite() || !y.is_finite() {
        return [0.0; 4];
    }
    let (x0, y0) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = ((x - x0 as f64) as f32, (y - y0 as f64) as f32);
    let n = surf.channels().min(4);
    let fmt = surf.format();
    // Stack scratch shared by the four taps: a heap Vec per tap would dominate the render.
    let mut scratch = [[0.0f32; 4]; 4];
    let mut tap = |dx: i32, dy: i32| -> [f32; 4] {
        let v = &mut scratch[(dx as usize) * 2 + dy as usize % 2];
        v.fill(0.0);
        surf.read_pixel(x0 + dx, y0 + dy, &mut v[..n]);
        photocraft_raster::to_rgba(&fmt, v)
    };
    let (a, b, c, d) = (tap(0, 0), tap(1, 0), tap(0, 1), tap(1, 1));
    let mut out = [0.0f32; 4];
    for i in 0..4 {
        let top = a[i] + (b[i] - a[i]) * fx;
        let bot = c[i] + (d[i] - c[i]) * fx;
        out[i] = top + (bot - top) * fy;
    }
    out
}

/// Join a reduced top over its base (see [`crate::render_reduced_split`]).
pub fn join_top(top: &Buffer, base: &Buffer) -> Buffer {
    blend_over(top, base)
}
