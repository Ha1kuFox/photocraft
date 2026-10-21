//! Paint Bucket options: sampling mode, reference layer, Expand and Close gaps.

use egui_kittest::kittest::NodeT;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

fn app_with_layers() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.prefs.edit(|p| p.interface.language = "en".into());
    app.run("file.new", json!({"width": 40, "height": 30})).unwrap();
    app.run("layer.new.layer", json!({"name": "Lineart"})).unwrap();
    app.ui.tool = Tool::PaintBucket;
    app
}

#[test]
fn click_params_default_to_the_current_layer() {
    let mut app = app_with_layers();
    let p = crate::bucket_ui::click_params(&mut app, 3.0, 4.0);
    assert_eq!(p["sample"], json!("active"));
    assert_eq!(p["expand"], json!(0.0));
    assert_eq!(p["closeGap"], json!(0.0));
    assert_eq!(p["x"], json!(3.0));
}

#[test]
fn click_params_clamp_expand_and_close_gap() {
    let mut app = app_with_layers();
    app.ui.tool_options.bucket_expand = 999.0;
    app.ui.tool_options.bucket_close_gap = -5.0;
    let p = crate::bucket_ui::click_params(&mut app, 3.0, 4.0);
    assert_eq!(p["expand"], json!(50.0));
    assert_eq!(p["closeGap"], json!(0.0));
}

#[test]
fn click_params_sample_layer_and_stale_fallback() {
    let mut app = app_with_layers();
    let st = app.session.active().unwrap();
    let lineart = st.doc.layers.iter().find(|l| l.name == "Lineart").unwrap().id.0;
    app.ui.tool_options.bucket_sample = "layer".into();
    app.ui.tool_options.bucket_sample_layer = lineart;
    let p = crate::bucket_ui::click_params(&mut app, 3.0, 4.0);
    assert_eq!(p["sample"], json!({"layer": lineart}));
    // A deleted choice falls back to the current layer with a status note.
    app.ui.tool_options.bucket_sample_layer = 99999;
    let p = crate::bucket_ui::click_params(&mut app, 3.0, 4.0);
    assert_eq!(p["sample"], json!("active"));
    assert!(app.ui.status.contains("reference layer"), "status note: {}", app.ui.status);
    // "layer" with no choice yet is the same fallback.
    app.ui.tool_options.bucket_sample_layer = 0;
    let p = crate::bucket_ui::click_params(&mut app, 3.0, 4.0);
    assert_eq!(p["sample"], json!("active"));
    assert!(app.ui.status.contains("reference layer"), "status note: {}", app.ui.status);
}

/// Options-bar harness in the Studio (non-pro) theme, like the brush panel tests.
fn options_bar_harness() -> egui_kittest::Harness<'static, PhotocraftApp> {
    let app = app_with_layers();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1400.0, 60.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::panels::options_bar(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(4);
    h
}

#[test]
fn options_bar_has_sampling_expand_and_close_gaps() {
    let mut h = options_bar_harness();
    for label in ["Fill", "Opacity", "Tolerance", "Anti-alias", "Contiguous", "Sample", "Expand Area", "Close Gaps"] {
        assert!(h.query_by_label(label).is_some(), "options bar shows {label}");
    }
    // No reference-layer dropdown until that mode is picked.
    assert!(h.query_by_label("Lineart").is_none());
    h.get_all_by_role(egui::accesskit::Role::ComboBox)
        .find(|c| c.accesskit_node().value().as_deref() == Some("Current Layer"))
        .expect("the Sample dropdown")
        .click();
    h.run_steps(3);
    h.key_press(egui::Key::ArrowDown);
    h.run_steps(3);
    assert_eq!(h.state().ui.tool_options.bucket_sample, "all");
    h.key_press(egui::Key::ArrowDown);
    h.run_steps(3);
    assert_eq!(h.state().ui.tool_options.bucket_sample, "layer");
    // The layer dropdown lists the document's pixel layers.
    let lineart = h.state().session.active().unwrap().doc.layers.iter().find(|l| l.name == "Lineart").unwrap().id.0;
    h.state_mut().ui.tool_options.bucket_sample_layer = lineart;
    h.run_steps(3);
    assert!(
        h.query_all_by_role(egui::accesskit::Role::ComboBox).any(|c| c.accesskit_node().value().as_deref() == Some("Lineart")),
        "the reference-layer dropdown shows the picked layer"
    );
}
