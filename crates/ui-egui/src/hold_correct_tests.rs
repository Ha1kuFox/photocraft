//! Hold-to-correct end to end: draw, dwell, move the fit, release for two undo steps.
//!
//! The stylus clock is driven by hand (`stylus.clock_ms`), so the dwell is deterministic:
//! no wall-clock sleeps, no repaint loop.

use egui::Modifiers;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200, "background": "transparent"})).unwrap();
    app.ui.tool = Tool::Brush;
    assert!(app.ui.tool_options.hold_correct, "hold-to-correct is on by default");
    app.ui.tool_options.hold_correct_ms = 100;
    let b = &mut app.session.tools.brush;
    b.size = 9.0;
    b.hardness = 1.0;
    b.pressure_size = false;
    b.opacity = 1.0;
    b.flow = 1.0;
    b.smoothing.amount = 0.0;
    app
}

fn layer_alpha(app: &PhotocraftApp, x: i32, y: i32) -> f32 {
    let st = app.session.active().unwrap();
    let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
    layer.surface().unwrap().rgba(x, y)[3]
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

/// A wobbly horizontal stroke ending near (150, 50); the clock advances `dt` per move.
fn draw_wobbly(app: &mut PhotocraftApp, clock: &mut f64, dt: f64) {
    tool_event(app, ToolEvent::Down { x: 10.0, y: 50.0, pressure: 1.0 }, Modifiers::NONE);
    for i in 1..=20 {
        *clock += dt;
        app.stylus.clock_ms = *clock;
        let y = 50.0 + if i % 2 == 0 { 3.0 } else { -3.0 };
        tool_event(app, ToolEvent::Move { x: 10.0 + i as f64 * 7.0, y, pressure: 1.0 }, Modifiers::NONE);
    }
}

/// Hold the pointer still: same position, advancing clock.
fn hold(app: &mut PhotocraftApp, clock: &mut f64, at: [f64; 2], ms: f64) {
    *clock += ms;
    app.stylus.clock_ms = *clock;
    tool_event(app, ToolEvent::Move { x: at[0], y: at[1], pressure: 1.0 }, Modifiers::NONE);
}

#[test]
fn dbg_plain_undo() {
    let mut app = app();
    let mut clock = 1000.0;
    app.stylus.clock_ms = clock;
    tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 50.0, pressure: 1.0 }, Modifiers::NONE);
    for i in 1..=10 {
        clock += 20.0;
        app.stylus.clock_ms = clock;
        tool_event(&mut app, ToolEvent::Move { x: 10.0 + i as f64 * 7.0, y: 50.0, pressure: 1.0 }, Modifiers::NONE);
    }
    tool_event(&mut app, ToolEvent::Up { x: 80.0, y: 50.0 }, Modifiers::NONE);
    eprintln!("plain steps={} entries={:?} a={}", steps(&app), app.session.active().unwrap().history.entries(), layer_alpha(&app, 40, 50));
    let u = app.session.undo();
    eprintln!("plain undo={u} a={}", layer_alpha(&app, 40, 50));
}

#[test]
fn dbg_undo_flow() {
    let mut app = app();
    let mut clock = 1000.0;
    app.stylus.clock_ms = clock;
    draw_wobbly(&mut app, &mut clock, 20.0);
    hold(&mut app, &mut clock, [150.0, 53.0], 150.0);
    hold(&mut app, &mut clock, [150.0, 53.0], 150.0);
    eprintln!("correcting={}", app.drag.as_ref().is_some_and(|d| d.correcting.is_some()));
    clock += 20.0;
    app.stylus.clock_ms = clock;
    tool_event(&mut app, ToolEvent::Move { x: 150.0, y: 90.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 150.0, y: 90.0 }, Modifiers::NONE);
    eprintln!("steps={} entries={:?} a8070={} a8053={}", steps(&app), app.session.active().unwrap().history.entries(), layer_alpha(&app, 80, 70), layer_alpha(&app, 80, 53));
    let u1 = app.session.undo();
    eprintln!("undo1={u1} steps={} a8070={} a8053={}", steps(&app), layer_alpha(&app, 80, 70), layer_alpha(&app, 80, 53));
    let u2 = app.session.undo();
    eprintln!("undo2={u2} steps={} a8070={} a8053={}", steps(&app), layer_alpha(&app, 80, 70), layer_alpha(&app, 80, 53));
}

#[test]
fn dwell_fits_move_stretches_release_commits_two_steps() {
    let mut app = app();
    let before = steps(&app);
    let mut clock = 1000.0;
    app.stylus.clock_ms = clock;
    draw_wobbly(&mut app, &mut clock, 20.0);
    assert!(app.drag.as_ref().is_some_and(|d| d.correcting.is_none()), "drawing does not correct");
    // Dwell past the 100 ms threshold without moving.
    hold(&mut app, &mut clock, [150.0, 53.0], 150.0);
    hold(&mut app, &mut clock, [150.0, 53.0], 150.0);
    assert!(app.drag.as_ref().is_some_and(|d| d.correcting.is_some()), "the hold fits the stroke");
    // Drag the end down: the start stays, the line stretches to the pointer.
    clock += 20.0;
    app.stylus.clock_ms = clock;
    tool_event(&mut app, ToolEvent::Move { x: 150.0, y: 90.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 150.0, y: 90.0 }, Modifiers::NONE);
    assert_eq!(steps(&app), before + 2, "raw + straighten entries");
    assert!(layer_alpha(&app, 80, 70) > 0.5, "the stretched fitted line shows");
    app.session.undo();
    assert!(layer_alpha(&app, 80, 53) > 0.0, "first undo steps back to the raw stroke");
    assert_eq!(layer_alpha(&app, 80, 70), 0.0, "the fit is gone");
    app.session.undo();
    assert_eq!(layer_alpha(&app, 80, 53), 0.0, "second undo clears the stroke");
}

#[test]
fn quick_release_without_dwell_commits_one_step() {
    let mut app = app();
    let before = steps(&app);
    let mut clock = 1000.0;
    app.stylus.clock_ms = clock;
    draw_wobbly(&mut app, &mut clock, 5.0);
    tool_event(&mut app, ToolEvent::Up { x: 150.0, y: 53.0 }, Modifiers::NONE);
    assert!(app.drag.is_none());
    assert_eq!(steps(&app), before + 1, "no dwell, no correction");
}
