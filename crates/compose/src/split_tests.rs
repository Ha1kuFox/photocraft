//! Split reduced composite oracle (`render_reduced_split` vs `render_reduced`): a live
//! preview that changes one clip group per frame composites the static part once and only
//! the changed group fresh. Closeness is per-pixel `close4` (2/255, like the other oracles):
//! the split reassociates float blends across one Normal boundary.

use super::*;
use photocraft_color::{BlendMode, Color, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Adjustment, Document, Fill, GradientStyle, Knockout, Layer, LayerContent, LayerId, LayerMask};
use photocraft_geom::{Rect, Size};

const E: f32 = 2.0 / 255.0;

fn max_diff(a: &Buffer, b: &Buffer) -> f32 {
    assert_eq!(a.rect, b.rect);
    a.px.iter()
        .zip(&b.px)
        .map(|(x, y)| x.iter().zip(y).map(|(u, v)| (u - v).abs()).fold(0.0, f32::max))
        .fold(0.0, f32::max)
}

fn assert_split(doc: &Document, grad: LayerId, w: u32, h: u32) {    let k = preview_split(doc, grad).expect("eligible stack must split");
    let full = render_reduced(doc, w, h);
    let (split, base) = render_reduced_split(doc, k, w, h, None);
    assert!(max_diff(&full, &split) <= E, "split matches full");
    // The cached-base path renders only the top part and joins identically.
    let (split2, _) = render_reduced_split(doc, k, w, h, Some(base));
    assert!(max_diff(&full, &split2) <= E, "cached-base split matches full");
}

fn photo_doc() -> Document {
    let mut d = Document::with_background("t", Size::new(96, 64), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let mut photo = Layer::raster("photo", PixelFormat::RGBA8);
    photo.id = LayerId(7);
    photo.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 96, 64), &[0.2, 0.5, 0.3, 1.0]);
    d.layers.push(photo);
    d
}

fn raster_layer(name: &str, rect: Rect, rgba: [f32; 4]) -> Layer {
    let mut l = Layer::raster(name, PixelFormat::RGBA8);
    l.surface_mut().unwrap().fill_rect(rect, &rgba);
    l
}

fn gradient_layer(angle: f32) -> Layer {
    Layer::new(
        "gradient",
        LayerContent::Fill(Fill::gradient(
            vec![(0.0, Color::BLACK), (0.5, Color::rgb(0.8, 0.1, 0.1)), (1.0, Color::WHITE)],
            angle,
            100.0,
            GradientStyle::Linear,
            false,
        )),
    )
}

fn grad_id(doc: &Document) -> LayerId {
    doc.layers.iter().find(|l| matches!(l.content, LayerContent::Fill(_))).unwrap().id
}

#[test]
fn gradient_over_photo_with_normal_top() {
    let mut doc = photo_doc();
    doc.layers.push(gradient_layer(30.0));
    let mut top = raster_layer("top", Rect::new(0, 0, 48, 64), [0.9, 0.1, 0.1, 0.8]);
    top.opacity = 0.8;
    doc.layers.push(top);
    assert_split(&doc, grad_id(&doc), 48, 32);
}

#[test]
fn exotic_blend_inside_top_stays_exact() {
    let mut doc = photo_doc();
    doc.layers.push(gradient_layer(30.0));
    let mut top = raster_layer("multiply", Rect::new(0, 0, 96, 64), [0.9, 0.9, 0.2, 0.9]);
    top.blend = BlendMode::Multiply;
    doc.layers.push(top);
    assert_split(&doc, grad_id(&doc), 48, 32);
}

#[test]
fn exotic_blend_at_boundary_falls_back() {
    let mut doc = photo_doc();
    let mut g = gradient_layer(30.0);
    g.blend = BlendMode::Multiply;
    doc.layers.push(g);
    assert!(preview_split(&doc, grad_id(&doc)).is_none());
}

#[test]
fn adjustment_above_falls_back_adjustment_below_is_fine() {
    let mut doc = photo_doc();
    doc.layers.push(gradient_layer(30.0));
    doc.layers.push(Layer::new("curves", LayerContent::Adjustment(Adjustment::identity_curves())));
    assert!(preview_split(&doc, grad_id(&doc)).is_none(), "adjustment above: full path");
    let mut doc = photo_doc();
    doc.layers.push(Layer::new("curves", LayerContent::Adjustment(Adjustment::identity_curves())));
    doc.layers.push(gradient_layer(30.0));
    assert_split(&doc, grad_id(&doc), 48, 32);
}

