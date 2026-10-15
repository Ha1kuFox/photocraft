//! Document canvas: display texture cache, view transform, tool dispatch, extra document windows.
//!
//! Rendering is CPU (`photocraft-compose`) for now, uploaded into an egui texture. Brush strokes
//! update only their damage rectangle (`set_partial`). The wgpu compositor (M5) will replace this
//! with direct GPU rendering behind the same `CanvasCache` interface.

use craft::behavior::PressPhase;
use egui::Rect;
use photocraft_doc::Document;

use crate::PhotocraftApp;
use crate::state::View;

#[cfg(test)]
use egui::{Color32, Pos2, Stroke, TextureOptions, Vec2, pos2, vec2};
#[cfg(test)]
use photocraft_doc::LayerContent;
#[cfg(test)]
use photocraft_geom::Rect as DRect;
#[cfg(test)]
use serde_json::json;

#[cfg(test)]
use crate::state::Tool;

pub mod artboard_ui;
pub(crate) mod background_redraw;
pub(crate) mod cam_preview;
pub mod canvas_tool_menu;
pub mod channel_view;
pub(crate) mod cursor_policy;
mod display;
pub(crate) mod document_area;
pub(crate) mod document_display;
pub(crate) mod document_windows;
mod eyedropper_tool;
pub(crate) mod filter_preview;
pub mod gpu_canvas;
mod gpu_picture;
pub mod guide_select;
pub mod layer_pick_ui;
pub mod move_ui;
mod navigator;
pub mod outline;
pub mod paint_mouse;
mod paint_thread;
#[cfg(test)]
mod paint_thread_tests;
pub(crate) mod paint_tool;
#[cfg(test)]
mod pasteboard_menu_separator_tests;
#[cfg(test)]
mod pasteboard_menu_tools_tests;
pub mod patch_preview;
pub mod quick_pick;
mod raster_source;
pub mod rulers;
pub mod sampling;
pub mod scroll_all_windows;
pub mod select_mask_view;
pub mod snap_ui;
pub(crate) mod start_screen;
pub(crate) mod strokes;
pub(crate) use strokes::ClonePreviewCache;
pub(crate) mod tabs;
#[cfg(test)]
pub(crate) mod testing;
mod textures;
pub(super) mod tile_overlay;
pub mod tool_feedback;
pub(crate) mod tools;
mod transform_controls;
pub(crate) mod viewport_pages;
mod window_drop;

pub(crate) use cursor_policy::alt_samples;
pub(crate) use display::GPU_OUTPUT;
#[cfg(test)]
pub(crate) use display::cache_key;
#[cfg(test)]
use display::sync_display_lut;
use display::texture_key;
pub(crate) use display::{canvas_display, display_doc, display_image};
pub(crate) use document_area::arranged_cells;
pub use document_area::document_area;
pub(crate) use document_display::view_target;
pub use document_display::{CanvasHost, active_view};
pub use document_windows::extra_windows;
#[cfg(test)]
pub(crate) use filter_preview::committed_filter_preview;
#[cfg(test)]
pub(crate) use filter_preview::ensure_filter_preview;
#[cfg(test)]
use filter_preview::filter_channel_preview;
#[cfg(test)]
use filter_preview::filter_preview_display_doc;
pub(crate) use filter_preview::filter_preview_factor;
pub(crate) use navigator::NavigatorCache;
pub use navigator::{NAVIGATOR_SIDE, navigator_gpu_view, navigator_texture, navigator_waits_for_gpu};
#[cfg(test)]
pub(crate) use raster_source::CompositePrep;
pub(crate) use raster_source::SourceLogs;
pub(crate) use sampling::composite_color;
#[cfg(test)]
pub(crate) use strokes::live_retouch_points;
pub(crate) use strokes::multiple_color_target;
pub use strokes::paint_target;
#[cfg(test)]
pub(crate) use strokes::strokes_live;
pub(crate) use strokes::{CommittedStroke, LiveStroke, unsmoothed_line};
#[cfg(test)]
use tabs::{main_canvas_document, move_target_after_filter};
#[cfg(test)]
pub(crate) use testing::{ToolEvent, tool_event, tool_event_response};
pub use textures::MAX_TEXTURE;
pub(crate) use textures::visible_doc_rect;
pub(crate) use textures::{damage_since, effect_reach, retain_gpu_documents, shown_as_document};
#[cfg(test)]
use textures::{ensure_gpu, redraw_focus};
#[cfg(test)]
pub(crate) use tools::selection::floating_outline;
#[cfg(test)]
pub(crate) use transform_controls::layer_edges;

/// One document view's cached display state.
#[derive(Default)]
pub struct CanvasCache {
    pub revision: u64,
    /// Hash of the live-adjust preview used for this render (0 = none).
    pub preview_key: u64,
    /// The current image lives in the GPU canvas (`gpu_canvas`).
    pub on_gpu: bool,
}

/// Photoshop-style zoom label: "33.3", "100", "12.5".
pub fn fmt_zoom(pct: f32) -> String {
    if (pct - pct.round()).abs() < 0.05 { format!("{}", pct.round() as i64) } else { format!("{pct:.1}") }
}

pub fn mode_label(doc: &Document) -> &'static str {
    match doc.mode {
        photocraft_doc::ColorMode::Rgb => "RGB",
        photocraft_doc::ColorMode::Grayscale => "Gray",
        photocraft_doc::ColorMode::Cmyk => "CMYK",
        photocraft_doc::ColorMode::Lab => "Lab",
        photocraft_doc::ColorMode::Indexed => "Indexed",
        photocraft_doc::ColorMode::Bitmap => "Bitmap",
        photocraft_doc::ColorMode::Duotone => "Duotone",
        photocraft_doc::ColorMode::Multichannel => "Multichannel",
    }
}

/// Per-frame reconciliation of the camera with its document: a resize re-centres it. Returns the pixel aspect the view
/// is drawn with.
fn sync_view_to_document(app: &PhotocraftApp, view: &mut View, doc: &Document) -> f32 {
    let size = [doc.size.width, doc.size.height];
    if view.doc_size != size {
        // Like Photoshop: keep the zoom level, re-centre the resized document.
        if view.doc_size != [0, 0] {
            view.center = [size[0] as f32 / 2.0, size[1] as f32 / 2.0];
        }
        view.doc_size = size;
    }
    // View › Pixel Aspect Ratio Correction: document pixels are shown this much wider.
    app.ui.view.display_aspect()
}

/// Draws one document's canvas into `rect` and runs its tools. `host` names the canvas (document, viewport and document
/// window); `primary` is the main window's, where the tools are active. Returns the view after the pass and the
/// canvas's output, whose press the caller reads (a press activates its document).
pub fn canvas_view(
    app: &mut PhotocraftApp,
    ui: &mut egui::Ui,
    host: document_display::CanvasHost,
    rect: Rect,
    mut view: View,
    primary: bool,
) -> (View, craft::ui::canvas::CanvasOutput) {
    let ctx = ui.ctx().clone();
    let idx = host.idx;
    // The primary canvas keeps its view in `app.ui.views`: the pass starts from the stored view and stores the one it
    // ends with, unless something stored another view while the pass ran (a tool's commit that moved it), which then
    // stands; view commands asked of it meanwhile are kept for the next pass.
    // A New Window for Document keeps a view of its own (`window_views`), which is not the document's.
    let own_window = host.window.is_some_and(|window| app.ui.window_views.contains_key(&window));
    let stored = (primary && !own_window).then(|| app.ui.views.get(idx).cloned()).flatten();
    // `View::zoom` is device pixels per document pixel; this canvas's geometry is in its own viewport's egui points. A
    // document window can sit on another display with a different scale, so only the primary canvas records `app.ppp`
    // for the frame's shared helpers (`point_zoom`, the navigator, control-channel coordinate mapping).
    // `pixels_per_point` folds in the display scale and Interface › UI Scale alike, so dividing by it keeps 100% one
    // document pixel per physical display pixel at any UI scale (#2121).
    let ppp = ctx.pixels_per_point();
    if primary {
        app.ppp = ppp;
    }
    // The display this window is on: its monitor profile (#569). A document window on a display the last reading
    // didn't know asks for a new one.
    let output = crate::monitor_status::view_display(app, &ctx);
    if !primary {
        crate::monitor_status::check_window(app, &ctx);
    }
    let doc = app.session.documents()[idx].doc.clone();
    let aspect = sync_view_to_document(app, &mut view, &doc);
    let active = primary && app.session.active_index() == Some(idx);
    if active {
        // A document switched while a button is down ends the gesture of the document it began on.
        tools::end_foreign_gesture(app, &ctx, doc.id.0);
    }
    // Pen pressure/tilt for this frame's tool events (mouse = 1.0), unless Preferences › Tools › Use Tablet Pressure
    // is off; the pen's eraser end selects the Eraser.
    app.stylus.use_pressure = app.session.prefs().tools.use_tablet_pressure;
    let curve = app.session.prefs().tools.pressure_curve.clone();
    app.stylus.set_pressure_curve(&curve);
    crate::stylus::Stylus::sync_eraser_tool(app);
    // A Properties Levels/Curves eyedropper left behind by a tool or layer switch is dropped.
    crate::layer_picker::prune(app);
    let flip = app.ui.view.flip_horizontal;
    let pass = document_display::CanvasPass { ctx: &ctx, host, doc: &doc, output, rect, ppp, flip, aspect };
    let shown = document_display::show(app, ui, &mut view, &pass);
    canvas_tool_menu::run(ui, app, &shown.output, &shown.response, shown.guides.as_ref(), shown.overlay_context);
    // A press on the canvas while a tool letter is held makes that tool temporary (#2672).
    if active && !app.dialog_open() && matches!(shown.output.press.phase, PressPhase::Down { .. }) {
        crate::spring_tools::note_canvas_press(app);
    }
    if let Some(stored) = stored
        && let Some(current) = app.ui.views.get(idx)
        && let Some(asked) = current.pending.strip_prefix(&stored.pending[..])
        && (View { pending: stored.pending.clone(), ..current.clone() }) == stored
    {
        // View commands asked while the pass ran (a canvas menu row) wait for the next pass.
        let mut next = view.clone();
        next.pending.extend_from_slice(asked);
        app.ui.views[idx] = next;
    }
    // Scrollbars (scrollbars.rs): drawn over the canvas edges, they take the pointer there.
    let t0 = crate::gpu_canvas::now_ms();
    let before = view.center;
    let over_bars = app.ui.view.shows_scrollbars()
        && crate::scrollbars::show(ui, rect, &mut view, flip, aspect, app.session.prefs().tools.overscroll, egui::Id::new(("pc-canvas", idx)));
    ctx.data_mut(|d| d.insert_temp(bars_id, over_bars));
    if view.center != before {
        ctx.request_repaint();
    }
    if primary {
        app.perf.span("scrollbars", crate::gpu_canvas::now_ms() - t0);
        app.hover_doc = response.hover_pos().map(|p| xf.to_doc(p));
    }
    if primary && app.ui.extras.rulers {
        crate::rulers::draw_rulers(app, ui, full, &xf);
    }
    crate::rotate_view::overlay(app, &ctx, &painter, &xf, &mut view, &response);
    if primary {
        // Tool input can change the stored camera while navigation edits the local view.
        // Merge changed camera values so write-back preserves tool-driven pan and rotation.
        if let (Some((center, rotation)), Some(now)) = (camera, app.ui.views.get(idx)) {
            if now.center != center {
                view.center = now.center;
                ctx.request_repaint();
            }
            if now.rotation != rotation {
                view.rotation = now.rotation;
                ctx.request_repaint();
            }
        }
        app.ui.views[idx] = view.clone();
    }
    view
}

/// Zoom to `new_zoom` about the pointer `p`. By default the document point under it stays under
/// it; with Preferences › Tools › Zoom Clicked Point to Center (`center_on_point`) it moves to the
/// centre of the view instead (#204).
/// Where a wheel or pinch zoom anchors for a pointer at `p`: the pointer itself over the image,
/// otherwise the nearest point of the image (#3720). Anchoring on a pointer out on the gray
/// pasteboard slid the image away from it, out of the window after a few notches; Photoshop keeps
/// the image in view. Clamped in document space, so a rotated or flipped view behaves the same.
fn wheel_anchor(xf: &ViewXform, p: Pos2, doc_size: [u32; 2]) -> Pos2 {
    let [w, h] = doc_size;
    let d = xf.to_doc(p);
    if w == 0 || h == 0 || !(d[0].is_finite() && d[1].is_finite()) {
        return p;
    }
    let c = [d[0].clamp(0.0, f64::from(w)), d[1].clamp(0.0, f64::from(h))];
    if c == d { p } else { xf.to_screen(c[0] as f32, c[1] as f32) }
}

fn zoom_about(view: &mut View, xf: &ViewXform, p: Pos2, new_zoom: f32, center_on_point: bool, ppp: f32) {
    let before = xf.to_doc(p);
    view.zoom = new_zoom;
    if center_on_point {
        view.center = [before[0] as f32, before[1] as f32];
        return;
    }
    // `new_zoom` is device px per document pixel; the pointer vector is in egui points.
    let ppp = if ppp.is_finite() && ppp > 0.0 { ppp } else { 1.0 };
    let u = xf.unmap_vec((p - xf.rect.center()) / (new_zoom / ppp));
    view.center = [before[0] as f32 - u.x, before[1] as f32 - u.y];
}

/// Textured document quad whose corners follow [`ViewXform`] (rotation + flip).
fn paint_mapped_image(painter: &egui::Painter, xf: &ViewXform, tex: egui::TextureId, uv: Rect, size: [f32; 2], tint: Color32) {
    let [w, h] = size;
    let pts = [xf.to_screen(0.0, 0.0), xf.to_screen(w, 0.0), xf.to_screen(w, h), xf.to_screen(0.0, h)];
    let uvs = [uv.min, pos2(uv.max.x, uv.min.y), uv.max, pos2(uv.min.x, uv.max.y)];
    let mut mesh = Mesh::with_texture(tex);
    for i in 0..4 {
        mesh.vertices.push(egui::epaint::Vertex { pos: pts[i], uv: uvs[i], color: tint });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}

/// The GPU canvas view, then its View › Pattern Preview copies (#1067): the same texture drawn
/// again whole document sizes away, each with its own view uniform and no shadow.
fn paint_gpu_view(painter: &egui::Painter, rect: Rect, params: crate::gpu_canvas::ViewParams, copies: &[[i32; 2]]) {
    crate::gpu_canvas::GpuCanvas::paint(painter, rect, params);
    for (k, &c) in copies.iter().enumerate() {
        let copy = crate::gpu_canvas::ViewParams {
            offset: crate::pattern_preview::offset(c, params.doc_size),
            shadow: false,
            // Keyed by slot, not by tile index, so panning reuses the same few uniform buffers.
            view_key: egui::Id::new((params.view_key, "pattern", k)).value(),
            ..params
        };
        crate::gpu_canvas::GpuCanvas::paint(painter, rect, copy);
    }
}

/// The mapping that draws Pattern Preview copy `c` of a `size` document where it sits next to
/// the document: the same view, looking the matching number of document sizes back.
fn pattern_xf(xf: &ViewXform, c: [i32; 2], size: [u32; 2]) -> ViewXform {
    let [ox, oy] = crate::pattern_preview::offset(c, size);
    ViewXform { center: [xf.center[0] - ox, xf.center[1] - oy], ..*xf }
}

/// CPU canvas: the transparency checkerboard (or white) and the display texture, for the
/// document or one of its Pattern Preview copies (`xf` from [`pattern_xf`]).
fn paint_cpu_document(
    painter: &egui::Painter,
    xf: &ViewXform,
    bounds: DRect,
    rotated: bool,
    checker: Option<(egui::TextureId, f32)>,
    tex: Option<egui::TextureId>,
) {
    let size = [bounds.width() as f32, bounds.height() as f32];
    let img_rect = xf.doc_rect(bounds);
    match checker {
        Some((checker_id, square)) => {
            if rotated {
                let tiles = vec2(size[0], size[1]) / (2.0 * square);
                paint_mapped_image(painter, xf, checker_id, Rect::from_min_max(Pos2::ZERO, pos2(tiles.x, tiles.y)), size, Color32::WHITE);
            } else {
                let tiles = img_rect.size() / (2.0 * square);
                painter.image(checker_id, img_rect, Rect::from_min_max(Pos2::ZERO, pos2(tiles.x, tiles.y)), Color32::WHITE);
            }
        }
        None => {
            if rotated {
                paint_mapped_quad(painter, xf, size, Color32::WHITE);
            } else {
                painter.rect_filled(img_rect, 0.0, Color32::WHITE);
            }
        }
    }
    if let Some(tex) = tex {
        if rotated {
            paint_mapped_image(painter, xf, tex, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), size, Color32::WHITE);
        } else {
            let uv = if xf.flip { Rect::from_min_max(pos2(1.0, 0.0), pos2(0.0, 1.0)) } else { Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)) };
            painter.image(tex, img_rect, uv, Color32::WHITE);
        }
    }
}

fn paint_mapped_quad(painter: &egui::Painter, xf: &ViewXform, size: [f32; 2], color: Color32) {
    let [w, h] = size;
    let pts = vec![xf.to_screen(0.0, 0.0), xf.to_screen(w, 0.0), xf.to_screen(w, h), xf.to_screen(0.0, h)];
    painter.add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
}

/// Draw boundary segments as marching ants: white base, black dashes phased along x + y.
fn marching_ants_segments(painter: &egui::Painter, xf: &ViewXform, segs: &[crate::outline::Segment], time: f64) {
    marching_ants_mapped(painter, segs, time, |p| xf.to_screen(p[0] as f32, p[1] as f32));
}

/// [`marching_ants_segments`] with each outline point placed on screen by `to_screen` (a
/// transformed outline: Content-Aware Move's Transform On Drop).
pub(crate) fn marching_ants_mapped(painter: &egui::Painter, segs: &[crate::outline::Segment], time: f64, to_screen: impl Fn([i32; 2]) -> Pos2) {
    let dash = 4.0f32;
    let phase = ((time * 10.0) % (dash as f64 * 2.0)) as f32;
    let clip = painter.clip_rect();
    // Horizontal and vertical segments go in one mesh of 1 px quads, which egui draws as it is: a
    // line shape per dash took egui ~35 ms a frame to tessellate for a speckled selection seen
    // whole (about 10⁵ segments), ten frames a second while the selection is shown. A slanted
    // segment (a transformed outline) stays a line shape, which egui antialiases.
    let mut mesh = egui::Mesh::default();
    // A 1 px wide quad from `a` to `b` (`side` is half a pixel across the line).
    let mut quad = |a: egui::Pos2, b: egui::Pos2, side: egui::Vec2, color: Color32| {
        let i = mesh.vertices.len() as u32;
        for p in [a + side, a - side, b - side, b + side] {
            mesh.colored_vertex(p, color);
        }
        mesh.add_triangle(i, i + 1, i + 2);
        mesh.add_triangle(i, i + 2, i + 3);
    };
    for (a, b) in segs {
        let pa = to_screen(*a);
        let pb = to_screen(*b);
        if !clip.intersects(Rect::from_two_pos(pa, pb).expand(1.0)) {
            continue;
        }
        let pa = pos2(pa.x.round() + 0.5, pa.y.round() + 0.5);
        let pb = pos2(pb.x.round() + 0.5, pb.y.round() + 0.5);
        let len = pa.distance(pb);
        if len < 0.5 {
            continue;
        }
        let dir = (pb - pa) / len;
        let side = dir.rot90() * 0.5;
        let straight = pa.x == pb.x || pa.y == pb.y;
        if straight {
            // The white base runs half a pixel past both ends, so joined segments meet at corners.
            quad(pa - dir * 0.5, pb + dir * 0.5, side, Color32::WHITE);
        } else {
            painter.line_segment([pa, pb], Stroke::new(1.0, Color32::WHITE));
        }
        // Phase by screen position so dashes line up across joined segments.
        let start = (pa.x + pa.y + phase).rem_euclid(dash * 2.0);
        let mut t = -start;
        while t < len {
            let s0 = t.max(0.0);
            let s1 = (t + dash).min(len);
            if s1 > s0 {
                if straight {
                    quad(pa + dir * s0, pa + dir * s1, side, Color32::BLACK);
                } else {
                    painter.line_segment([pa + dir * s0, pa + dir * s1], Stroke::new(1.0, Color32::BLACK));
                }
            }
            t += dash * 2.0;
        }
    }
    if !mesh.is_empty() {
        painter.add(egui::Shape::mesh(mesh));
    }
}

#[allow(dead_code)]
fn marching_ants(painter: &egui::Painter, r: Rect, time: f64) {
    let dash = 4.0;
    let offset = ((time * 8.0) % (dash as f64 * 2.0)) as f32;
    painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::WHITE), egui::StrokeKind::Middle);
    let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = a.distance(b);
        let dir = (b - a) / len.max(1e-3);
        let mut t = -offset;
        while t < len {
            let s = t.max(0.0);
            let e = (t + dash).min(len);
            if e > s {
                painter.line_segment([a + dir * s, a + dir * e], Stroke::new(1.0, Color32::BLACK));
            }
            t += dash * 2.0;
        }
    }
}

/// Photoshop crop overlay: the `shield` outside (`crop_shield::fill`; none when disabled), bright
/// frame, the composition `guides` (unit-square polylines, `crop_overlay::guides`), corner and
/// edge handles.
fn crop_overlay(painter: &egui::Painter, r: Rect, shield: Option<Color32>, guides: &[crate::crop_overlay::Polyline]) {
    let clip = painter.clip_rect();
    if let Some(dim) = shield {
        for band in [
            Rect::from_min_max(clip.min, egui::pos2(clip.max.x, r.min.y)),
            Rect::from_min_max(egui::pos2(clip.min.x, r.max.y), clip.max),
            Rect::from_min_max(egui::pos2(clip.min.x, r.min.y), egui::pos2(r.min.x, r.max.y)),
            Rect::from_min_max(egui::pos2(r.max.x, r.min.y), egui::pos2(clip.max.x, r.max.y)),
        ] {
            painter.rect_filled(band, 0.0, dim);
        }
    }
    painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::WHITE), egui::StrokeKind::Middle);
    draw_crop_guides(painter, guides, |a, b| r.min + r.size() * vec2(a, b));
    let h = Stroke::new(3.0, Color32::WHITE);
    let l = 14.0f32.min(r.width() / 3.0).min(r.height() / 3.0);
    for (c, dx, dy) in [(r.left_top(), 1.0, 1.0), (r.right_top(), -1.0, 1.0), (r.left_bottom(), 1.0, -1.0), (r.right_bottom(), -1.0, -1.0)] {
        painter.line_segment([c, c + vec2(l * dx, 0.0)], h);
        painter.line_segment([c, c + vec2(0.0, l * dy)], h);
    }
    // Edge handles: short bars at the middle of each side.
    let (lx, ly) = (l.min(r.width() / 4.0) / 2.0, l.min(r.height() / 4.0) / 2.0);
    for (c, horizontal) in [(r.center_top(), true), (r.center_bottom(), true), (r.left_center(), false), (r.right_center(), false)] {
        let e = if horizontal { vec2(lx, 0.0) } else { vec2(0.0, ly) };
        painter.line_segment([c - e, c + e], h);
    }
}

/// [`crop_overlay`] for a turned frame (or a rotated view): `q` is the frame's top-left, top-right,
/// bottom-right and bottom-left corner on screen, a parallelogram.
fn crop_overlay_turned(painter: &egui::Painter, q: [Pos2; 4], shield: Option<Color32>, guides: &[crate::crop_overlay::Polyline]) {
    let (u, v) = (q[1] - q[0], q[3] - q[0]);
    let (lu, lv) = (u.length(), v.length());
    if !(lu.is_finite() && lv.is_finite()) || lu < 1e-3 || lv < 1e-3 {
        return;
    }
    let at = |a: f32, b: f32| q[0] + u * a + v * b;
    // The shield: four trapezoids from the frame out to the frame grown past the whole view.
    let clip = painter.clip_rect();
    let reach = clip.size().length() + (clip.center() - at(0.5, 0.5)).length();
    let (eu, ev) = (reach / lu, reach / lv);
    let outer = [at(-eu, -ev), at(1.0 + eu, -ev), at(1.0 + eu, 1.0 + ev), at(-eu, 1.0 + ev)];
    if let Some(dim) = shield {
        let mut mesh = egui::Mesh::default();
        for p in q.iter().chain(outer.iter()) {
            mesh.colored_vertex(*p, dim);
        }
        for i in 0..4u32 {
            let j = (i + 1) % 4;
            mesh.add_triangle(i, j, 4 + j);
            mesh.add_triangle(i, 4 + j, 4 + i);
        }
        painter.add(egui::Shape::mesh(mesh));
    }
    painter.add(egui::Shape::closed_line(q.to_vec(), Stroke::new(1.0, Color32::WHITE)));
    draw_crop_guides(painter, guides, at);
    let h = Stroke::new(3.0, Color32::WHITE);
    let l = 14.0f32.min(lu / 3.0).min(lv / 3.0);
    let (du, dv) = (u / lu, v / lv);
    for (c, su, sv) in [(q[0], 1.0, 1.0), (q[1], -1.0, 1.0), (q[3], 1.0, -1.0), (q[2], -1.0, -1.0)] {
        painter.line_segment([c, c + du * (l * su)], h);
        painter.line_segment([c, c + dv * (l * sv)], h);
    }
    // Edge handles: short bars at the middle of each side, along it.
    let (lx, ly) = (l.min(lu / 4.0) / 2.0, l.min(lv / 4.0) / 2.0);
    for (c, e) in [(at(0.5, 0.0), du * lx), (at(0.5, 1.0), du * lx), (at(0.0, 0.5), dv * ly), (at(1.0, 0.5), dv * ly)] {
        painter.line_segment([c - e, c + e], h);
    }
}

/// The crop overlay's guides, each unit-square point placed on screen by `at`.
fn draw_crop_guides(painter: &egui::Painter, guides: &[crate::crop_overlay::Polyline], at: impl Fn(f32, f32) -> Pos2) {
    let thin = Stroke::new(1.0, Color32::from_white_alpha(90));
    for l in guides {
        painter.add(egui::Shape::line(l.iter().map(|p| at(p[0], p[1])).collect(), thin));
    }
}

/// What the Crop tool shows past the canvas, cached per document state ([`draw_beyond_canvas`]).
#[derive(Clone)]
struct BeyondCanvas {
    snapshot: std::sync::Weak<Document>,
    key: u64,
    /// Every layer's pixels and the canvas (`extra_cmds::reveal_all_bounds`).
    extent: DRect,
    /// The composite around the canvas: each band, the area its texture covers, the texture.
    bands: Vec<(DRect, DRect, egui::TextureHandle)>,
}

/// Band `r` around the canvas `c` (on screen) pushed a point under the canvas's edge, so that the
/// edge's anti-aliased pixels blend with the band rather than with the pasteboard.
fn under_canvas(mut r: Rect, c: Rect) -> Rect {
    let near = |a: f32, b: f32| (a - b).abs() < 0.01;
    if near(r.max.y, c.min.y) {
        r.max.y += 1.0;
    }
    if near(r.min.y, c.max.y) {
        r.min.y -= 1.0;
    }
    if near(r.max.x, c.min.x) {
        r.max.x += 1.0;
    }
    if near(r.min.x, c.max.x) {
        r.min.x -= 1.0;
    }
    r
}

/// `outer` minus `inner`: the bands above, below, left and right of it (empty ones dropped).
fn ring(outer: DRect, inner: DRect) -> Vec<DRect> {
    let i = inner.intersect(&outer);
    if i.is_empty() {
        return if outer.is_empty() { Vec::new() } else { vec![outer] };
    }
    [
        DRect::new(outer.x0, outer.y0, outer.x1, i.y0),
        DRect::new(outer.x0, i.y1, outer.x1, outer.y1),
        DRect::new(outer.x0, i.y0, i.x0, i.y1),
        DRect::new(i.x1, i.y0, outer.x1, i.y1),
    ]
    .into_iter()
    .filter(|r| !r.is_empty())
    .collect()
}

