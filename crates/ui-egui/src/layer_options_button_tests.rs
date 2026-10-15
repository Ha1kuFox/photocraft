//! The Layers header's "···" button: pens can't right-click, so it opens the row menu for
//! the selected layer.

use egui::{vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;

fn harness() -> Harness<'static, PhotocraftApp> {
    let mut session = photocraft_engine::Session::new();
    session.execute("prefs.set", json!({"path": "interface.language", "value": "en"})).unwrap();
    session.execute("file.new", json!({"width": 64, "height": 64, "background": "white"})).unwrap();
    let mut h = Harness::builder().with_size(vec2(300.0, 400.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::i18n::with_language(crate::i18n::Lang::EN, || {
                app.ppp = ui.ctx().pixels_per_point();
                super::layers(app, ui);
            });
        },
        PhotocraftApp::new(session, Default::default()),
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
    h.run_steps(4);
    h
}

#[test]
fn ellipsis_opens_the_selected_layers_row_menu() {
    let mut h = harness();
    // No "Lock:" text anymore; the "···" button sits in its place.
    assert!(h.query_by_label("Lock:").is_none());
    assert!(h.query_by_label("Lock").is_none());
    h.get_by_label("Layer options").click();
    h.run_steps(3);
    // The same menu as a row right-click: layer actions plus Rename.
    for label in ["Duplicate Layer…", "Delete Layer", "Rename Layer…"] {
        assert!(h.query_by_label(label).is_some(), "the menu shows {label}");
    }
}

#[test]
fn ellipsis_menu_acts_on_the_selected_layer() {
    let mut h = harness();
    h.get_by_label("Layer options").click();
    h.run_steps(3);
    // "Duplicate Layer…" runs through the menus like its menu-bar twin: with no params it
    // opens the Duplicate dialog for the selected layer.
    h.get_by_label("Duplicate Layer…").click();
    h.run_steps(6);
    assert_eq!(h.state().ui.dialogs.len(), 1, "the Duplicate dialog opened");
    let fields = &h.state().ui.dialogs[0].fields;
    assert_eq!(fields.get("name").and_then(|v| v.as_str()), Some("Background copy"));
}
