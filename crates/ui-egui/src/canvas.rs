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
mod doc_window_dialog_tests;

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
