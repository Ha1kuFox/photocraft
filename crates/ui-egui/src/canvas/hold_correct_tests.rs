//! Hold-to-correct through the canvas tool host: draw, dwell, release for two undo steps.

use egui::{Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;
use crate::app::framework::BuildApp as _;
use crate::state::Tool;

fn harness() -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 200, "background": "transparent"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.ui.tool = Tool::Brush;
    app.ui.tool_options.hold_correct = true;
    app.ui.tool_options.hold_correct_ms = 100;
    {
        let b = &mut app.session.tools.brush;
        b.size = 9.0;
        b.hardness = 1.0;
        b.pressure_size = false;
        b.opacity = 1.0;
        b.flow = 1.0;
        b.smoothing.amount = 0.0;
    }
    app.sync_views();
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1200.0, 800.0)).with_step_dt(1.0 / 60.0).build_app(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    h.run_steps(4);
    h
}

fn screen(h: &Harness<'static, Box<PhotocraftApp>>, at: [f64; 2]) -> Pos2 {
    let v = h.state().ui.views[0].clone();
    let r = h.state().last_canvas_rect;
    r.center() + vec2((at[0] as f32 - v.center[0]) * v.zoom, (at[1] as f32 - v.center[1]) * v.zoom)
}

fn button(h: &mut Harness<'static, Box<PhotocraftApp>>, at: Pos2, pressed: bool) {
    h.event(egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

fn move_to(h: &mut Harness<'static, Box<PhotocraftApp>>, at: [f64; 2]) {
    h.event(egui::Event::PointerMoved(screen(h, at)));
    h.run_steps(1);
}

fn history(h: &Harness<'static, Box<PhotocraftApp>>) -> usize {
    h.state().session.active().unwrap().history.past_len()
}

fn layer_alpha(h: &Harness<'static, Box<PhotocraftApp>>, x: i32, y: i32) -> f32 {
    let st = h.state().session.active().unwrap();
    let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
    layer.surface().unwrap().rgba(x, y)[3]
}

/// A wobbly horizontal stroke ending near (150, 50).
fn draw_wobbly(h: &mut Harness<'static, Box<PhotocraftApp>>) {
    let start = screen(h, [10.0, 50.0]);
    h.event(egui::Event::PointerMoved(start));
    h.run_steps(1);
    button(h, start, true);
    for i in 1..=20 {
        let y = if i % 2 == 0 { 53.0 } else { 47.0 };
        move_to(h, [10.0 + i as f64 * 7.0, y]);
    }
}

#[test]
fn dwell_fits_and_release_commits_two_undo_steps() {
    let mut h = harness();
    let before = history(&h);
    draw_wobbly(&mut h);
    assert!(h.state().canvas_tools.paint.stroke.as_ref().is_some_and(|s| s.correcting.is_none()), "drawing does not correct");
    // The pointer rests; canvas passes advance the dwell on the tool clock.
    h.run_steps(10);
    assert!(h.state().canvas_tools.paint.stroke.as_ref().is_some_and(|s| s.correcting.is_some()), "the hold fits the stroke");

    move_to(&mut h, [150.0, 90.0]);
    let up = screen(&h, [150.0, 90.0]);
    button(&mut h, up, false);
    assert_eq!(history(&h), before + 2, "raw stroke then corrected stroke");
    let ids: Vec<&str> = h.state().session.journal.iter().map(|(id, _)| id.as_str()).collect();
    assert!(ids.contains(&"paint.correctStroke"), "journal: {ids:?}");
    assert!(h.state().canvas_tools.paint.stroke.is_none());
    assert!(layer_alpha(&h, 80, 70) > 0.5, "the stretched fitted line shows");
    assert_eq!(layer_alpha(&h, 80, 53), 0.0, "the raw wobble was replaced");
    assert!(h.state_mut().session.undo(), "undo the fit");
    assert!(layer_alpha(&h, 80, 53) > 0.0, "the raw stroke is back");
    assert_eq!(layer_alpha(&h, 80, 70), 0.0, "the fit is gone");
    assert!(h.state_mut().session.undo(), "undo the raw stroke");
    assert_eq!(layer_alpha(&h, 80, 53), 0.0, "the stroke is cleared");
}

#[test]
fn quick_release_without_dwell_commits_one_step() {
    let mut h = harness();
    let before = history(&h);
    draw_wobbly(&mut h);
    let up = screen(&h, [150.0, 50.0]);
    button(&mut h, up, false);
    assert_eq!(history(&h), before + 1, "no dwell, no correction");
    assert!(!h.state().session.journal.iter().any(|(id, _)| id == "paint.correctStroke"));
}