/// Crop tool, while a frame is edited (`crop_ui::shows_beyond_canvas`): like Photoshop's crop
/// preview, the canvas grows to every layer's pixels (a crop with Delete Cropped Pixels off keeps
/// them; a moved layer reaches past the edges) and to the frame, transparency drawn as the
/// checkerboard, all under the shield `crop_overlay` adds. Only the ring around the canvas is
/// composited, on the CPU, once per document state: at most `MAX_TEXTURE` texels per side, from a
/// downsampled proxy past that (so a far-off layer costs no more than a 16 MP composite).
/// Drawn before the canvas, each band reaching a point under its edge. Returns whether anything
/// past the canvas was drawn.
fn draw_beyond_canvas(app: &mut PhotocraftApp, painter: &egui::Painter, xf: &ViewXform, idx: usize, output: Option<u32>) -> bool {
    let ctx = painter.ctx().clone();
    let Some((snapshot, id)) = app.session.documents().get(idx).map(|st| (std::sync::Arc::downgrade(&st.doc), st.doc.id)) else { return false };
    let key = egui::Id::new(("crop-beyond-canvas", id.0, output));
    let frame =
        app.ui.crop_rect.filter(|f| f.iter().all(|v| v.is_finite()) && app.session.active_index() == Some(idx) && crate::crop_ui::shows_beyond_canvas(app));
    let Some(f) = frame else {
        ctx.data_mut(|d| d.remove::<BeyondCanvas>(key));
        return false;
    };
    let (doc, preview_key) = display_doc(app, idx);
    if doc.has_artboards() {
        return false;
    }
    let (display, display_key) = canvas_display(app, &doc, output);
    let doc_key = preview_key ^ display_key;
    let cached: Option<BeyondCanvas> = ctx.data(|d| d.get_temp(key));
    let c = match cached.filter(|c| c.snapshot.ptr_eq(&snapshot) && c.key == doc_key) {
        Some(c) => c,
        None => {
            let t0 = crate::gpu_canvas::now_ms();
            let extent = photocraft_engine::extra_cmds::reveal_all_bounds(&doc);
            // One texel per `k` document pixels keeps every texture within MAX_TEXTURE.
            let k = extent.width().max(extent.height()).div_ceil(MAX_TEXTURE).max(1);
            let proxy = (k > 1).then(|| photocraft_compose::proxy::proxy_document(&doc, k));
            let src = proxy.as_ref().unwrap_or(&*doc);
            let k = k as i32;
            let opts = TextureOptions { magnification: egui::TextureFilter::Nearest, ..TextureOptions::LINEAR };
            let bands = ring(extent, doc.bounds())
                .into_iter()
                .map(|b| {
                    // The proxy pixels over `b`: document pixels / k, rounded outwards.
                    let up = |v: i32| v.div_euclid(k) + i32::from(v.rem_euclid(k) != 0);
                    let p = DRect::new(b.x0.div_euclid(k), b.y0.div_euclid(k), up(b.x1), up(b.y1));
                    let buf = photocraft_compose::render_reduced_rect(src, p, p.width(), p.height());
                    let tex = ctx.load_texture("crop-beyond-canvas", display_image(display.as_deref(), &buf), opts);
                    (b, DRect::new(p.x0.saturating_mul(k), p.y0.saturating_mul(k), p.x1.saturating_mul(k), p.y1.saturating_mul(k)), tex)
                })
                .collect();
            app.perf.span("crop-beyond-canvas", crate::gpu_canvas::now_ms() - t0);
            let c = BeyondCanvas { snapshot, key: doc_key, extent, bands };
            ctx.data_mut(|d| d.insert_temp(key, c.clone()));
            c
        }
    };
    // Transparency out to the frame, in phase with the canvas's own checkerboard.
    let canvas = xf.doc_rect(doc.bounds());
    // A turned frame reaches as far as its corners.
    let f = crate::crop_ui::turned_bounds(f, crate::crop_ui::angle(app));
    let fr = DRect::new(f[0].floor() as i32, f[1].floor() as i32, f[2].ceil() as i32, f[3].ceil() as i32);
    let square = app.session.prefs().transparency_and_gamut.square();
    let checker_id = square.map(|_| checker(app, &ctx));
    let around = ring(c.extent.union(&fr), doc.bounds());
    // A turned view (Rotate View, or the Crop tool's default mode turning the image behind its
    // box) draws each band as its turned quad: its screen bounding box would stretch the band's
    // texture and spread the checkerboard past the band.
    let turned = xf.rotation != 0.0;
    let checker_uv = |p: Pos2, sq: f32| ((p - canvas.min) / (2.0 * sq)).to_pos2();
    for &b in &around {
        if turned {
            let q = doc_quad(xf, b);
            match square.zip(checker_id) {
                Some((sq, id)) => textured_quad(painter, id, q, q.map(|p| checker_uv(p, sq))),
                None => {
                    painter.add(egui::Shape::convex_polygon(q.to_vec(), Color32::WHITE, Stroke::NONE));
                }
            }
            continue;
        }
        let r = under_canvas(xf.doc_rect(b), canvas);
        match square.zip(checker_id) {
            Some((sq, id)) => {
                let uv = Rect::from_min_max(checker_uv(r.min, sq), checker_uv(r.max, sq));
                painter.image(id, r, uv, Color32::WHITE);
            }
            None => {
                painter.rect_filled(r, 0.0, Color32::WHITE);
            }
        }
    }
    for (b, t, tex) in &c.bands {
        let part = |a: i32, o: i32, len: u32| (i64::from(a) - i64::from(o)) as f32 / len.max(1) as f32;
        let (mut u0, mut u1) = (part(b.x0, t.x0, t.width()), part(b.x1, t.x0, t.width()));
        if turned {
            // The quad's corners are the band's own (flip included), so no swap.
            let (v0, v1) = (part(b.y0, t.y0, t.height()), part(b.y1, t.y0, t.height()));
            textured_quad(painter, tex.id(), doc_quad(xf, *b), [pos2(u0, v0), pos2(u1, v0), pos2(u1, v1), pos2(u0, v1)]);
            continue;
        }
        if xf.flip {
            (u0, u1) = (u1, u0);
        }
        let uv = Rect::from_min_max(pos2(u0, part(b.y0, t.y0, t.height())), pos2(u1, part(b.y1, t.y0, t.height())));
        // The texture's edge texels stretch over the point under the canvas.
        let (r, e) = (xf.doc_rect(*b), under_canvas(xf.doc_rect(*b), canvas));
        let at = |p: Pos2| uv.min + (p - r.min) / r.size() * uv.size();
        painter.image(tex.id(), e, Rect::from_min_max(at(e.min), at(e.max)), Color32::WHITE);
    }
    !around.is_empty()
}

/// Document rect `b`'s corners on screen, clockwise from its top-left (a parallelogram in a turned
/// view).
fn doc_quad(xf: &ViewXform, b: DRect) -> [Pos2; 4] {
    [(b.x0, b.y0), (b.x1, b.y0), (b.x1, b.y1), (b.x0, b.y1)].map(|(x, y)| xf.to_screen(x as f32, y as f32))
}

/// Texture `tex` drawn over the quad `q`, `uv` at its corners.
fn textured_quad(painter: &egui::Painter, tex: egui::TextureId, q: [Pos2; 4], uv: [Pos2; 4]) {
    if !q.iter().chain(uv.iter()).all(|p| p.x.is_finite() && p.y.is_finite()) {
        return;
    }
    let mut mesh = egui::Mesh::with_texture(tex);
    for (pos, uv) in q.into_iter().zip(uv) {
        mesh.vertices.push(egui::epaint::Vertex { pos, uv, color: Color32::WHITE });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}

/// Overlays that persist between gestures: polygonal or magnetic lasso in progress, pending crop box.
fn draw_tool_state(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform, hover: Option<Pos2>) {
    crate::magnetic_lasso_ui::draw(app, painter, xf, hover);
    if !app.ui.polygon.is_empty() {
        let mut pts: Vec<Pos2> = app.ui.polygon.iter().map(|p| xf.to_screen(p[0] as f32, p[1] as f32)).collect();
        if let Some(h) = hover {
            // The rubber-band preview must use the same constrained point as the click,
            // including when the canvas is zoomed, rotated or flipped.
            let mods = painter.ctx().input(|i| i.modifiers);
            if app.ui.tool == Tool::PolygonLasso && mods.shift && !mods.alt {
                let raw = xf.to_doc(h);
                let last = app.ui.polygon.last().copied().unwrap_or(raw);
                let snapped = constrained_polygon_point(last, raw);
                pts.push(xf.to_screen(snapped[0] as f32, snapped[1] as f32));
            } else {
                pts.push(h);
            }
        }
        // Just the outline and its rubber band: the vertices aren't handles to grab.
        crate::tool_feedback::draw_ants(painter, &pts, false);
    }
    crate::crop_straighten::draw(app, painter, |p| xf.to_screen(p[0] as f32, p[1] as f32), hover);
    if let Some(c) = app.ui.crop_rect {
        let deg = crate::crop_ui::angle(app);
        let shield = crate::crop_shield::current(app, painter.ctx());
        if deg == 0.0 && xf.rotation == 0.0 {
            let r = Rect::from_two_pos(xf.to_screen(c[0] as f32, c[1] as f32), xf.to_screen(c[2] as f32, c[3] as f32));
            crop_overlay(painter, r, shield, &crate::crop_overlay::current_guides(app, r.width(), r.height()));
        } else {
            let q = crate::crop_ui::corners(c, deg).map(|p| xf.to_screen(p[0] as f32, p[1] as f32));
            crop_overlay_turned(painter, q, shield, &crate::crop_overlay::current_guides(app, q[0].distance(q[1]), q[0].distance(q[3])));
        }
        // The frame's angle beside the pointer while it turns (#1792).
        if let (Some(a), Some(h)) = (crate::crop_ui::turning(app), hover) {
            draw_value_readout(app, painter.ctx(), "crop-angle-readout", h, ["Angle:"], [format!("{a:.1}°")]);
        } else if let (Some([w, h]), Some(at)) = (crate::crop_ui::sizing(app), hover) {
            // Its W × H while it is drawn or resized (#1919), like the marquee's.
            draw_marquee_readout(app, painter.ctx(), at, size_readout(app, [0.0, 0.0, w, h]));
        }
    }
}

/// View › Show › Layer Edges: the active layer's content bounds (cached per tile) where the canvas
/// shows them. During a Move drag that is at the offset the image was drawn with this frame, so
/// the outline moves in lockstep with the layer's pixels (#2413).
pub(crate) fn layer_edges_rect(app: &PhotocraftApp, idx: usize) -> Option<photocraft_geom::Rect> {
    if !app.ui.view.shows(app.ui.view.show.layer_edges) {
        return None;
    }
    let st = app.session.documents().get(idx)?;
    let id = st.active_layer?;
    let b = st.doc.layer(id)?.surface().map(photocraft_compose::bounds::content_bounds)?;
    if b.is_empty() {
        return None;
    }
    let (dx, dy) = crate::move_ui::shown_offset(app, st.doc.id, st.revision, id).unwrap_or((0, 0));
    Some(b.translate(dx, dy))
}

/// Move tool › Show Transform Controls: the active layer's transform bounds.
fn transform_controls_rect(app: &PhotocraftApp, xf: &ViewXform) -> Option<Rect> {
    if app.ui.tool != Tool::Move || !app.ui.tool_options.move_show_transform || app.ui.transform.is_some() || app.drag.is_some() {
        return None;
    }
    let st = app.session.active()?;
    let l = st.active_layer.and_then(|id| st.doc.layer(id))?;
    if crate::doc_props_ui::is_background(&st.doc, l) {
        return None;
    }
    let b = photocraft_engine::transform_cmds::transform_bounds(&st.doc, l);
    let (dx, dy) = crate::move_ui::floating_offset(app).unwrap_or((0, 0));
    let b = b.translate(dx, dy);
    if b.is_empty() {
        return None;
    }
    Some(Rect::from_two_pos(xf.to_screen(b.x0 as f32, b.y0 as f32), xf.to_screen(b.x1 as f32, b.y1 as f32)))
}

/// The Move tool shows transform handles even before a Free Transform session exists.
/// Use the same hit areas for pointer-down and hover, so a control never behaves like
/// resize/rotate while showing the ordinary four-arrow Move cursor (#2871).
fn transform_controls_cursor(r: Rect, p: Pos2) -> Option<egui::CursorIcon> {
    let handles = [r.left_top(), r.center_top(), r.right_top(), r.right_center(), r.right_bottom(), r.center_bottom(), r.left_bottom(), r.left_center()];
    let grab = crate::transform_tool::HANDLE_PX as f32;
    for (i, handle) in handles.into_iter().enumerate() {
        if handle.distance(p) <= grab {
            return Some(match i {
                0 | 4 => egui::CursorIcon::ResizeNwSe,
                2 | 6 => egui::CursorIcon::ResizeNeSw,
                1 | 5 => egui::CursorIcon::ResizeVertical,
                _ => egui::CursorIcon::ResizeHorizontal,
            });
        }
    }
    // The rotation band: the pointer gives way to the curved rotate arrow (`transform_controls_turn`).
    in_rotation_band(r, p).then_some(egui::CursorIcon::None)
}

fn in_rotation_band(r: Rect, p: Pos2) -> bool {
    r.expand(18.0).contains(p) && !r.expand(5.0).contains(p)
}

/// In the rotation band just outside the Move tool's transform controls, the screen direction the
/// curved rotate arrow bends round: into the box along the nearest corner's bisector, as Free
/// Transform draws it (#4658). Before, the band showed a plain alias pointer, so nothing said a
/// drag there rotates.
fn transform_controls_turn(r: Rect, p: Pos2) -> Option<Vec2> {
    if !in_rotation_band(r, p)
        || [r.left_top(), r.center_top(), r.right_top(), r.right_center(), r.right_bottom(), r.center_bottom(), r.left_bottom(), r.left_center()]
            .into_iter()
            .any(|h| h.distance(p) <= crate::transform_tool::HANDLE_PX as f32)
    {
        return None;
    }
    let corner = pos2(if p.x < r.center().x { r.left() } else { r.right() }, if p.y < r.center().y { r.top() } else { r.bottom() });
    Some(vec2((r.center().x - corner.x).signum(), (r.center().y - corner.y).signum()).normalized() * 16.0)
}

/// A visible handle starts scaling; the narrow band just outside the box starts rotation.
/// The interior stays the normal Move-tool drag target.
fn transform_controls_hit(r: Rect, p: Pos2) -> bool {
    transform_controls_cursor(r, p).is_some()
}

/// Enter the existing Free Transform session when a Move-tool transform control is pressed.
fn begin_transform_controls_at(app: &mut PhotocraftApp, ctx: &egui::Context, xf: &ViewXform, p: Pos2) -> bool {
    let Some(r) = transform_controls_rect(app, xf) else { return false };
    if !transform_controls_hit(r, p) {
        return false;
    }
    match crate::transform_tool::begin(app, ctx) {
        Ok(()) => true,
        Err(e) => {
            app.ui.status = e;
            false
        }
    }
}

/// Move tool › Show Transform Controls: the active layer's bounding box with its eight handles.
fn draw_transform_controls(app: &mut PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    let Some(r) = transform_controls_rect(app, xf) else { return };
    let accent = crate::theme::Tokens::get(painter.ctx()).accent;
    painter.rect_stroke(r, 0.0, Stroke::new(1.0, accent), egui::StrokeKind::Middle);
    for p in [r.left_top(), r.center_top(), r.right_top(), r.right_center(), r.right_bottom(), r.center_bottom(), r.left_bottom(), r.left_center()] {
        let h = Rect::from_center_size(p, vec2(7.0, 7.0));
        painter.rect_filled(h, 0.0, Color32::WHITE);
        painter.rect_stroke(h, 0.0, Stroke::new(1.0, accent), egui::StrokeKind::Inside);
    }
}

/// The line from the brush to the pointer while stroke smoothing lags (Pulled String Mode and
/// Stroke Catch-up, #3479): a dark underlay and a light line stay visible on any pixels, and a dot
/// marks the brush end.
fn draw_smoothing_string(painter: &egui::Painter, xf: &ViewXform, (brush, pointer): ([f64; 2], [f64; 2])) {
    let (a, b) = (xf.to_screen(brush[0] as f32, brush[1] as f32), xf.to_screen(pointer[0] as f32, pointer[1] as f32));
    if a.distance(b) < 2.0 {
        return;
    }
    painter.line_segment([a, b], Stroke::new(3.0, Color32::from_black_alpha(110)));
    painter.line_segment([a, b], Stroke::new(1.0, Color32::WHITE));
    painter.circle_filled(a, 2.5, Color32::WHITE);
}

fn draw_drag_preview(app: &mut PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    draw_tool_state(app, painter, xf, painter.ctx().input(|i| i.pointer.hover_pos()));
    // Remove strokes waiting for ↩ stay tinted between drags.
    crate::retouch_ui::draw_remove_pending(app, painter, xf);
    let Some(d) = &app.drag else {
        app.trail = None;
        return;
    };
    // Dragging the selection: its own ants follow the pointer (`selection_shown_offset`).
    if d.sel_move.is_some() {
        return;
    }
    let last = d.points.last().map(|p| [p[0], p[1]]).unwrap_or(d.start);
    let marquee = marquee_preview_px(&app.ui.tool_options, d, app.point_zoom());
    if let Some(r) = marquee {
        draw_marquee_readout(app, painter.ctx(), xf.to_screen(last[0] as f32, last[1] as f32), size_readout(app, r));
    }
    match d.tool {
        // The canvas shows the live stroke itself (`LiveStroke`); a stroke that couldn't start one
        // (Sample All Layers, say) keeps the trail.
        t if strokes_live(t) && app.live_stroke.is_some() => {
            // Smoothing holds the brush back from the pointer: draw the string between them.
            if let Some(EngineStroke::Brush(l)) = app.live_stroke.as_ref().map(|l| &l.stroke)
                && let Some(s) = l.smoothing_string()
            {
                draw_smoothing_string(painter, xf, s);
            }
        }
        t if t.is_brushlike() || t == Tool::QuickSelection => {
            // Retouching strokes preview as a translucent trail of the brush footprint: a mask,
            // not a brush-wide egui polyline (which zoomed in tessellates into wedges, #189).
            let col = match t {
                Tool::QuickSelection => Color32::from_white_alpha(40),
                // The Remove Tool tints what it will remove, like Photoshop's.
                Tool::Remove => crate::theme::Tokens::get(painter.ctx()).danger.gamma_multiply(0.45),
                _ => Color32::from_white_alpha(60),
            };
            let Some(st) = app.session.active() else { return };
            let size = [st.doc.size.width, st.doc.size.height];
            let doc_rect = xf.doc_rect(st.doc.bounds());
            let trail = app.trail.get_or_insert_with(|| crate::stroke_trail::Trail::new(size));
            if t == Tool::SpotHealing {
                // What the heal covers, dab by dab: soft edges, pressure and shape dynamics (#2364).
                trail.feed_dabs(&d.points, || photocraft_engine::retouch_cmds::stroke_brush(&app.session.tools.brush, &json!({})));
            } else {
                trail.feed(&d.points, app.session.tools.brush.size);
            }
            trail.draw(painter, doc_rect, xf.flip, col);
        }
        t if crate::vector_ui::is_shape_tool(t) => crate::vector_ui::draw_shape_preview(app, painter, xf, t, d.start, last, d.live),
        Tool::SingleRowMarquee | Tool::SingleColumnMarquee => {
            // Mapped corner by corner like the other marquees, so a rotated view shows the line
            // where the release commits it (#2512).
            if let Some([x0, y0, x1, y1]) = single_line_drag_px(app, d) {
                crate::tool_feedback::draw_ants(painter, &marquee_ants(xf, d.tool, [x0, y0], [x1, y1]), true);
            }
        }
        // Nothing selected yet: no outline, as the release would deselect.
        Tool::RectMarquee | Tool::EllipseMarquee if marquee.is_none() => {}
        Tool::RectMarquee | Tool::EllipseMarquee | Tool::ObjectSelection => {
            // Marching ants, visible on any pixels (#172): built in document space and mapped
            // corner by corner, so a rotated view shows them where the release commits (#2512).
            let pts = marquee_drag_ants(d, xf, marquee);
            crate::tool_feedback::draw_ants(painter, &pts, true);
        }
        // Patch / Content-Aware Move dragging the selection: its outline follows the pointer.
        Tool::Patch | Tool::ContentAwareMove if crate::retouch_ui::patch_drags_selection(app, d.start, d.modifiers) => {
            let start = d.start;
            let [dx, dy] = crate::retouch_ui::patch_offset(app, start, last);
            if let Some((_, _, segs)) = &app.outline_cache {
                let moved: Vec<crate::outline::Segment> = segs.iter().map(|(a, b)| ([a[0] + dx, a[1] + dy], [b[0] + dx, b[1] + dy])).collect();
                marching_ants_segments(painter, xf, &moved, painter.ctx().input(|i| i.time));
            }
        }
        Tool::FreeformPen => {
            let mut pts: Vec<Pos2> = d.points.iter().map(|p| xf.to_screen(p[0] as f32, p[1] as f32)).collect();
            // Between ⌥-clicks: the straight segment the next click makes.
            if let Some(lasso) = &d.lasso {
                pts.push(xf.to_screen(lasso.cursor[0] as f32, lasso.cursor[1] as f32));
            }
            painter.add(egui::Shape::line(pts, Stroke::new(1.5, crate::theme::Tokens::get(painter.ctx()).accent)));
        }
        Tool::Lasso | Tool::Patch | Tool::ContentAwareMove => {
            let mut pts: Vec<Pos2> = d.points.iter().map(|p| xf.to_screen(p[0] as f32, p[1] as f32)).collect();
            if let Some(lasso) = &d.lasso {
                pts.push(xf.to_screen(lasso.cursor[0] as f32, lasso.cursor[1] as f32));
            }
            // On pixel centres like the marquee's ants, so a 1 pt line stays crisp wherever the
            // canvas sits (a pixel edge would blur it to grey).
            let pts: Vec<Pos2> = pts.into_iter().map(|p| p.round() + vec2(0.5, 0.5)).collect();
            crate::tool_feedback::draw_ants(painter, &pts, false);
        }
        Tool::Gradient => {
            let a = xf.to_screen(d.start[0] as f32, d.start[1] as f32);
            let b = xf.to_screen(last[0] as f32, last[1] as f32);
            painter.line_segment([a, b], Stroke::new(3.0, Color32::from_black_alpha(140)));
            painter.line_segment([a, b], Stroke::new(1.0, Color32::WHITE));
            painter.circle_filled(a, 3.0, Color32::WHITE);
            painter.circle_filled(b, 3.0, Color32::WHITE);
        }
        Tool::Type | Tool::VerticalType | Tool::HorizontalTypeMask | Tool::VerticalTypeMask => {
            let r = Rect::from_two_pos(xf.to_screen(d.start[0] as f32, d.start[1] as f32), xf.to_screen(last[0] as f32, last[1] as f32));
            let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
            painter.add(egui::Shape::line(pts.to_vec(), Stroke::new(1.0, Color32::WHITE)));
            painter.add(egui::Shape::dashed_line(&pts, Stroke::new(1.0, Color32::BLACK), 3.0, 3.0));
        }
        // The layers themselves follow the pointer (`move_ui`); the arrow only when they can't.
        Tool::Move if !crate::move_ui::showing(app) => {
            let from = xf.to_screen(d.start[0] as f32, d.start[1] as f32);
            let off = xf.to_screen(last[0] as f32, last[1] as f32) - from;
            painter.arrow(from, off, Stroke::new(2.0, crate::theme::Tokens::get(painter.ctx()).accent));
        }
        _ => {}
    }
}

/// Tools on which holding ⌥ (Alt) switches to the Eyedropper: a click or drag sets the
/// foreground colour, as with the Eyedropper itself (#417).
pub(crate) fn alt_samples(tool: Tool, mods: egui::Modifiers) -> bool {
    // Control+Alt is the brush-resize drag (`brush_resize`), not sampling.
    mods.alt && !mods.ctrl && matches!(tool, Tool::Brush | Tool::Pencil | Tool::Gradient | Tool::PaintBucket)
}

/// Preferences › Cursors › Other Cursors asks for the precise crosshair, or Caps Lock swaps the
/// standard cursor for it (and back), as in Photoshop (#4639).
fn precise_other_cursor(app: &PhotocraftApp) -> bool {
    (app.session.prefs().cursors.other == photocraft_engine::prefs::OtherCursor::Precise) != app.caps_lock
}

/// A pen tool's standard cursor: its toolbar icon with the nib (the Convert Point tool: the
/// anchor) on `p`. Draws it at `p` and returns the OS cursor to set (hidden).
pub(crate) fn pen_cursor(ctx: &egui::Context, tool: Tool, p: Pos2) -> egui::CursorIcon {
    // Where each icon's point is in its 24-unit box: the full-size pen's nib at (2.3, 2.3); the
    // Freeform and Curvature pens draw it at 0.68 × from (7, 7); Convert Point's apex anchor.
    let hot = match tool {
        Tool::FreeformPen | Tool::CurvaturePen => vec2(8.56, 8.56),
        Tool::ConvertPoint => vec2(12.0, 7.0),
        _ => vec2(2.3, 2.3),
    };
    crate::icons::cursor(ctx, crate::icons::tool_icon(tool), p, hot / 24.0, 22.0);
    egui::CursorIcon::None
}

/// The sampling cursor: a pipette whose tip is the sampled pixel. Draws it at `p` and returns the
/// OS cursor to set (hidden).
pub(crate) fn pipette_cursor(ctx: &egui::Context, p: Pos2) -> egui::CursorIcon {
    // The tip of the icon's pipette is at (2, 22) of its 24-unit box.
    crate::icons::cursor(ctx, "pipette", p, vec2(2.0, 22.0) / 24.0, 20.0);
    egui::CursorIcon::None
}

/// The Paint Bucket's cursor: a bucket whose pouring corner is the filled pixel. Draws it at `p`
/// and returns the OS cursor to set (hidden).
pub(crate) fn paint_bucket_cursor(ctx: &egui::Context, p: Pos2) -> egui::CursorIcon {
    // The left corner of the icon's bucket is at (2, 12) of its 24-unit box.
    crate::icons::cursor(ctx, "paint-bucket", p, vec2(2.0, 12.0) / 24.0, 22.0);
    egui::CursorIcon::None
}

/// Decided when the press starts, so ⌥ pressed or released mid-stroke never switches between
/// painting and sampling.
fn alt_eyedropper(app: &mut PhotocraftApp, ev: ToolEvent, mods: egui::Modifiers) -> bool {
    if matches!(ev, ToolEvent::Down { .. }) {
        app.alt_sampling = alt_samples(app.active_tool(), mods);
    }
    if !app.alt_sampling {
        return false;
    }
    match ev {
        // Without ⌥: the sample sets the foreground colour, whatever the Eyedropper would do.
        ToolEvent::Down { x, y, .. } | ToolEvent::Move { x, y, .. } => sample_eyedropper(app, x, y, egui::Modifiers::NONE),
        ToolEvent::Up { .. } => app.alt_sampling = false,
    }
    true
}

fn sample_eyedropper(app: &mut PhotocraftApp, x: f64, y: f64, mods: egui::Modifiers) {
    if let Some([r, g, b]) = eyedropper_color(app, x, y) {
        let key = if mods.alt { "background" } else { "foreground" };
        let _ = app.run("tools.setColors", json!({ key: [r, g, b, 1.0] }));
    }
}

/// The colour at document point (x, y) with the Eyedropper's Sample Size (the composite pixel,
/// or the average of the square around it) from the layers `sample_layer` names (`document.sampleColor`).
/// `None` off the image or over transparency.
fn sample_color(app: &mut PhotocraftApp, x: f64, y: f64, sample_layer: &str) -> Option<[f32; 3]> {
    if !(x.is_finite() && y.is_finite()) {
        return None;
    }
    let size = app.ui.tool_options.eyedropper_size;
    let v = app.run("document.sampleColor", json!({"x": x, "y": y, "size": size, "sampleLayer": sample_layer})).ok()?;
    match serde_json::from_value::<Vec<f32>>(v).ok()?[..] {
        [r, g, b, a] if a > 0.0 => Some([r, g, b]),
        _ => None,
    }
}

/// What the Eyedropper tool (and a painting tool's ⌥-click) picks at document point (x, y): its
/// options bar's Sample Size and Sample (#1649).
pub(crate) fn eyedropper_color(app: &mut PhotocraftApp, x: f64, y: f64) -> Option<[f32; 3]> {
    let layers = app.ui.tool_options.eyedropper_sample.clone();
    sample_color(app, x, y, &layers)
}

/// The active document's composite colour at document point (x, y), averaged over the
/// Eyedropper's Sample Size as Photoshop's dialog eyedroppers (Curves, Color Picker) do.
/// `None` off the image or over transparency.
pub(crate) fn composite_color(app: &mut PhotocraftApp, x: f64, y: f64) -> Option<[f32; 3]> {
    sample_color(app, x, y, "all")
}

/// The body of a `Move` for every tool: tracked position, ⇧ constraint, the moving layer, and so
/// on. It does not feed the live stroke, so the canvas can push a whole frame's recovered samples
/// and update the live stroke once (see `canvas_view`).
fn tool_move(app: &mut PhotocraftApp, x: f64, y: f64, pressure: f32, mods: egui::Modifiers) {
    let tool = app.active_tool();
    if tool.is_type() && app.drag.is_none() {
        crate::type_tool::pointer_move(app, x, y);
    }
    if tool == Tool::Pen && mods.alt {
        crate::vector_ui::pen_break_handle(app);
    }
    if tool == Tool::Pen {
        crate::vector_ui::pen_move(app, x, y);
    } else if tool == Tool::CurvaturePen {
        crate::vector_ui::curvature_move(app, x, y);
    }
    let zoom = app.point_zoom();
    if let Some(d) = app.drag.as_mut().filter(|d| d.reposition) {
        d.track(mods);
        d.shift_to([x, y]);
    } else if let Some(d) = &mut app.drag {
        d.track(mods);
        // ⇧: straight 0/45/90° strokes, 45° gradient angles (stroke_constraint.rs).
        let last = d.points.last().map_or(d.start, |p| [p[0], p[1]]);
        let [x, y] = crate::stroke_constraint::constrain(d.tool, &mut d.constrain, d.start, last, [x, y], mods.shift, zoom);
        let moved = d.points.last().is_none_or(|p| (p[0] - x).abs() + (p[1] - y).abs() > 0.25);
        // A held pen repeats its point now and then, so the stroke's time keeps advancing.
        let held = app.stylus.last_point_ms().is_some_and(|t| app.stylus.clock_ms - t >= HOLD_REPEAT_MS)
            && app.live_stroke.as_ref().is_some_and(|l| l.stroke.wants_time());
        if moved || held {
            d.points.push([x, y, pressure as f64]);
            app.stylus.record_point();
        }
    }
}

/// Tool state machine. Shared by mouse input and automation.
pub fn tool_event(app: &mut PhotocraftApp, ev: ToolEvent, mods: egui::Modifiers) {
    if crate::symmetry_ui::pointer(app, ev, crate::workspace_ui::sticky_mods(app, mods)) {
        return;
    }
    // An Alt+right-drag armed for this press (`paint_mouse`): taken before anything else can
    // consume the event, so it never outlives the press it was armed for (#297).
    let armed = std::mem::take(&mut app.brush_resize_armed);
    crate::transform_tool::end_if_left(app);
    let mods = crate::workspace_ui::sticky_mods(app, mods);
    if app.ui.transform.is_none() && crate::type_transform::pointer(app, ev, mods) {
        return;
    }
    // Content-Aware Move's Transform On Drop box takes every press until ↩ or Esc.
    if crate::cam_transform::pointer(app, ev, mods) {
        return;
    }
    // View › Snap / Snap To and smart guides (snap_ui.rs).
    let raw = ev;
    // The Move tool's marquee selecting layers takes its moves and release unsnapped (move_box.rs).
    if crate::move_box::pointer(app, raw) {
        return;
    }
    // A press anywhere but on the floating piece (or with ⇧ / ⌥, to draw) drops it first; the
    // Move tool (⌘ with a painting tool too, `event_tool`) drags it from anywhere.
    if let ToolEvent::Down { x, y, .. } = raw
        && app.session.active().is_some_and(|st| photocraft_engine::float_cmds::floating(st).is_some())
        && !crate::move_ui::moves_selected_pixels_with(app, event_tool(app, mods))
        && selection_drag_kind(app, app.ui.tool, [x, y], mods) != Some(true)
    {
        let _ = app.run("select.drop", json!({}));
    }
    let ev = crate::snap_ui::filter_event(app, ev, mods);
    // Free Transform picks the handle under the press where the user clicked: the snapped press
    // can land outside the corner's grab area and turn a corner drag into a move of the whole box.
    let transform_ev = if matches!(raw, ToolEvent::Down { .. }) { raw } else { ev };
    if crate::transform_tool::pointer(app, transform_ev, mods) {
        return;
    }
    if crate::distort_ui::pointer(app, ev, mods) {
        return;
    }
    if crate::perspective_crop_ui::pointer(app, ev) {
        return;
    }
    // Window › Modifier Keys: sticky Shift/⌘/⌥ act as held keys.
    let mods = crate::workspace_ui::sticky_mods(app, mods);
    // ⌘⌥⌃-click with any tool selects the topmost layer with pixels there (quick_pick.rs); it
    // must run before the ⌃⌥ brush resize below, which the same keys would trigger.
    if crate::quick_pick::pointer(app, ev, mods) {
        return;
    }
    // Control+Alt-drag or Alt+right-drag with a painting tool resizes the brush instead of
    // painting (#231, #297).
    if crate::brush_resize::pointer(app, ev, mods, armed) {
        return;
    }
    // ⌥ (Alt) with a painting tool is the Eyedropper for that press.
    if alt_eyedropper(app, ev, mods) {
        return;
    }
    // Move tool: ⇧ locks the axis, ⌥ duplicates (move_mods.rs).
    let ev = crate::move_mods::filter_event(app, ev, mods);
    // Direct Selection, and the Pen's ⌘ (Direct Selection) and ⌥ (Convert Point) modes (#790).
    if crate::direct_select::pointer(app, ev, mods) {
        return;
    }
    // Add / Delete Anchor Point tools (#1044).
    if crate::anchor_tools::pointer(app, ev, mods) {
        return;
    }
    // Ruler, Count and Note tools.
    if crate::analysis_ui::pointer(app, ev, mods) {
        return;
    }
    // Slice and Slice Select tools.
    if crate::slice_ui::pointer(app, ev, mods) {
        return;
    }
    // Frame tool: draw rectangular and elliptical frames.
    if crate::frame_tool::pointer(app, ev, mods) {
        return;
    }
    // Artboard tool: draw, select, move and resize boards.
    if crate::artboard_tool::pointer(app, ev, mods) {
        return;
    }
    // Crop tool: draw, move and resize the frame.
    if crate::crop_ui::pointer(app, ev, mods) {
        return;
    }
    // Gradient tool, live mode: draw and edit Gradient Fill layers.
    if crate::gradient_ui::pointer(app, ev, mods) {
        return;
    }
    // ⌘ with a selection tool over a guide takes the guide (#2690); the guide drag below moves it.
    if let ToolEvent::Down { x, y, .. } = raw
        && let Some((vertical, i)) = command_guide_at(app, app.active_tool(), [x, y], mods)
    {
        app.guide_drag = Some(crate::rulers::GuideDrag { vertical, index: Some(i), pos: if vertical { x } else { y } });
        crate::snap_ui::begin_guide(app, [x, y]);
        return;
    }
    if crate::lasso_ui::pointer(app, ev, mods) {
        return;
    }
    // Magnetic Lasso: fastening points and the border following the pointer.
    if crate::magnetic_lasso_ui::pointer(app, ev, mods) {
        return;
    }
    let tool = event_tool(app, mods);
    if tool == Tool::Eyedropper {
        match ev {
            ToolEvent::Down { x, y, .. } | ToolEvent::Move { x, y, .. } => {
                sample_eyedropper(app, x, y, mods);
                return;
            }
            ToolEvent::Up { .. } => return,
        }
    }
    // A locked layer (the Background without a selection, say): no drag, and Photoshop's message
    // once the pointer moves (a click says nothing). ⌘ with a selection tool here (#896); the Move
    // tool below, once Auto-Select has picked the layer.
    match ev {
        ToolEvent::Down { x, y, .. } if tool != Tool::Move && app.ui.transform.is_none() && crate::move_lock::blocked(app, tool, [x, y], mods) => {
            app.move_blocked = true;
            return;
        }
        ToolEvent::Move { .. } if app.move_blocked => {
            app.move_blocked = false;
            crate::move_lock::prompt(app);
            return;
        }
        ToolEvent::Up { .. } if app.move_blocked => {
            app.move_blocked = false;
            return;
        }
        _ => {}
    }
    // Move tool over a guide drags the guide (off the canvas deletes it).
    match ev {
        ToolEvent::Down { x, y, pressure } if tool == Tool::Move => {
            if let Some((vertical, i)) = crate::rulers::guide_at(app, x, y) {
                app.guide_drag = Some(crate::rulers::GuideDrag { vertical, index: Some(i), pos: if vertical { x } else { y } });
                return;
            }
            // Auto-Select (or ⌘-click while it is off) picks the layer under the pointer first
            // (not when the selected pixels move: those are the active layer's).
            if !crate::move_ui::moves_selected_pixels_with(app, tool) && app.ui.tool_options.move_auto_select != mods.command {
                let target = app.ui.tool_options.move_target.clone();
                let mode = if mods.shift { "add" } else { "replace" };
                let before = app.session.active().map(|st| st.selected_layers());
                // A locked layer under the pointer (the Background) can't move. Photoshop draws
                // the layer marquee instead of reporting the lock (#4058). Probe without
                // selecting so a locked hit doesn't become the selection first.
                let probe = app.run("layer.pickAt", json!({"x": x, "y": y, "target": target, "select": false}));
                let locked_hit = probe.as_ref().ok().and_then(|v| v["layer"].as_u64()).is_some_and(|id| {
                    app.session.active().is_some_and(|st| {
                        let id = photocraft_doc::LayerId(id);
                        let locks = st.doc.effective_locks(id);
                        locks.position || locks.all || st.doc.is_background(id)
                    })
                });
                if locked_hit || (probe.as_ref().is_ok_and(|v| v["layer"].is_null()) && crate::move_box::empty_at(app, [x, y])) {
                    if locked_hit {
                        // A click still selects the locked layer, as in Photoshop; a drag's
                        // marquee then replaces that selection on release.
                        let _ = app.run("layer.pickAt", json!({"x": x, "y": y, "target": target, "mode": mode}));
                    }
                    crate::move_box::begin(app, [x, y], mods.shift);
                    return;
                }
                let picked = app.run("layer.pickAt", json!({"x": x, "y": y, "target": target, "mode": mode}));
                // No pixels there at all (an empty spot, or off the canvas): drag a marquee that
                // selects the layers it touches instead (move_box.rs).
                if picked.is_ok_and(|v| v["layer"].is_null()) && crate::move_box::empty_at(app, [x, y]) {
                    crate::move_box::begin(app, [x, y], mods.shift);
                    return;
                }
                // Snapping (and the drag's box) started before the pick: point it at what moves.
                if app.session.active().map(|st| st.selected_layers()) != before {
                    crate::snap_ui::retarget_move(app, [x, y], mods);
                }
            }
            // A locked layer: no drag, and Photoshop's message once the pointer moves (`move_lock`).
            if crate::move_lock::blocked(app, tool, [x, y], mods) {
                app.move_blocked = true;
                return;
            }
            // With a selection: cut the selected pixels (⌥ copies them) and drag them as a floating
            // piece, from anywhere, as a marquee ⌘-drag does.
            if crate::move_ui::moves_selected_pixels_with(app, tool) {
                if crate::move_ui::float_selected(app, mods.alt, 0.0, 0.0) {
                    let mut d = Drag::new(tool, [x, y], vec![[x, y, pressure as f64]], mods, false);
                    d.sel_move = Some(true);
                    app.drag = Some(d);
                }
                return;
            }
        }
        ToolEvent::Move { x, y, .. } => {
            if let Some(d) = app.guide_drag.as_mut().filter(|d| d.index.is_some()) {
                d.pos = if d.vertical { x } else { y };
                return;
            }
        }
        ToolEvent::Up { x, y } => {
            if let Some(mut d) = app.guide_drag.filter(|d| d.index.is_some()) {
                app.guide_drag = None;
                d.pos = if d.vertical { x } else { y };
                crate::rulers::finish_drag(app, d);
                return;
            }
        }
        _ => {}
    }
    match ev {
        ToolEvent::Down { x, y, pressure } => {
            // Painting a type, shape, Smart Object or fill layer asks to rasterize it first
            // (⌥-click with the Clone Stamp or Healing Brush only sets the source).
            let sets_source = matches!(tool, Tool::CloneStamp | Tool::Healing) && mods.alt;
            // Painting on or moving a hidden layer is refused at the press, as in Photoshop (#571).
            if !sets_source && let Some(why) = hidden_target(app, tool) {
                app.ui.status = why.into();
                app.ui.status_error = true;
                return;
            }
            if !sets_source && crate::rasterize_prompt::intercept(app, tool, x, y, pressure) {
                return;
            }
            // ⌘ with a selection tool is the Move tool for the drag: outside the selection (or
            // without one) it moves the whole layer, ⌘⌥ a duplicate of it. It never combines
            // selections.
            if command_moves_layer(app, tool, [x, y], mods) {
                if mods.alt
                    && let Err(e) = app.run("layer.duplicate", json!({"inPlace": true}))
                {
                    app.ui.status = e;
                    app.ui.status_error = true;
                    return;
                }
                app.drag = Some(Drag::new(Tool::Move, [x, y], vec![[x, y, pressure as f64]], mods, false));
                return;
            }
            // Marquee / lasso inside the selection: drag the outline, or ⌘-drag to cut the selected
            // pixels into a floating piece (`select.float`) and drag that.
            if let Some(cut) = selection_drag_kind(app, tool, [x, y], mods) {
                // ⌘⌥ on a floating piece leaves it where it is and drags a copy of it.
                if cut
                    && mods.alt
                    && mods.command
                    && app.session.active().is_some_and(|st| photocraft_engine::float_cmds::floating(st).is_some())
                    && let Err(e) = app.run("select.drop", json!({}))
                {
                    app.ui.status = e;
                    app.ui.status_error = true;
                    return;
                }
                if cut
                    && app.session.active().is_some_and(|st| photocraft_engine::float_cmds::floating(st).is_none())
                    && let Err(e) = app.run("select.float", json!({"dx": 0, "dy": 0, "copy": mods.alt}))
                {
                    app.ui.status = e;
                    app.ui.status_error = true;
                    return;
                }
                let mut d = Drag::new(tool, [x, y], vec![[x, y, pressure as f64]], mods, false);
                d.sel_move = Some(cut);
                app.drag = Some(d);
                return;
            }
            match tool {
                Tool::PathSelection => {
                    let tol = 6.0 / f64::from(app.point_zoom().max(0.01));
                    if let Some(st) = app.session.active()
                        && let Some(id) = crate::vector_ui::pick_shape_at(&st.doc, [x, y], tol)
                    {
                        let _ = app.run("layer.select", json!({"layer": id.0}));
                    }
                }
                Tool::Pen => {
                    if !(mods.alt && crate::vector_ui::pen_grab_handle(app, x, y)) {
                        crate::vector_ui::pen_down(app, x, y);
                    }
                    return;
                }
                Tool::CurvaturePen => {
                    crate::vector_ui::curvature_down(app, x, y);
                    return;
                }
                Tool::CloneStamp | Tool::Healing if mods.alt => {
                    crate::retouch_ui::set_source(app, x, y);
                    return;
                }
                Tool::MagicWand => {
                    let o = app.ui.tool_options.clone();
                    let mode = selection_mode(app, mods);
                    let _ = app.run("select.magicWand", json!({"x": x.floor(), "y": y.floor(), "tolerance": o.tolerance, "contiguous": o.contiguous, "antiAlias": o.anti_alias, "sampleAllLayers": o.sample_all_layers, "mode": mode}));
                    return;
                }
                Tool::MagicEraser => {
                    crate::eraser_ui::click(app, tool, x, y);
                    return;
                }
                Tool::PaintBucket => {
                    let p = crate::bucket_ui::click_params(app, x, y);
                    let _ = app.run("paint.bucket", p);
                    return;
                }
                Tool::RedEye => {
                    let o = app.ui.tool_options.clone();
                    let _ = app.run(
                        "paint.redEye",
                        json!({
                            "x": x.floor(),
                            "y": y.floor(),
                            "pupilSize": o.red_eye_pupil_size.clamp(1.0, 100.0),
                            "darken": o.red_eye_darken.clamp(0.0, 100.0),
                            "target": paint_target(app),
                        }),
                    );
                    return;
                }
                Tool::PolygonLasso => {
                    polygon_click(app, x, y, mods);
                    return;
                }
                Tool::Type | Tool::VerticalType | Tool::HorizontalTypeMask | Tool::VerticalTypeMask if crate::type_tool::pointer_down(app, x, y, mods) => {
                    return;
                }
                _ => {}
            }
            crate::paint_mouse::sync_tool_brush(app);
            // Clear mode (`~` held, or chosen in the options bar) erases with the tool's own brush.
            let clear = crate::paint_mouse::clears(app, tool);
            let erase = std::mem::take(&mut app.secondary_erase) || tool == Tool::Eraser || clear;
            // A 1 px pencil line depends on the pixels, not where inside the pixel the click
            // landed (#4071). Snap both ends of the stroke to the aliased tip's grid.
            let (x, y) = snap_aliased_tip(app, tool, x, y);
            // ⇧-click connects to the last endpoint, including its pen pressure (#2827), so
            // the renderer interpolates size and opacity across the segment.
            let active = app.session.active().map(|st| st.doc.id);
            let from = app.last_stroke_end.filter(|(doc, _)| mods.shift && crate::stroke_constraint::connects(tool) && Some(*doc) == active).map(|(_, p)| {
                let (sx, sy) = snap_aliased_tip(app, tool, p[0], p[1]);
                [sx, sy, p[2]]
            });
            let mut points = vec![[x, y, pressure as f64]];
            if let Some(p) = from {
                points.insert(0, p);
            }
            // ⌥ flips Dodge/Burn and Blur/Sharpen for this stroke.
            let tool = crate::retouch_ui::alt_flipped(tool, mods.alt);
            app.drag = Some(Drag { clear, ..Drag::new(tool, from.map_or([x, y], |p| [p[0], p[1]]), points, mods, erase) });
            app.trail = None;
            app.stylus.begin_stroke();
            if from.is_some() {
                app.stylus.record_point();
            }
            app.live_stroke = if strokes_live(tool) { begin_live_stroke(app) } else { None };
        }
        ToolEvent::Move { x, y, pressure } => {
            // Polygonal Lasso: ⌥ held while the button is down draws freehand into the polygon;
            // releasing ⌥ goes back to straight segments (the polygon stays open).
            if tool == Tool::PolygonLasso && mods.alt && app.drag.is_none() && !app.ui.polygon.is_empty() {
                if app.ui.polygon.last().is_none_or(|l| (l[0] - x).hypot(l[1] - y) >= 1.0) {
                    app.ui.polygon.push([x, y]);
                }
                return;
            }
            tool_move(app, x, y, pressure, mods);
            feed_live_stroke(app);
        }
        ToolEvent::Up { x, y } => {
            if tool.is_type()
                && let Some(e) = app.ui.text_edit.as_mut()
            {
                e.dragging = false;
                e.resize = None;
            }
            if tool == Tool::Pen {
                crate::vector_ui::pen_up(app);
            } else if tool == Tool::CurvaturePen {
                crate::vector_ui::curvature_up(app);
            }
            let zoom = app.point_zoom();
            let Some(mut d) = app.drag.take() else { return };
            d.track(mods);
            if d.reposition {
                d.shift_to([x, y]);
            }
            let last = d.points.last().map_or(d.start, |p| [p[0], p[1]]);
            let [x, y] = crate::stroke_constraint::constrain(d.tool, &mut d.constrain, d.start, last, [x, y], mods.shift, zoom);
            // A Move-tool click (released where it was pressed) selects, it never moves: snapping
            // the release point would otherwise nudge the layer onto a nearby edge.
            if d.tool == Tool::Move && d.points.len() < 2 && matches!(raw, ToolEvent::Up { x, y } if [x, y] == d.start) {
                app.move_preview = None;
                crate::move_mods::finish(app);
                if d.sel_move.is_some() {
                    finish_selection_drag(app, true, d.start, d.start);
                }
                return;
            }
            if d.points.last().is_none_or(|p| p[0] != x || p[1] != y) {
                d.points.push([x, y, d.points.last().map_or(1.0, |p| p[2])]);
                app.stylus.record_point();
            }
            finish_gesture(app, d);
            crate::move_mods::finish(app);
        }
    }
}

/// Why a press with `tool` must not start: it would paint on or move a hidden target layer (the
/// engine refuses the command; refusing the press keeps the live stroke from drawing first).
fn hidden_target(app: &PhotocraftApp, tool: Tool) -> Option<&'static str> {
    let cmd = match tool {
        Tool::Move => "layer.translate",
        // The live Gradient makes a new Gradient Fill layer above the target, which Photoshop
        // allows over a hidden layer; only the classic one paints the target itself.
        Tool::Gradient if !app.ui.tool_options.gradient_classic => return None,
        // Every painting command follows the same target (layer, mask or channel) as a stroke.
        t if t.is_brushlike() || matches!(t, Tool::Gradient | Tool::PaintBucket | Tool::MagicEraser) => "paint.stroke",
        _ => return None,
    };
    photocraft_engine::hidden_target::refusal(&app.session, cmd, &json!({}))
}

