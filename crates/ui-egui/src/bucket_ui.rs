//! Paint Bucket options bar: Fill source, Opacity, Tolerance, Anti-alias, Contiguous,
//! sampling (the current layer, all layers, or one named reference layer), Expand and Close gaps.

#[cfg(test)]
#[path = "bucket_ui_tests.rs"]
mod tests;

use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;
use crate::theme::Tokens;
use crate::{widgets};

fn opt(ui: &mut egui::Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(tl!(&text)).color(t.text_dim).size(12.0));
}

/// Named reference layers for the "Reference layer" sampling mode: visible pixel layers as
/// `(id, name)`, innermost last (so a same-named layer reads unambiguously).
fn reference_layers(app: &PhotocraftApp) -> Vec<(u64, String)> {
    let Some(st) = app.session.active() else { return Vec::new() };
    st.doc.layers.iter().filter(|l| l.surface().is_some()).map(|l| (l.id.0, l.name.clone())).collect()
}

/// The `paint.bucket` params for a click at document `(x, y)`. A stale reference-layer choice
/// (deleted, or without pixels now) falls back to the current layer with a status note.
pub fn click_params(app: &mut PhotocraftApp, x: f64, y: f64) -> serde_json::Value {
    let o = app.ui.tool_options.clone();
    let contents = if o.bucket_fill_pattern { "pattern" } else { "foreground" };
    let mut p = json!({
        "x": x.floor(),
        "y": y.floor(),
        "tolerance": o.tolerance,
        "contiguous": o.contiguous,
        "antiAlias": o.anti_alias,
        "opacity": o.fill_opacity,
        "contents": contents,
        "expand": o.bucket_expand.clamp(0.0, 50.0),
        "closeGap": o.bucket_close_gap.clamp(0.0, 20.0),
        "target": crate::canvas::paint_target(app),
    });
    let sample = match o.bucket_sample.as_str() {
        "all" => json!("all"),
        "layer" if o.bucket_sample_layer != 0 => json!({"layer": o.bucket_sample_layer}),
        "layer" => {
            app.ui.status = tl!("No reference layer chosen — sampling the current layer").into();
            app.ui.status_error = false;
            json!("active")
        }
        _ => json!("active"),
    };
    // A chosen layer that is gone (or lost its pixels) falls back with a note, as agreed.
    if sample.get("layer").is_some()
        && reference_layers(app).iter().all(|(id, _)| *id != o.bucket_sample_layer)
    {
        app.ui.status = tl!("The reference layer is gone — sampling the current layer").into();
        app.ui.status_error = false;
        p["sample"] = json!("active");
    } else {
        p["sample"] = sample;
    }
    p
}

/// Options bar for the Paint Bucket; shared by the Pro and Studio layouts.
pub fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if app.ui.tool != Tool::PaintBucket {
        return;
    }
    // The layer list borrows the session; take it before the options borrow below.
    let layers = if app.ui.tool_options.bucket_sample == "layer" { reference_layers(app) } else { Vec::new() };
    let o = &mut app.ui.tool_options;
    opt(ui, tl!("Fill"));
    let mut src = u8::from(o.bucket_fill_pattern);
    if widgets::dropdown(ui, "bucket-src", &mut src, &[(0u8, tl!("Foreground")), (1, tl!("Pattern"))], 100.0) {
        o.bucket_fill_pattern = src == 1;
    }
    opt(ui, tl!("Opacity"));
    widgets::value_field(ui, &mut o.fill_opacity, 1.0..=100.0, "%", 62.0);
    opt(ui, tl!("Tolerance"));
    widgets::value_field(ui, &mut o.tolerance, 0.0..=255.0, "", 58.0);
    widgets::checkbox(ui, &mut o.anti_alias, tl!("Anti-alias"));
    widgets::checkbox(ui, &mut o.contiguous, tl!("Contiguous"));
    widgets::vline(ui, 22.0);
    opt(ui, tl!("Sample"));
    let mut mode: u8 = match o.bucket_sample.as_str() {
        "all" => 1,
        "layer" => 2,
        _ => 0,
    };
    if widgets::dropdown(
        ui,
        "bucket-sample",
        &mut mode,
        &[(0u8, tl!("Current Layer")), (1, tl!("All Layers")), (2, tl!("Reference Layer"))],
        130.0,
    ) {
        o.bucket_sample = ["active", "all", "layer"][mode as usize].into();
    }
    if o.bucket_sample == "layer" {
        let refs: Vec<(u64, &str)> = layers.iter().map(|(id, name)| (*id, name.as_str())).collect();
        // An unmatched choice (deleted layer) reads "—" until a new one is picked; the click
        // itself falls back with a status note (see `click_params`).
        widgets::dropdown(ui, "bucket-sample-layer", &mut o.bucket_sample_layer, &refs, 130.0);
    }
    widgets::vline(ui, 22.0);
    opt(ui, tl!("Expand Area"));
    widgets::value_field(ui, &mut o.bucket_expand, 0.0..=50.0, "px", 58.0);
    opt(ui, tl!("Close Gaps"));
    widgets::value_field(ui, &mut o.bucket_close_gap, 0.0..=20.0, "px", 58.0);
}