#[test]
fn clipped_gradient_splits_at_its_base() {
    // The painter's main case: the gradient clipped to the photo below.
    let mut doc = photo_doc();
    let mut g = gradient_layer(120.0);
    g.clipped = true;
    doc.layers.push(g);
    let gid = grad_id(&doc);
    assert_eq!(preview_split(&doc, gid), Some(1), "split starts at the clip base");
    assert_split(&doc, gid, 48, 32);
}

#[test]
fn mask_and_opacity_on_gradient_stay_exact() {
    let mut doc = photo_doc();
    let mut g = gradient_layer(200.0);
    let mut m = LayerMask::reveal_all();
    m.surface.fill_rect(Rect::new(0, 0, 48, 64), &[0.4]);
    g.mask = Some(m);
    g.opacity = 0.7;
    doc.layers.push(g);
    assert_split(&doc, grad_id(&doc), 48, 32);
}

#[test]
fn nested_gradient_and_knockout_fall_back() {
    let mut group = Layer::group("group", vec![gradient_layer(10.0)]);
    let gid = group.children_mut().unwrap()[0].id;
    let mut doc = photo_doc();
    doc.layers.push(group);
    assert!(preview_split(&doc, gid).is_none(), "nested layers never split");
    let mut doc = photo_doc();
    doc.layers.push(gradient_layer(10.0));
    doc.layers[2].advanced.knockout = Knockout::Shallow;
    assert!(preview_split(&doc, grad_id(&doc)).is_none(), "knockout top: full path");
}

/// Live-drag preview path: exact base + reduced top. Preview-only approximation, so the
/// epsilon is wider than the exact split's (measured, with headroom: smooth photo content
/// stays within a few levels).
const PE: f32 = 6.0 / 255.0;

fn assert_preview(doc: &Document, grad: LayerId, w: u32, h: u32) {
    let k = preview_split(doc, grad).expect("eligible stack must split");
    let full = render_reduced(doc, w, h);
    let (prev, base) = render_reduced_preview(doc, k, w, h, None).expect("plain stack previews reduced");
    let d = max_diff(&full, &prev);
    assert!(d <= PE, "preview within {PE}: got {d}");
    let (prev2, _) = render_reduced_preview(doc, k, w, h, Some(base)).expect("cached-base path");
    assert!(max_diff(&full, &prev2) <= PE, "cached-base preview matches");
}

#[test]
fn preview_gradient_over_photo() {
    let mut doc = photo_doc();
    doc.layers.push(gradient_layer(30.0));
    assert_preview(&doc, grad_id(&doc), 48, 32);
}

#[test]
fn preview_clipped_gradient() {
    let mut doc = photo_doc();
    let mut g = gradient_layer(120.0);
    g.clipped = true;
    doc.layers.push(g);
    assert_preview(&doc, grad_id(&doc), 48, 32);
}

#[test]
fn preview_masked_gradient_with_top() {
    let mut doc = photo_doc();
    let mut g = gradient_layer(200.0);
    let mut m = LayerMask::reveal_all();
    m.surface.fill_rect(Rect::new(0, 0, 48, 64), &[0.4]);
    g.mask = Some(m);
    doc.layers.push(g);
    doc.layers.push(raster_layer("top", Rect::new(48, 0, 48, 64), [0.9, 0.1, 0.1, 0.8]));
    assert_preview(&doc, grad_id(&doc), 48, 32);
}

#[test]
fn preview_falls_back_on_adjustment_above() {
    let mut doc = photo_doc();
    doc.layers.push(gradient_layer(30.0));
    doc.layers.push(Layer::new("curves", LayerContent::Adjustment(Adjustment::identity_curves())));
    assert!(preview_split(&doc, grad_id(&doc)).is_none(), "adjustment above: full path");
    // The reduced renderer itself refuses adjustments too (defense in depth).
    let mut plain = photo_doc();
    plain.layers.push(gradient_layer(30.0));
    let k = preview_split(&plain, grad_id(&plain)).unwrap();
    let mut with_adj = plain.clone();
    with_adj.layers.push(Layer::new("curves", LayerContent::Adjustment(Adjustment::identity_curves())));
    assert!(render_reduced_preview(&with_adj, k, 48, 32, None).is_none());
}