/// Is document point `p` inside the active document's selection?
fn inside_selection(app: &PhotocraftApp, p: [f64; 2]) -> bool {
    let Some(sel) = app.session.active().and_then(|st| st.doc.selection.as_ref()) else { return false };
    let mut v = [0.0f32];
    sel.read_pixel(p[0].floor() as i32, p[1].floor() as i32, &mut v);
    v[0] > 0.0
}

/// Does a press with `tool` at `p` drag the selection rather than draw a new one? `Some(true)`
/// moves the floating piece (⌘ cuts one first, ⌘⌥ copies; a floating piece moves with a plain
/// drag; the Polygonal Lasso and Magic Wand need ⌘, as a plain press there places or samples),
/// `Some(false)` just the outline (no ⇧ / ⌥ and the options bar on New Selection, so a combining
/// drag still draws).
pub fn selection_drag_kind(app: &PhotocraftApp, tool: Tool, p: [f64; 2], mods: egui::Modifiers) -> Option<bool> {
    // Click-driven selection tools (a press places a point or samples): only ⌘ drags the
    // selection, and not while a polygon is being drawn.
    let clicky = matches!(tool, Tool::PolygonLasso | Tool::MagicWand);
    if !(clicky || matches!(tool, Tool::RectMarquee | Tool::EllipseMarquee | Tool::Lasso)) || (clicky && !app.ui.polygon.is_empty()) {
        return None;
    }
    // ⌘ cuts, ⌘⌥ copies (`select.float`'s `copy`).
    let cut = mods.command && !mods.shift;
    if let Some(f) = app.session.active().and_then(photocraft_engine::float_cmds::floating) {
        let on = inside_selection(app, [p[0] - f64::from(f.offset.0), p[1] - f64::from(f.offset.1)]);
        return (on && !mods.shift && (!mods.alt || mods.command) && (cut || !clicky)).then_some(true);
    }
    if !inside_selection(app, p) {
        return None;
    }
    if cut {
        return Some(true);
    }
    (!clicky && !mods.command && selection_mode(app, mods) == "replace").then_some(false)
}

/// The tool a pointer event with `mods` goes to: ⌘ held is the Move tool (`hold_keys::cmd_moves`).
/// The canvas resolves the held key before the event (`tool_override`); automation and tests send
/// the modifier with the event.
pub(crate) fn event_tool(app: &PhotocraftApp, mods: egui::Modifiers) -> Tool {
    match app.active_tool() {
        t if app.tool_override.is_none() && mods.command && crate::hold_keys::cmd_moves(t) && app.ui.transform.is_none() && app.ui.text_edit.is_none() => {
            Tool::Move
        }
        t => t,
    }
}

/// Does a ⌘ (⌘⌥) press with selection tool `tool` at `p` move the whole layer (a duplicate with
/// ⌥), as the Move tool would? Outside the selection or without one; inside it, ⌘ drags the
/// selected pixels instead (`selection_drag_kind`).
pub(crate) fn command_moves_layer(app: &PhotocraftApp, tool: Tool, p: [f64; 2], mods: egui::Modifiers) -> bool {
    let selection_tool = matches!(tool, Tool::RectMarquee | Tool::EllipseMarquee | Tool::Lasso | Tool::PolygonLasso | Tool::MagicWand);
    let floating = app.session.active().is_some_and(|st| photocraft_engine::float_cmds::floating(st).is_some());
    selection_tool && mods.command && !mods.shift && !floating && app.ui.polygon.is_empty() && !inside_selection(app, p)
}

/// The guide a ⌘ press with selection tool `tool` at `p` drags (vertical?, index), as the Move
/// tool's press would (#2690): ⌘ is the Move tool, and a guide under the pointer comes before the
/// layer or the selected pixels. Not while a polygon or lasso outline is being drawn.
pub(crate) fn command_guide_at(app: &PhotocraftApp, tool: Tool, p: [f64; 2], mods: egui::Modifiers) -> Option<(bool, usize)> {
    let selection_tool = matches!(tool, Tool::RectMarquee | Tool::EllipseMarquee | Tool::Lasso | Tool::PolygonLasso | Tool::MagicWand);
    let idle = app.ui.polygon.is_empty() && !crate::lasso_ui::active(app) && app.ui.transform.is_none();
    (selection_tool && mods.command && idle).then(|| crate::rulers::guide_at(app, p[0], p[1])).flatten()
}

/// Photoshop's cursor over a selection: what a press (or the drag under way) would do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelCursor {
    /// A marquee or lasso inside the selection moves the outline (the move cursor, #1428), also
    /// while it is being dragged.
    Outline,
    /// Arrow with scissors: the Move tool (⌘ with a marquee) cuts and moves the selected pixels.
    Cut,
    /// Arrow with a second arrow: ⌥ lifts a copy of them instead.
    Copy,
    /// Plain arrow: a floating piece, moved again by a plain drag until it is dropped.
    Piece,
    /// Hollow arrowhead: the selected pixels are being dragged.
    Dragging,
}

/// The cursor at document point `p` with `tool` in effect and `mods` held (`None`: not over a
/// selection the press would move). The Move tool moves the selected pixels from anywhere
/// (`move_ui::moves_selected_pixels_with`); a selection tool from inside the selection, or on the
/// floating piece (`selection_drag_kind`).
pub fn selection_cursor(app: &PhotocraftApp, tool: Tool, p: [f64; 2], mods: egui::Modifiers) -> Option<SelCursor> {
    if let Some(d) = &app.drag {
        return match d.sel_move {
            Some(true) => Some(SelCursor::Dragging),
            Some(false) => Some(SelCursor::Outline),
            None => None,
        };
    }
    let floating = app.session.active().and_then(photocraft_engine::float_cmds::floating).is_some();
    let pixels = if tool == Tool::Move {
        crate::move_ui::moves_selected_pixels_with(app, tool)
    } else {
        match selection_drag_kind(app, tool, p, mods)? {
            false => return Some(SelCursor::Outline),
            true => true,
        }
    };
    if !pixels {
        return None;
    }
    // ⌥ (⌘⌥ with a selection tool) floats a copy, of the layer's pixels or of the piece.
    let copy = mods.alt && (tool == Tool::Move || mods.command);
    Some(match (floating, copy) {
        (_, true) => SelCursor::Copy,
        (true, false) => SelCursor::Piece,
        (false, false) => SelCursor::Cut,
    })
}

/// Whole-pixel offset of a selection drag in progress (`Drag::sel_move`).
pub(crate) fn selection_drag_delta(app: &PhotocraftApp) -> Option<(i32, i32)> {
    let d = app.drag.as_ref().filter(|d| d.sel_move.is_some())?;
    let end = d.points.last().map_or(d.start, |p| [p[0], p[1]]);
    Some(((end[0] - d.start[0]).round().clamp(-1e7, 1e7) as i32, (end[1] - d.start[1]).round().clamp(-1e7, 1e7) as i32))
}

/// Where the selection is drawn relative to where it is: a floating piece's offset plus a drag
/// in progress.
fn selection_shown_offset(app: &PhotocraftApp) -> Option<(i32, i32)> {
    crate::move_ui::floating_offset(app).or_else(|| selection_drag_delta(app))
}

