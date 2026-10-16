//! Studio-layout options bars: marquee, lasso, crop and perspective-crop tools
//! show the same controls as the Pro layout (with the hint), not just the hint.

use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

/// Options-bar harness in the Studio (non-pro) theme, like the bucket UI tests.
fn options_bar_harness(tool: Tool) -> egui_kittest::Harness<'static, PhotocraftApp> {
    options_bar_harness_themed(tool, crate::theme::ThemeKind::Studio)
}

/// Options-bar harness in an explicit theme.
fn options_bar_harness_themed(
    tool: Tool,
    theme: crate::theme::ThemeKind,
) -> egui_kittest::Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.prefs.edit(|p| p.interface.language = "en".into());
    app.run("file.new", json!({"width": 40, "height": 30})).unwrap();
    app.ui.tool = tool;
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1600.0, 60.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::panels::options_bar(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, theme);
    h.run_steps(4);
    h
}

#[test]
fn studio_marquee_bar_has_feather_style_and_select_and_mask() {
    let h = options_bar_harness(Tool::RectMarquee);
    for label in ["Feather", "Style", "Select and Mask…"] {
        assert!(h.query_by_label(label).is_some(), "options bar shows {label}");
    }
}

#[test]
fn studio_lasso_bar_has_feather_and_anti_alias() {
    let h = options_bar_harness(Tool::Lasso);
    for label in ["Feather", "Anti-alias", "Select and Mask…"] {
        assert!(h.query_by_label(label).is_some(), "options bar shows {label}");
    }
}

#[test]
fn studio_magnetic_lasso_bar_has_edge_controls() {
    let h = options_bar_harness(Tool::MagneticLasso);
    for label in ["Width", "Contrast", "Frequency"] {
        assert!(h.query_by_label(label).is_some(), "options bar shows {label}");
    }
}

#[test]
fn studio_crop_bar_has_fill_and_commit_controls() {
    let h = options_bar_harness(Tool::Crop);
    for label in ["Delete Cropped Pixels", "Clear"] {
        assert!(h.query_by_label(label).is_some(), "options bar shows {label}");
    }
}

#[test]
fn studio_perspective_crop_bar_has_grid_and_front_image() {
    let h = options_bar_harness(Tool::PerspectiveCrop);
    for label in ["Show Grid", "Front Image", "Clear"] {
        assert!(h.query_by_label(label).is_some(), "options bar shows {label}");
    }
}

#[test]
fn studio_move_bar_has_target_and_align_controls() {
    use egui_kittest::kittest::NodeT;
    let h = options_bar_harness(Tool::Move);
    for label in ["Auto-Select:", "Show Transform Controls", "Align and distribute"] {
        assert!(h.query_by_label(label).is_some(), "options bar shows {label}");
    }
    assert!(
        h.query_all_by_role(egui::accesskit::Role::ComboBox).any(|c| c.accesskit_node().value().as_deref() == Some("Group")),
        "the target dropdown offers Group"
    );
}

#[test]
fn studio_brush_bar_has_airbrush_and_smoothing_options() {
    let h = options_bar_harness(Tool::Brush);
    for label in ["Enable airbrush-style build-up effects", "Set additional smoothing options"] {
        assert!(h.query_by_label(label).is_some(), "options bar shows {label}");
    }
}

#[test]
fn studio_bar_has_home_toggle() {
    let h = options_bar_harness(Tool::Move);
    assert!(h.query_by_label("Home").is_some(), "options bar shows Home");
}

#[test]
fn hold_to_correct_toggle_in_both_layouts_and_flips_the_setting() {
    use egui_kittest::kittest::Queryable;
    for theme in [crate::theme::ThemeKind::Pro, crate::theme::ThemeKind::Studio] {
        let h = options_bar_harness_themed(Tool::Brush, theme);
        assert!(h.query_by_label("Hold to correct").is_some(), "{theme:?} options bar shows Hold to correct");
    }
    let mut h = options_bar_harness(Tool::Brush);
    assert!(h.state().ui.tool_options.hold_correct);
    h.get_by_label("Hold to correct").click();
    h.run_steps(2);
    assert!(!h.state().ui.tool_options.hold_correct, "clicking the toggle disables hold-to-correct");
}