/// End of a selection drag: move the outline, or the floating piece. A click without moving
/// deselects, like a marquee click (a click on a floating piece leaves it floating, unless it never
/// moved: then it is put back, so Undo isn't spent on it).
fn finish_selection_drag(app: &mut PhotocraftApp, floating: bool, start: [f64; 2], end: [f64; 2]) {
    let (dx, dy) = ((end[0] - start[0]).round(), (end[1] - start[1]).round());
    let unmoved = app.session.active().and_then(photocraft_engine::float_cmds::floating).is_some_and(|f| f.offset == (0, 0));
    let r = match (floating, dx == 0.0 && dy == 0.0) {
        (true, true) if unmoved => app.run("select.drop", json!({})),
        (true, true) => return,
        (false, true) if app.session.is_enabled("select.deselect") => app.run("select.deselect", json!({})),
        (false, true) => return,
        (true, false) => app.run("select.float", json!({"dx": dx, "dy": dy})),
        (false, false) => app.run("select.transformSelection", json!({"dx": dx, "dy": dy})),
    };
    if let Err(e) = r {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// Commit a Brush, Pencil or Eraser drag. The live stroke already rendered it: its pixels are
/// offered to the commit (`Session::offer_live_stroke`), so release costs no second render, and
/// the canvas then refreshes only what it hadn't shown yet (`CommittedStroke`).
fn commit_brush_stroke(app: &mut PhotocraftApp, d: &Drag) {
    let live = app.live_stroke.take();
    let mut p = stroke_params(app, d, &app.stylus.stroke_points(&d.points));
    if let Some(seed) = live.as_ref().and_then(|l| l.stroke.seed()) {
        p["seed"] = json!(seed);
    }
    let Some(mut l) = live else {
        let _ = app.run(stroke_command(app, d.tool), p);
        return;
    };
    // Points the live stroke hasn't rendered yet (the release frame's last moves).
    let t0 = app.stylus.point_ms(0);
    let rest: Vec<_> = (l.fed..d.points.len())
        .filter_map(|i| {
            let q = d.points.get(i)?;
            let t = app.stylus.stroke.get(i).or(app.stylus.stroke.last()).copied().unwrap_or_default();
            let mut sp = photocraft_engine::paint::StrokePoint::new(q[0], q[1], q[2] as f32);
            (sp.tilt_x, sp.tilt_y, sp.rotation) = (t[0], t[1], t[2]);
            sp.time = app.stylus.point_ms(i) - t0;
            Some(sp)
        })
        .collect();
    if !rest.is_empty() {
        match l.stroke.push(&rest) {
            Ok(r) => l.damage.push(r),
            // The live stroke can't show these points: the commit renders the stroke itself.
            Err(_) => {
                let _ = app.run(stroke_command(app, d.tool), p);
                return;
            }
        }
    }
    let LiveStroke { stroke, doc, key, damage, .. } = l;
    if let EngineStroke::Brush(stroke) = stroke
        && app.session.offer_live_stroke(*stroke).is_err()
    {
        app.session.clear_live_stroke();
    }
    let result = app.run(stroke_command(app, d.tool), p);
    app.session.clear_live_stroke();
    let Ok(r) = result else { return };
    let revision = app.session.documents().iter().find(|st| st.doc.id == doc).map(|st| st.revision);
    if r.get("fromLive").and_then(serde_json::Value::as_bool) == Some(true)
        && let Some(revision) = revision
    {
        app.committed_stroke = Some(CommittedStroke { doc, revision, key, damage });
    } else {
        // Rendered again from scratch: the commit's damage rect refreshes the stroke.
        let shown = CommittedStroke { doc, revision: 0, key, damage };
        // Raw preview key 0 = the document itself (its colour display folded in).
        shown_as_document(app, doc, |k| shown.since(k).is_some());
    }
}

/// Pencil (and a Pencil-mode Eraser) dabs sit on the pixel grid. Clicks anywhere inside one
/// pixel must produce the same point, or a ⇧-click line changes with the sub-pixel position.
fn snap_aliased_tip(app: &PhotocraftApp, tool: Tool, x: f64, y: f64) -> (f64, f64) {
    let pencil = tool == Tool::Pencil || crate::eraser_ui::pencil_mode(app, tool);
    if !pencil {
        return (x, y);
    }
    let (cx, cy) = photocraft_engine::paint::grid_center(x, y, app.session.tools.brush.size);
    (f64::from(cx), f64::from(cy))
}

pub(crate) fn finish_gesture(app: &mut PhotocraftApp, d: Drag) {
    let mut end = d.points.last().copied().unwrap_or([d.start[0], d.start[1], 1.0]);
    let (sx, sy) = snap_aliased_tip(app, d.tool, end[0], end[1]);
    end[0] = sx;
    end[1] = sy;
    if let Some(floating) = d.sel_move {
        finish_selection_drag(app, floating, d.start, [end[0], end[1]]);
        return;
    }
    // Where the next ⇧-click line starts.
    if crate::stroke_constraint::connects(d.tool)
        && let Some(st) = app.session.active()
    {
        app.last_stroke_end = Some((st.doc.id, end));
    }
    // A live Clone Stamp preview ends here; the commit below replaces it (the Clone Stamp's and
    // History Brush's with the pixels it already drew).
    offer_live_retouch(app, &d);
    if live_retouch_command(d.tool).is_some() || crate::retouch_ui::dab_params(app, d.tool).is_some() || d.tool == Tool::ColorReplacement {
        app.live_stroke = None;
    }
    let finished = crate::eraser_ui::finish_stroke(app, d.tool, &d.points) || crate::retouch_ui::finish_stroke(app, d.tool, &d.points, d.modifiers);
    app.session.clear_live_stroke();
    if finished {
        return;
    }
    match d.tool {
        Tool::ObjectSelection => crate::retouch_ui::finish_object_selection(app, d.start, [end[0], end[1]], d.modifiers),
        Tool::FreeformPen => crate::vector_ui::freeform_finish(app, &d.points, d.lasso.as_ref().map_or(&[], |l| l.straight())),
        // Photoshop: a click without dragging opens Create Rectangle / Ellipse / … instead.
        t if crate::vector_ui::is_shape_tool(t)
            && crate::shape_dialog::is_click(d.start, &d.points, app.point_zoom())
            && crate::shape_dialog::open(app, t, d.start).is_some() => {}
        t if crate::vector_ui::is_shape_tool(t) => crate::vector_ui::finish_shape(app, t, d.start, [end[0], end[1]], d.live),
        Tool::PathSelection => crate::vector_ui::path_selection_finish(app, d.start, [end[0], end[1]]),
        Tool::Type | Tool::VerticalType | Tool::HorizontalTypeMask | Tool::VerticalTypeMask => crate::type_tool::pointer_up(app, d.start, [end[0], end[1]]),
        Tool::Brush | Tool::Pencil | Tool::Eraser => commit_brush_stroke(app, &d),
        Tool::RectMarquee | Tool::EllipseMarquee => {
            // Exactly what the drag showed (`marquee_preview_px`); a click deselects.
            let Some([x0, y0, x1, y1]) = marquee_preview_px(&app.ui.tool_options, &d, app.point_zoom()) else {
                if app.session.is_enabled("select.deselect") {
                    let _ = app.run("select.deselect", json!({}));
                }
                return;
            };
            let mode = selection_mode(app, d.modifiers);
            let (aa, feather) = (app.ui.tool_options.anti_alias, app.ui.tool_options.feather);
            let _ = app.run("select.rect", json!({"x": x0, "y": y0, "width": x1 - x0, "height": y1 - y0, "mode": mode, "ellipse": d.tool == Tool::EllipseMarquee, "antiAlias": aa, "feather": feather}));
        }
        // A click (or the end of a drag) selects the line under the pointer, never deselects.
        Tool::SingleRowMarquee | Tool::SingleColumnMarquee => {
            let Some([x0, y0, x1, y1]) = single_line_drag_px(app, &d) else { return };
            let mode = selection_mode(app, d.modifiers);
            let feather = app.ui.tool_options.feather;
            // `line` names the History step after the tool (Single Row / Single Column Marquee).
            let line = if d.tool == Tool::SingleRowMarquee { "row" } else { "column" };
            let _ = app.run(
                "select.rect",
                json!({"x": x0, "y": y0, "width": x1 - x0, "height": y1 - y0, "mode": mode, "line": line, "antiAlias": false, "feather": feather}),
            );
        }
        Tool::Patch if crate::retouch_ui::patch_drags_selection(app, d.start, d.modifiers) => crate::retouch_ui::finish_patch(app, d.start, [end[0], end[1]]),
        Tool::ContentAwareMove if crate::retouch_ui::patch_drags_selection(app, d.start, d.modifiers) => {
            crate::retouch_ui::finish_content_aware_move(app, d.start, [end[0], end[1]])
        }
        Tool::Lasso | Tool::Patch | Tool::ContentAwareMove => {
            let pts: Vec<[f64; 2]> = d.points.iter().map(|p| [p[0], p[1]]).collect();
            if pts.len() >= 3 {
                let mode = selection_mode(app, d.modifiers);
                let o = &app.ui.tool_options;
                // The Patch and Content-Aware Move tools have no Feather in their options bar.
                let feather = if d.tool == Tool::Lasso { o.feather } else { 0.0 };
                let made = app.run("select.lasso", json!({"points": pts, "mode": mode, "antiAlias": o.anti_alias, "feather": feather}));
                // The lasso only outlines the patch; say that it is dragged next (#1715).
                if made.is_ok() && d.tool != Tool::Lasso {
                    crate::retouch_ui::patch_hint(app);
                }
            } else if app.session.is_enabled("select.deselect") {
                let _ = app.run("select.deselect", json!({}));
            }
        }
        Tool::Gradient => {
            if (end[0] - d.start[0]).abs() + (end[1] - d.start[1]).abs() >= 2.0 {
                let o = app.ui.tool_options.clone();
                let _ = app.run(
                    "paint.gradient",
                    json!({"from": [d.start[0], d.start[1]], "to": [end[0], end[1]], "style": o.gradient_style, "reverse": o.gradient_reverse, "dither": o.gradient_dither, "opacity": o.fill_opacity, "mode": o.gradient_blend_mode.label(), "target": paint_target(app)}),
                );
            }
        }
        Tool::Move => {
            let (dx, dy) = ((end[0] - d.start[0]).round(), (end[1] - d.start[1]).round());
            crate::move_ui::finish(app, dx, dy);
        }
        _ => {}
    }
}

/// Extra OS windows showing documents (multi-window / multi-monitor).
pub fn extra_windows(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let wins = app.ui.windows.clone();
    for w in wins.into_iter().filter(|w| w.open) {
        let Some(st) = app.session.documents().get(w.document) else { continue };
        let title = format!("{} — window {}", st.doc.name, w.id);
        let vid = egui::ViewportId::from_hash_of(("docwin", w.id));
        let mut builder = egui::ViewportBuilder::default().with_title(title).with_inner_size([800.0, 600.0]);
        if let Some([x, y]) = w.position {
            builder = builder.with_position([x, y]);
        }
        let mut view = w.view.clone();
        let mut close = false;
        ctx.show_viewport_immediate(vid, builder, |ui, _class| {
            if ui.ctx().input(|i| i.viewport().close_requested()) {
                close = true;
            }
            egui::CentralPanel::default().frame(egui::Frame::NONE.fill(crate::theme::Tokens::get(ui.ctx()).canvas)).show(ui, |ui| {
                let rect = ui.available_rect_before_wrap();
                view = canvas_view(app, ui, w.document, rect, view.clone(), false);
            });
        });
        if let Some(win) = app.ui.windows.iter_mut().find(|x| x.id == w.id) {
            win.view = view;
            if close {
                win.open = false;
            }
        }
    }
    app.ui.windows.retain(|w| w.open);
}

/// Selection mode from the options bar, overridden by modifier keys (⇧ add, ⌥ subtract, ⇧⌥ intersect).
/// The cursor badge announces the same mode (`tool_feedback`).
pub(crate) fn selection_mode(app: &PhotocraftApp, m: egui::Modifiers) -> &'static str {
    crate::tool_feedback::document_selection_mode(app, Tool::Lasso, m)
}

/// Shift constrains a Polygonal Lasso segment to Photoshop's 45° directions.
/// Work in document coordinates so zoom, view rotation and mirroring cannot skew it.
pub(crate) fn constrained_polygon_point(from: [f64; 2], point: [f64; 2]) -> [f64; 2] {
    let dx = point[0] - from[0];
    let dy = point[1] - from[1];
    if !dx.is_finite() || !dy.is_finite() {
        return point;
    }
    let length = dx.hypot(dy);
    if length == 0.0 {
        return point;
    }
    let angle = (dy.atan2(dx) / std::f64::consts::FRAC_PI_4).round() * std::f64::consts::FRAC_PI_4;
    [from[0] + length * angle.cos(), from[1] + length * angle.sin()]
}

/// A polygonal lasso click adds a vertex; clicking near the first vertex closes the polygon. The
/// first click fixes the selection mode; a new-selection polygon hides the old outline while it is
/// drawn, and replaces it in one history step when it closes.
fn polygon_click(app: &mut PhotocraftApp, x: f64, y: f64, mods: egui::Modifiers) {
    let close = app
        .ui
        .polygon
        .first()
        .is_some_and(|p0| app.ui.polygon.len() >= 3 && ((p0[0] - x).powi(2) + (p0[1] - y).powi(2)).sqrt() < 8.0 / app.point_zoom().max(0.01) as f64);
    if close {
        commit_polygon(app);
        return;
    }
    if app.ui.polygon.is_empty() {
        // ⌥ with nothing selected draws freehand; there is nothing to subtract from.
        let nothing_selected = app.session.active().is_none_or(|st| st.doc.selection.is_none());
        let intent = if nothing_selected { egui::Modifiers { alt: false, ..mods } } else { mods };
        app.ui.polygon_mode = selection_mode(app, intent).into();
    }
    // Only constrain segments after the first vertex; the first click still sets selection intent.
    // Alt-drag is freehand and must not snap to an angle.
    let point = if mods.shift && !mods.alt { app.ui.polygon.last().copied().map_or([x, y], |last| constrained_polygon_point(last, [x, y])) } else { [x, y] };
    app.ui.polygon.push(point);
}

/// Remove the Polygonal Lasso's last vertex (⌫, Delete or a right-click while drawing, #1229);
/// removing the only one cancels the polygon. False when no polygon is being drawn.
pub fn polygon_retract(app: &mut PhotocraftApp) -> bool {
    if app.ui.polygon.pop().is_none() {
        return false;
    }
    if app.ui.polygon.is_empty() {
        app.ui.polygon_mode.clear();
    }
    true
}

/// A new-selection polygonal (or magnetic) lasso is being drawn, so the selection it will replace
/// is hidden.
pub fn polygon_replaces_selection(app: &PhotocraftApp) -> bool {
    (!app.ui.polygon.is_empty() && app.ui.polygon_mode == "replace") || crate::magnetic_lasso_ui::replaces_selection(app)
}

/// Close the polygonal lasso and make the selection in the mode it started in.
pub fn commit_polygon(app: &mut PhotocraftApp) {
    let pts = std::mem::take(&mut app.ui.polygon);
    let mode = std::mem::take(&mut app.ui.polygon_mode);
    if pts.len() >= 3 {
        let mode = if mode.is_empty() { selection_mode(app, egui::Modifiers::NONE).to_owned() } else { mode };
        let o = &app.ui.tool_options;
        let _ = app.run("select.lasso", json!({"points": pts, "mode": mode, "antiAlias": o.anti_alias, "feather": o.feather}));
    }
}

/// Apply the crop tool's rectangle.
pub fn commit_crop(app: &mut PhotocraftApp) {
    // Never another document's frame (#1918).
    crate::crop_ui::cancel_stale(app);
    // Committing ends the box's own Undo/Redo: from here Edit › Undo reverts the document.
    crate::crop_ui::clear_history(app);
    let Some(r) = app.ui.crop_rect.take() else { return };
    // The crop's default mode leaves the view upright (`crop_mode`).
    crate::crop_mode::upright(app);
    // The untouched default frame around the whole canvas crops nothing (Photoshop's ↵ on it does
    // nothing); one framing the selection's bounds crops to them (#1789).
    if std::mem::take(&mut app.crop.default_frame) {
        let whole = app.session.active().map(|st| st.doc.bounds()).map(|b| [f64::from(b.x0), f64::from(b.y0), f64::from(b.x1), f64::from(b.y1)]);
        if whole.is_none_or(|b| b == r) {
            return;
        }
    }
    let (x, y) = (r[0].round(), r[1].round());
    let (w, h) = ((r[2] - r[0]).round().max(1.0), (r[3] - r[1]).round().max(1.0));
    let delete = app.ui.tool_options.crop_delete;
    let mut p = json!({"x": x, "y": y, "width": w, "height": h, "deleteCroppedPixels": delete});
    // A turned frame (#1792): the document turns with it, then is cropped to it. (The untouched
    // default frame above is never turned: turning it makes it a real frame.)
    let angle = crate::crop_ui::angle(app);
    app.ui.crop_angle = 0.0;
    if angle != 0.0 {
        p["angle"] = json!(angle);
    }
    // W x H x Resolution (#2443): resample to that size and set the resolution.
    crate::crop_size::commit_params(app, &mut p);
    if app.run("image.crop", p).is_ok()
        && let Some(i) = app.session.active_index()
    {
        app.ui.views[i].fit_pending = true;
    }
}

/// "mask" when the Layers panel targets (or the canvas shows) the active layer's mask, or the layer
/// is an adjustment layer with one (it has no pixels: #4298), else "pixels".
pub fn paint_target(app: &PhotocraftApp) -> serde_json::Value {
    use photocraft_engine::channel_cmds::ChannelTarget;
    let Some(st) = app.session.active() else { return json!("pixels") };
    // A targeted alpha channel (Channels panel) or Quick Mask mode wins over the layer.
    match st.channel_view.target {
        ChannelTarget::Alpha(i) if i < st.doc.channels.len() => return json!({ "channel": i }),
        ChannelTarget::Composite if st.doc.quick_mask.is_some() => return json!("quickMask"),
        ChannelTarget::LayerMask => return json!("mask"),
        _ => {}
    }
    let has_mask = st.active_layer.and_then(|id| st.doc.layer(id)).is_some_and(|l| l.mask.is_some());
    // Viewing the mask (⌥-click its thumbnail, #196) paints the mask.
    let viewing = photocraft_engine::mask_view_cmds::current(st).is_some();
    let mask = app.ui.mask_target || viewing || crate::adjustment_layer_active(st);
    json!(if mask && has_mask { "mask" } else { "pixels" })
}

#[cfg(test)]
mod pattern_drag_tests;

#[cfg(test)]
mod tabs_tests;

#[cfg(test)]
mod tests {
    /// #5472: a ⇧ square pressed between pixels snapped to 101 × 102; the short side grows to
    /// match, on the side the drag points to.
    #[test]
    fn square_px_equalises_the_sides_away_from_the_press_point() {
        use super::{marquee_px, square_px};
        // Down-right drag: the continuous square (50.3, 50.7)..(150.8, 151.2) snaps unevenly.
        let r = marquee_px([50.3, 50.7], [150.8, 151.2]);
        assert_eq!(r, [50.0, 50.0, 151.0, 152.0], "uneven before the fix");
        assert_eq!(square_px(r, [50.3, 50.7], [150.8, 90.0]), [50.0, 50.0, 152.0, 152.0]);
        // Up-left drag: the short side grows towards lower coordinates.
        let r = marquee_px([150.8, 151.2], [50.3, 50.7]);
        assert_eq!(square_px(r, [150.8, 151.2], [60.0, 50.7]), [49.0, 50.0, 151.0, 152.0]);
        // Already square: unchanged.
        assert_eq!(square_px([10.0, 20.0, 30.0, 40.0], [10.0, 20.0], [30.0, 40.0]), [10.0, 20.0, 30.0, 40.0]);
    }

    #[test]
    fn display_image_matches_egui_unmultiplied_conversion() {
        // Several parallel pieces and a ragged one; translucent, out-of-range and NaN values.
        let n: usize = 3 * 16 * 1024 + 77;
        let px: Vec<[f32; 4]> = (0..n)
            .map(|i| {
                let v = match i % 9 {
                    0 => f32::NAN,
                    1 => -0.5,
                    2 => 3.25,
                    _ => (i % 1001) as f32 / 1000.0,
                };
                [v, (i % 257) as f32 / 256.0, 1.0 - (i % 113) as f32 / 112.0, (i % 6) as f32 / 5.0]
            })
            .collect();
        let buf = photocraft_compose::Buffer { rect: photocraft_geom::Rect::new(0, 0, n as i32, 1), px };
        let reference = |img: photocraft_raster::Rgba8Image| egui::ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &img.pixels);
        assert_eq!(super::display_image(None, &buf).pixels, reference(buf.to_rgba8()).pixels);
        let mut doc = photocraft_doc::Document::new("t", photocraft_geom::Size::new(1, 1), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
        doc.icc_profile = Some(photocraft_cms::Builtin::AdobeRgbCompat.profile().to_bytes());
        let mut color = photocraft_engine::color_cmds::ColorState::default();
        color.settings.monitor_profile = "srgb".into();
        let d = color.canvas_display(&doc).unwrap();
        assert!(!d.is_identity());
        assert_eq!(super::display_image(Some(&d), &buf).pixels, reference(d.to_rgba8(&buf)).pixels);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn every_filter_previews_off_the_ui_thread_and_a_newer_value_cancels_the_old() {
        // Dragging Gaussian Blur's radius to 1000 px froze the window: the preview ran inline in
        // the frame, and every slider step waited for the previous blur to finish.
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        app.background_jobs = true;
        crate::filter_dialog::open(&mut app, "filter.blur.gaussianBlur").unwrap();
        let ctx = egui::Context::default();
        let wait_for = |app: &mut PhotocraftApp, radius: f64| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while app.filter_preview.as_ref().is_none_or(|p| p.key.params["radius"].as_f64() != Some(radius)) {
                assert!(std::time::Instant::now() < deadline, "no preview for radius {radius}");
                ensure_filter_preview(app, 0, &ctx);
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        };
        // The first frame hands the blur to the worker and returns.
        assert!(ensure_filter_preview(&mut app, 0, &ctx).is_none());
        assert!(app.filter_preview_worker.busy(), "the preview computes on the worker");
        let first = app.ui.dialogs[0].fields["radius"].as_f64().unwrap();
        wait_for(&mut app, first);
        assert!(app.filter_preview.as_ref().unwrap().result.is_some());

        // A preview for old values occupies the worker until it is cancelled.
        let st = app.session.active().unwrap();
        let d = &app.ui.dialogs[0];
        let stale = crate::filter_dialog::FilterPreviewKey {
            doc: st.doc.id,
            revision: st.revision,
            dialog: d.id,
            active: st.active_layer,
            target: st.channel_view.target,
            command: "filter.blur.gaussianBlur".into(),
            params: json!({"radius": 999.0}),
            k: 1,
        };
        let (told, cancelled) = std::sync::mpsc::channel();
        app.filter_preview_worker
            .start(stale, ctx.clone(), move |job| {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                while !job.cancelled() && std::time::Instant::now() < deadline {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                let _ = told.send(job.cancelled());
                crate::filter_preview_worker::Computed { result: None, ms: 0.0 }
            })
            .unwrap();
        // The slider moves on and stops: once the value settles, the stale preview is cancelled
        // instead of being waited out.
        app.ui.dialogs[0].fields.insert("radius".into(), json!(12.0));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let was_cancelled = loop {
            ensure_filter_preview(&mut app, 0, &ctx);
            if let Ok(c) = cancelled.try_recv() {
                break c;
            }
            assert!(std::time::Instant::now() < deadline, "the stale preview was never cancelled");
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert!(was_cancelled, "the stale preview stopped because it was cancelled, not by timing out");
        wait_for(&mut app, 12.0);
        assert!(app.filter_preview.as_ref().unwrap().result.is_some());
    }

    // Starts a worker job that computes a real preview for the dialog as it stands, but only
    // finishes when the returned sender is signalled. Gating it here keeps these tests off the
    // clock: `supersede` cancels a running job once the values have been still for SETTLE_MS, so a
    // test that relies on a real blur beating a 150 ms timer is a test that fails on a loaded box.
    #[cfg(not(target_arch = "wasm32"))]
    fn gated_blur_preview(app: &mut PhotocraftApp, ctx: &egui::Context) -> (std::sync::mpsc::Sender<()>, f64) {
        let d = app.ui.dialogs.first().expect("a dialog is open");
        let st = app.session.active().expect("a document is active");
        let command = "filter.blur.gaussianBlur";
        let params = app.with_mask_target(command, crate::filter_dialog::params_of(&d.fields));
        assert_eq!(
            crate::filter_dialog::preview_factor(command, &params, 1),
            1,
            "a 64x48 document previews at full resolution, and these tests rely on k staying 1"
        );
        let key = crate::filter_dialog::FilterPreviewKey {
            doc: st.doc.id,
            revision: st.revision,
            dialog: d.id,
            active: st.active_layer,
            target: st.channel_view.target,
            command: command.into(),
            params,
            k: 1,
        };
        let doc = app.session.documents()[0].doc.clone();
        let documents: Vec<_> = app.session.documents().iter().map(|d| d.doc.clone()).collect();
        let radius = key.params["radius"].as_f64().expect("the dialog seeds a radius");
        let (release, wait) = std::sync::mpsc::channel::<()>();
        app.filter_preview_worker
            .start(key.clone(), ctx.clone(), move |_| {
                wait.recv_timeout(std::time::Duration::from_secs(60)).expect("the test releases the job");
                let result =
                    crate::filter_dialog::preview_document_for_target(&doc, key.active, &key.command, &key.params, key.k, None, key.target, &documents).map(
                        |d| {
                            let buffer = photocraft_compose::flatten(&d);
                            (std::sync::Arc::new(d), buffer)
                        },
                    );
                crate::filter_preview_worker::Computed { result, ms: 0.0 }
            })
            .expect("the worker starts");
        (release, radius)
    }

    // The worker hands its result to the channel only after `compute` returns, so a signal sent
    // from inside `compute` would still race that send. These tests drive the canvas until the
    // state they need appears instead of assuming a timing.
    #[cfg(not(target_arch = "wasm32"))]
    fn drive_until(app: &mut PhotocraftApp, ctx: &egui::Context, what: &str, done: impl Fn(&PhotocraftApp) -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while !done(app) {
            assert!(std::time::Instant::now() < deadline, "timed out waiting for {what}");
            ensure_filter_preview(app, 0, ctx);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_preview_computed_mid_drag_reaches_the_canvas_and_then_converges() {
        // A drag used to paint nothing at all: every completion was dropped for carrying stale
        // parameters, so the canvas only ever showed an effect once the pointer had come to rest.
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        app.background_jobs = true;
        crate::filter_dialog::open(&mut app, "filter.blur.gaussianBlur").unwrap();
        let ctx = egui::Context::default();
        let (release, first) = gated_blur_preview(&mut app, &ctx);

        // The pointer moves on before the frame lands.
        let moved = first + 4.0;
        app.ui.dialogs[0].fields.insert("radius".into(), json!(moved));
        release.send(()).expect("released");
        drive_until(&mut app, &ctx, "the mid-drag frame", |a| a.filter_preview.is_some());

        // The finished frame is painted even though the dialog has moved on.
        let shown = app.filter_preview.as_ref().expect("the mid-drag frame was published");
        assert_eq!(shown.key.params["radius"].as_f64(), Some(first), "the frame is the one the worker finished");
        assert!(shown.result.is_some());
        assert_eq!(app.ui.dialogs[0].fields["radius"].as_f64(), Some(moved), "the slider kept moving");
        // The display gate lets it through, so the canvas has something to draw: this is what
        // re-adding `params` to that gate would break.
        assert!(ensure_filter_preview(&mut app, 0, &ctx).is_some(), "the canvas draws the mid-drag frame");

        // And the preview still converges on the value the dialog holds. `fresh` has to stay an
        // exact match for that: relaxed, the canvas would believe it already had the current
        // request after the first frame of a drag and never compute again.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.filter_preview.as_ref().is_none_or(|p| p.key.params["radius"].as_f64() != Some(moved)) {
            assert!(std::time::Instant::now() < deadline, "the preview never converged on the current radius");
            ensure_filter_preview(&mut app, 0, &ctx);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_mid_drag_frame_does_not_replace_a_frame_that_is_already_exact() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        app.background_jobs = true;
        crate::filter_dialog::open(&mut app, "filter.blur.gaussianBlur").unwrap();
        let ctx = egui::Context::default();

        // A frame for the radius the dialog holds right now reaches the canvas. This also proves
        // the hand-built key is the one the canvas builds: it can only be published by the exact
        // arm, which requires `finished == request`.
        let (release, first) = gated_blur_preview(&mut app, &ctx);
        release.send(()).expect("released");
        drive_until(&mut app, &ctx, "the first frame", |a| a.filter_preview.is_some());
        assert_eq!(app.filter_preview.as_ref().expect("published").key.params["radius"].as_f64(), Some(first));

        // The slider moves away and a frame for the new radius is computed...
        let moved = first + 4.0;
        app.ui.dialogs[0].fields.insert("radius".into(), json!(moved));
        let (release_moved, _) = gated_blur_preview(&mut app, &ctx);
        // ...and back to `first` before that frame lands.
        app.ui.dialogs[0].fields.insert("radius".into(), json!(first));
        release_moved.send(()).expect("released");

        // The frame for `moved` arrives late. It belongs to this preview, but the frame on screen
        // is already exact for the dialog, so it must never be shown in place of it. Checked on
        // every pass: a wrong frame published and then overwritten before the loop ends would
        // otherwise never be seen.
        drive_until(&mut app, &ctx, "the late frame to arrive", |a| {
            if let Some(shown) = &a.filter_preview {
                assert_ne!(shown.key.params["radius"].as_f64(), Some(moved), "a late frame replaced an exact one");
            }
            !a.filter_preview_worker.busy()
        });
        let shown = app.filter_preview.as_ref().expect("a frame is on screen");
        assert_eq!(shown.key.params["radius"].as_f64(), Some(first), "a late frame replaced an exact one");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_mid_drag_empty_result_leaves_the_preview_alone() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        app.background_jobs = true;
        crate::filter_dialog::open(&mut app, "filter.blur.gaussianBlur").unwrap();
        let ctx = egui::Context::default();

        let (release, first) = gated_blur_preview(&mut app, &ctx);
        release.send(()).expect("released");
        drive_until(&mut app, &ctx, "the first frame", |a| a.filter_preview.is_some());
        assert!(app.filter_preview.as_ref().expect("published").result.is_some());

        // The slider moves and a job for the new radius is started, then the slider moves again
        // before that job lands: its result is stale by the time it arrives.
        let moved = first + 4.0;
        app.ui.dialogs[0].fields.insert("radius".into(), json!(moved));
        let d = app.ui.dialogs.first().expect("a dialog is open");
        let st = app.session.active().expect("a document is active");
        let key = crate::filter_dialog::FilterPreviewKey {
            doc: st.doc.id,
            revision: st.revision,
            dialog: d.id,
            active: st.active_layer,
            target: st.channel_view.target,
            command: "filter.blur.gaussianBlur".into(),
            params: json!({"radius": moved}),
            k: 1,
        };
        let (release_empty, wait) = std::sync::mpsc::channel::<()>();
        app.filter_preview_worker
            .start(key, ctx.clone(), move |_| {
                wait.recv_timeout(std::time::Duration::from_secs(60)).expect("released");
                crate::filter_preview_worker::Computed { result: None, ms: 0.0 }
            })
            .expect("the worker starts");
        app.ui.dialogs[0].fields.insert("radius".into(), json!(moved + 4.0));
        release_empty.send(()).expect("released");

        // Storing an empty result here would drop the canvas back to the unfiltered layer for a
        // frame. The exact request may still store one; a stale one may not. Checked on every
        // pass, because the reopened preview that follows would otherwise repair the symptom.
        drive_until(&mut app, &ctx, "the empty frame to arrive", |a| {
            if let Some(shown) = &a.filter_preview {
                assert!(shown.result.is_some(), "an empty mid-drag result blanked the canvas");
            }
            !a.filter_preview_worker.busy()
        });
        assert!(app.filter_preview.as_ref().expect("a preview is still shown").result.is_some(), "an empty mid-drag result blanked the canvas");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_reopened_dialog_never_paints_the_previous_dialogs_preview() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        app.background_jobs = true;
        crate::filter_dialog::open(&mut app, "filter.blur.gaussianBlur").unwrap();
        let ctx = egui::Context::default();
        let first_dialog = app.ui.dialogs.first().expect("a dialog is open").id;
        let (release, _) = gated_blur_preview(&mut app, &ctx);

        // Close that dialog and open another one for the same layer and the same command. Only the
        // dialog id differs, which is exactly what `same_target` has to notice.
        app.ui.dialogs.clear();
        crate::filter_dialog::open(&mut app, "filter.blur.gaussianBlur").unwrap();
        let second_dialog = app.ui.dialogs.first().expect("a dialog is open").id;
        assert_ne!(second_dialog, first_dialog, "a reopened dialog gets a new id");
        release.send(()).expect("released");

        // The first dialog's frame arrives late and must not be painted under the second one.
        // `discard_closed` usually reclaims it a frame earlier, which is why this drives the gate
        // directly: the id check is the part that has to hold when that cleanup misses. The
        // invariant is checked on every pass, not only at the end, because a wrongly published
        // frame can still be replaced by the reopened dialog's own preview before we stop.
        drive_until(&mut app, &ctx, "the first dialog's frame to arrive", |a| {
            if let Some(preview) = &a.filter_preview {
                assert_ne!(preview.key.dialog, first_dialog, "a reopened dialog painted the previous dialog's preview");
            }
            !a.filter_preview_worker.busy()
        });
    }

    #[test]
    fn windows_uses_a_visible_crosshair() {
        // #737: Windows' inverting crosshair vanishes over mid-grey; use a black/white glyph.
        let ctx = egui::Context::default();
        let painter = egui::Painter::new(ctx, egui::LayerId::background(), egui::Rect::EVERYTHING);
        let p = egui::pos2(10.0, 10.0);
        assert_eq!(super::visible_crosshair(egui::CursorIcon::Crosshair, &painter, p, true), egui::CursorIcon::None);
        assert_eq!(super::visible_crosshair(egui::CursorIcon::Crosshair, &painter, p, false), egui::CursorIcon::Crosshair);
        assert_eq!(super::visible_crosshair(egui::CursorIcon::Move, &painter, p, true), egui::CursorIcon::Move);
    }

    use super::*;

    /// #569: displays 1 (sRGB) and 4 (Display P3) side by side, and a document filled with an
    /// sRGB colour.
    fn app_on_two_displays() -> PhotocraftApp {
        use photocraft_engine::color_cmds::resolve_profile;
        use photocraft_engine::display_color::Display;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        app.run("edit.fill", json!({"color": "#cc8040"})).unwrap();
        let icc = |id: &str| Some(resolve_profile(id, None, None).unwrap().to_bytes());
        app.session.color.set_displays(Ok(vec![
            Display { id: 1, name: "A".into(), frame: [0.0, 0.0, 100.0, 100.0], profile_name: None, icc: icc("srgb") },
            Display { id: 4, name: "B".into(), frame: [100.0, 0.0, 100.0, 100.0], profile_name: None, icc: icc("display-p3") },
        ]));
        app
    }

    #[test]
    #[ignore = "manual texture-retention measurement on six 6 MP documents"]
    fn closed_canvas_memory_measurement() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        ctx.input_mut(|i| i.max_texture_side = 8192);
        for cycle in 1..=6 {
            app.run("file.new", json!({"width":3000,"height":2000})).unwrap();
            ensure_texture(&mut app, &ctx, 0, None).unwrap();
            // Drain uploaded image deltas just as a renderer does; count live texture owners.
            ctx.tex_manager().write().take_delta().clear();
            app.run("file.close", json!({})).unwrap();
            let bytes: usize = ctx.tex_manager().read().allocated().map(|(_, m)| m.bytes_used()).sum();
            println!("closed cycle={cycle} texture_bytes={bytes} rss={:?}", photocraft_testkit::perf::current_rss_bytes());
        }
    }

    #[test]
    fn closed_documents_release_navigator_but_keep_open_document_textures() {
        let mut app = app_on_two_displays();
        let ctx = egui::Context::default();
        let first = app.session.active().unwrap().doc.clone();
        let a = ensure_texture(&mut app, &ctx, 0, Some(1)).unwrap().0;
        // The same handle ownership as navigator_texture, without requiring a GPU adapter.
        let tex = ctx.load_texture("navigator-test", egui::ColorImage::filled([64, 64], egui::Color32::WHITE), TextureOptions::LINEAR);
        let nav = tex.id();
        app.navigator_textures.insert(first.id, (std::sync::Arc::downgrade(&first), 0, 0, tex));
        app.run("file.new", json!({"width":32,"height":32})).unwrap();
        let other = app.session.active().unwrap().doc.id;
        let b = ensure_texture(&mut app, &ctx, 1, Some(1)).unwrap().0;
        drop(app.session.close(0));
        app.sync_views();
        assert!(ctx.tex_manager().read().meta(a).is_none());
        assert!(ctx.tex_manager().read().meta(nav).is_none());
        assert!(ctx.tex_manager().read().meta(b).is_some());
        assert!(app.canvases.keys().all(|(id, _)| *id == other));
        assert!(app.navigator_textures.is_empty());
        app.session.add_document((*first).clone(), None);
        app.sync_views();
        assert_ne!(ensure_texture(&mut app, &ctx, 1, Some(1)).unwrap().0, a, "reopening a preserved ID creates a fresh texture");
    }

    #[test]
    fn only_themes_with_a_dot_pattern_tile_the_canvas_surround() {
        // #4542: switching to Pro filled the empty workspace with red blocks on one GPU. Pro's
        // dots are its canvas colour, so the tiled texture had nothing to show in the first place.
        use crate::theme::ThemeKind;
        let tiles = |ctx: &egui::Context, kind: ThemeKind| {
            crate::theme::apply(ctx, kind);
            let mut output = ctx.run_ui(Default::default(), |ui| paint_dots(ui, ui.max_rect(), true));
            output.textures_delta.clear();
            output.shapes.iter().any(|s| matches!(s.shape, egui::Shape::Mesh(_)))
        };
        let uploaded = |ctx: &egui::Context| ctx.tex_manager().read().allocated().filter(|(_, meta)| meta.name == "canvas-dots").count();
        for kind in ThemeKind::ALL {
            let ctx = egui::Context::default();
            let dotted = matches!(kind, ThemeKind::Studio | ThemeKind::StudioLight | ThemeKind::SolarizedDark
            // Fork: Noctalia is the Studio layout with live colours, so it tiles dots too.
            | ThemeKind::Noctalia);
            assert_eq!(tiles(&ctx, kind), dotted, "{kind:?}");
            assert_eq!(uploaded(&ctx), usize::from(dotted), "{kind:?}");
        }
        // The reported switch: a dotted theme, then Pro.
        let ctx = egui::Context::default();
        assert!(tiles(&ctx, ThemeKind::Studio));
        assert!(!tiles(&ctx, ThemeKind::Pro));
        assert!(!tiles(&ctx, ThemeKind::ProMedium));
        assert_eq!(uploaded(&ctx), 1, "only Studio's dots were uploaded");
    }

    #[test]
    fn closed_documents_release_cpu_canvas_textures() {
        let mut app = app_on_two_displays();
        let ctx = egui::Context::default();
        let original = app.session.active().unwrap().doc.clone();
        for cycle in 0..12 {
            if cycle > 0 {
                app.session.add_document((*original).clone(), None);
                app.sync_views();
            }
            let a = ensure_texture(&mut app, &ctx, 0, Some(1)).unwrap().0;
            let b = ensure_texture(&mut app, &ctx, 0, Some(4)).unwrap().0;
            assert_ne!(a, b);
            app.run("file.close", json!({})).unwrap();
            assert!(app.canvases.is_empty(), "closed canvas metadata retained at cycle {cycle}");
            assert!(ctx.tex_manager().read().meta(a).is_none(), "first display texture retained");
            assert!(ctx.tex_manager().read().meta(b).is_none(), "second display texture retained");
        }
    }

    #[test]
    fn cpu_canvas_textures_are_per_display() {
        // Two windows of one document on different displays: each has its own texture of
        // monitor values, and drawing them in turn re-renders neither.
        let mut app = app_on_two_displays();
        let ctx = egui::Context::default();
        let id = app.session.documents()[0].doc.id;
        let a = ensure_texture(&mut app, &ctx, 0, Some(1)).unwrap().0;
        let b = ensure_texture(&mut app, &ctx, 0, Some(4)).unwrap().0;
        assert_ne!(a, b);
        app.perf.last_refresh = "";
        for _ in 0..3 {
            assert_eq!(ensure_texture(&mut app, &ctx, 0, Some(1)).unwrap().0, a);
            assert_eq!(ensure_texture(&mut app, &ctx, 0, Some(4)).unwrap().0, b);
        }
        assert_eq!(app.perf.last_refresh, "", "no re-render");
        let doc = app.session.documents()[0].doc.clone();
        assert!(canvas_display(&app, &doc, Some(1)).0.unwrap().is_identity());
        assert!(!canvas_display(&app, &doc, Some(4)).0.unwrap().is_identity());
        assert_ne!(app.canvases[&(id, 1)].tex_preview_key, app.canvases[&(id, 4)].tex_preview_key);
        // An edit updates both.
        app.run("edit.fill", json!({"color": "#2060c0"})).unwrap();
        ensure_texture(&mut app, &ctx, 0, Some(1));
        ensure_texture(&mut app, &ctx, 0, Some(4));
        let rev = app.session.documents()[0].revision;
        assert_eq!((app.canvases[&(id, 1)].tex_revision, app.canvases[&(id, 4)].tex_revision), (rev, rev));
    }

    #[test]
    fn cpu_canvas_shows_crisp_pixels_when_zoomed_in() {
        // #4810: above 100% the CPU canvas shows each pixel as a square, like the GPU canvas's
        // sharp nearest filter; below 100% it still filters. Both the first upload and a
        // damage-rect update carry the sampling, since the renderer takes it from each delta.
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 2, "height": 2})).unwrap();
        let ctx = egui::Context::default();
        let tex = ensure_texture(&mut app, &ctx, 0, None).unwrap().0;
        let uploads = |ctx: &egui::Context| {
            let mut delta = ctx.tex_manager().write().take_delta();
            let seen = delta.set.get(&tex).into_iter().flatten().map(|d| (d.pos.is_some(), d.options)).collect::<Vec<_>>();
            delta.clear();
            seen
        };
        let crisp = TextureOptions { magnification: egui::TextureFilter::Nearest, ..TextureOptions::LINEAR };
        assert_eq!(uploads(&ctx), vec![(false, crisp)]);
        app.run("select.rect", json!({"x": 0, "y": 0, "width": 1, "height": 1})).unwrap();
        ensure_texture(&mut app, &ctx, 0, None).unwrap();
        uploads(&ctx);
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        assert_eq!(ensure_texture(&mut app, &ctx, 0, None).unwrap().0, tex);
        assert_eq!(uploads(&ctx), vec![(true, crisp)], "a partial update keeps the crisp sampling");
    }

    #[test]
    fn cpu_canvas_of_a_reduced_document_stays_bilinear() {
        // Over MAX_TEXTURE the texture holds one texel per `factor` pixels: a texel is not a
        // document pixel, so showing it as a square would draw blocks that aren't in the image.
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": MAX_TEXTURE + 1, "height": 2})).unwrap();
        let ctx = egui::Context::default();
        let (tex, scale) = ensure_texture(&mut app, &ctx, 0, None).unwrap();
        assert!(scale < 1.0);
        let mut delta = ctx.tex_manager().write().take_delta();
        let uploads: Vec<_> = delta.set.get(&tex).into_iter().flatten().map(|d| d.options).collect();
        delta.clear();
        assert_eq!(uploads, vec![TextureOptions::LINEAR]);
    }

    #[test]
    fn gpu_display_luts_are_per_display() {
        let Ok(rs) = std::panic::catch_unwind(|| egui_kittest::wgpu::create_render_state(crate::gpu_canvas::wgpu_setup(), Default::default())) else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        let mut app = app_on_two_displays();
        app.set_wgpu(rs);
        let doc = app.session.documents()[0].doc.clone();
        let key = doc.id.0;
        // sRGB on the sRGB display needs no LUT; on the P3 display it does.
        assert_eq!(sync_display_lut(&mut app, &doc, key, Some(1)), 0);
        assert_eq!(sync_display_lut(&mut app, &doc, key, Some(4)), 1);
        let gpu = app.gpu.clone().unwrap();
        let (s1, s4) = (gpu.display_lut_signature(key, 1).unwrap(), gpu.display_lut_signature(key, 4).unwrap());
        assert_ne!(s1.0, s4.0);
        assert!(gpu.has_display_lut(key, 4) && !gpu.has_display_lut(key, 1));
        // Drawing one window leaves the other's LUT alone.
        for _ in 0..3 {
            assert_eq!(sync_display_lut(&mut app, &doc, key, Some(1)), 0);
            assert_eq!(sync_display_lut(&mut app, &doc, key, Some(4)), 1);
        }
        assert_eq!((gpu.display_lut_signature(key, 1), gpu.display_lut_signature(key, 4)), (Some(s1), Some(s4)));
        assert!(gpu.has_display_lut(key, 4));
        // The shared texture's key is the same for both displays.
        assert_eq!(texture_key(canvas_display(&app, &doc, Some(1)).0.as_deref()), texture_key(canvas_display(&app, &doc, Some(4)).0.as_deref()));
    }

    /// The navigator's update after an edit (its image with the edit's damage, grown by the
    /// effect reach, rendered over it) equals a new thumbnail of the edited document.
    #[test]
    fn navigator_damage_updates_match_a_full_thumbnail() {
        use photocraft_compose::{render_reduced_damage, thumbnail_buffer, thumbnail_size};
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 1500, "height": 900, "background": "white"})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("select.rect", json!({"x": 200, "y": 150, "width": 300, "height": 200})).unwrap();
        app.run("edit.fill", json!({"color": "#3070a0"})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        app.run("layer.layerStyle.dropShadow", json!({"distance": 30, "size": 20})).unwrap();
        let steps = [
            ("layer.translate", json!({"dx": 37, "dy": -11})),
            ("layer.setProps", json!({"opacity": 0.4})),
            ("paint.stroke", json!({"points": [[700, 500], [900, 520]], "size": 25})),
            ("layer.setProps", json!({"blend": "Multiply"})),
            ("layer.new.layer", json!({})),
        ];
        for (id, p) in steps {
            let before = app.session.active().unwrap().doc.clone();
            app.run(id, p).unwrap();
            let st = app.session.active().unwrap();
            let d = st.last_damage.unwrap_or_else(|| panic!("{id}: a bounded damage"));
            let (w, h) = thumbnail_size(&st.doc, NAVIGATOR_SIDE);
            let mut shown = thumbnail_buffer(&before, NAVIGATOR_SIDE);
            assert_eq!((shown.rect.width(), shown.rect.height()), (w, h));
            if !d.is_empty() {
                let patch = render_reduced_damage(&st.doc, w, h, d.inflate(effect_reach(&st.doc.layers)));
                let r = patch.rect;
                for y in 0..r.height() as usize {
                    let at = (r.y0 as usize + y) * w as usize + r.x0 as usize;
                    shown.px[at..at + r.width() as usize].copy_from_slice(&patch.px[y * r.width() as usize..][..r.width() as usize]);
                }
            }
            assert!(shown.px == thumbnail_buffer(&st.doc, NAVIGATOR_SIDE).px, "{id}: the updated navigator differs from a new thumbnail");
        }
    }

    #[test]
    fn tab_context_uses_clicked_document_and_existing_close_commands() {
        let items = tab_context_items(2, 3, true);
        assert_eq!(items.iter().map(|(_, id, _, _)| *id).collect::<Vec<_>>(), ["file.close", "file.closeOthers", "file.closeAll", "file.revealInFinder"]);
        assert_eq!(items[0].2, json!({"document": 2}));
        assert_eq!(items[1].2, json!({"document": 2}));
        assert_eq!(items[2].2, json!({}));
        assert_eq!(items[3].2, json!({"document": 2}));
        assert!(items[3].3, "a saved document can be revealed in the file manager");
        assert!(!tab_context_items(2, 3, false)[3].3, "an unsaved document cannot");
        assert!(!tab_context_items(0, 1, false)[1].3);
        assert!(items.iter().all(|(_, id, _, _)| photocraft_engine::commands::find(id).is_some()));
    }

    #[test]
    fn tab_context_reveal_names_the_platform_file_manager() {
        let (label, id, _, _) = &tab_context_items(0, 1, true)[3];
        assert_eq!((*label, *id), (crate::menus::reveal_label(), "file.revealInFinder"));
    }

    /// Photoshop's document tab × is after the title on Windows and Linux, before it on macOS (#619).
    #[test]
    fn document_tab_close_button_sits_on_the_platform_side() {
        use egui::os::OperatingSystem as Os;
        use egui_kittest::kittest::Queryable;
        for (os, after_title) in [(Os::Windows, true), (Os::Nix, true), (Os::Mac, false)] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
            app.sync_views();
            // Drawn from the second frame, once the Pro theme is set.
            let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(600.0, 40.0)).build_ui_state(
                |ui, (app, ready): &mut (PhotocraftApp, bool)| {
                    if *ready {
                        tabs(app, ui);
                    }
                },
                (app, false),
            );
            PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
            h.ctx.set_os(os);
            h.state_mut().1 = true;
            h.run_steps(2);
            let tab = h.get_by_label_contains("Untitled @").rect();
            let x = h.get_by_label("Close").rect();
            assert!(tab.contains_rect(x), "{os:?}: the × is inside its tab");
            assert_eq!(x.center().x > tab.center().x, after_title, "{os:?}: × at {x:?} in tab {tab:?}");
            h.get_by_label("Close").click();
            h.run_steps(2);
            assert!(h.state().0.session.documents().is_empty(), "{os:?}: the × closes the document");
        }
    }

    /// Middle-clicking a document tab closes it (#2570), in both tab strips, and the tab under the
    /// pointer is the one that goes even when another document is active.
    #[test]
    fn middle_clicking_a_tab_closes_that_document() {
        use egui_kittest::kittest::Queryable;
        for kind in [crate::theme::ThemeKind::Studio, crate::theme::ThemeKind::Pro] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            for name in ["keep", "doomed", "active"] {
                app.run("file.new", json!({"width": 8, "height": 8, "name": name})).unwrap();
            }
            app.sync_views();
            assert_eq!(app.session.active_index(), Some(2));
            let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(700.0, 40.0)).build_ui_state(
                |ui, (app, ready): &mut (PhotocraftApp, bool)| {
                    if *ready {
                        tabs(app, ui);
                    }
                },
                (app, false),
            );
            PhotocraftApp::setup_context(&h.ctx, kind);
            h.state_mut().1 = true;
            h.run_steps(2);
            h.get_by_label_contains("doomed").click_button(egui::PointerButton::Middle);
            h.run_steps(2);
            let names: Vec<String> = h.state().0.session.documents().iter().map(|d| d.doc.name.clone()).collect();
            assert_eq!(names, ["keep", "active"], "{kind:?}: the middle-clicked tab closed");
            // A clean document closes straight away; a dirty one would park in `discard`.
            assert!(h.state().0.discard.is_none(), "{kind:?}: nothing to prompt about");
        }
    }

    /// A middle click goes through the same guarded `file.close` as the ×, so unsaved changes
    /// still get the Save / Don’t Save / Cancel prompt instead of being thrown away (#2570).
    #[test]
    fn middle_clicking_an_edited_tab_prompts_instead_of_discarding() {
        use egui_kittest::kittest::Queryable;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 8, "height": 8, "name": "edited"})).unwrap();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        app.sync_views();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(700.0, 40.0)).build_ui_state(
            |ui, (app, ready): &mut (PhotocraftApp, bool)| {
                if *ready {
                    tabs(app, ui);
                }
            },
            (app, false),
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
        h.state_mut().1 = true;
        h.run_steps(2);
        h.get_by_label_contains("edited").click_button(egui::PointerButton::Middle);
        h.run_steps(2);
        assert!(h.state().0.discard.is_some(), "the unsaved-changes prompt must come up");
        assert_eq!(h.state().0.session.documents().len(), 1, "and the document is still open");
    }

    #[test]
    fn spectrum_document_tabs_meet_canvas_without_a_gap() {
        for kind in [crate::theme::ThemeKind::Pro, crate::theme::ThemeKind::ProMedium] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.run("file.new", json!({"width": 80, "height": 60})).unwrap();
            app.sync_views();
            let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(680.0, 480.0)).build_ui_state(
                |ui, (app, ready): &mut (PhotocraftApp, bool)| {
                    if *ready {
                        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| document_area(app, ui));
                    }
                },
                (app, false),
            );
            PhotocraftApp::setup_context(&h.ctx, kind);
            h.state_mut().1 = true;
            h.run_steps(3);
            let app = &h.state().0;
            let strip = app.tab_strip.as_ref().expect("a document opens with tabs").rect;
            let gap = app.last_canvas_rect.top() - strip.bottom();
            assert!(gap.abs() < 0.1, "{kind:?}: document strip should touch canvas, got {gap} pt");
        }
    }

    #[test]
    fn large_tabs_preference_increases_tab_strip_height() {
        use egui_kittest::kittest::Queryable;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        app.sync_views();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(600.0, 60.0)).build_ui_state(
            |ui, (app, ready): &mut (PhotocraftApp, bool)| {
                if *ready {
                    tabs(app, ui);
                }
            },
            (app, false),
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
        h.state_mut().1 = true;
        h.run_steps(2);
        let normal_h = h.get_by_label_contains("Untitled @").rect().height();
        assert_eq!(normal_h, 26.0);

        h.state_mut().0.session.execute("prefs.set", json!({"values": {"workspace.largeTabs": true}})).unwrap();
        h.run_steps(2);
        let large_h = h.get_by_label_contains("Untitled @").rect().height();
        assert_eq!(large_h, 34.0);
    }

    #[test]
    fn tab_close_others_prompts_for_unsaved_nonactive_document() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 8, "height": 8, "name": "Keep"})).unwrap();
        app.run("file.new", json!({"width": 8, "height": 8, "name": "Edited"})).unwrap();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        assert_eq!(app.session.active_index(), Some(1));
        let (_, id, params, _) = tab_context_items(0, 2, false)[1].clone();
        crate::menus::invoke(&mut app, &egui::Context::default(), id, params).unwrap();
        assert!(app.discard.is_some(), "close others must ask before discarding the edited tab");
        assert_eq!(app.session.documents().len(), 2);
    }

    /// The pure half of tab drag-to-reorder (UI-217-6): where a drop lands and which
    /// `document.move` target that is.
    #[test]
    fn tab_drop_index_and_move_target_cover_reorder_drops() {
        let r = |x: f32| Rect::from_min_size(egui::pos2(x, 0.0), egui::vec2(100.0, 26.0));
        let placed = vec![(0, r(0.0)), (1, r(100.0)), (2, r(200.0))];
        assert_eq!(tab_drop_index(&placed, 10.0), 0);
        assert_eq!(tab_drop_index(&placed, 160.0), 2);
        assert_eq!(tab_drop_index(&placed, 350.0), 3, "past the end lands after the last shown tab");
        assert_eq!(tab_move_target(0, 3), 2);
        assert_eq!(tab_move_target(2, 0), 0);
        assert_eq!(tab_move_target(1, 1), 1, "before itself: no move");
        assert_eq!(tab_move_target(1, 2), 1, "just after itself: no move");
        assert_eq!(tab_move_target(0, 1), 0);
    }

    /// Dragging a tab onto another's slot reorders the open documents (UI-217-6), in both the
    /// studio and the Photoshop-style strips.
    #[test]
    fn dragging_a_tab_reorders_the_documents() {
        use egui_kittest::kittest::Queryable;
        for kind in [crate::theme::ThemeKind::Studio, crate::theme::ThemeKind::Pro] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            for name in ["a", "b", "c"] {
                app.run("file.new", json!({"width": 8, "height": 8, "name": name})).unwrap();
            }
            app.sync_views();
            let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(700.0, 40.0)).build_ui_state(
                |ui, (app, ready): &mut (PhotocraftApp, bool)| {
                    if *ready {
                        tabs(app, ui);
                    }
                },
                (app, false),
            );
            PhotocraftApp::setup_context(&h.ctx, kind);
            h.state_mut().1 = true;
            h.run_steps(2);
            let b = h.get_by_label_contains("b").rect();
            let a = h.get_by_label_contains("a").rect();
            let target = egui::pos2(a.left() + 2.0, a.center().y);
            h.drag_at(b.center());
            h.run_steps(1);
            h.hover_at(target);
            h.run_steps(1);
            h.drop_at(target);
            h.run_steps(2);
            let names: Vec<String> = h.state().0.session.documents().iter().map(|d| d.doc.name.clone()).collect();
            assert_eq!(names, ["b", "a", "c"], "{kind:?}: the dragged tab took the dropped slot");
        }
    }

    /// Dragging a tab off the strip floats its document in its own OS window instead of
    /// reordering (#3733), in both strips; a short wobble below the strip still reorders.
    #[test]
    fn dragging_a_tab_off_the_strip_floats_it_in_a_window() {
        use egui_kittest::kittest::Queryable;
        let strip = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(700.0, 26.0));
        assert!(!torn_off(strip, egui::pos2(50.0, 13.0)));
        assert!(!torn_off(strip, egui::pos2(50.0, 26.0 + TEAR_OFF - 1.0)), "a wobble below the strip still reorders");
        assert!(torn_off(strip, egui::pos2(50.0, 26.0 + TEAR_OFF + 1.0)));
        assert!(torn_off(strip, egui::pos2(50.0, -TEAR_OFF - 1.0)), "dragged up, off the window's top");
        for kind in [crate::theme::ThemeKind::Studio, crate::theme::ThemeKind::Pro] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            for name in ["a", "b", "c"] {
                app.run("file.new", json!({"width": 8, "height": 8, "name": name})).unwrap();
            }
            app.sync_views();
            let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(700.0, 300.0)).build_ui_state(
                |ui, (app, ready): &mut (PhotocraftApp, bool)| {
                    if *ready {
                        tabs(app, ui);
                    }
                },
                (app, false),
            );
            PhotocraftApp::setup_context(&h.ctx, kind);
            h.state_mut().1 = true;
            h.run_steps(2);
            let b = h.get_by_label_contains("b").rect();
            let target = egui::pos2(b.center().x, b.bottom() + 150.0);
            h.drag_at(b.center());
            h.run_steps(1);
            h.hover_at(target);
            h.run_steps(1);
            h.drop_at(target);
            h.run_steps(2);
            let app = &h.state().0;
            let names: Vec<&str> = app.session.documents().iter().map(|d| d.doc.name.as_str()).collect();
            assert_eq!(names, ["a", "b", "c"], "{kind:?}: tearing off doesn't reorder");
            assert_eq!(app.ui.windows.iter().map(|w| w.document).collect::<Vec<_>>(), [1], "{kind:?}: b floats in its own window");
            assert_eq!(app.session.active_index(), Some(1), "{kind:?}: the floated document is active");
        }
    }

    #[test]
    fn clone_overlay_is_circular_and_respects_visibility() {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        app.run("cloneSource.set", json!({"source": [16,16]})).unwrap();
        app.ui.tool = Tool::CloneStamp;
        app.session.tools.brush.size = 20.0;
        let xf =
            ViewXform { rect: Rect::from_min_size(Pos2::ZERO, vec2(100.0, 100.0)), zoom: 1.0, center: [32.0, 32.0], flip: false, rotation: 0.0, aspect: 1.0 };
        let revision = app.session.active().unwrap().revision;
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let ctx = ui.ctx();
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("clone-test")));
            draw_clone_preview(&mut app, &painter, &xf, [32.0, 32.0], None);
        });
        output.textures_delta.clear();
        let mesh = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Mesh(m) => Some(m),
                _ => None,
            })
            .unwrap();
        assert_eq!(mesh.indices.len(), 64 * 3);
        let center = xf.to_screen(32.0, 32.0);
        assert!(mesh.vertices.iter().all(|v| v.pos.distance(center) <= 10.01));
        assert_eq!(app.session.active().unwrap().revision, revision);
        assert!(app.session.presets.clone.active().anchor.is_none());
        let ordinary_key = app.clone_preview.as_ref().unwrap().key.clone();
        app.ui.view.pattern_preview = true;
        let mut periodic_output = ctx.run_ui(Default::default(), |ui| {
            let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("clone-test")));
            draw_clone_preview(&mut app, &painter, &xf, [32.0, 32.0], None);
        });
        assert_ne!(app.clone_preview.as_ref().unwrap().key, ordinary_key, "toggling Pattern Preview invalidates source pixels");
        assert!(!periodic_output.textures_delta.set.is_empty(), "the overlay texture is refreshed without a document edit");
        periodic_output.textures_delta.clear();
        assert_eq!(app.session.active().unwrap().revision, revision);
        app.session.presets.clone.overlay.show = false;
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let ctx = ui.ctx();
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("clone-test")));
            draw_clone_preview(&mut app, &painter, &xf, [32.0, 32.0], None);
        });
        output.textures_delta.clear();
        assert!(output.shapes.iter().all(|s| !matches!(s.shape, egui::Shape::Mesh(_))));
    }

    #[test]
    fn clone_overlay_blend_mode_mixes_with_the_image_under_the_brush() {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        app.run("cloneSource.set", json!({"source": [16,16]})).unwrap();
        app.ui.tool = Tool::CloneStamp;
        app.session.tools.brush.size = 20.0;
        let xf =
            ViewXform { rect: Rect::from_min_size(Pos2::ZERO, vec2(100.0, 100.0)), zoom: 1.0, center: [32.0, 32.0], flip: false, rotation: 0.0, aspect: 1.0 };
        // The centre texel of the overlay texture uploaded for `mode`.
        let mut centre = |mode: &str| {
            app.run("cloneSource.overlay", json!({"blend": mode})).unwrap();
            let mut output = ctx.run_ui(Default::default(), |ui| {
                let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("clone-test")));
                draw_clone_preview(&mut app, &painter, &xf, [32.0, 32.0], None);
            });
            // The brush's 20 × 20 px square (the font atlas may upload too).
            let image = output.textures_delta.set.values().flatten().find_map(|d| match &d.image {
                egui::ImageData::Color(c) if c.size == [20, 20] => Some(c.clone()),
                _ => None,
            });
            output.textures_delta.clear();
            let image = image.expect("the overlay uploads a texture");
            image.pixels[image.width() * (image.height() / 2) + image.width() / 2]
        };
        // A white source over the white image: Normal shows white, Difference black.
        assert_eq!(centre("normal"), Color32::WHITE);
        assert_eq!(centre("difference"), Color32::BLACK, "Difference turns black where source and image match");
        assert_eq!(centre("lighten"), Color32::WHITE);
    }

    /// A shaped tip clips the Clone Source overlay to its outline, not to the brush circle.
    #[test]
    fn clone_overlay_is_clipped_to_a_shaped_tip() {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        app.run("cloneSource.set", json!({"source": [16,16]})).unwrap();
        app.ui.tool = Tool::CloneStamp;
        app.session.tools.brush.size = 20.0;
        app.session.tools.brush.roundness = 0.25;
        let xf =
            ViewXform { rect: Rect::from_min_size(Pos2::ZERO, vec2(100.0, 100.0)), zoom: 1.0, center: [32.0, 32.0], flip: false, rotation: 0.0, aspect: 1.0 };
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("clone-test")));
            draw_clone_preview(&mut app, &painter, &xf, [32.0, 32.0], None);
        });
        let image = output
            .textures_delta
            .set
            .values()
            .flatten()
            .find_map(|d| match &d.image {
                egui::ImageData::Color(c) if c.size == [20, 20] => Some(c.clone()),
                _ => None,
            })
            .expect("the overlay uploads a texture");
        output.textures_delta.clear();
        let alpha = |x: usize, y: usize| image.pixels[y * image.width() + x].a();
        assert_eq!(alpha(10, 10), 255, "the tip's centre shows the source");
        assert_eq!(alpha(10, 2), 0, "inside the circle but outside the flat tip: clipped");
        assert_eq!(alpha(1, 10), 255, "the tip's long axis stays");
    }

    #[test]
    fn wayland_start_screen_hint_does_not_claim_file_drop_works() {
        let x11 = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let wayland = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services { is_wayland: true, ..Default::default() });
        assert!(start_screen_drop_hint(&x11).starts_with("Drop"));
        // Pasting a copied image file works on Wayland (#338): the hint says how.
        let hint = start_screen_drop_hint(&wayland);
        assert!(!hint.contains("Drop"), "{hint}");
        assert!(hint.ends_with(&format!("paste a copied image with {}.", crate::notices::paste_hint(&wayland))), "{hint}");
    }

    /// #4382: Interface › Show home recent files off hides the Home screen's list, not the recents.
    #[test]
    fn home_recent_list_hides_when_the_preference_is_off() {
        use egui_kittest::kittest::Queryable;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.push_recent("/tmp/photocraft-217/hidden.psd");
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 760.0)).build_ui_state(
            |ui, (app, ready): &mut (PhotocraftApp, bool)| {
                if *ready {
                    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| document_area(app, ui));
                }
            },
            (app, false),
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::default());
        h.state_mut().1 = true;
        h.run_steps(3);
        assert!(h.query_by_label("hidden.psd").is_some(), "listed by default");
        h.state_mut().0.run("prefs.set", serde_json::json!({"values": {"interface.showHomeRecentFiles": false}})).unwrap();
        h.run_steps(3);
        assert!(h.query_by_label("hidden.psd").is_none(), "hidden when off");
        assert_eq!(h.state().0.ui.recent_files.len(), 1, "the list itself is kept");
    }

    /// The Home screen's × takes a file off Recent without opening it, and its right-click menu
    /// removes one entry or clears the list; the preferences keep the change (#2691).
    #[test]
    fn home_recent_entries_can_be_removed() {
        use egui_kittest::kittest::Queryable;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        for name in ["c.tif", "b.psd", "a.png"] {
            app.push_recent(&format!("/tmp/photocraft-2691/{name}"));
        }
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 760.0)).build_ui_state(
            |ui, (app, ready): &mut (PhotocraftApp, bool)| {
                if *ready {
                    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| document_area(app, ui));
                }
            },
            (app, false),
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::default());
        h.state_mut().1 = true;
        h.run_steps(3);
        let listed = |h: &egui_kittest::Harness<'_, (PhotocraftApp, bool)>| {
            let app = &h.state().0;
            assert_eq!(app.session.prefs().file_handling.recent_files, app.ui.recent_files, "stored in the preferences");
            app.ui.recent_files.iter().map(|p| crate::file_open::display_name(p)).collect::<Vec<_>>()
        };
        assert_eq!(listed(&h), ["a.png", "b.psd", "c.tif"]);
        assert!(h.query_by_label("Remove b.psd from Recent").is_none(), "the × shows on the hovered row only");

        h.get_by_label("b.psd").hover();
        h.run_steps(2);
        h.get_by_label("Remove b.psd from Recent").click();
        h.run_steps(2);
        assert_eq!(listed(&h), ["a.png", "c.tif"]);
        assert!(h.state().0.session.documents().is_empty(), "the file was not opened");

        h.get_by_label("c.tif").click_secondary();
        h.run_steps(2);
        h.get_by_label("Remove from Recent").click();
        h.run_steps(2);
        assert_eq!(listed(&h), ["a.png"]);

        h.get_by_label("a.png").click_secondary();
        h.run_steps(2);
        h.get_by_label("Clear Recent Files").click();
        h.run_steps(2);
        assert!(listed(&h).is_empty());
        assert!(h.state().0.session.documents().is_empty());
    }

    #[test]
    fn pointer_moves_are_bounded_by_the_press_and_release() {
        use egui::{Event, PointerButton, pos2};
        let button = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        let events = [
            Event::PointerMoved(pos2(1.0, 0.0)),
            button(pos2(2.0, 0.0), true),
            Event::PointerMoved(pos2(3.0, 0.0)),
            Event::PointerMoved(pos2(4.0, 0.0)),
            button(pos2(5.0, 0.0), false),
            Event::PointerMoved(pos2(6.0, 0.0)),
        ];
        // The press is in this frame: only the moves between press and release count.
        assert_eq!(pointer_moves(&events, PointerButton::Primary, false), vec![pos2(3.0, 0.0), pos2(4.0, 0.0)]);
        // The button was already down when the frame began: the move before the release counts,
        // the one after it does not.
        assert_eq!(pointer_moves(&events, PointerButton::Primary, true), vec![pos2(1.0, 0.0), pos2(3.0, 0.0), pos2(4.0, 0.0)]);
    }

    #[test]
    fn freehand_tools_are_the_ones_that_follow_a_path() {
        for t in [
            Tool::Brush,
            Tool::Pencil,
            Tool::MixerBrush,
            Tool::ColorReplacement,
            Tool::Eraser,
            Tool::BackgroundEraser,
            Tool::CloneStamp,
            Tool::PatternStamp,
            Tool::Smudge,
            Tool::Dodge,
            Tool::Lasso,
            Tool::QuickSelection,
        ] {
            assert!(freehand_tool(t), "{t:?} paints or retouches along a path");
        }
        for t in [Tool::Move, Tool::Eyedropper, Tool::Gradient, Tool::Crop, Tool::RectMarquee, Tool::Type, Tool::Hand] {
            assert!(!freehand_tool(t), "{t:?} is driven by the pointer's latest position");
        }
    }

    #[test]
    fn clone_stamp_centre_appears_only_with_option() {
        assert!(!brush_tip_centre(Tool::CloneStamp, false, false));
        assert!(brush_tip_centre(Tool::CloneStamp, true, false));
        assert!(brush_tip_centre(Tool::Healing, true, false));
        assert!(!brush_tip_centre(Tool::Healing, false, false));
        assert!(brush_tip_centre(Tool::CloneStamp, false, true));
        assert!(!brush_tip_centre(Tool::PatternStamp, true, false), "Pattern Stamp keeps the brush cursor; Option does not switch it to a clone-source mark");
        assert!(!brush_tip_centre(Tool::QuickSelection, false, true));
        assert!(brush_tip_centre(Tool::BackgroundEraser, false, false));
    }

    #[test]
    fn brush_tip_centre_follows_the_crosshair_preference_at_any_size() {
        // A large tip used to show the centre mark whatever Show Crosshair in Brush Tip said.
        for tool in [Tool::Brush, Tool::Eraser, Tool::PatternStamp, Tool::Dodge] {
            assert!(!brush_tip_centre(tool, false, false), "{tool:?}: off");
            assert!(brush_tip_centre(tool, false, true), "{tool:?}: on");
        }
    }

    #[test]
    fn view_transform_roundtrip() {
        for flip in [false, true] {
            let xf = ViewXform {
                rect: Rect::from_min_size(pos2(100.0, 50.0), vec2(800.0, 600.0)),
                zoom: 2.5,
                center: [320.0, 240.0],
                flip,
                rotation: 0.0,
                aspect: 1.0,
            };
            let s = xf.to_screen(10.0, 20.0);
            let d = xf.to_doc(s);
            assert!((d[0] - 10.0).abs() < 1e-3 && (d[1] - 20.0).abs() < 1e-3);
            assert_eq!(xf.to_screen(320.0, 240.0), xf.rect.center());
            // Flipped, document x grows to the left.
            assert_eq!(xf.to_screen(330.0, 240.0).x > xf.rect.center().x, !flip);
            let r = xf.doc_rect(DRect::new(0, 0, 10, 10));
            assert!(r.width() > 0.0 && r.height() > 0.0);
        }
    }

    #[test]
    fn eyedropper_ring_colors_pair_the_sampled_and_current_colours() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 40, "height": 20, "background": "transparent"})).unwrap();
        app.run("shape.create", json!({"kind": "rect", "rect": [0, 0, 20, 20], "fill": "#ff0000"})).unwrap();
        app.run("tools.setColors", json!({"foreground": [0.0, 0.2, 1.0, 1.0]})).unwrap();
        let (new, current) = super::eyedropper_ring_colors(&mut app, 10.0, 10.0).expect("over the red square");
        let near = |c: egui::Color32, r: u8, g: u8, b: u8| {
            let d = |a: u8, b: u8| i32::from(a).abs_diff(i32::from(b));
            d(c.r(), r) <= 1 && d(c.g(), g) <= 1 && d(c.b(), b) <= 1
        };
        assert!(near(new, 255, 0, 0), "{new:?}");
        assert!(near(current, 0, 51, 255), "{current:?}");
        // Transparency and points outside the canvas have nothing to compare.
        assert!(super::eyedropper_ring_colors(&mut app, 30.0, 10.0).is_none());
        assert!(super::eyedropper_ring_colors(&mut app, -1.0, 10.0).is_none());
    }

    #[test]
    fn eyedropper_drag_updates_the_sampled_colour() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 40, "height": 20, "background": "transparent"})).unwrap();
        app.run("shape.create", json!({"kind": "rect", "rect": [0, 0, 20, 20], "fill": "#ff0000"})).unwrap();
        app.run("shape.create", json!({"kind": "rect", "rect": [20, 0, 20, 20], "fill": "#00ff00"})).unwrap();
        app.ui.tool = Tool::Eyedropper;

        tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, egui::Modifiers::NONE);
        assert!(app.session.tools.foreground[0] > 0.99 && app.session.tools.foreground[1] < 0.01);

        tool_event(&mut app, ToolEvent::Move { x: 30.0, y: 10.0, pressure: 1.0 }, egui::Modifiers::NONE);
        assert!(app.session.tools.foreground[1] > 0.99 && app.session.tools.foreground[0] < 0.01);
    }

    /// #4298: an adjustment layer has no pixels, so after its icon thumbnail is clicked (the
    /// Layers panel no longer targets the mask) Fill, the fill keys and the Paint Bucket still
    /// edit its mask, as in Photoshop, instead of failing with "not a pixel layer".
    #[test]
    fn fills_on_an_adjustment_layer_edit_its_mask_when_the_icon_is_targeted() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 40, "height": 20, "background": "white"})).unwrap();
        app.run("layer.newAdjustmentLayer.curves", json!({})).unwrap();
        // Click the Curves icon thumbnail: the panel targets the layer, not its mask.
        app.ui.mask_target = false;
        app.sync_mask_targets();
        assert!(!app.ui.mask_target, "the panel's target is the icon");
        let mask_at = |app: &PhotocraftApp| {
            let st = app.session.active().unwrap();
            st.doc.layer(st.active_layer.unwrap()).unwrap().mask.as_ref().unwrap().surface.sample_channel(10, 10, 0)
        };
        assert_eq!(paint_target(&app), json!("mask"));
        assert!(crate::menus::is_enabled(&app, "edit.fill"), "Edit > Fill (⇧F5) is enabled");
        assert!(crate::menus::is_enabled(&app, "edit.fillForeground"));
        app.ui.tool = Tool::PaintBucket;
        app.run("tools.setColors", json!({"foreground": [0.0, 0.0, 0.0, 1.0], "background": [1.0, 1.0, 1.0, 1.0]})).unwrap();
        tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, egui::Modifiers::NONE);
        tool_event(&mut app, ToolEvent::Up { x: 10.0, y: 10.0 }, egui::Modifiers::NONE);
        assert_eq!(mask_at(&app), 0.0, "the Paint Bucket filled the mask black");
        app.run("edit.fill", json!({"contents": "white"})).unwrap();
        assert_eq!(mask_at(&app), 1.0, "Edit > Fill filled the mask white");
        assert!(!app.ui.mask_target, "filling doesn't move the panel's target");
        // A pixel layer with an untargeted mask still paints its pixels.
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("layer.layerMask.revealAll", json!({})).unwrap();
        app.ui.mask_target = false;
        assert_eq!(paint_target(&app), json!("pixels"));
        assert!(!app.layer_mask_targeted());
    }

    #[test]
    fn alt_with_a_painting_tool_samples_the_foreground_instead_of_painting() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 40, "height": 20, "background": "transparent"})).unwrap();
        app.run("shape.create", json!({"kind": "rect", "rect": [0, 0, 20, 20], "fill": "#ff0000"})).unwrap();
        app.run("shape.create", json!({"kind": "rect", "rect": [20, 0, 20, 20], "fill": "#00ff00"})).unwrap();
        let alt = egui::Modifiers::ALT;
        for tool in [Tool::Brush, Tool::Pencil, Tool::Gradient, Tool::PaintBucket] {
            app.ui.tool = tool;
            app.run("tools.setColors", json!({"foreground": [0.0, 0.0, 1.0, 1.0], "background": [1.0, 1.0, 1.0, 1.0]})).unwrap();
            let rev = app.session.active().unwrap().revision;
            tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, alt);
            assert_eq!(app.session.tools.foreground, [1.0, 0.0, 0.0, 1.0], "{tool:?}: ⌥-click sets the foreground");
            // A drag keeps sampling, even after ⌥ is let go; the background is untouched.
            tool_event(&mut app, ToolEvent::Move { x: 30.0, y: 10.0, pressure: 1.0 }, egui::Modifiers::NONE);
            assert_eq!(app.session.tools.foreground, [0.0, 1.0, 0.0, 1.0], "{tool:?}");
            tool_event(&mut app, ToolEvent::Up { x: 30.0, y: 10.0 }, egui::Modifiers::NONE);
            assert_eq!(app.session.tools.background, [1.0, 1.0, 1.0, 1.0]);
            assert_eq!(app.session.active().unwrap().revision, rev, "{tool:?}: sampling must not edit the document");
            assert!(app.drag.is_none() && !app.alt_sampling);
        }
        // Agents get the same through `ui.pointer` (MCP `ui_pointer`).
        app.ui.tool = Tool::Brush;
        let ctx = egui::Context::default();
        let events = json!([{"kind": "down", "x": 10, "y": 10}, {"kind": "up", "x": 10, "y": 10}]);
        let (req, _rx) = crate::control::ControlRequest::new("ui.pointer", json!({"modifiers": {"alt": true}, "events": events}));
        let _ = crate::control::handle(&mut app, &ctx, &req);
        assert_eq!(app.session.tools.foreground, [1.0, 0.0, 0.0, 1.0]);
        app.run("tools.setColors", json!({"foreground": [0.0, 1.0, 0.0, 1.0]})).unwrap();
        // Without ⌥ the Brush paints again, and ⌥ pressed mid-stroke doesn't switch to sampling.
        app.run("layer.new.layer", json!({})).unwrap();
        let rev = app.session.active().unwrap().revision;
        tool_event(&mut app, ToolEvent::Down { x: 5.0, y: 5.0, pressure: 1.0 }, egui::Modifiers::NONE);
        tool_event(&mut app, ToolEvent::Move { x: 30.0, y: 5.0, pressure: 1.0 }, alt);
        tool_event(&mut app, ToolEvent::Up { x: 30.0, y: 5.0 }, alt);
        assert_eq!(app.session.tools.foreground, [0.0, 1.0, 0.0, 1.0]);
        assert!(app.session.active().unwrap().revision > rev, "the stroke was painted");
        // Control+Alt stays the brush-resize gesture, never a sample.
        assert!(!alt_samples(Tool::Brush, egui::Modifiers { alt: true, ctrl: true, ..Default::default() }));
        assert!(!alt_samples(Tool::Eraser, alt), "⌥ with the Eraser is not the Eyedropper");
    }

    /// #2078: the opaque mask-view texture must show the live stroke, not cover it with
    /// the committed mask until the mouse is released.
    #[test]
    fn mask_view_uploads_each_live_stroke_before_release() {
        for depth in [8, 16, 32] {
            for mode in ["gray", "overlay"] {
                for (width, factor) in [(120u32, 1u32), (513, 3)] {
                    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
                    app.run("file.new", json!({"width": width, "height": 60, "depth": depth})).unwrap();
                    app.run("layer.layerMask.revealAll", json!({})).unwrap();
                    app.run("view.layerMask", json!({"mode": mode})).unwrap();
                    app.run("tools.setColors", json!({"foreground": [0.0, 0.0, 0.0, 1.0]})).unwrap();
                    app.run("tools.setBrush", json!({"brush": {"size": 12, "hardness": 1.0, "smoothing": {"amount": 0.0}}})).unwrap();
                    app.ui.tool = Tool::Brush;
                    let ctx = egui::Context::default();
                    ctx.input_mut(|i| i.max_texture_side = 256);
                    let tex = crate::channel_view::ensure(&mut app, &ctx, 0).unwrap();
                    ctx.tex_manager().write().take_delta().clear();
                    let revision = app.session.documents()[0].revision;
                    let steps = app.session.documents()[0].history.past_len();
                    let layer = app.session.documents()[0].active_layer.unwrap();
                    let original = app.session.documents()[0].doc.layer(layer).unwrap().clone();
                    tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 30.0, pressure: 1.0 }, egui::Modifiers::NONE);
                    for x in [30.0, 50.0, 70.0] {
                        tool_event(&mut app, ToolEvent::Move { x, y: 30.0, pressure: 1.0 }, egui::Modifiers::NONE);
                        let shown = display_doc(&mut app, 0).0;
                        assert!(shown.layer(layer).unwrap().mask.as_ref().unwrap().value(x as i32, 30) < 0.1);
                        assert_eq!(crate::channel_view::ensure(&mut app, &ctx, 0), Some(tex));
                        let mut delta = ctx.tex_manager().write().take_delta();
                        let uploads = delta.set.get(&tex).unwrap_or_else(|| panic!("{depth}-bit {mode}: live mask texture must upload before release at {x}"));
                        assert_eq!(uploads.len(), 1);
                        let upload = &uploads[0];
                        let pos = upload.pos.expect("only the damaged region is uploaded");
                        let egui::ImageData::Color(image) = &upload.image;
                        assert!(image.pixels.len() < (width.div_ceil(factor) * 60u32.div_ceil(factor)) as usize, "the whole mask is not uploaded");
                        let f = factor as i32;
                        let r = DRect::new(pos[0] as i32 * f, pos[1] as i32 * f, (pos[0] + image.size[0]) as i32 * f, (pos[1] + image.size[1]) as i32 * f)
                            .intersect(&shown.bounds());
                        let expected = crate::channel_view::render(
                            &shown,
                            &app.session.documents()[0].channel_view,
                            r,
                            factor,
                            false,
                            app.session.documents()[0].active_layer,
                        )
                        .unwrap();
                        assert_eq!(image.pixels, expected, "uploaded mask values match the live stroke");
                        delta.clear();
                        assert_eq!(app.session.documents()[0].revision, revision);
                        assert_eq!(app.session.documents()[0].history.past_len(), steps);
                        assert_eq!(app.session.documents()[0].doc.layer(layer).unwrap(), &original);
                        crate::channel_view::ensure(&mut app, &ctx, 0);
                        assert!(ctx.tex_manager().write().take_delta().set.is_empty(), "idle frame does not upload");
                    }
                    tool_event(&mut app, ToolEvent::Up { x: 70.0, y: 30.0 }, egui::Modifiers::NONE);
                    let st = &app.session.documents()[0];
                    assert_eq!(st.history.past_len(), steps + 1);
                    assert!(st.doc.layer(layer).unwrap().mask.as_ref().unwrap().value(50, 30) < 0.1);
                    assert_eq!(st.doc.layer(layer).unwrap().content, original.content);
                    crate::channel_view::ensure(&mut app, &ctx, 0);
                    let mut delta = ctx.tex_manager().write().take_delta();
                    // The commit takes the live stroke's pixels, which the texture already shows: at
                    // most a partial upload, never the whole mask again.
                    assert!(delta.set.get(&tex).is_none_or(|u| u.iter().all(|u| u.pos.is_some())), "commit keeps its partial upload");
                    delta.clear();
                    app.session.undo();
                    assert_eq!(app.session.documents()[0].doc.layer(layer).unwrap(), &original);
                    crate::channel_view::ensure(&mut app, &ctx, 0);
                    let mut delta = ctx.tex_manager().write().take_delta();
                    let egui::ImageData::Color(image) = &delta.set.get(&tex).unwrap()[0].image;
                    let st = &app.session.documents()[0];
                    assert_eq!(
                        image.pixels,
                        crate::channel_view::render(&st.doc, &st.channel_view, st.doc.bounds(), factor, false, st.active_layer).unwrap(),
                        "undo restores the shown mask"
                    );
                    delta.clear();
                }
            }
        }
    }

    #[test]
    fn channel_views_show_live_targets_and_discarded_strokes() {
        for target in ["quickMask", "alpha", "red"] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            app.run("file.new", json!({"width": 120, "height": 60})).unwrap();
            match target {
                "quickMask" => {
                    app.run("select.rect", json!({"x": 0, "y": 0, "width": 120, "height": 60})).unwrap();
                    app.run("select.editInQuickMaskMode", json!({})).unwrap();
                }
                "alpha" => {
                    app.run("channel.new", json!({"fill": "white"})).unwrap();
                }
                _ => {
                    app.run("channel.target", json!({"channel": "red"})).unwrap();
                }
            }
            app.run("tools.setColors", json!({"foreground": [0.0, 0.0, 0.0, 1.0]})).unwrap();
            app.run("tools.setBrush", json!({"brush": {"size": 12, "hardness": 1.0, "smoothing": {"amount": 0.0}}})).unwrap();
            app.ui.tool = Tool::Brush;
            let ctx = egui::Context::default();
            let tex = crate::channel_view::ensure(&mut app, &ctx, 0).unwrap();
            ctx.tex_manager().write().take_delta().clear();
            let original = app.session.documents()[0].doc.clone();
            let steps = app.session.documents()[0].history.past_len();
            tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 30.0, pressure: 1.0 }, egui::Modifiers::NONE);
            tool_event(&mut app, ToolEvent::Move { x: 50.0, y: 30.0, pressure: 1.0 }, egui::Modifiers::NONE);
            let shown = display_doc(&mut app, 0).0;
            crate::channel_view::ensure(&mut app, &ctx, 0);
            let mut delta = ctx.tex_manager().write().take_delta();
            let upload = &delta.set.get(&tex).expect("live channel upload")[0];
            let pos = upload.pos.expect("partial channel upload");
            let egui::ImageData::Color(image) = &upload.image;
            let r = DRect::new(pos[0] as i32, pos[1] as i32, (pos[0] + image.size[0]) as i32, (pos[1] + image.size[1]) as i32);
            assert_eq!(
                image.pixels,
                crate::channel_view::render(&shown, &app.session.documents()[0].channel_view, r, 1, false, app.session.documents()[0].active_layer).unwrap(),
                "{target}"
            );
            assert_ne!(
                image.pixels,
                crate::channel_view::render(&original, &app.session.documents()[0].channel_view, r, 1, false, app.session.documents()[0].active_layer).unwrap(),
                "target was painted"
            );
            delta.clear();
            assert_eq!(app.session.documents()[0].history.past_len(), steps);
            assert!(std::sync::Arc::ptr_eq(&original, &app.session.documents()[0].doc));
            // Dropping a preview without committing must restore its texture, even though the
            // document revision did not change. There is no committed damage rectangle to use.
            app.drag = None;
            app.live_stroke = None;
            crate::channel_view::ensure(&mut app, &ctx, 0);
            let mut delta = ctx.tex_manager().write().take_delta();
            let upload = &delta.set.get(&tex).unwrap()[0];
            assert!(upload.pos.is_none(), "discarding a preview refreshes the view");
            let egui::ImageData::Color(image) = &upload.image;
            assert_eq!(
                image.pixels,
                crate::channel_view::render(
                    &original,
                    &app.session.documents()[0].channel_view,
                    original.bounds(),
                    1,
                    false,
                    app.session.documents()[0].active_layer
                )
                .unwrap()
            );
            delta.clear();
        }
    }

    #[test]
    fn brush_drag_shows_the_real_stroke_and_commits_it() {
        // The drag used to draw a hard, flat stand-in and only showed the soft brush on release.
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 120, "height": 60, "background": "transparent"})).unwrap();
        app.run("tools.setBrush", json!({"brush": {"size": 20, "hardness": 0.0}})).unwrap();
        app.ui.tool = Tool::Brush;
        let m = egui::Modifiers::NONE;
        let alpha = |d: &Document, x, y| d.layers[0].surface().unwrap().rgba(x, y)[3];
        let rev = app.session.documents()[0].revision;
        tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 30.0, pressure: 1.0 }, m);
        for x in [40.0, 70.0, 100.0] {
            tool_event(&mut app, ToolEvent::Move { x, y: 30.0, pressure: 1.0 }, m);
        }
        let (shown, key) = display_doc(&mut app, 0);
        assert_ne!(key, 0);
        // A partial value two thirds of the way out: the soft tip, not a hard flat stand-in.
        assert!(alpha(&shown, 40, 30) > 0.5 && (0.01..0.95).contains(&alpha(&shown, 40, 36)), "soft stroke while drawing");
        assert_eq!(alpha(&app.session.documents()[0].doc, 40, 30), 0.0, "not committed yet");
        // The canvas redraws only what the stroke touched.
        let dk = canvas_display(&app, &app.session.documents()[0].doc, None).1;
        let d = damage_since(&app, 0, (rev, dk), (rev, key), dk, None).unwrap();
        assert!(d.contains(40, 30) && !d.contains(40, 2) && d.height() < 60, "{d:?}");
        tool_event(&mut app, ToolEvent::Up { x: 100.0, y: 30.0 }, m);
        let doc = app.session.documents()[0].doc.clone();
        assert!(app.live_stroke.is_none() && display_doc(&mut app, 0).1 == 0);
        assert_eq!((alpha(&doc, 40, 30), alpha(&doc, 40, 38)), (alpha(&shown, 40, 30), alpha(&shown, 40, 38)), "commit matches the preview");
    }

    #[test]
    fn live_stroke_uses_pen_tilt_and_updates_a_reduced_texture_partially() {
        // Pen tilt drives the size; the preview must use the tilt the commit gets. The document
        // is wider than MAX_TEXTURE, so the CPU texture is reduced (factor 2) and each step
        // must update only the reduced pixels it touched.
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 4097, "height": 90, "background": "transparent"})).unwrap();
        let tilt = json!({"size": 30, "hardness": 1.0, "spacing": 0.05, "shapeDynamics": {"enabled": true, "size": {"control": "penTilt"}}});
        app.run("tools.setBrush", json!({ "brush": tilt })).unwrap();
        app.ui.tool = Tool::Brush;
        assert_eq!(ensure_texture(&mut app, &ctx, 0, None).map(|t| t.1), Some(0.5));
        let m = egui::Modifiers::NONE;
        app.stylus.feed.set(Some(crate::stylus::PenSample { pressure: 1.0, tilt_x: 60.0, tilt_y: 0.0, rotation: 0.0, eraser: false }));
        tool_event(&mut app, ToolEvent::Down { x: 100.0, y: 45.0, pressure: 1.0 }, m);
        for x in [300.0, 600.0, 900.0] {
            tool_event(&mut app, ToolEvent::Move { x, y: 45.0, pressure: 1.0 }, m);
            ensure_texture(&mut app, &ctx, 0, None);
            assert_eq!(app.perf.last_refresh, "rect");
            assert!(app.perf.last_refresh_px < 4097 * 90 / 4, "{}", app.perf.last_refresh_px);
        }
        let shown = display_doc(&mut app, 0).0;
        tool_event(&mut app, ToolEvent::Up { x: 900.0, y: 45.0 }, m);
        ensure_texture(&mut app, &ctx, 0, None);
        assert_eq!(app.perf.last_refresh, "rect", "the commit refreshes only the stroke");
        let doc = app.session.documents()[0].doc.clone();
        let (a, b) = (doc.layers[0].surface().unwrap(), shown.layers[0].surface().unwrap());
        // 60° tilt shrinks the dab well below 30 px: the preview shows that size, as committed.
        assert!(a.rgba(500, 45)[3] > 0.5 && a.rgba(500, 45 + 12)[3] == 0.0);
        // (The smoothed tail catches up to the end point only when the stroke finishes.)
        assert!((0..90).all(|y| (0..700).all(|x| a.rgba(x, y) == b.rgba(x, y))), "commit matches the preview");
    }

    /// Brush drag along y = 40 with one canvas frame per pointer move; returns the document the
    /// canvas showed at the last move, the committed one, and whether the release refreshed only
    /// the stroke's rectangle.
    fn drag_frames(smoothing: f32, xs: &[f64]) -> (std::sync::Arc<Document>, std::sync::Arc<Document>, bool, PhotocraftApp) {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 200, "height": 80, "background": "transparent"})).unwrap();
        app.run("tools.setBrush", json!({"brush": {"size": 16, "hardness": 0.5}})).unwrap();
        // What the options bar's Smoothing field writes.
        app.session.tools.brush.smoothing.amount = smoothing;
        app.ui.tool = Tool::Brush;
        ensure_texture(&mut app, &ctx, 0, None);
        let m = egui::Modifiers::NONE;
        tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 40.0, pressure: 1.0 }, m);
        for &x in xs {
            tool_event(&mut app, ToolEvent::Move { x, y: 40.0 + (x / 7.0).sin() * 8.0, pressure: 1.0 }, m);
            ensure_texture(&mut app, &ctx, 0, None);
        }
        let live = display_doc(&mut app, 0).0;
        let last = *xs.last().unwrap();
        tool_event(&mut app, ToolEvent::Up { x: last, y: 40.0 + (last / 7.0).sin() * 8.0 }, m);
        let (next, key) = display_doc(&mut app, 0);
        assert_eq!(key, 0, "the frame after release shows the committed document");
        ensure_texture(&mut app, &ctx, 0, None);
        let partial = app.perf.last_refresh == "rect";
        (live, next, partial, app)
    }

    fn same_pixels(a: &Document, b: &Document) -> bool {
        let (a, b) = (a.layers[0].surface().unwrap(), b.layers[0].surface().unwrap());
        (0..80).all(|y| (0..200).all(|x| a.rgba(x, y) == b.rgba(x, y)))
    }

    #[test]
    fn release_shows_nothing_new_without_smoothing() {
        // #73: at 0 % the stroke follows the pointer; the last frame drawn while dragging is
        // exactly the committed stroke, so release changes nothing on screen.
        let (live, done, partial, app) = drag_frames(0.0, &[40.0, 70.0, 100.0, 130.0, 160.0]);
        assert!(same_pixels(&live, &done), "preview at the last move = committed stroke");
        assert!(partial, "the handover refreshes only the stroke");
        let p = app.session.journal.iter().rev().find(|(id, _)| id == "paint.stroke").map(|(_, p)| p.clone()).unwrap();
        assert!(p.get("smoothing").is_none(), "the commit uses the session brush's smoothing, not a hard-coded one");
    }

    #[test]
    fn navigator_keeps_its_image_during_a_live_brush_stroke() {
        // #1631: every step of a live stroke is a new preview key, so a navigator keyed by it
        // re-composited the whole document per frame while painting. It waits for the release.
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 80, "background": "transparent"})).unwrap();
        app.ui.tool = Tool::Brush;
        let m = egui::Modifiers::NONE;
        assert!(!navigator_waits_for_drag(&app, 0));
        tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 40.0, pressure: 1.0 }, m);
        let mut keys = Vec::new();
        for x in [40.0, 70.0, 100.0] {
            tool_event(&mut app, ToolEvent::Move { x, y: 40.0, pressure: 1.0 }, m);
            keys.push(display_doc(&mut app, 0).1);
            assert!(navigator_waits_for_drag(&app, 0), "painting at x = {x}");
        }
        assert!(keys.windows(2).all(|k| k[0] != k[1]), "each step is a new preview: {keys:?}");
        tool_event(&mut app, ToolEvent::Up { x: 100.0, y: 40.0 }, m);
        assert!(!navigator_waits_for_drag(&app, 0), "the navigator catches up on release");
    }

    #[test]
    fn smoothed_stroke_shows_its_catch_up_tail_while_drawing() {
        // #73: with smoothing the brush lags behind the pointer and catches up at the end; the
        // preview draws that tail live, so no frame after release is missing the end.
        let xs = [30.0, 50.0, 70.0, 90.0, 110.0, 130.0, 150.0, 170.0];
        let (live, done, partial, _) = drag_frames(0.5, &xs);
        assert!(same_pixels(&live, &done), "preview at the last move = committed stroke, tail included");
        assert!(partial);
        let end = |d: &Document| (0..200).rev().find(|&x| (0..80).any(|y| d.layers[0].surface().unwrap().rgba(x, y)[3] > 0.0)).unwrap();
        assert!(end(&live) >= 170, "the end reaches the pointer while drawing ({})", end(&live));
        // The options-bar value reaches the stroke: 50 % smooths the wiggle, 0 % doesn't.
        let (_, rough, _, _) = drag_frames(0.0, &xs);
        assert!(!same_pixels(&rough, &done), "smoothing changes the stroke");
        // And the commit is what `paint.stroke` gives with that smoothing.
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 200, "height": 80, "background": "transparent"})).unwrap();
        s.execute("tools.setBrush", json!({"brush": {"size": 16, "hardness": 0.5, "smoothing": {"amount": 0.5}}})).unwrap();
        let mut pts = vec![json!([10.0, 40.0, 1.0])];
        pts.extend(xs.iter().map(|&x| json!([x, 40.0 + (x / 7.0).sin() * 8.0, 1.0])));
        // (Round tips without dynamics draw the same for any seed.)
        s.execute("paint.stroke", json!({"points": pts, "seed": 0})).unwrap();
        assert!(same_pixels(&s.active().unwrap().doc, &done));
    }

    #[test]
    fn smoothing_preview_redraws_the_tail_each_step() {
        // The tail drawn at one step must not linger once the brush moves on: a sharp turn would
        // leave a stale tail behind if it weren't restored.
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 120, "background": "transparent"})).unwrap();
        app.run("tools.setBrush", json!({"brush": {"size": 10, "hardness": 1.0, "smoothing": {"amount": 0.6}}})).unwrap();
        app.ui.tool = Tool::Brush;
        let m = egui::Modifiers::NONE;
        tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 20.0, pressure: 1.0 }, m);
        for (x, y) in [(60.0, 20.0), (110.0, 20.0), (110.0, 70.0), (110.0, 110.0), (60.0, 110.0)] {
            tool_event(&mut app, ToolEvent::Move { x, y, pressure: 1.0 }, m);
        }
        let live = display_doc(&mut app, 0).0;
        tool_event(&mut app, ToolEvent::Up { x: 60.0, y: 110.0 }, m);
        let done = app.session.documents()[0].doc.clone();
        let (a, b) = (live.layers[0].surface().unwrap(), done.layers[0].surface().unwrap());
        assert!((0..120).all(|y| (0..200).all(|x| a.rgba(x, y) == b.rgba(x, y))), "no stale tails in the preview");
    }

    /// Shapes the drag preview paints for `app` this frame.
    fn drag_preview_shapes(app: &mut PhotocraftApp, ctx: &egui::Context, zoom: f32) -> Vec<egui::epaint::ClippedShape> {
        let out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let ctx = ui.ctx();
            let rect = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
            let painter = ctx.layer_painter(egui::LayerId::background()).with_clip_rect(rect);
            let xf = ViewXform { rect, zoom, center: [60.0, 30.0], flip: false, rotation: 0.0, aspect: 1.0 };
            draw_drag_preview(app, &painter, &xf);
        });
        let egui::FullOutput { mut textures_delta, shapes, .. } = out;
        textures_delta.clear();
        shapes
    }

    #[test]
    fn a_huge_view_centre_draws_the_selection_outline_without_overflow() {
        // #980: `ui.set` takes any finite centre, and the outline's visible region was quantised
        // with unchecked i32 arithmetic: the frame panicked ("attempt to multiply/subtract with
        // overflow") and release builds wrapped to an inverted region.
        let ctx = egui::Context::default();
        // Zoomed far out the quantisation step, and so the padding, is largest (64 · 256 px).
        let cases = [([1e30, 1e30], false), ([-1e30, -1e30], false), ([3e9, -3e9], false), ([512.0, 512.0], true)];
        for ((centre, traced), zoom) in cases.into_iter().flat_map(|c| [(c, 1.0), (c, 1.0 / 64.0)]) {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            app.run("file.new", json!({"width": 1024, "height": 1024})).unwrap();
            app.run("select.rect", json!({"x": 0, "y": 0, "width": 512, "height": 512})).unwrap();
            app.sync_views();
            let (req, _rx) = crate::control::ControlRequest::new("ui.set", json!({ "center": centre, "zoom": zoom }));
            let _ = crate::control::handle(&mut app, &ctx, &req);
            let view = app.ui.views[0].clone();
            assert_eq!(view.zoom, zoom, "ui.set applied the zoom");
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                canvas_view(&mut app, ui, 0, Rect::from_min_size(Pos2::ZERO, vec2(200.0, 200.0)), view.clone(), false);
            });
            out.textures_delta.clear();
            // Control: a centre on the selection's corner still traces the outline (the selection
            // spans several 64 px trace cells even zoomed out).
            let segs = app.outline_cache.as_ref().map_or(0, |(_, _, s)| s.len());
            assert_eq!(segs > 0, traced, "{centre:?} at zoom {zoom}");
        }
    }

    #[test]
    fn a_one_pixel_selection_traces_at_hundred_percent_on_a_scaled_display() {
        // #1943 review: the outline step is display resolution in device pixels, not points, so a
        // 1 px selection still traces at 100 % on a 2x display (a points-based step sampled every
        // second pixel and lost it: zero segments).
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("select.rect", json!({"x": 50, "y": 50, "width": 1, "height": 100})).unwrap();
        app.run("view.actualPixels", json!({})).unwrap();
        app.sync_views();
        let view = app.ui.views[0].clone();
        let mut input = egui::RawInput::default();
        input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(2.0);
        let mut out = ctx.run_ui(input, |ui| {
            canvas_view(&mut app, ui, 0, Rect::from_min_size(Pos2::ZERO, vec2(200.0, 200.0)), view.clone(), true);
        });
        out.textures_delta.clear();
        assert_eq!(app.ppp, 2.0, "the primary canvas records the display scale");
        let segs = app.outline_cache.as_ref().map_or(0, |(_, _, s)| s.len());
        assert!(segs > 0, "a 1 px selection traces at 100 % on a 2x display");
    }

    #[test]
    fn a_secondary_canvas_does_not_overwrite_the_main_canvas_scale() {
        // #1943 review: a document window can sit on a display with another scale; only the
        // primary canvas records `app.ppp`, so the main mapping doesn't change under it.
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("view.actualPixels", json!({})).unwrap();
        app.sync_views();
        let view = app.ui.views[0].clone();
        let frame = |app: &mut PhotocraftApp, ppp: f32, primary: bool| {
            let mut input = egui::RawInput::default();
            input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(ppp);
            let mut out = ctx.run_ui(input, |ui| {
                canvas_view(app, ui, 0, Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0)), view.clone(), primary);
            });
            out.textures_delta.clear();
        };
        frame(&mut app, 2.0, true);
        assert_eq!(app.ppp, 2.0);
        frame(&mut app, 1.0, false);
        assert_eq!(app.ppp, 2.0, "a secondary window's scale doesn't replace the main canvas's");
        // The main canvas mapping still uses its own scale: 100 % is 1 doc px per physical px.
        let xf = ViewXform::active(&app).unwrap();
        let step = xf.to_screen(1.0, 0.0) - xf.to_screen(0.0, 0.0);
        assert!((step.x - 0.5).abs() < 1e-3, "{step:?}");
    }

    #[test]
    fn a_flicked_hand_drag_glides_on_only_with_flick_panning() {
        // #204: Preferences › Tools › Enable Flick Panning. A fast Hand drag let go while moving
        // keeps the view gliding for a while; off, the view stops where the drag ended.
        for flick in [true, false] {
            let ctx = egui::Context::default();
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            app.run("file.new", json!({"width": 2000, "height": 1500})).unwrap();
            app.run("prefs.set", json!({"path": "tools.enableFlickPanning", "value": flick})).unwrap();
            app.ui.tool = Tool::Hand;
            app.sync_views();
            let mut view = app.ui.views[0].clone();
            view.zoom = 1.0;
            view.center = [1000.0, 750.0];
            view.fit_pending = false;
            view.fill_pending = false;
            let rect = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0));
            let mut frame = |app: &mut PhotocraftApp, events: Vec<egui::Event>| -> View {
                let input = egui::RawInput { screen_rect: Some(rect), events, ..Default::default() };
                let mut out = ctx.run_ui(input, |ui| {
                    view = canvas_view(app, ui, 0, rect, view.clone(), true);
                });
                out.textures_delta.clear();
                view.clone()
            };
            let start = Pos2::new(200.0, 150.0);
            let press =
                |pos: Pos2, pressed: bool| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
            frame(&mut app, vec![egui::Event::PointerMoved(start)]);
            frame(&mut app, vec![press(start, true)]);
            let mut end = start;
            for i in 1..=8 {
                end = start + vec2(-20.0 * i as f32, 0.0);
                frame(&mut app, vec![egui::Event::PointerMoved(end)]);
            }
            let released = frame(&mut app, vec![press(end, false)]).center;
            let mut centers = vec![released];
            for _ in 0..5 {
                centers.push(frame(&mut app, vec![]).center);
            }
            if flick {
                // Dragged left: the image follows the pointer, the view keeps moving right.
                for w in centers.windows(2) {
                    assert!(w[1][0] > w[0][0], "glides on: {centers:?}");
                }
                // ...and comes to rest.
                let mut last = *centers.last().unwrap();
                for _ in 0..120 {
                    last = frame(&mut app, vec![]).center;
                }
                assert_eq!(frame(&mut app, vec![]).center, last, "the glide stops");
            } else {
                assert!(centers.iter().all(|c| *c == released), "no glide when off: {centers:?}");
            }
        }
    }

    #[test]
    fn a_secondary_canvas_hand_pans_while_a_transform_is_open() {
        // #2302 review: `app.ui.transform` is global, but only the primary canvas draws and drives
        // the box. A secondary window's Hand must still pan while the main window transforms.
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("select.rect", json!({"x": 100, "y": 100, "width": 100, "height": 60})).unwrap();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        app.run("view.actualPixels", json!({})).unwrap();
        app.ui.tool = Tool::Hand;
        app.sync_views();
        crate::transform_tool::begin(&mut app, &ctx).expect("a transform starts");
        let mut view0 = app.ui.views[0].clone();
        view0.zoom = 1.0;
        view0.center = [200.0, 150.0];
        view0.fit_pending = false;
        view0.fill_pending = false;
        let center0 = view0.center;
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0));
        let frame = |app: &mut PhotocraftApp, events: Vec<egui::Event>| -> View {
            let input = egui::RawInput { screen_rect: Some(rect), events, ..Default::default() };
            let mut seen = view0.clone();
            let mut out = ctx.run_ui(input, |ui| {
                seen = canvas_view(app, ui, 0, rect, view0.clone(), false);
            });
            out.textures_delta.clear();
            seen
        };
        let start = Pos2::new(200.0, 150.0);
        let press =
            |pos: Pos2, pressed: bool| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        frame(&mut app, vec![egui::Event::PointerMoved(start)]);
        frame(&mut app, vec![press(start, true)]);
        for dx in [10.0, 20.0, 40.0] {
            frame(&mut app, vec![egui::Event::PointerMoved(start + vec2(dx, 0.0))]);
        }
        let dragged = frame(&mut app, vec![egui::Event::PointerMoved(start + vec2(60.0, 0.0))]);
        frame(&mut app, vec![press(start + vec2(60.0, 0.0), false)]);
        assert_ne!(dragged.center, center0, "a secondary window's Hand pans while transforming");
    }

    #[test]
    fn guides_draw_over_the_transform_preview_and_under_its_box() {
        // #5086: a guide crossing a layer under Free Transform was hidden behind the preview of
        // the moving pixels. The preview is image content, drawn before the grid and guides like
        // the document is; the box and handles still go over them, as in Photoshop.
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("select.rect", json!({"x": 100, "y": 100, "width": 100, "height": 60})).unwrap();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        app.run("view.newGuide", json!({"orientation": "vertical", "position": 150})).unwrap();
        app.ui.extras.guides = true;
        app.ui.extras.rulers = false;
        app.sync_views();
        crate::transform_tool::begin(&mut app, &ctx).expect("a transform starts");
        let mut view = app.ui.views[0].clone();
        view.zoom = 1.0;
        view.center = [200.0, 150.0];
        view.fit_pending = false;
        view.fill_pending = false;
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0));
        let input = egui::RawInput { screen_rect: Some(rect), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            canvas_view(&mut app, ui, 0, rect, view.clone(), true);
        });
        out.textures_delta.clear();
        // At 100 % centred on the document, screen and document coordinates coincide.
        let layer = Rect::from_min_max(pos2(100.0, 100.0), pos2(200.0, 160.0));
        let near = |r: Rect| (r.min - layer.min).length() < 1.0 && (r.max - layer.max).length() < 1.0;
        let position = |pred: &dyn Fn(&egui::Shape) -> bool| out.shapes.iter().position(|s| pred(&s.shape));
        // The moving pixels: the mesh that spans the layer's box.
        let preview = position(&|s| matches!(s, egui::Shape::Mesh(m) if near(m.calc_bounds())));
        // The guide: a vertical line at x = 150.
        let guide = position(&|s| matches!(s, egui::Shape::LineSegment { points, .. } if points.iter().all(|p| (p.x - 150.0).abs() < 0.01)));
        // The box: the closed four-point outline round the layer.
        let frame = position(&|s| matches!(s, egui::Shape::Path(p) if p.closed && p.points.len() == 4 && near(Rect::from_points(&p.points))));
        let (Some(preview), Some(guide), Some(frame)) = (preview, guide, frame) else {
            panic!("all painted: preview {preview:?}, guide {guide:?}, box {frame:?}");
        };
        assert!(preview < guide, "the guide ({guide}) draws over the transform preview ({preview})");
        assert!(guide < frame, "the box ({frame}) draws over the guide ({guide})");
    }

    #[test]
    fn brush_drags_paint_no_stand_in_shape_over_the_canvas() {
        // #189: v0.1 drew the stroke being dragged as a foreground-coloured egui polyline as wide
        // as the brush; zoomed in, egui tessellated it into hard black wedges fanning out from the
        // start. The canvas shows the real stroke (`LiveStroke`) and nothing is drawn over it.
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 120, "height": 60})).unwrap();
        app.run("tools.setBrush", json!({"brush": {"size": 30, "hardness": 0.0}})).unwrap();
        // A smoothed stroke also draws its string line (see
        // `smoothed_brush_drag_draws_the_string_from_brush_to_pointer`): that is no stand-in shape.
        let overlay = |app: &mut PhotocraftApp, ctx: &egui::Context| {
            drag_preview_shapes(app, ctx, 12.0).iter().filter(|s| !matches!(s.shape, egui::Shape::LineSegment { .. } | egui::Shape::Circle(_))).count()
        };
        for tool in [Tool::Brush, Tool::Eraser] {
            app.ui.tool = tool;
            let idle = overlay(&mut app, &ctx);
            tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 30.0, pressure: 1.0 }, egui::Modifiers::NONE);
            for i in 1..40 {
                let t = f64::from(i);
                tool_event(&mut app, ToolEvent::Move { x: 20.0 + t, y: 30.0 + (t / 3.0).sin() * 4.0, pressure: 1.0 }, egui::Modifiers::NONE);
            }
            assert!(app.live_stroke.is_some(), "{tool:?}: the canvas shows the live stroke");
            assert_eq!(overlay(&mut app, &ctx), idle, "{tool:?}: no overlay while dragging");
            tool_event(&mut app, ToolEvent::Up { x: 59.0, y: 30.0 }, egui::Modifiers::NONE);
        }
    }

    #[test]
    fn smoothed_brush_drag_draws_the_string_from_brush_to_pointer() {
        // #3479: with smoothing the brush lags behind the pointer; a line joins them while drawing.
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 120, "height": 60})).unwrap();
        app.ui.tool = Tool::Brush;
        let lines = |app: &mut PhotocraftApp| drag_preview_shapes(app, &ctx, 4.0).iter().filter(|s| matches!(s.shape, egui::Shape::LineSegment { .. })).count();
        for (amount, string) in [(0.0, false), (0.8, true)] {
            app.run("tools.setBrush", json!({"brush": {"size": 10, "smoothing": {"amount": amount}}})).unwrap();
            tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 30.0, pressure: 1.0 }, egui::Modifiers::NONE);
            for x in [30.0, 60.0, 90.0] {
                tool_event(&mut app, ToolEvent::Move { x, y: 30.0, pressure: 1.0 }, egui::Modifiers::NONE);
            }
            assert_eq!(lines(&mut app) > 0, string, "smoothing {amount}");
            tool_event(&mut app, ToolEvent::Up { x: 90.0, y: 30.0 }, egui::Modifiers::NONE);
        }
    }

    #[test]
    fn retouch_drags_show_the_footprint_without_wedges() {
        // The retouching tools' trail was the same brush-wide polyline (#189): now a mask of the
        // footprint, with nothing outside the brush radius of the path, however small the steps.
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 120, "height": 60})).unwrap();
        app.run("tools.setBrush", json!({"brush": {"size": 30}})).unwrap();
        // Remove: the dab tools draw their stroke live instead of a trail, and Spot Healing's
        // trail is its dabs' coverage.
        app.ui.tool = Tool::Remove;
        tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 30.0, pressure: 1.0 }, egui::Modifiers::NONE);
        let mut pts = vec![[20.0, 30.0]];
        for i in 1..80 {
            let t = f64::from(i) * 0.5;
            let p = [20.0 + t, 30.0 + (t / 2.0).sin() * 3.0];
            tool_event(&mut app, ToolEvent::Move { x: p[0], y: p[1], pressure: 1.0 }, egui::Modifiers::NONE);
            pts.push(p);
            if i % 20 == 0 {
                // One mesh for the trail, however long the drag.
                let shapes = drag_preview_shapes(&mut app, &ctx, 12.0);
                assert!(!shapes.iter().any(|s| matches!(s.shape, egui::Shape::Path(_))), "no polyline");
            }
        }
        drag_preview_shapes(&mut app, &ctx, 12.0);
        let trail = app.trail.as_ref().unwrap();
        assert_eq!(trail.scale(), 1.0);
        for y in 0..60 {
            for x in 0..120 {
                let d = pts.iter().map(|p| (x as f64 + 0.5 - p[0]).hypot(y as f64 + 0.5 - p[1])).fold(f64::MAX, f64::min);
                if d < 14.0 {
                    assert_eq!(trail.coverage(x, y), 255, "({x}, {y}) inside the footprint");
                } else if d > 16.0 {
                    assert_eq!(trail.coverage(x, y), 0, "({x}, {y}) outside the footprint");
                }
            }
        }
        tool_event(&mut app, ToolEvent::Up { x: 59.5, y: 30.0 }, egui::Modifiers::NONE);
        drag_preview_shapes(&mut app, &ctx, 12.0);
        assert!(app.trail.is_none(), "the trail ends with the drag");
    }

    #[test]
    fn spot_healing_trail_is_the_coverage_the_heal_uses() {
        // #2364: a soft brush with pen-pressure size showed as a hard footprint at full size.
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 120, "height": 60})).unwrap();
        let brush = json!({"size": 30, "hardness": 0.0, "shapeDynamics": {"enabled": true, "size": {"control": "penPressure"}}});
        app.run("tools.setBrush", json!({ "brush": brush })).unwrap();
        app.ui.tool = Tool::SpotHealing;
        tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 30.0, pressure: 0.5 }, egui::Modifiers::NONE);
        for x in [40.0, 60.0, 80.0] {
            tool_event(&mut app, ToolEvent::Move { x, y: 30.0, pressure: 0.5 }, egui::Modifiers::NONE);
            drag_preview_shapes(&mut app, &ctx, 12.0);
        }
        // The stroke the heal gets on release, and its coverage.
        let points = app.drag.as_ref().unwrap().points.iter().map(|p| photocraft_paint::StrokePoint::new(p[0], p[1], p[2] as f32)).collect();
        let brush = photocraft_engine::retouch_cmds::stroke_brush(&app.session.tools.brush, &json!({}));
        let (b, cov) = photocraft_paint::retouch::stroke_coverage(&photocraft_paint::Stroke { brush, points });
        let trail = app.trail.as_ref().unwrap();
        let mut soft = 0;
        for y in 0..60 {
            for x in 0..120 {
                let want = if b.contains(x, y) { cov[(y - b.y0) as usize * b.width() as usize + (x - b.x0) as usize] } else { 0.0 };
                let got = trail.coverage(x as usize, y as usize);
                assert!((f32::from(got) - want * 255.0).abs() <= 1.0, "({x}, {y}): trail {got}, heal {want}");
                soft += usize::from(got > 0 && got < 200);
            }
        }
        assert!(soft > 50, "a soft brush shows a soft edge");
        // Half pressure halves the 30 px tip: nothing past its (soft) reach.
        let reach = photocraft_paint::tip_reach(7.5, app.session.tools.brush.hardness).ceil() as usize + 2;
        assert_eq!(trail.coverage(50, 30 + reach), 0, "half pressure halves the 30 px tip");
    }

    #[test]
    fn layer_style_dialog_previews_live_and_cancel_restores() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        let fx = |d: &Document| d.layers.iter().map(|l| l.effects.items.len()).sum::<usize>();
        let id = crate::layer_style::open(&mut app, Some("colorOverlay")).unwrap();
        let (shown, key) = display_doc(&mut app, 0);
        assert_eq!((fx(&shown), fx(&app.session.documents()[0].doc)), (1, 0), "previewed, not committed");
        // Adding a stroke through the dialog's instance list re-renders the canvas.
        if let Some(d) = app.ui.dialog_mut(id) {
            d.fields["effects"].as_array_mut().unwrap().push(json!({"id": "fx9", "kind": "stroke", "on": true, "params": {"size": 3}}));
        }
        let (shown, key2) = display_doc(&mut app, 0);
        assert_eq!(fx(&shown), 2);
        assert_ne!(key, key2, "an edit re-renders the canvas");
        app.ui.close_dialog(id);
        let (shown, key) = display_doc(&mut app, 0);
        assert_eq!((fx(&shown), key), (0, 0));
        assert!(app.style_preview.is_none());
    }

    /// Layer Style opened while Free Transform is open applies the box first, as Photoshop does,
    /// so the dialog previews on the transformed layer (#4821): a dropped file (placed in the box,
    /// untouched) and a box opened by hand and moved.
    #[test]
    fn layer_style_opened_during_free_transform_applies_the_box_and_previews() {
        for placed in [true, false] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
            app.run("layer.new.layer", json!({})).unwrap();
            app.run("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
            app.run("edit.fill", json!({"color": "#0000ff"})).unwrap();
            app.run("select.deselect", json!({})).unwrap();
            app.sync_views();
            let ctx = egui::Context::default();
            let dx = if placed {
                crate::transform_tool::begin_placed(&mut app, &ctx).unwrap();
                0
            } else {
                crate::transform_tool::begin(&mut app, &ctx).unwrap();
                app.ui.transform.as_mut().unwrap().quad = [[28.0, 8.0], [44.0, 8.0], [44.0, 24.0], [28.0, 24.0]];
                20
            };
            crate::layer_style::open(&mut app, Some("dropShadow")).unwrap();
            assert!(app.ui.transform.is_none() && app.transform_preview.is_none(), "placed {placed}: the box is applied");
            let (shown, _) = display_doc(&mut app, 0);
            let rect = photocraft_geom::Rect::new(0, 0, 64, 64);
            let with = photocraft_compose::render(&shown, rect);
            let without = photocraft_compose::render(&app.session.documents()[0].doc, rect);
            assert!(
                with.get(16 + dx, 16)[0] < 0.1 && with.get(16 + dx, 16)[2] > 0.9,
                "placed {placed}: the blue block where the box left it, not the white under it"
            );
            // The shadow darkens the white around the block, which the document doesn't have yet.
            let block = |x: i32, y: i32| (8 + dx..24 + dx).contains(&x) && (8..24).contains(&y);
            let shadow =
                (0..64).flat_map(|y| (0..64).map(move |x| (x, y))).filter(|&(x, y)| !block(x, y) && without.get(x, y)[0] - with.get(x, y)[0] > 0.05).count();
            assert!(shadow > 20, "placed {placed}: the Drop Shadow is previewed ({shadow} px)");
        }
    }

    #[test]
    fn fill_view_covers_the_canvas_and_centres_the_document() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
        let doc = app.session.active().unwrap().doc.clone();
        let mut view = View { fill_pending: true, ..View::default() };

        fill_view(&mut view, &doc, vec2(600.0, 600.0), 1.0, 1.0);

        assert_eq!(view.zoom, 3.0, "the short edge fills the available height");
        assert_eq!(view.center, [200.0, 100.0]);
        assert!(!view.fill_pending);

        // A scaled display stores the physical factor: the same point zoom is 2× the device px.
        let mut view = View { fill_pending: true, ..View::default() };
        fill_view(&mut view, &doc, vec2(600.0, 600.0), 2.0, 1.0);
        assert_eq!(view.zoom, 6.0);

        // Fit never magnifies past 100% = one image pixel per physical pixel, at any display
        // scale (it used to cap at one per *point*, so a 150% display opened images at 150%).
        for ppp in [1.0f32, 1.5, 2.0] {
            let mut view = View { fit_pending: true, ..View::default() };
            fit_view(&mut view, &doc, vec2(600.0, 600.0), ppp, 1.0);
            assert_eq!(view.zoom, 1.0, "ppp {ppp}: a small image opens at 100%");
        }
        // A large image still shrinks to fit: 1000×500 into 440 points is 0.44 pt/px, so 0.66
        // physical at ppp 1.5.
        let big = {
            app.run("file.new", json!({"width": 1000, "height": 500})).unwrap();
            app.session.active().unwrap().doc.clone()
        };
        let mut view = View { fit_pending: true, ..View::default() };
        fit_view(&mut view, &big, vec2(480.0, 480.0), 1.5, 1.0);
        assert!((view.zoom - 0.66).abs() < 1e-5, "{}", view.zoom);
    }

    #[test]
    fn a_hundred_percent_is_one_physical_pixel_per_document_pixel() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
        app.sync_views();
        for ppp in [1.0f32, 1.25, 2.0] {
            app.ppp = ppp;
            let xf = ViewXform::active(&app).unwrap();
            // 100% zoom: one document pixel is one physical pixel, i.e. 1/ppp egui points.
            let step = xf.to_screen(1.0, 0.0) - xf.to_screen(0.0, 0.0);
            // `to_screen` differences around the view centre cancel in f32; 1e-3 is still exact
            // enough to see a wrong ppp factor (which would be a 20–100% error).
            assert!((step.x - 1.0 / ppp).abs() < 1e-3, "ppp {ppp}: {step:?} want {:?}", 1.0 / ppp);
            assert!((app.point_zoom() - 1.0 / ppp).abs() < 1e-5, "ppp {ppp}: {}", app.point_zoom());
            // Screen ↔ document stays an exact round trip.
            let back = xf.to_doc(xf.to_screen(123.0, 45.0));
            assert!((back[0] - 123.0).abs() < 1e-3 && (back[1] - 45.0).abs() < 1e-3, "ppp {ppp}: {back:?}");
        }
    }

    #[test]
    fn wheel_anchor_is_the_pointer_over_the_image_and_the_nearest_image_point_off_it() {
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        for (rotation, flip) in [(0.0, false), (30.0, false), (90.0, true)] {
            let xf = ViewXform { rect, zoom: 0.5, center: [400.0, 300.0], flip, rotation, aspect: 1.0 };
            let inside = xf.to_screen(100.0, 200.0);
            assert_eq!(wheel_anchor(&xf, inside, [800, 600]), inside);
            for (p, want) in [((-300.0, -200.0), [0.0, 0.0]), ((1200.0, 300.0), [800.0, 300.0]), ((400.0, 900.0), [400.0, 600.0])] {
                let a = xf.to_doc(wheel_anchor(&xf, xf.to_screen(p.0, p.1), [800, 600]));
                assert!((a[0] - want[0]).abs() < 0.01 && (a[1] - want[1]).abs() < 0.01, "rot {rotation}: {p:?} -> {a:?}");
            }
        }
        let p = pos2(5.0, 5.0);
        let xf = ViewXform { rect, zoom: 1.0, center: [400.0, 300.0], flip: false, rotation: 0.0, aspect: 1.0 };
        assert_eq!(wheel_anchor(&xf, p, [0, 0]), p, "no document: the pointer");
    }

    #[test]
    fn zoom_about_centres_the_clicked_point_with_the_preference() {
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        let p = pos2(600.0, 200.0);
        for ppp in [1.0f32, 2.0] {
            // `View::zoom` is physical; the transform works in points.
            let xf = ViewXform { rect, zoom: 1.0 / ppp, center: [400.0, 300.0], flip: false, rotation: 0.0, aspect: 1.0 };
            let doc = xf.to_doc(p);
            // Off: the document point under the pointer stays under it.
            let mut view = View { zoom: 1.0, center: [400.0, 300.0], ..Default::default() };
            zoom_about(&mut view, &xf, p, 2.0, false, ppp);
            let after = ViewXform { rect, zoom: 2.0 / ppp, center: view.center, flip: false, rotation: 0.0, aspect: 1.0 };
            let s = after.to_screen(doc[0] as f32, doc[1] as f32);
            assert!((s - p).length() < 0.5, "ppp {ppp}: {s:?} vs {p:?}");
            // On: the clicked point is the view centre.
            let mut view = View { zoom: 1.0, center: [400.0, 300.0], ..Default::default() };
            zoom_about(&mut view, &xf, p, 2.0, true, ppp);
            assert!((view.center[0] - doc[0] as f32).abs() < 0.5, "ppp {ppp}: {:?}", view.center);
            assert!((view.center[1] - doc[1] as f32).abs() < 0.5, "ppp {ppp}: {:?}", view.center);
        }
    }

    #[test]
    fn damage_grows_by_nested_effect_reach() {
        use photocraft_doc::{Effect, Layer};
        let fmt = photocraft_color::PixelFormat::RGBA8;
        assert_eq!(effect_reach(&[Layer::raster("plain", fmt)]), 0);
        let mut inner = Layer::raster("inner", fmt);
        inner.effects.items.push(Effect::default_drop_shadow());
        let m = photocraft_compose::effects::margin(&inner);
        let mut group = Layer::group("g", vec![inner.clone()]);
        group.effects.items.push(Effect::default_drop_shadow());
        assert_eq!(effect_reach(std::slice::from_ref(&inner)), m);
        // A child's edit moves the group's shape, whose effects reach further.
        assert_eq!(effect_reach(&[group.clone()]), 2 * m);
        group.visible = false;
        assert_eq!(effect_reach(&[group]), 0);
    }
}

#[cfg(test)]
mod document_window_title_tests;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod transform_controls_tests;

#[cfg(test)]
mod layer_edges_tests;

#[cfg(test)]
#[path = "canvas/move_readout_arrows_tests.rs"]
mod move_readout_arrows_tests;

#[cfg(test)]
mod live_frame_tests;

#[cfg(test)]
#[path = "canvas/art_history_shift_click_tests.rs"]
mod art_history_shift_click_tests;

#[cfg(test)]
mod rotation_preview_isolation_tests {
    use craft::ui::dialog::DialogId;
    use photocraft_engine::channel_cmds::ChannelTarget;
    use photocraft_engine::color_cmds::ColorExt;

    use super::*;

    /// Opens the filter dialog of `command` with its Preview box and returns its id. Rotate Canvas has no Preview
    /// (#14462); its size-changing preview (#1436) is reached through `open_previewing`.
    fn open(app: &mut PhotocraftApp, command: &str) -> DialogId {
        let id = crate::filter_dialog::open_previewing(app, command).expect("the dialog opens");
        // The Preview box is on from the dialog's first drawn pass.
        for _ in 0..2 {
            crate::smart_filter_dialog::tests::pass(app);
        }
        id
    }

    /// A harness running the app's dialogs, for the Preview box's clicks.
    fn dialogs_harness(app: Box<PhotocraftApp>) -> egui_kittest::Harness<'static, Box<PhotocraftApp>> {
        let mut h = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1200.0, 900.0))
            .with_context(app.ctx().clone())
            .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app);
        h.run_steps(2);
        h
    }

    /// Clicks the open dialog's Preview box.
    fn toggle_preview(h: &mut egui_kittest::Harness<'static, Box<PhotocraftApp>>) {
        use egui_kittest::kittest::Queryable;
        h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Preview").click();
        h.run_steps(2);
    }

    /// Writes a field of the hosted dialog `id`, as the form's control would.
    fn set(app: &mut PhotocraftApp, ctx: &egui::Context, id: DialogId, field: &str, value: serde_json::Value) {
        app.framework.dialogs.set_json_field(ctx, id, field, value).unwrap_or_else(|e| panic!("the dialog has a `{field}` field: {e}"));
    }

    #[test]
    fn channel_overlay_shows_filter_preview_and_restores_on_cancel() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.background_jobs = false;
        app.run("file.new", json!({"width":16,"height":16})).unwrap();
        app.run("channel.new", json!({"name":"Alpha","fill":"black"})).unwrap();
        app.session
            .edit("Fixture", |doc, _| {
                for y in 0..16 {
                    for x in 0..8 {
                        doc.channels[0].surface.write_pixel(x, y, &[1.0]);
                    }
                }
                Ok(())
            })
            .unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let ctx = app.ctx().clone();
        let dialog = open(&mut app, "filter.blur.gaussianBlur");
        set(&mut app, &ctx, dialog, "radius", json!(2.0));
        ensure_filter_preview(&mut app, 0, &ctx).unwrap();
        let st = app.session.active().unwrap();
        let original =
            crate::channel_view::render(&before, &st.channel_view, before.bounds(), 1, crate::channel_view::Look::default(), st.active_layer).unwrap();
        assert_ne!(overlay_pixels(&mut app, &ctx), original, "a nontrivial filter must be visible before OK");
        assert!(std::sync::Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
        let mut h = dialogs_harness(app);
        toggle_preview(&mut h);
        assert_eq!(overlay_pixels(h.state_mut(), &ctx), original, "Preview off restores the channel");
        toggle_preview(&mut h);
        let app = h.state_mut();
        crate::dialogs::cancel(app, &ctx, dialog).unwrap();
        assert_eq!(overlay_pixels(app, &ctx), original, "Cancel restores the channel");
        assert!(std::sync::Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    }

    /// The whole of document 0's channel view as the canvas draws it over the composite.
    fn overlay_pixels(app: &mut PhotocraftApp, ctx: &egui::Context) -> Vec<Color32> {
        use craft::ui::canvas::{PixelRect, RasterSource};
        let source = raster_source::OverlayPrep::new(app, ctx, 0, None, None).expect("a channel view").source(app);
        let size = source.size();
        let mut out = egui::ColorImage::filled([size.width as usize, size.height as usize], Color32::TRANSPARENT);
        source.read(PixelRect::new(0, 0, size.width as i32, size.height as i32), 1, &mut out);
        out.pixels
    }

    fn assert_filter_preview_target(target: ChannelTarget) {
        for depth in [8, 16, 32] {
            let mut session = photocraft_engine::Session::new();
            session.execute("file.new", json!({"width":16,"height":16,"depth":depth})).unwrap();
            session.execute("channel.new", json!({"name":"Alpha","fill":"black"})).unwrap();
            session
                .edit("Fixture", |doc, active| {
                    let layer = doc.layer_mut(active.unwrap()).unwrap();
                    layer.mask = Some(photocraft_doc::LayerMask::hide_all());
                    for y in 0..16 {
                        for x in 0..8 {
                            layer.surface_mut().unwrap().write_pixel(x, y, &[0.0, 0.25, 0.5, 1.0]);
                            layer.mask.as_mut().unwrap().surface.write_pixel(x, y, &[1.0]);
                        }
                    }
                    for y in 0..16 {
                        for x in 0..8 {
                            doc.channels[0].surface.write_pixel(x, y, &[1.0]);
                        }
                    }
                    Ok(())
                })
                .unwrap();
            session.active_mut().unwrap().channel_view.target = target;
            let mut app = PhotocraftApp::new(session, crate::Services::default());
            app.background_jobs = false;
            let ctx = app.ctx().clone();
            let dialog = open(&mut app, "filter.blur.gaussianBlur");
            set(&mut app, &ctx, dialog, "radius", json!(2.0));
            ensure_filter_preview(&mut app, 0, &ctx).unwrap();
            let preview = app.filter_preview.as_ref().unwrap().result.as_ref().unwrap().clone();
            crate::dialogs::confirm(&mut app, &ctx, dialog).unwrap();
            let final_doc = &app.session.active().unwrap().doc;
            let bounds = final_doc.bounds();
            assert_eq!(
                preview.layers[0].surface().unwrap().read_region(bounds),
                final_doc.layers[0].surface().unwrap().read_region(bounds),
                "RGB: {depth} {target:?}"
            );
            assert_eq!(preview.channels[0].surface.read_region(bounds), final_doc.channels[0].surface.read_region(bounds), "alpha: {depth} {target:?}");
            assert_eq!(
                preview.layers[0].mask.as_ref().unwrap().surface.read_region(bounds),
                final_doc.layers[0].mask.as_ref().unwrap().surface.read_region(bounds),
                "mask: {depth} {target:?}"
            );
        }
    }

    #[test]
    fn filter_preview_matches_the_applied_result_for_every_channel_target() {
        for target in [ChannelTarget::Alpha(0), ChannelTarget::Color(0), ChannelTarget::LayerMask] {
            assert_filter_preview_target(target);
        }
    }

    #[test]
    fn filter_preview_refreshes_when_channel_target_changes() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.background_jobs = false;
        app.run("file.new", json!({"width":16,"height":16})).unwrap();
        for name in ["First", "Second"] {
            app.run("channel.new", json!({"name":name,"fill":"black"})).unwrap();
        }
        app.session
            .edit("Fixture", |doc, _| {
                for y in 0..16 {
                    for x in 0..8 {
                        doc.channels[1].surface.write_pixel(x, y, &[1.0]);
                    }
                }
                Ok(())
            })
            .unwrap();
        app.session.active_mut().unwrap().channel_view.target = ChannelTarget::Alpha(0);
        let revision = app.session.active().unwrap().revision;
        let ctx = app.ctx().clone();
        let dialog = open(&mut app, "filter.blur.gaussianBlur");
        set(&mut app, &ctx, dialog, "radius", json!(2.0));
        let (first, first_key) = filter_channel_preview(&mut app, 0, &ctx).unwrap();
        app.session.active_mut().unwrap().channel_view.target = ChannelTarget::Alpha(1);
        let (second, second_key) = filter_channel_preview(&mut app, 0, &ctx).unwrap();
        assert_eq!(app.session.active().unwrap().revision, revision);
        assert_ne!(first_key, second_key);
        assert_ne!(first.channels[1].surface.read_region(first.bounds()), second.channels[1].surface.read_region(second.bounds()));
        let (unchanged, unchanged_key) = filter_channel_preview(&mut app, 0, &ctx).unwrap();
        assert!(std::sync::Arc::ptr_eq(&second, &unchanged));
        assert_eq!(second_key, unchanged_key);
        crate::dialogs::confirm(&mut app, &ctx, dialog).unwrap();
        let applied = &app.session.active().unwrap().doc;
        assert_eq!(second.channels[1].surface.read_region(second.bounds()), applied.channels[1].surface.read_region(applied.bounds()));
    }

    #[test]
    fn filter_preview_overlay_matches_quick_mask_and_mixed_color_views() {
        for quick_mask in [false, true] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.background_jobs = false;
            app.run("file.new", json!({"width":16,"height":16})).unwrap();
            app.session
                .edit("Fixture", |doc, active| {
                    if quick_mask {
                        doc.quick_mask =
                            Some(photocraft_doc::AlphaChannel::new("Quick Mask", photocraft_raster::Surface::new(photocraft_color::PixelFormat::GRAY8)));
                    }
                    for y in 0..16 {
                        for x in 0..8 {
                            if let Some(mask) = &mut doc.quick_mask {
                                mask.surface.write_pixel(x, y, &[1.0]);
                            } else {
                                doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().write_pixel(x, y, &[0.0, 0.25, 0.5, 1.0]);
                            }
                        }
                    }
                    Ok(())
                })
                .unwrap();
            if !quick_mask {
                let view = &mut app.session.active_mut().unwrap().channel_view;
                view.target = ChannelTarget::Color(0);
                view.set_color_visible(2, false);
            }
            let ctx = app.ctx().clone();
            let dialog = open(&mut app, "filter.blur.gaussianBlur");
            set(&mut app, &ctx, dialog, "radius", json!(2.0));
            let shown = overlay_pixels(&mut app, &ctx);
            crate::dialogs::confirm(&mut app, &ctx, dialog).unwrap();
            let st = app.session.active().unwrap();
            let expected =
                crate::channel_view::render(&st.doc, &st.channel_view, st.doc.bounds(), 1, crate::channel_view::Look::default(), st.active_layer).unwrap();
            assert_eq!(shown, expected, "quick_mask={quick_mask}");
        }
    }

    /// Assign and Convert to Profile have Photoshop's Preview. Assign changes only the profile, so
    /// its preview must be shown through the profile it assigns (through the document's own it
    /// looked unchanged); Convert keeps the look of in-gamut colours. Their `working` is the
    /// working space from Color Settings, not the built-in default.
    #[test]
    fn profile_dialogs_preview_through_the_profile_they_choose() {
        let mut session = photocraft_engine::Session::new();
        session.color_mut().settings.monitor_profile = "srgb".into();
        session.color_mut().settings.working_rgb = "display-p3".into();
        session.execute("file.new", json!({"width": 4, "height": 4})).unwrap();
        session.execute("edit.assignProfile", json!({"profile": "srgb"})).unwrap();
        session
            .edit("Fixture", |doc, _| {
                let layer = doc.layers.last_mut().unwrap();
                for y in 0..4 {
                    for x in 0..4 {
                        layer.surface_mut().unwrap().write_pixel(x, y, &[0.6, 0.35, 0.25, 1.0]);
                    }
                }
                Ok(())
            })
            .unwrap();
        let mut app = PhotocraftApp::new(session, crate::Services::default());
        let shown_pixels = |app: &PhotocraftApp, doc: &Document| {
            let (display, _) = canvas_display(app, doc, None);
            display_image(display.as_deref(), &photocraft_compose::flatten(doc)).pixels[0]
        };
        let original = app.session.active().unwrap().doc.clone();
        let before = shown_pixels(&app, &original);
        let preview = |app: &mut PhotocraftApp, command: &str, profile: &str| {
            let ctx = app.ctx().clone();
            let dialog = open(app, command);
            set(app, &ctx, dialog, "profile", json!(profile));
            ensure_filter_preview(app, 0, &ctx).expect("a preview");
            let result = app.filter_preview.as_ref().unwrap().result.clone().unwrap();
            let key = original.id.0 ^ (1u64 << 61);
            assert_eq!(filter_preview_display_doc(app, &original, key).icc_profile, result.icc_profile, "the canvas shows it through its own profile");
            assert_eq!(app.session.active().unwrap().doc.icc_profile, original.icc_profile, "previewing changes nothing");
            crate::dialogs::cancel(app, &ctx, dialog).unwrap();
            app.filter_preview = None;
            result
        };
        let description = |doc: &Document| photocraft_engine::color_cmds::profile_from_bytes(doc.icc_profile.as_ref().unwrap()).unwrap().description.clone();

        let assigned = preview(&mut app, "edit.assignProfile", "prophoto-compat");
        assert_eq!(description(&assigned), craft::cms::Builtin::ProPhotoCompat.description());
        let shown = shown_pixels(&app, &assigned);
        assert!(shown.r().abs_diff(before.r()) > 10 || shown.b().abs_diff(before.b()) > 10, "the assigned profile shows: {before:?} → {shown:?}");

        let converted = preview(&mut app, "edit.convertToProfile", "working");
        assert_eq!(description(&converted), craft::cms::Builtin::DisplayP3.description(), "`working` is Color Settings' working space");
        let shown = shown_pixels(&app, &converted);
        for (a, b) in [(shown.r(), before.r()), (shown.g(), before.g()), (shown.b(), before.b())] {
            assert!(a.abs_diff(b) <= 2, "converting keeps the look: {before:?} → {shown:?}");
        }
    }

    /// A CPU-only app must be able to generate the actual rotation preview without a
    /// GPU adapter; checking the result also verifies Preview off and document integrity.
    #[test]
    fn rotation_preview_is_computed_headlessly_without_mutating_document() {
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", json!({"width": 96, "height": 48})).unwrap();
        let mut app = PhotocraftApp::new(session, crate::Services::default());
        assert!(app.gpu.is_none());
        let ctx = app.ctx().clone();
        let dialog = open(&mut app, "image.rotation.arbitrary");
        set(&mut app, &ctx, dialog, "angle", json!(90.0));
        set(&mut app, &ctx, dialog, "direction", json!("cw"));

        let (factor, _, size) = ensure_filter_preview(&mut app, 0, &ctx).expect("CPU preview computed");
        assert_eq!(factor, 1);
        assert_eq!(size, [48, 96]);
        let original = &app.session.active().unwrap().doc;
        assert_eq!([original.size.width, original.size.height], [96, 48]);

        let mut h = dialogs_harness(app);
        toggle_preview(&mut h);
        let app = h.state_mut();
        assert!(ensure_filter_preview(app, 0, &ctx).is_none(), "disabling Preview must hide the preview");
        assert_eq!([app.session.active().unwrap().doc.size.width, app.session.active().unwrap().doc.size.height], [96, 48]);
    }

    /// A dialog operates on the active document and cannot render a preview into a
    /// background tab, even if that tab has a valid canvas index.
    #[test]
    fn filter_preview_is_not_returned_for_inactive_document() {
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", json!({"width": 96, "height": 48})).unwrap();
        let mut app = PhotocraftApp::new(session, crate::Services::default());
        let ctx = app.ctx().clone();
        let dialog = open(&mut app, "image.rotation.arbitrary");
        set(&mut app, &ctx, dialog, "angle", json!(90.0));
        assert!(ensure_filter_preview(&mut app, 0, &ctx).is_some());

        app.session.execute("file.new", json!({"width": 40, "height": 20})).unwrap();
        assert_eq!(app.session.active_index(), Some(1));
        assert!(ensure_filter_preview(&mut app, 0, &ctx).is_none(), "an inactive tab must never inherit the command preview");
    }
}

#[cfg(test)]
mod zoom_animation_tests {
    use craft::behavior::CanvasView;
    use craft::behavior::kurbo::Point;

    use super::*;

    /// A view of a 1000×700 canvas at `zoom` on `center`, turned by `rotation` degrees.
    fn camera(zoom: f64, center: Point, rotation: f64) -> CanvasView {
        CanvasView::new(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 700.0)), center, zoom).rotation(rotation)
    }

    /// Deferred uploads prioritize both visible areas throughout a recentering zoom.
    #[test]
    fn deferred_redraw_focus_covers_target_and_displayed_cameras() {
        for rotation in [0.0, 37.0] {
            let initial = camera(1.0, Point::new(200.0, 150.0), rotation);
            let target = camera(2.0, Point::new(1600.0, 900.0), rotation);
            let target_visible = visible_doc_rect(&target);
            // The cameras a zoom ease shows on its way from `initial` to `target`.
            for t in [0.0f64, 0.5, 1.0] {
                let lerp = |a: f64, b: f64| a + (b - a) * t;
                let center = Point::new(lerp(initial.center.x, target.center.x), lerp(initial.center.y, target.center.y));
                let displayed = camera(lerp(initial.zoom, target.zoom), center, rotation);
                let displayed_visible = visible_doc_rect(&displayed);
                let focus = redraw_focus(&target, &displayed);
                assert_eq!(focus.intersect(&target_visible), target_visible);
                assert_eq!(focus.intersect(&displayed_visible), displayed_visible);
                if t == 0.0 {
                    assert_ne!(focus, target_visible, "the old view must also receive priority");
                }
            }
            assert_eq!(redraw_focus(&target, &target), target_visible, "settled views keep the same focus");
        }
    }
}
#[cfg(test)]
mod display_memo_tests;

#[cfg(test)]
mod cpu_background_gpu_tests;

#[cfg(test)]
#[path = "canvas/blend_viewport_gpu_tests.rs"]
mod blend_viewport_gpu_tests;

#[cfg(test)]
#[path = "canvas/pixel_alignment_tests.rs"]
mod pixel_alignment_tests;

#[cfg(test)]
mod multi_channel_ui_tests {
    use super::*;
    fn app() -> Box<PhotocraftApp> {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width":64,"height":64})).unwrap();
        app.sync_views();
        app.run("channel.target", json!({"colors":[0,2]})).unwrap();
        app
    }
    #[test]
    fn pixel_tools_capture_the_whole_channel_set_and_gradients_stay_on_pixels() {
        let mut app = app();
        assert_eq!(paint_target(&app), json!({"colors":[0,2]}));
        app.ui.tool = Tool::Gradient;
        app.ui.tool_options.gradient_classic = false;
        assert!(!crate::gradient_ui::live_mode(&app));
    }
    #[test]
    fn viewing_a_layer_mask_overrides_colour_targets_for_live_brush_strokes() {
        for depth in [8, 16, 32] {
            for view in ["gray", "overlay"] {
                for colors in [vec![0], vec![0, 2]] {
                    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
                    app.run("file.new", json!({"width":64,"height":64,"depth":depth})).unwrap();
                    app.run("layer.layerMask.revealAll", json!({})).unwrap();
                    app.run("channel.target", json!({"colors":colors})).unwrap();
                    app.run("view.layerMask", json!({"mode":view})).unwrap();
                    assert_eq!(paint_target(&app), json!("mask"), "{depth}-bit {view} {colors:?}");
                    assert!(app.layer_mask_targeted());
                    app.run("tools.setColors", json!({"foreground":[0.0,0.0,0.0,1.0]})).unwrap();
                    app.run("tools.setBrush", json!({"brush":{"size":12,"hardness":1.0,"smoothing":{"amount":0.0}}})).unwrap();
                    app.ui.tool = Tool::Brush;
                    let st = app.session.active().unwrap();
                    let layer = st.active_layer.unwrap();
                    let original = st.doc.layer(layer).unwrap().surface().unwrap().clone();
                    let steps = st.history.past_len();
                    tool_event(&mut app, ToolEvent::Down { x: 12.0, y: 32.0, pressure: 1.0 }, egui::Modifiers::NONE);
                    tool_event(&mut app, ToolEvent::Move { x: 32.0, y: 32.0, pressure: 1.0 }, egui::Modifiers::NONE);
                    let shown = display_doc(&mut app, 0).0;
                    let shown_layer = shown.layer(layer).unwrap();
                    assert_eq!(shown_layer.surface().unwrap(), &original);
                    assert!(shown_layer.mask.as_ref().unwrap().surface.sample_channel(28, 32, 0) < 0.1);
                    tool_event(&mut app, ToolEvent::Up { x: 32.0, y: 32.0 }, egui::Modifiers::NONE);
                    let st = app.session.active().unwrap();
                    let committed = st.doc.layer(layer).unwrap();
                    assert_eq!(committed.surface().unwrap(), &original);
                    assert_eq!(committed.mask.as_ref().unwrap().surface, shown_layer.mask.as_ref().unwrap().surface);
                    assert_eq!(st.history.past_len(), steps + 1);
                    app.run("channel.target", json!({"colors":colors})).unwrap();
                    assert_eq!(paint_target(&app), json!({"colors":colors}));
                    assert!(!app.layer_mask_targeted());
                }
            }
        }
    }
    /// The colour rows stay selected when another layer becomes active. An adjustment layer has no
    /// pixels, so the Brush paints its mask, as it does without colour rows selected.
    #[test]
    fn colour_rows_left_selected_still_paint_an_adjustment_layers_mask() {
        for colors in [vec![0], vec![0, 2]] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.run("file.new", json!({"width":64,"height":64})).unwrap();
            app.run("channel.target", json!({"colors":colors})).unwrap();
            app.run("layer.newAdjustmentLayer.levels", json!({})).unwrap();
            app.sync_views();
            assert_eq!(paint_target(&app), json!("mask"), "{colors:?}");
            app.run("tools.setColors", json!({"foreground":[0.0,0.0,0.0,1.0]})).unwrap();
            app.run("tools.setBrush", json!({"brush":{"size":12,"hardness":1.0,"smoothing":{"amount":0.0}}})).unwrap();
            app.ui.tool = Tool::Brush;
            let st = app.session.active().unwrap();
            let layer = st.active_layer.unwrap();
            let steps = st.history.past_len();
            tool_event(&mut app, ToolEvent::Down { x: 12.0, y: 32.0, pressure: 1.0 }, egui::Modifiers::NONE);
            tool_event(&mut app, ToolEvent::Move { x: 32.0, y: 32.0, pressure: 1.0 }, egui::Modifiers::NONE);
            tool_event(&mut app, ToolEvent::Up { x: 32.0, y: 32.0 }, egui::Modifiers::NONE);
            let st = app.session.active().unwrap();
            assert_eq!(st.history.past_len(), steps + 1, "{colors:?}: one stroke");
            assert!(st.doc.layer(layer).unwrap().mask.as_ref().unwrap().surface.sample_channel(28, 32, 0) < 0.1, "{colors:?}: the mask was painted");
        }
    }

    /// Targeting a layer mask with colour rows selected (its thumbnail, or adding the mask) moves
    /// the Channels panel onto the mask, as in Photoshop: tools, dialogs and menu commands then
    /// all edit the mask. Several rows still stop the Move tool on a layer that needs
    /// rasterizing, which paints only after the prompt.
    #[test]
    fn targeting_a_mask_over_colour_rows_edits_the_mask_everywhere() {
        let set = |app: &mut PhotocraftApp, p: serde_json::Value| {
            let (req, _) = crate::control::ControlRequest::new("ui.set", p);
            let ctx = app.ctx().clone();
            assert!(matches!(crate::control::handle(app, &ctx, &req), crate::control::Reply::Done(v) if v["ok"] != false));
        };
        for colors in [vec![0], vec![0, 2]] {
            for add in [false, true] {
                let mut app = self::app();
                if !add {
                    app.run("layer.layerMask.revealAll", json!({})).unwrap();
                    set(&mut app, json!({"maskTarget": false}));
                }
                app.run("channel.target", json!({"colors":colors})).unwrap();
                app.sync_views();
                assert!(paint_target(&app).get("colors").is_some(), "{colors:?}: the rows are the target first");
                let journal = app.session.journal.len();
                if add {
                    app.run("layer.layerMask.revealAll", json!({})).unwrap();
                } else {
                    set(&mut app, json!({"maskTarget": true}));
                }
                app.sync_views();
                assert_eq!(app.session.journal.len(), journal + usize::from(add), "{colors:?} add={add}: no extra recorded step");
                assert_eq!(paint_target(&app), json!("mask"), "{colors:?} add={add}");
                assert!(app.layer_mask_targeted(), "{colors:?} add={add}");
                assert_eq!(app.with_mask_target("filter.blur.gaussianBlur", json!({}))["target"], "mask");
                assert!(crate::distort_ui::colour_set_target(&app).is_null());
                assert!(!multiple_color_target(&app));
                // Clicking a colour row again takes the rows back.
                app.run("channel.target", json!({"colors":colors})).unwrap();
                app.sync_views();
                assert!(paint_target(&app).get("colors").is_some(), "{colors:?} add={add}: the rows again");
            }
        }
        // A mask added to another layer, or a fill layer's mask, leaves the rows targeted.
        let mut app = self::app();
        let active = app.session.active().unwrap().active_layer.unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        let other = app.session.active().unwrap().active_layer.unwrap();
        app.run("layer.select", json!({"layer": active.0})).unwrap();
        app.run("layer.layerMask.revealAll", json!({})).unwrap();
        app.run("channel.target", json!({"colors":[0,2]})).unwrap();
        app.run("layer.layerMask.revealAll", json!({"layer": other.0})).unwrap();
        app.sync_views();
        assert_eq!(paint_target(&app), json!({"colors":[0,2]}), "a mask on another layer");
        app.run("layer.newFillLayer.solidColor", json!({})).unwrap();
        app.run("channel.target", json!({"colors":[0,2]})).unwrap();
        set(&mut app, json!({"maskTarget": true}));
        app.sync_views();
        assert!(matches!(app.session.active().unwrap().channel_view.target, photocraft_engine::channel_cmds::ChannelTarget::Color(_)), "a fill layer");
        let mut app = self::app();
        app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        app.run("channel.target", json!({"colors":[0,2]})).unwrap();
        app.sync_views();
        assert_eq!(paint_target(&app), json!("pixels"));
        assert!(crate::rasterize_prompt::needed(&app).is_some());
        assert!(multiple_color_target(&app));
        let ctx = app.ctx().clone();
        let refused = crate::liquify_ui::open(&mut app, &ctx).unwrap_err();
        assert!(refused.contains("Rasterize"), "{refused}");
    }

    #[test]
    fn subset_does_not_block_vector_path_transforms() {
        let mut app = app();
        app.run("path.set", json!({"name":"work","path":{"subpaths":[{"closed":true,"knots":[[10,10],[30,10],[30,20],[10,20]]}]}})).unwrap();
        app.ui.tool = Tool::PathSelection;
        assert!(crate::menus::is_enabled(&app, "edit.freeTransform"));
        let ctx = app.ctx().clone();
        crate::transform_tool::begin(&mut app, &ctx).unwrap();
        assert!(app.ui.transform.as_ref().unwrap().path.is_some());
    }
}

#[cfg(test)]
#[path = "pen_gesture_owner_tests.rs"]
mod pen_gesture_owner_tests;

#[cfg(test)]
#[path = "path_selection_cursor_tests.rs"]
mod path_selection_cursor_tests;
