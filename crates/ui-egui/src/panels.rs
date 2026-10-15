//! Chrome around the canvas: title bar, options bar, toolbar, status bar, dock cards, Properties.

use craft::ui::patterns::blend_keys::{BlendKeyEntry, BlendShortcut};
#[cfg(test)]
use egui::{Rect, vec2};
use photocraft_color::BlendMode;
use photocraft_doc::LayerId;
use photocraft_engine::history_source_cmds::HistorySource;
use serde_json::{Value, json};

use crate::state::Tool;
#[cfg(test)]
use crate::theme;
use crate::{PhotocraftApp, icons};

pub mod actions;
pub mod adjust_ui;
pub mod apply_color;
pub mod apply_data_set_ui;
pub(crate) mod blend_preview;
#[cfg(test)]
mod blend_rows_tests;
pub(super) mod brand;
pub mod brush_panel;
pub mod brush_preview;
pub mod brush_sections;
mod brush_settings;
mod brush_settings_events;
pub mod channels_panel;
pub mod chrome_ui;
pub mod clip_line_ui;
pub(crate) mod color;
pub mod color_hud;
pub mod color_panel_ui;
pub mod comps_ui;
pub mod doc_props_ui;
pub mod histogram_panel;
pub(crate) mod history;
pub mod history_menu_ui;
pub(crate) mod info;
pub mod layer_filter_ui;
pub mod layer_menu_ui;
pub mod layer_props_ui;
pub(super) mod layer_reveal;
pub mod layer_row_ui;
pub(super) mod layer_transfer;
mod layers;
pub(crate) mod layers_events;
pub mod layers_panel_ui;
pub(crate) mod layers_tree;
pub(super) mod mask_props_ui;
pub mod mask_thumbs_ui;
mod navigator;
pub(crate) mod options_bar;
pub(super) mod panel_docking;
pub(crate) mod preset_browser;
pub mod preset_panels;
mod properties;
pub(crate) mod region_versions;
pub(crate) mod registry;
pub mod smart_ui;
pub(crate) mod status_bar;
pub mod swatches_ui;
pub(super) mod timeline_ui;
pub(crate) mod title_bar;
pub(crate) mod tool_commands;
pub(crate) mod toolbar;
pub mod type_panels_ui;
pub(super) mod variables_ui;

#[cfg(test)]
mod title_tests {
    use super::{system_title_bar, title_x, window_title};

    #[test]
    fn title_never_overlaps_menus_or_controls() {
        // Wide window: centred.
        assert_eq!(title_x(800.0, 420.0, 1300.0, 60.0), Some(770.0));
        // Centre would hit the menus: slid right into the gap.
        assert_eq!(title_x(400.0, 420.0, 1300.0, 60.0), Some(436.0));
        // Centre would hit the controls: slid left.
        assert_eq!(title_x(1280.0, 420.0, 1300.0, 60.0), Some(1224.0));
        // No room for the whole title: the caller elides or drops it.
        assert_eq!(title_x(400.0, 420.0, 480.0, 60.0), None);
        assert_eq!(title_x(f32::NAN, 420.0, 1300.0, 60.0), Some(436.0));
        assert_eq!(title_x(400.0, f32::INFINITY, 1300.0, 60.0), None);
    }

    fn app(custom_titlebar: bool) -> crate::PhotocraftApp {
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.custom_titlebar = custom_titlebar;
        // The preference that makes the shell launch without its own title bar.
        app.session.edit_prefs(|p| p.interface.system_title_bar = !custom_titlebar);
        app
    }

    #[test]
    fn without_the_preference_the_mark_stays() {
        // The default (e.g. the web build or the snapshot example): no custom bar, no preference.
        let app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        assert!(!system_title_bar(&app));
    }

    fn open_named(app: &mut crate::PhotocraftApp, name: &str) {
        app.session.add_document(
            photocraft_doc::Document::new(name, photocraft_doc::Size::new(4, 4), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8),
            None,
        );
    }

    #[test]
    fn window_title_with_no_document_is_the_app_name() {
        assert_eq!(window_title(&app(true)), "PhotoCraft");
    }

    #[test]
    fn window_title_follows_the_active_file() {
        let mut app = app(true);
        open_named(&mut app, "foo.psd");
        assert_eq!(window_title(&app), "foo.psd \u{2014} PhotoCraft");
        // Unsaved changes gain a `*` prefix, like the `•` in the app-drawn title.
        if let Some(st) = app.session.active_mut() {
            st.revision += 1;
        }
        assert_eq!(window_title(&app), "*foo.psd \u{2014} PhotoCraft");
    }

    #[test]
    fn system_mode_hides_the_mark_and_the_centred_title() {
        for custom in [true, false] {
            let mut app = app(custom);
            open_named(&mut app, "foo.psd");
            let ctx = egui::Context::default();
            crate::PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
            let out = ctx.run_ui(egui::RawInput::default(), |ui| super::title_bar(&mut app, ui));
            let mut out = out;
            out.textures_delta.clear();
            let mut texts = Vec::new();
            for shape in &out.shapes {
                collect_text(&shape.shape, &mut texts);
            }
            let system = system_title_bar(&app);
            assert_eq!(system, !custom && !cfg!(target_os = "macos"));
            assert_eq!(crate::brand::mark_rect(&ctx).is_some(), !system, "mark in system mode (custom={custom})");
            assert_eq!(texts.iter().any(|t| t.contains("foo.psd")), !system, "centred title in system mode (custom={custom}): {texts:?}");
        }
    }

    fn collect_text(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect_text(s, out)),
            _ => {}
        }
    }

    #[test]
    fn sync_window_title_sends_only_on_change() {
        let mut app = app(true);
        let ctx = egui::Context::default();
        crate::PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
        super::sync_window_title(&mut app, &ctx);
        assert_eq!(app.last_window_title, "PhotoCraft");
        let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
        out.textures_delta.clear();
        let cmds = out.viewport_output.remove(&egui::ViewportId::ROOT).map(|v| v.commands).unwrap_or_default();
        assert!(cmds.iter().any(|c| matches!(c, egui::ViewportCommand::Title(t) if t == "PhotoCraft")), "{cmds:?}");
        // Same document state again: the cached title stops a repeat command.
        super::sync_window_title(&mut app, &ctx);
        let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
        out.textures_delta.clear();
        let cmds = out.viewport_output.remove(&egui::ViewportId::ROOT).map(|v| v.commands).unwrap_or_default();
        assert!(!cmds.iter().any(|c| matches!(c, egui::ViewportCommand::Title(_))), "repeat Title: {cmds:?}");
        open_named(&mut app, "foo.psd");
        super::sync_window_title(&mut app, &ctx);
        assert_eq!(app.last_window_title, "foo.psd \u{2014} PhotoCraft");
    }
}

// Custom controls also need to bring keyboard focus into a scroll viewport.
fn reveal_focused_control(ui: &egui::Ui) {
    // read_response reprocesses pointer events. Reading a popup's newly focused field
    // during its opening click can undo the focus that the popup just requested.
    if ui.input(|input| input.pointer.any_pressed() || input.pointer.any_released()) {
        return;
    }
    let focused = ui.memory(|memory| memory.focused());
    let key = ui.id().with("scroll-focused-control");
    let changed = ui.data_mut(|data| {
        let previous = data.get_temp::<Option<egui::Id>>(key).flatten();
        data.insert_temp(key, focused);
        previous != focused
    });
    if changed
        && let Some(response) = focused.and_then(|id| ui.ctx().read_response(id))
        && response.layer_id == ui.layer_id()
        && ui.min_rect().contains_rect(response.rect)
    {
        ui.scroll_to_rect(response.rect, Some(egui::Align::Center));
    }
}

// ----------------------------------------------------------------------------- options bar

/// Options bar height: Preferences ▸ Workspace ▸ Enable Narrow Options Bar trims it, as in
/// Photoshop, leaving the document more room.
fn options_bar_height(pro: bool, narrow: bool) -> f32 {
    match (pro, narrow) {
        (true, false) => 36.0,
        (true, true) => 30.0,
        (false, false) => 42.0,
        (false, true) => 34.0,
    }
}

pub fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let narrow = app.session.prefs().workspace.narrow_options_bar();
    egui::Panel::top("options_bar")
        .exact_size(options_bar_height(t.pro, narrow))
        .frame(egui::Frame::NONE.fill(t.chrome).inner_margin(egui::Margin::symmetric(10, 0)))
        .show(ui, |ui| {
            ui.style_mut().spacing.scroll = egui::style::ScrollStyle { bar_width: 4.0, ..egui::style::ScrollStyle::floating() };
            egui::ScrollArea::horizontal().id_salt("tool-options").auto_shrink([false, false]).show(ui, |ui| {
            // Font ink can extend beyond its advance width at fractional positions.
            // Include its gutter in the container layout, including right-aligned actions.
            egui::Frame::NONE.inner_margin(egui::Margin { right: 2, ..Default::default() }).show(ui, |ui| {
            crate::direction::horizontal_centered(ui, |ui| {
                if t.pro {
                    crate::chrome_ui::home_button(app, ui);
                    widgets::vline(ui, 22.0);
                }
                let tool_icon = icons::button(ui, icons::tool_icon(app.ui.tool), if t.pro { 26.0 } else { 28.0 }, !t.pro, tl!(app.ui.tool.label()));
                // Photoshop: right-click the tool icon for Reset Tool / Reset All Tools (#2825).
                let mut reset = None;
                tool_icon.context_menu(|ui| {
                    if ui.button(tl!("Reset Tool")).clicked() {
                        reset = Some("tool.reset");
                        ui.close();
                    }
                    if ui.button(tl!("Reset All Tools")).clicked() {
                        reset = Some("tool.resetAll");
                        ui.close();
                    }
                });
                if let Some(id) = reset
                    && let Err(e) = crate::menus::invoke(app, ui.ctx(), id, json!({}))
                {
                    app.ui.status = e;
                }
                if t.pro {
                    let (r, _) = ui.allocate_exact_size(vec2(10.0, 20.0), Sense::hover());
                    icons::paint(ui, r, "chevron-down", 10.0, t.text_faint);
                }
                widgets::vline(ui, 22.0);
                if app.ui.transform.is_some() {
                    crate::transform_tool::options_bar(app, ui);
                    return;
                }
                if crate::distort_ui::options_bar(app, ui) {
                    return;
                }
                let tool = app.ui.tool;
                // The options bar shows the active tool's brush: switch it in before drawing the
                // chip, so a tool change swaps the brush with it (Photoshop keeps one per tool).
                crate::paint_mouse::sync_tool_brush(app);
                // Brush edits here go through `tools.setBrush`, one journal entry per gesture (Rule 1).
                if (tool.is_brushlike() && !matches!(tool, Tool::Brush | Tool::Pencil | Tool::MixerBrush | Tool::Eraser)) || tool == Tool::QuickSelection {
                    brush_preset_chip(ui, &app.session.tools.brush, &mut app.ui);
                    crate::brush_picker::settings_toggle(app, ui);
                    widgets::vline(ui, 22.0);
                }
                if tool == Tool::CloneStamp {
                    let before = app.session.tools.brush.clone();
                    let mut brush = before.clone();
                    let width = if t.pro { 62.0 } else { 66.0 };
                    percent_field(ui, tl!("Opacity"), &mut brush.opacity, 1.0..=100.0, width);
                    percent_field(ui, tl!("Flow"), &mut brush.flow, 1.0..=100.0, width);
                    crate::brush_panel::commit_gesture(app, ui.ctx(), &before, &brush);
                    widgets::vline(ui, 22.0);
                }
                if crate::eraser_ui::options_bar(app, ui, tool)
                    || crate::retouch_ui::options_bar(app, ui, tool)
                    || crate::vector_ui::options_bar(app, ui, tool)
                    || crate::analysis_ui::options_bar(app, ui, tool)
                    || crate::slice_ui::options_bar(app, ui, tool)
                    || crate::frame_tool::options_bar(app, ui, tool)
                    || crate::artboard_tool::options_bar(app, ui, tool)
                {
                    return;
                }
                let brush_before = app.session.tools.brush.clone();
                let mut brush = brush_before.clone();
                let b = &mut brush;
                match app.ui.tool {
                    Tool::Brush | Tool::Eraser if t.pro => {
                        // A Block-mode Eraser has a fixed size, opacity and flow (Photoshop, #2770).
                        let block = crate::eraser_ui::block_mode(app, app.ui.tool);
                        ui.add_enabled_ui(!block, |ui| {
                            brush_preset_chip(ui, b, &mut app.ui);
                            crate::brush_picker::settings_toggle(app, ui);
                        });
                        widgets::vline(ui, 22.0);
                        // The Eraser has no blend mode: its Mode is Brush, Pencil or Block (#2662).
                        if app.ui.tool == Tool::Eraser {
                            crate::eraser_ui::mode_dropdown(ui, &mut app.ui.tool_options.eraser_mode);
                        } else {
                            opt_label(ui, tl!("Mode"));
                            let held = app.held_erase;
                            paint_mode_dropdown(ui, "brush-mode", b, &mut app.ui.tool_options.brush_clear, held);
                        }
                        ui.add_enabled_ui(!block, |ui| {
                            percent_field(ui, tl!("Opacity"), &mut b.opacity, 0.0..=100.0, 62.0);
                            if icons::button(ui, "circle-dot", 24.0, b.pressure_opacity, tl!("Always use pressure for opacity")).clicked() {
                                b.pressure_opacity = !b.pressure_opacity;
                            }
                        });
                        // A Pencil- or Block-mode Eraser is always full flow, without build-up (Photoshop).
                        ui.add_enabled_ui(!block && !crate::eraser_ui::pencil_mode(app, app.ui.tool), |ui| {
                            percent_field(ui, tl!("Flow"), &mut b.flow, 1.0..=100.0, 62.0);
                            let airbrush = icons::button(ui, "sparkles", 24.0, b.build_up, tl!("Enable airbrush-style build-up effects"));
                            if crate::brush_picker::named(airbrush, "Enable airbrush-style build-up effects").clicked() {
                                b.build_up = !b.build_up;
                            }
                        });
                        opt_label(ui, tl!("Smoothing"));
                        smoothing_field(ui, b, 58.0);
                        smoothing_options(ui, b);
                        widgets::vline(ui, 22.0);
                        if icons::button(ui, "circle-dot", 24.0, b.pressure_size, tl!("Always use pressure for size")).clicked() {
                            b.pressure_size = !b.pressure_size;
                        }
                    }
                    // Pencil: Photoshop's options (no hardness or flow: the pencil is always hard).
                    Tool::Pencil => {
                        brush_preset_chip(ui, b, &mut app.ui);
                        crate::brush_picker::settings_toggle(app, ui);
                        widgets::vline(ui, 22.0);
                        if !t.pro {
                            opt_label(ui, tl!("Size"));
                            widgets::value_field(ui, &mut b.size, 1.0..=photocraft_engine::paint::MAX_BRUSH_SIZE, "px", 76.0);
                            widgets::vline(ui, 22.0);
                        }
                        opt_label(ui, tl!("Mode"));
                        let held = app.held_erase;
                        paint_mode_dropdown(ui, "pencil-mode", b, &mut app.ui.tool_options.pencil_clear, held);
                        percent_field(ui, tl!("Opacity"), &mut b.opacity, 0.0..=100.0, if t.pro { 62.0 } else { 66.0 });
                        opt_label(ui, tl!("Smoothing"));
                        smoothing_field(ui, b, if t.pro { 58.0 } else { 66.0 });
                        widgets::vline(ui, 22.0);
                        widgets::checkbox(ui, &mut app.ui.tool_options.pencil_auto_erase, tl!("Auto Erase"));
                    }
                    Tool::Brush | Tool::Eraser => {
                        brush_preset_chip(ui, b, &mut app.ui);
                        crate::brush_picker::settings_toggle(app, ui);
                        widgets::vline(ui, 22.0);
                        // A Block-mode Eraser has a fixed size, opacity and flow (Photoshop, #2770).
                        let block = crate::eraser_ui::block_mode(app, app.ui.tool);
                        ui.add_enabled_ui(!block, |ui| {
                            opt_label(ui, tl!("Size"));
                            widgets::value_field(ui, &mut b.size, 1.0..=2500.0, "px", 76.0);
                        });
                        widgets::vline(ui, 22.0);
                        if app.ui.tool == Tool::Eraser {
                            crate::eraser_ui::mode_dropdown(ui, &mut app.ui.tool_options.eraser_mode);
                        } else {
                            opt_label(ui, tl!("Mode"));
                            let held = app.held_erase;
                            paint_mode_dropdown(ui, "brush-mode", b, &mut app.ui.tool_options.brush_clear, held);
                        }
                        // A Pencil-mode Eraser is always hard and at full flow (Photoshop).
                        let fixed = block || crate::eraser_ui::pencil_mode(app, app.ui.tool);
                        ui.add_enabled_ui(!fixed, |ui| percent_field(ui, tl!("Hardness"), &mut b.hardness, 0.0..=100.0, 66.0));
                        ui.add_enabled_ui(!block, |ui| percent_field(ui, tl!("Opacity"), &mut b.opacity, 0.0..=100.0, 66.0));
                        ui.add_enabled_ui(!fixed, |ui| percent_field(ui, tl!("Flow"), &mut b.flow, 1.0..=100.0, 66.0));
                        opt_label(ui, tl!("Smoothing"));
                        smoothing_field(ui, b, 66.0);
                        widgets::vline(ui, 22.0);
                        widgets::toggle(ui, &mut b.pressure_size, tl!("Pressure for Size"));
                        widgets::toggle(ui, &mut b.pressure_opacity, tl!("Pressure for Opacity"));
                    }
                    Tool::MixerBrush => {
                        brush_preset_chip(ui, b, &mut app.ui);
                        crate::brush_picker::settings_toggle(app, ui);
                        widgets::vline(ui, 22.0);
                        for (label, value) in [
                            (tl!("Wet"), &mut b.mixer.wet),
                            (tl!("Load"), &mut b.mixer.load),
                            (tl!("Mix"), &mut b.mixer.mix),
                            (tl!("Flow"), &mut b.mixer.flow),
                        ] {
                            percent_field(ui, label, value, 0.0..=100.0, 62.0);
                        }
                        widgets::vline(ui, 22.0);
                        widgets::checkbox(ui, &mut b.mixer.sample_all_layers, tl!("Sample All Layers"));
                    }
                    Tool::RectMarquee | Tool::EllipseMarquee | Tool::SingleRowMarquee | Tool::SingleColumnMarquee if t.pro => {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        for (i, (icon, tip)) in [
                            ("square", tl!("New selection").to_string()),
                            ("plus", crate::i18n::fmt(tl!("Add to selection  ({key})"), &[("key", &crate::shortcuts::pretty("Shift"))])),
                            ("minus", crate::i18n::fmt(tl!("Subtract from selection  ({key})"), &[("key", &crate::shortcuts::pretty("Alt"))])),
                            (
                                "squares-subtract",
                                crate::i18n::fmt(tl!("Intersect with selection  ({key})"), &[("key", &crate::shortcuts::pretty("Shift+Alt"))]),
                            ),
                        ]
                        .iter()
                        .enumerate()
                        {
                            if icons::button(ui, icon, 24.0, app.ui.selection_mode == i as u8, tip).clicked() {
                                app.ui.selection_mode = i as u8;
                            }
                        }
                        ui.spacing_mut().item_spacing.x = 8.0;
                        widgets::vline(ui, 22.0);
                        let o = &mut app.ui.tool_options;
                        opt_label(ui, tl!("Feather"));
                        widgets::value_field(ui, &mut o.feather, 0.0..=1000.0, "px", 62.0);
                        // Photoshop greys Anti-alias for the Rectangular Marquee (its edges are always hard).
                        ui.add_enabled_ui(app.ui.tool == Tool::EllipseMarquee, |ui| widgets::checkbox(ui, &mut o.anti_alias, tl!("Anti-alias")));
                        widgets::vline(ui, 22.0);
                        // A single row or column has no style or size: Photoshop greys them.
                        let sized = matches!(app.ui.tool, Tool::RectMarquee | Tool::EllipseMarquee);
                        ui.add_enabled_ui(sized, |ui| opt_label(ui, tl!("Style")));
                        let styles = [
                            ("normal".to_string(), tl!("Normal")),
                            ("fixedRatio".to_string(), tl!("Fixed Ratio")),
                            ("fixedSize".to_string(), tl!("Fixed Size")),
                        ];
                        if ui.add_enabled_ui(sized, |ui| widgets::dropdown(ui, "marquee-style", &mut o.marquee_style, &styles, 96.0)).inner {
                            (o.marquee_width, o.marquee_height) = if o.marquee_style == "fixedSize" { (64.0, 64.0) } else { (1.0, 1.0) };
                        }
                        let fixed = sized && o.marquee_style != "normal";
                        let (unit, range) = if o.marquee_style == "fixedSize" { ("px", 1.0..=300_000.0) } else { ("", 0.001..=999.0) };
                        ui.add_enabled_ui(fixed, |ui| {
                            opt_label(ui, tl!("Width"));
                            widgets::value_field(ui, &mut o.marquee_width, range.clone(), unit, 62.0);
                            if icons::button(ui, "arrow-left-right", 22.0, false, tl!("Swaps height and width")).clicked() {
                                std::mem::swap(&mut o.marquee_width, &mut o.marquee_height);
                            }
                            opt_label(ui, tl!("Height"));
                            widgets::value_field(ui, &mut o.marquee_height, range, unit, 62.0);
                        });
                        widgets::vline(ui, 22.0);
                        if widgets::secondary_button(ui, tl!("Select and Mask…"), 0.0).clicked() {
                            let _ = crate::menus::invoke(app, ui.ctx(), "select.selectAndMask", json!({}));
                        }
                    }
                    Tool::Lasso | Tool::PolygonLasso | Tool::MagneticLasso if t.pro => {
                        selection_mode_buttons(app, ui);
                        widgets::vline(ui, 22.0);
                        opt_label(ui, tl!("Feather"));
                        widgets::value_field(ui, &mut app.ui.tool_options.feather, 0.0..=1000.0, "px", 62.0);
                        widgets::checkbox(ui, &mut app.ui.tool_options.anti_alias, tl!("Anti-alias"));
                        widgets::vline(ui, 22.0);
                        if app.ui.tool == Tool::MagneticLasso {
                            let o = &mut app.ui.tool_options;
                            opt_label(ui, tl!("Width"));
                            widgets::value_field(ui, &mut o.magnetic_width, 1.0..=256.0, "px", 58.0)
                                .on_hover_text(tl!("Follows only edges this close to the pointer ([ and ] change it)"));
                            opt_label(ui, tl!("Contrast"));
                            widgets::value_field(ui, &mut o.magnetic_contrast, 1.0..=100.0, "%", 58.0)
                                .on_hover_text(tl!("Higher values follow only edges that contrast sharply with their surroundings"));
                            opt_label(ui, tl!("Frequency"));
                            widgets::value_field(ui, &mut o.magnetic_frequency, 0.0..=100.0, "", 50.0)
                                .on_hover_text(tl!("How often fastening points are placed by themselves"));
                            if icons::button(ui, "circle-dot", 24.0, o.magnetic_pressure, tl!("Use tablet pressure to change pen width")).clicked() {
                                o.magnetic_pressure = !o.magnetic_pressure;
                            }
                            widgets::vline(ui, 22.0);
                        }
                        if widgets::secondary_button(ui, tl!("Select and Mask…"), 0.0).clicked() {
                            let _ = crate::menus::invoke(app, ui.ctx(), "select.selectAndMask", json!({}));
                        }
                        if crate::lasso_ui::active(app) || (app.ui.tool == Tool::PolygonLasso && !app.ui.polygon.is_empty()) {
                            hint(
                                ui,
                                &crate::i18n::fmt(
                                    tl!("Click the first point or press {key} to close · Esc cancels"),
                                    &[("key", &crate::shortcuts::pretty("Enter"))],
                                ),
                            );
                        } else if app.ui.tool == Tool::Lasso {
                            hint(ui, tl!("Hold Alt while drawing for straight segments"));
                        }
                        if app.ui.tool == Tool::MagneticLasso && app.ui.magnetic.active() {
                            hint(
                                ui,
                                &crate::i18n::fmt(
                                    tl!("Click the first point or press {key} to close · {del} removes a point · Esc cancels"),
                                    &[("key", &crate::shortcuts::pretty("Enter")), ("del", &crate::shortcuts::pretty("Backspace"))],
                                ),
                            );
                        }
                    }
                    Tool::MagicWand => {
                        selection_mode_buttons(app, ui);
                        widgets::vline(ui, 22.0);
                        opt_label(ui, tl!("Tolerance"));
                        widgets::value_field(ui, &mut app.ui.tool_options.tolerance, 0.0..=255.0, "", 58.0);
                        widgets::checkbox(ui, &mut app.ui.tool_options.anti_alias, tl!("Anti-alias"));
                        widgets::checkbox(ui, &mut app.ui.tool_options.contiguous, tl!("Contiguous"));
                        widgets::checkbox(ui, &mut app.ui.tool_options.sample_all_layers, tl!("Sample All Layers"));
                        widgets::vline(ui, 22.0);
                        if widgets::secondary_button(ui, tl!("Select Subject"), 0.0).clicked() {
                            let r = app.run("select.subject", json!({}));
                            crate::retouch_ui::report_smart_result(app, r, tl!("No subject found — try Object Selection or the Lasso"));
                        }
                        if widgets::secondary_button(ui, tl!("Select and Mask…"), 0.0).clicked() {
                            let _ = crate::menus::invoke(app, ui.ctx(), "select.selectAndMask", json!({}));
                        }
                    }
                    Tool::PaintBucket if t.pro => {
                        crate::bucket_ui::options_bar(app, ui);
                    }
                    // Studio layout never had bucket options; the same bar applies (with the hint).
                    Tool::PaintBucket => {
                        crate::bucket_ui::options_bar(app, ui);
                        hint(ui, tl!("Click to fill similar colours"));
                    }
                    Tool::Gradient => {
                        // Live "Gradient" (a Gradient Fill layer) or "Classic gradient" (pixels).
                        let mut classic = app.ui.tool_options.gradient_classic;
                        if widgets::dropdown(ui, "gradient-mode", &mut classic, &[(false, "Gradient"), (true, "Classic gradient")], 118.0) {
                            app.ui.tool_options.gradient_classic = classic;
                        }
                        // Live "Gradient" (a Gradient Fill layer) or "Classic gradient" (pixels):
                        // both paint with the same engine-owned preset (#1651), and either swatch
                        // click opens the Gradient Editor window, as in Photoshop.
                        if crate::gradient_ui::preset_swatch(app, ui) {
                            app.ui.panels.gradient_editor = true;
                        }
                        widgets::vline(ui, 22.0);
                        ui.spacing_mut().item_spacing.x = 2.0;
                        let before = app.ui.tool_options.clone();
                        for (style, icon, tip) in [
                            ("linear", "gradient", tl!("Linear Gradient")),
                            ("radial", "circle", tl!("Radial Gradient")),
                            ("angle", "rotate-cw", tl!("Angle Gradient")),
                            ("reflected", "arrow-left-right", tl!("Reflected Gradient")),
                            ("diamond", "diamond", tl!("Diamond Gradient")),
                        ] {
                            if icons::button(ui, icon, 24.0, app.ui.tool_options.gradient_style == style, tip).clicked() {
                                app.ui.tool_options.gradient_style = style.into();
                            }
                        }
                        ui.spacing_mut().item_spacing.x = 8.0;
                        widgets::vline(ui, 22.0);
                        opt_label(ui, tl!("Mode"));
                        let opts: Vec<(BlendMode, &str)> = BlendMode::LAYER_MODES.iter().map(|m| (*m, m.label())).collect();
                        // The wheel over it steps the mode, as over the Layers panel's (#2061).
                        widgets::dropdown_wheel_hovered(ui, "gradient-blend-mode", &mut app.ui.tool_options.gradient_blend_mode, &opts, 96.0);
                        opt_label(ui, tl!("Opacity"));
                        widgets::value_field(ui, &mut app.ui.tool_options.fill_opacity, 1.0..=100.0, "%", 62.0);
                        widgets::checkbox(ui, &mut app.ui.tool_options.gradient_reverse, tl!("Reverse"));
                        widgets::checkbox(ui, &mut app.ui.tool_options.gradient_dither, tl!("Dither"));
                        // Live mode: the options also change a selected gradient fill layer.
                        crate::gradient_ui::options_changed(app, &before);
                    }
                    Tool::PerspectiveCrop if t.pro => crate::perspective_crop_ui::options_bar(app, ui),
                    Tool::Crop if t.pro => {
                        let before_ratio = crate::crop_ui::preset_ratio(app);
                        let doc_size = app.session.active().map_or((0.0, 0.0), |s| (f64::from(s.doc.size.width), f64::from(s.doc.size.height)));
                        // Ratios, sizes, saved presets, New / Delete Crop Preset… (#2443, #1919).
                        crate::crop_presets::dropdown(app, ui);
                        let o = &mut app.ui.tool_options;
                        if o.crop_ratio == crate::crop_size::WHR {
                            crate::crop_size::fields(o, ui);
                        } else {
                            // Keep partial edits in ToolOptions so the first side survives a frame;
                            // Original Ratio shows the document's numbers, as Photoshop does.
                            let (mut rw, mut rh) = crate::crop_size::custom_ratio_fields(o, doc_size);
                            let cw = widgets::value_field(ui, &mut rw, 0.0..=99_999.0, "", 54.0).changed();
                            let swap = icons::button(ui, "arrow-left-right", 22.0, false, tl!("Swaps height and width")).clicked();
                            let ch = widgets::value_field(ui, &mut rh, 0.0..=99_999.0, "", 54.0).changed();
                            if swap {
                                std::mem::swap(&mut rw, &mut rh);
                            }
                            if cw || ch || swap {
                                crate::crop_size::set_custom_ratio_fields(o, rw, rh);
                            }
                        }
                        widgets::vline(ui, 22.0);
                        if widgets::secondary_button(ui, tl!("Clear"), 0.0).clicked() {
                            if o.crop_ratio == crate::crop_size::WHR {
                                crate::crop_size::clear(o);
                            } else {
                                o.crop_ratio.clear();
                            }
                        }
                        crate::crop_straighten::options_button(&mut app.crop.straighten, ui);
                        crate::crop_overlay::options_button(o, ui);
                        let pick_shield_color = crate::crop_shield::options_button(&mut o.crop_shield, ui);
                        widgets::checkbox(ui, &mut o.crop_delete, tl!("Delete Cropped Pixels"));
                        if crate::crop_ui::preset_ratio(app) != before_ratio {
                            crate::crop_ui::apply_ratio(app);
                        }
                        if pick_shield_color {
                            crate::crop_shield::pick_custom_color(app);
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if icons::button(
                                ui,
                                "check",
                                26.0,
                                false,
                                &crate::i18n::fmt(tl!("Commit current crop operation  ({key})"), &[("key", &crate::shortcuts::pretty("Enter"))]),
                            )
                            .clicked()
                            {
                                crate::canvas::commit_crop(app);
                            }
                            if icons::button(ui, "ban", 26.0, false, tl!("Cancel current crop operation  (Esc)")).clicked() {
                                crate::crop_ui::cancel(app);
                            }
                        });
                    }
                    // Every theme gets the font, style and size controls (#1387), as with Auto-Select (#1275).
                    Tool::Type | Tool::VerticalType | Tool::HorizontalTypeMask | Tool::VerticalTypeMask => crate::type_tool::options_bar(app, ui),
                    Tool::Move if t.pro => {
                        let o = &mut app.ui.tool_options;
                        widgets::checkbox(ui, &mut o.move_auto_select, tl!("Auto-Select:"));
                        widgets::dropdown(
                            ui,
                            "move-target",
                            &mut o.move_target,
                            &[("layer".to_string(), tl!("Layer")), ("group".to_string(), tl!("Group"))],
                            76.0,
                        );
                        widgets::vline(ui, 22.0);
                        widgets::checkbox(ui, &mut o.move_show_transform, tl!("Show Transform Controls"));
                        widgets::vline(ui, 22.0);
                        ui.spacing_mut().item_spacing.x = 0.0;
                        for (icon, tip, cmd) in [
                            ("panels-top-left", tl!("Align left edges"), "layer.align.leftEdges"),
                            ("app-window", tl!("Align horizontal centers"), "layer.align.horizontalCenters"),
                            ("panel-right", tl!("Align right edges"), "layer.align.rightEdges"),
                        ] {
                            let on = app.session.is_enabled(cmd);
                            if ui.add_enabled_ui(on, |ui| icons::button(ui, icon, 24.0, false, tip)).inner.clicked() {
                                let _ = app.run(cmd, json!({}));
                            }
                        }
                        ui.add_space(6.0);
                        let more = icons::button(ui, "ellipsis", 24.0, false, tl!("Align and distribute"));
                        egui::Popup::menu(&more).show(|ui| {
                            ui.set_min_width(200.0);
                            let mut section = |ui: &mut egui::Ui, title: &str, prefix: &str| {
                                ui.label(egui::RichText::new(tl!(&title)).small().color(t.text_dim));
                                for c in photocraft_engine::command_specs().iter().filter(|c| c.id.starts_with(prefix)) {
                                    if ui.add_enabled(app.session.is_enabled(c.id), egui::Button::new(tl!(c.label))).clicked() {
                                        let _ = app.run(c.id, json!({}));
                                        ui.close();
                                    }
                                }
                            };
                            section(ui, tl!("Align"), "layer.align.");
                            ui.separator();
                            section(ui, tl!("Distribute"), "layer.distribute.");
                        });
                    }
                    Tool::RectMarquee | Tool::EllipseMarquee => hint(
                        ui,
                        &format!(
                            "Drag to select  ·  {} add  ·  {} subtract  ·  {} intersect  ·  click to deselect",
                            crate::shortcuts::pretty("Shift"),
                            crate::shortcuts::pretty("Alt"),
                            crate::shortcuts::pretty("Shift+Alt")
                        ),
                    ),
                    Tool::SingleRowMarquee | Tool::SingleColumnMarquee => hint(
                        ui,
                        &format!(
                            "Click to select a one-pixel line  ·  {} add  ·  {} subtract  ·  {} intersect",
                            crate::shortcuts::pretty("Shift"),
                            crate::shortcuts::pretty("Alt"),
                            crate::shortcuts::pretty("Shift+Alt")
                        ),
                    ),
                    Tool::Move => {
                        // The Studio themes keep Photoshop's Auto-Select toggle too (#1275).
                        widgets::checkbox(ui, &mut app.ui.tool_options.move_auto_select, tl!("Auto-Select:"));
                        hint(ui, tl!("Drag to move the active layer"));
                    }
                    Tool::Eyedropper => crate::eyedropper_ui::options(app, ui),
                    Tool::Zoom => {
                        widgets::checkbox(ui, &mut app.ui.tool_options.zoom_scrubby, tl!("Scrubby Zoom"));
                        hint(
                            ui,
                            &crate::i18n::fmt(
                                tl!("Click to zoom in  ·  {key}-click to zoom out  ·  drag right/left to zoom in/out"),
                                &[("key", &crate::shortcuts::pretty("Alt"))],
                            ),
                        );
                        if widgets::secondary_button(ui, tl!("Fit Screen"), 0.0).clicked()
                            && let Some(i) = app.session.active_index()
                        {
                            app.ui.views[i].fit_pending = true;
                        }
                        if widgets::secondary_button(ui, "100%", 0.0).clicked()
                            && let Some(i) = app.session.active_index()
                        {
                            app.ui.views[i].zoom = 1.0;
                        }
                    }
                    Tool::RotateView => {
                        hint(ui, tl!("Drag around the centre to rotate the view  ·  Shift constrains to 15°"));
                        if let Some(i) = app.session.active_index() {
                            ui.label(tl!("Angle"));
                            let mut angle = app.ui.views.get(i).map(|v| v.rotation).unwrap_or(0.0);
                            if widgets::value_field(ui, &mut angle, -180.0..=180.0, "°", 56.0).changed()
                                && let Some(v) = app.ui.views.get_mut(i)
                            {
                                v.rotation = crate::rotate_view::wrap_deg(angle);
                            }
                            if widgets::secondary_button(ui, tl!("Reset View"), 0.0).clicked()
                                && let Some(v) = app.ui.views.get_mut(i)
                            {
                                v.rotation = 0.0;
                            }
                        }
                    }
                    Tool::Hand => {
                        hint(ui, tl!("Drag to pan  ·  hold Space with any tool"));
                        if widgets::secondary_button(ui, "100%", 0.0).clicked()
                            && let Some(i) = app.session.active_index()
                        {
                            app.ui.views[i].zoom = 1.0;
                            app.ui.views[i].fit_pending = false;
                            app.ui.views[i].fill_pending = false;
                        }
                        if widgets::secondary_button(ui, tl!("Fit Screen"), 0.0).clicked()
                            && let Some(i) = app.session.active_index()
                        {
                            app.ui.views[i].fit_pending = true;
                            app.ui.views[i].fill_pending = false;
                        }
                        if widgets::secondary_button(ui, tl!("Fill Screen"), 0.0).clicked()
                            && let Some(i) = app.session.active_index()
                        {
                            app.ui.views[i].fill_pending = true;
                            app.ui.views[i].fit_pending = false;
                        }
                    }
                    Tool::Lasso | Tool::PolygonLasso => hint(
                        ui,
                        &crate::i18n::fmt(
                            tl!("Drag or click polygon points · hold Alt during lasso for straight segments · {add} add · {sub} before drawing subtracts"),
                            &[("add", &crate::shortcuts::pretty("Shift")), ("sub", &crate::shortcuts::pretty("Alt"))],
                        ),
                    ),
                    Tool::MagneticLasso => hint(
                        ui,
                        &crate::i18n::fmt(
                            tl!("Click, then move along an edge · click to fasten a point · {alt}-click for a straight segment · double-click to close"),
                            &[("alt", &crate::shortcuts::pretty("Alt"))],
                        ),
                    ),
                    Tool::Crop => hint(
                        ui,
                        &crate::i18n::fmt(
                            tl!("Drag a crop box · drag inside to move · edges resize ({ratio} ratio, {centre} centre) · Space moves while drawing · arrows nudge · X swaps orientation · {commit} commits · Esc cancels"),
                            &[
                                ("ratio", &crate::shortcuts::pretty("Shift")),
                                ("centre", &crate::shortcuts::pretty("Alt")),
                                ("commit", &crate::shortcuts::pretty("Enter")),
                            ],
                        ),
                    ),
                    Tool::PerspectiveCrop => hint(
                        ui,
                        &crate::i18n::fmt(
                            tl!("Drag a box or click four corners · drag the corners onto the edges of a skewed shape · {commit} commits · Esc cancels"),
                            &[("commit", &crate::shortcuts::pretty("Enter"))],
                        ),
                    ),
                    // Retouching and smart-selection tools draw their bar in `retouch_ui::options_bar`.
                    _ => {}
                }
                // Painting symmetry is independent of visual theme. Keep the options-bar
                // action available for Brush, Pencil and Eraser in every palette (#2659).
                if matches!(tool, Tool::Brush | Tool::Pencil | Tool::Eraser) {
                    crate::symmetry_ui::menu(app, ui);
                }
                crate::brush_panel::commit_gesture(app, ui.ctx(), &brush_before, &brush);
            });
            reveal_focused_control(ui);
            });
            });
        });
}

/// Width of the Layers panel's Opacity and Fill fields, with room for their pop-up slider's ▾.
const LAYER_PCT_W: f32 = 66.0 + widgets::POPUP_ARROW_W;

/// The `layer.setProps` edit of a Layers panel Opacity or Fill (`key`) field at `pct` percent. A
/// drag of the pop-up slider (`drag`) coalesces into one history step, as in Photoshop.
fn pct_action(l: &Layer, key: &str, pct: f32, drag: Option<u64>) -> (String, Value) {
    let mut p = json!({"layer": l.id.0, key: pct / 100.0});
    if let Some(n) = drag {
        p["coalesce"] = json!(format!("layer-{key}-slider:{n}"));
    }
    ("layer.setProps".into(), p)
}

fn label(ui: &mut egui::Ui, s: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(RichText::new(s).color(t.text_dim));
}

/// Photoshop's scrubby Opacity/Fill labels: drag horizontally to change the percentage without
/// hitting the small number field. The unique drag ID coalesces every frame into one undo step.
fn scrub_pct_label(ui: &mut egui::Ui, text: &str, percent: &mut f32, max_w: f32) -> widgets::PopupFieldResponse {
    // Elide to the room left in the row (full text on hover): a longer translation shrinks here
    // instead of running over the fixed controls beside it (#3580).
    let resp = widgets::elided_label_sensed(ui, text, max_w, Sense::click_and_drag()).on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
    let key = resp.id.with("percent-label-drag");
    if resp.drag_started() {
        let stamp = ui.ctx().cumulative_pass_nr();
        ui.data_mut(|d| d.insert_temp(key, (*percent, stamp)));
    }
    let mut out = widgets::PopupFieldResponse::default();
    if resp.dragged()
        && let Some((start, stamp)) = ui.data(|d| d.get_temp::<(f32, u64)>(key))
        && let (Some(origin), Some(pos)) = (ui.input(|i| i.pointer.press_origin()), resp.interact_pointer_pos())
    {
        // Match DragValue's 0.5 percentage-point per screen point; do not jump on press.
        let next = (start + (pos.x - origin.x) * 0.5).clamp(0.0, 100.0);
        if next.is_finite() && next != *percent {
            *percent = next;
            out.changed = true;
            out.drag = Some(stamp);
        }
    }
    if resp.drag_stopped() {
        ui.data_mut(|d| d.remove::<(f32, u64)>(key));
    }
    out
}

/// Width of `s` as [`label`] draws it (body text).
fn body_text_width(ui: &egui::Ui, s: &str) -> f32 {
    let font = egui::TextStyle::Body.resolve(ui.style());
    ui.painter().layout_no_wrap(s.to_string(), font, Color32::WHITE).size().x
}

/// Options-bar label: Photoshop writes "Size:" with a colon.
/// An options-bar label; one right before a value field scrubs it when dragged (Photoshop).
fn opt_label(ui: &mut egui::Ui, s: &str) {
    let t = Tokens::get(ui.ctx());
    let text = if t.pro && !s.ends_with(':') { format!("{s}:") } else { s.to_string() };
    let label = ui.label(RichText::new(tl!(&text)).color(t.text_dim));
    widgets::scrub_label(ui, &label);
}

fn percent_field(ui: &mut egui::Ui, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>, width: f32) {
    opt_label(ui, label);
    let mut percent = *value * 100.0;
    if widgets::value_field(ui, &mut percent, range, "%", width).changed() {
        *value = percent / 100.0;
    }
}

fn hint(ui: &mut egui::Ui, s: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(RichText::new(s).color(t.text_faint));
}

// ----------------------------------------------------------------------------- status bar

/// The status bar's height and the height of the controls in it (zoom field, Fit), which leaves
/// them an even margin above and below inside the bar (#2823).
pub fn status_bar_heights(t: &Tokens) -> (f32, f32) {
    if t.pro { (24.0, 18.0) } else { (30.0, 22.0) }
}

pub fn status_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    egui::Panel::bottom("status_bar")
        .exact_size(status_bar_heights(&t).0)
        .frame(egui::Frame::NONE.fill(t.chrome).inner_margin(egui::Margin::symmetric(10, 0)))
        .show(ui, |ui| {
            let control_h = status_bar_heights(&t).1;
            // Read from the right in a right-to-left UI, with the trailing group at the left.
            crate::direction::horizontal_centered(ui, |ui| {
                if t.pro {
                    crate::chrome_ui::status_bar_pro(app, ui);
                    // Background job progress with Cancel, at the far end (#210).
                    ui.with_layout(crate::direction::trailing(), |ui| crate::jobs_ui::status_progress(app, ui));
                    return;
                }
                if let (Some(st), Some(i)) = (app.session.active(), app.session.active_index()) {
                    let (w, h, mode, bits, layers) =
                        (st.doc.size.width, st.doc.size.height, crate::canvas::mode_label(&st.doc), st.doc.depth.bits(), st.doc.layer_count());
                    let mut pct = app.ui.views[i].zoom * 100.0;
                    if widgets::compact_wrapping_value_field(ui, &mut pct, crate::zoom_levels::percent_range(&app.ui.views[i]), "%", vec2(78.0, control_h))
                        .changed()
                    {
                        app.ui.views[i].zoom = crate::zoom_levels::clamp(pct / 100.0, app.ui.views[i].doc_size);
                        app.ui.views[i].fit_pending = false;
                    }
                    widgets::vline(ui, 16.0);
                    label(ui, &crate::i18n::fmt(tl!("{mode} Color · {bits} bit"), &[("mode", mode), ("bits", &bits.to_string())]));
                    widgets::vline(ui, 16.0);
                    label(ui, &format!("{w} × {h} px"));
                    widgets::vline(ui, 16.0);
                    label(ui, &crate::i18n::trn(crate::i18n::current(), layers as u64, "{n} layer", "{n} layers"));
                } else {
                    label(ui, tl!("No document"));
                }
                if !app.ui.status.is_empty() {
                    widgets::vline(ui, 16.0);
                    let is_err = app.ui.status_error || app.ui.status.starts_with("Couldn");
                    ui.label(RichText::new(&app.ui.status).color(if is_err { t.warning } else { t.text_faint }));
                }
                ui.with_layout(crate::direction::trailing(), |ui| {
                    if app.session.active().is_some()
                        && !t.pro
                        && widgets::compact_button(ui, tl!("Fit"), control_h).clicked()
                        && let Some(i) = app.session.active_index()
                    {
                        app.ui.views[i].fit_pending = true;
                    }
                    if app.session.has_jobs() {
                        ui.add_space(6.0);
                        crate::jobs_ui::status_progress(app, ui);
                    }
                });
            });
        });
}

// ----------------------------------------------------------------------------- dock

pub fn right_dock(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    // Even legacy destinations must discard gestures captured by an earlier workspace.
    let customized = crate::panel_docking::show(app, ui);
    let t = Tokens::get(ui.ctx());
    let p = app.ui.panels.clone();
    if t.pro && !customized {
        dock_panels(app, ui, &p, &t);
    }
    // Narrow icon rail, unless an embedding app hides it: shows, expands or collapses panel groups.
    if p.rail {
        let (rw, rb) = if t.pro { (36.0, 28.0) } else { (44.0, 32.0) };
        crate::direction::end_panel("rail")
            .resizable(false)
            .exact_size(rw)
            .frame(egui::Frame::NONE.fill(t.chrome).inner_margin(egui::Margin::symmetric(4, 8)))
            .show(ui, |ui| {
                // The separator is on the canvas side.
                let r = ui.max_rect();
                let x = if crate::direction::is_rtl() { r.right() + 6.0 } else { r.left() - 6.0 };
                ui.painter().line_segment([pos2(x, r.top() - 8.0), pos2(x, r.bottom() + 8.0)], Stroke::new(1.0, t.separator));
                ui.spacing_mut().item_spacing.y = 4.0;
                use crate::dock::Group;
                let entries: [(&str, &str, Group); 5] = [
                    ("sliders-horizontal", tl!("Properties"), Group::Properties),
                    ("navigation", tl!("Navigator"), Group::Navigator),
                    ("palette", tl!("Color & Swatches"), Group::Color),
                    ("layers", tl!("Layers"), Group::Layers),
                    ("clock", tl!("History"), Group::History),
                ];
                for (icon, name, g) in entries {
                    // Studio floats Properties outside the dock.
                    let docked = t.pro || g != Group::Properties;
                    let on = if customized { crate::panel_docking::rail_on(app, g) } else { g.shown(&p) && !(docked && app.ui.dock.is_collapsed(g)) };
                    if icons::rail_button(ui, icon, rb, on, name).clicked() {
                        if customized {
                            crate::panel_docking::rail_click(app, g);
                        } else {
                            crate::dock::rail_click(app, g, docked);
                        }
                    }
                }
            });
    }
    if !t.pro && !customized {
        dock_panels(app, ui, &p, &t);
    }
    crate::dock::persist(app, ui.ctx());
}

/// The dock's width range (points). It is the right dock, on the left in a right-to-left UI.
pub(crate) const DOCK_WIDTH: std::ops::RangeInclusive<f32> = 250.0..=520.0;

pub(crate) fn dock_width_id() -> egui::Id {
    egui::Id::new("dock-width-request")
}

/// Set the right dock's width on the next frame (`ui.set {dockWidth}`); clamped to its range.
pub fn request_dock_width(ctx: &egui::Context, w: f32) {
    let w = if w.is_finite() { w.clamp(*DOCK_WIDTH.start(), *DOCK_WIDTH.end()) } else { *DOCK_WIDTH.start() };
    ctx.data_mut(|d| d.insert_temp(dock_width_id(), w));
}

fn dock_panels(app: &mut PhotocraftApp, ui: &mut egui::Ui, p: &crate::state::Panels, t: &Tokens) {
    use crate::dock::Group;
    // Floating in Studio, Properties docks only in Pro (Photoshop).
    let shown: Vec<Group> = [
        (Group::Color, p.color),
        (Group::Properties, t.pro && p.properties),
        (Group::Character, p.character),
        (Group::Navigator, p.navigator),
        (Group::History, p.history),
        (Group::Layers, p.layers),
    ]
    .into_iter()
    .filter_map(|(g, on)| on.then_some(g))
    .collect();
    if shown.is_empty() {
        return;
    }
    let margin = if t.pro { 2 } else { 8 };
    let mut panel = crate::direction::end_panel("dock").resizable(true).default_size(if t.pro { 290.0 } else { 300.0 }).size_range(DOCK_WIDTH);
    if let Some(w) = ui.ctx().data_mut(|d| d.remove_temp::<f32>(dock_width_id())) {
        panel = panel.exact_size(w);
    }
    panel.frame(egui::Frame::NONE.fill(t.dock).inner_margin(egui::Margin::same(margin))).show(ui, |ui| {
        // Groups keep their heights whatever they show (#88): see `dock`.
        crate::dock::show(app, ui, &shown, dock_body);
    });
}

/// One dock group's tab content; `dock` bounds it and scrolls it when it's taller.
pub(crate) fn dock_body(app: &mut PhotocraftApp, ui: &mut egui::Ui, group: crate::dock::Group, tab: usize) {
    use crate::dock::Group;
    let pro = Tokens::get(ui.ctx()).pro;
    match (group, tab) {
        (Group::Color, 2) => crate::preset_panels::gradients_panel(app, ui),
        (Group::Color, 3) => crate::preset_panels::patterns_panel(app, ui),
        (Group::Color, 0) if pro => color_field(app, ui),
        (Group::Color, _) if pro => crate::swatches_ui::panel(app, ui),
        (Group::Color, 0) => crate::swatches_ui::panel(app, ui),
        (Group::Color, _) => color_picker(app, ui),
        (Group::Properties, 0) => properties_body(app, ui),
        (Group::Properties, _) => adjustments_grid(app, ui),
        (Group::Character, tab) => crate::type_tool::character_panel(app, ui, tab == 1),
        (Group::Navigator, 0) => navigator(app, ui),
        (Group::Navigator, 1) => crate::tone::histogram_panel(app, ui),
        (Group::Navigator, _) => info_panel(app, ui),
        (Group::History, 0) => history(app, ui),
        (Group::History, 1) => crate::actions::panel(app, ui),
        (Group::History, _) => crate::comps_ui::panel(app, ui),
        (Group::Layers, 0) => layers(app, ui),
        (Group::Layers, 1) => channels(app, ui),
        (Group::Layers, _) => crate::vector_ui::paths_panel(app, ui),
    }
}

/// Photoshop's Info panel: colour under the pointer (RGB and CMYK), position, selection size.
fn info_panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(st) = app.session.active() else {
        empty(ui, tl!("No document"));
        return;
    };
    let (rev, size) = (st.revision, st.doc.size);
    // Positions and sizes in the ruler unit (Preferences › Units & Rulers).
    let units = app.session.prefs().units_and_rulers.clone();
    let dpi = st.doc.resolution_dpi as f64;
    let fx = move |px: f64| units.format(px, dpi, size.width as f64);
    let units_y = app.session.prefs().units_and_rulers.clone();
    let fy = move |px: f64| units_y.format(px, dpi, size.height as f64);
    let sel = st.doc.selection.as_ref().map(photocraft_compose::bounds::content_bounds);
    let pos = app
        .hover_doc
        .map(|p| (p[0].floor() as i32, p[1].floor() as i32))
        .filter(|(x, y)| *x >= 0 && *y >= 0 && *x < size.width as i32 && *y < size.height as i32);
    // The readout averages the Eyedropper's Sample Size, as in Photoshop (#1649).
    let sample_size = app.ui.tool_options.eyedropper_size;
    let rgba = pos.and_then(|(x, y)| {
        if let Some(((cx, cy, cr, cs), v)) = app.info_sample
            && (cx, cy, cr, cs) == (x, y, rev, sample_size)
        {
            return Some(v);
        }
        let params = json!({"x": f64::from(x) + 0.5, "y": f64::from(y) + 0.5, "size": sample_size});
        let v: Vec<f32> = serde_json::from_value(app.session.execute("document.sampleColor", params).ok()?).ok()?;
        let v = [*v.first()?, *v.get(1)?, *v.get(2)?, *v.get(3)?];
        app.info_sample = Some(((x, y, rev, sample_size), v));
        Some(v)
    });
    let mono = theme::mono(11.5);
    let row = |ui: &mut egui::Ui, k: &str, v: String| {
        ui.horizontal(|ui| {
            ui.add_sized([18.0, 16.0], egui::Label::new(RichText::new(k).color(t.text_dim).size(11.5)));
            ui.label(RichText::new(v).font(mono.clone()).color(t.text));
        });
    };
    ui.columns(2, |cols| {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as i32;
        let dash = || "—".to_string();
        row(&mut cols[0], "R:", rgba.map_or_else(dash, |c| q(c[0]).to_string()));
        row(&mut cols[0], "G:", rgba.map_or_else(dash, |c| q(c[1]).to_string()));
        row(&mut cols[0], "B:", rgba.map_or_else(dash, |c| q(c[2]).to_string()));
        // Naive device CMYK (no ICC profile yet), in percent like Photoshop's readout. The Color
        // Sampler readout uses the same conversion (`sampler_cmds::device_cmyk`).
        let cmyk = rgba.map(|c| photocraft_engine::sampler_cmds::device_cmyk(c[0], c[1], c[2]));
        for (i, k) in ["C:", "M:", "Y:", "K:"].iter().enumerate() {
            row(&mut cols[1], k, cmyk.map_or_else(dash, |c| format!("{}%", c[i])));
        }
    });
    widgets::hairline(ui);
    ui.columns(2, |cols| {
        let dash = || "—".to_string();
        row(&mut cols[0], "X:", pos.map_or_else(dash, |p| fx(p.0 as f64)));
        row(&mut cols[0], "Y:", pos.map_or_else(dash, |p| fy(p.1 as f64)));
        row(&mut cols[1], "W:", sel.map_or_else(dash, |r| fx(r.width() as f64)));
        row(&mut cols[1], "H:", sel.map_or_else(dash, |r| fy(r.height() as f64)));
    });
    widgets::hairline(ui);
    ui.label(
        RichText::new(crate::i18n::fmt(tl!("Doc: {w} × {h} px"), &[("w", &size.width.to_string()), ("h", &size.height.to_string())]))
            .color(t.text_dim)
            .size(11.5),
    );
    // Color Sampler points (#1046): each under its number, with its position in the current ruler
    // unit and the same RGB/CMYK readout as the pointer (one composite pixel each).
    let samplers = app.session.execute("view.colorSamplers.list", json!({})).ok();
    let samplers = samplers.as_ref().and_then(|v| v["samplers"].as_array()).cloned().unwrap_or_default();
    if !samplers.is_empty() {
        widgets::hairline(ui);
        let cell = |ui: &mut egui::Ui, s: String| {
            ui.label(RichText::new(s).font(mono.clone()).color(t.text));
        };
        for s in &samplers {
            let n = s["number"].as_i64().unwrap_or(0);
            let at = s["position"]
                .as_array()
                .map_or((0.0, 0.0), |a| (a.first().and_then(Value::as_f64).unwrap_or(0.0), a.get(1).and_then(Value::as_f64).unwrap_or(0.0)));
            let num = |v: &Value, i: usize| v.get(i).and_then(Value::as_i64).map_or_else(|| "—".to_string(), |n| n.to_string());
            let (rgb, cmyk) = (&s["rgb"], &s["cmyk"]);
            ui.horizontal(|ui| {
                ui.add_sized([24.0, 16.0], egui::Label::new(RichText::new(format!("#{n}")).color(t.text_dim).size(11.5)));
                cell(ui, format!("X: {}  Y: {}", fx(at.0), fy(at.1)));
            });
            ui.horizontal(|ui| {
                ui.add_sized([24.0, 16.0], egui::Label::new(RichText::new("").size(11.5)));
                cell(ui, format!("R:{} G:{} B:{}", num(rgb, 0), num(rgb, 1), num(rgb, 2)));
            });
            ui.horizontal(|ui| {
                ui.add_sized([24.0, 16.0], egui::Label::new(RichText::new("").size(11.5)));
                cell(ui, format!("C:{}% M:{}% Y:{}% K:{}%", num(cmyk, 0), num(cmyk, 1), num(cmyk, 2), num(cmyk, 3)));
            });
        }
    }
}

fn navigator(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(idx) = app.session.active_index() else {
        empty(ui, tl!("No document"));
        return;
    };
    let ctx = ui.ctx().clone();
    let Some(tex) = crate::canvas::navigator_texture(app, &ctx, idx) else { return };
    let size = app.session.documents()[idx].doc.size;
    let avail = ui.available_width();
    // The preview fills the group's height above the zoom controls (their height as measured
    // last frame), so it grows when the group is made taller, as in Photoshop (#2726).
    let controls_h = ui.data(|d| d.get_temp::<f32>(navigator_controls_id())).unwrap_or(48.0);
    let left = ui.available_height() - controls_h;
    let box_h = if left.is_finite() { left.max(40.0) } else { 150.0 };
    let (frame, resp) = ui.allocate_exact_size(vec2(avail, box_h), Sense::click_and_drag());
    ui.painter().rect_filled(frame, t.radius_sm, t.canvas);
    let rect = navigator_fit(frame, size.width, size.height);
    ui.data_mut(|d| d.insert_temp(navigator_preview_id(), rect));
    widgets::checker(ui.painter(), rect, 6.0);
    ui.painter().image(tex, rect, Rect::from_min_max(egui::Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
    let s = rect.width() / size.width.max(1) as f32;
    if (resp.clicked() || resp.dragged())
        && let Some(pp) = resp.interact_pointer_pos()
    {
        let d = (pp - rect.min) / s;
        app.ui.views[idx].center = [d.x.clamp(0.0, size.width as f32), d.y.clamp(0.0, size.height as f32)];
        app.ui.views[idx].fit_pending = false;
    }
    // The Navigator has its own zoom target: an unmodified wheel over the preview
    // should zoom even when the document canvas is configured to scroll on wheel.
    // Use the canvas wheel classifier to honour one ×1.1 step per notch without
    // applying egui's smoothed wheel tail a second time.
    let pinch_zooms = app.session.prefs().enhanced_controls.zoom_with_trackpad_pinch;
    if resp.hovered()
        && let Some(crate::wheel_nav::Wheel::Zoom(factor)) = crate::wheel_nav::read(&ctx, true, pinch_zooms)
    {
        let view = &mut app.ui.views[idx];
        let next = crate::zoom_levels::clamp(view.zoom * factor, view.doc_size);
        if next != view.zoom {
            view.zoom = next;
            view.fit_pending = false;
        }
    }
    // Visible-area rectangle.
    let v = app.ui.views[idx].clone();
    let canvas = app.last_canvas_rect;
    let point_zoom = (v.zoom / app.canvas_ppp()).max(1e-6);
    // Pixel aspect correction shows fewer document columns across the same canvas width.
    let vw = canvas.width() / point_zoom / app.ui.view.display_aspect() * s;
    let vh = canvas.height() / point_zoom * s;
    let c = pos2(rect.min.x + v.center[0] * s, rect.min.y + v.center[1] * s);
    let vr = Rect::from_center_size(c, vec2(vw, vh)).intersect(frame.shrink(1.0));
    ui.painter().rect_stroke(vr, 2.0, Stroke::new(1.5, Color32::from_rgb(255, 84, 84)), StrokeKind::Middle);
    let controls = ui.vertical(|ui| {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            label(ui, crate::i18n::tr_ctx(crate::i18n::current(), "navigator", "Zoom"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                for (lbl, z) in [("200%", 2.0), ("100%", 1.0)] {
                    if widgets::pill_tab(ui, lbl, (v.zoom - z).abs() < 1e-3).clicked() {
                        app.ui.views[idx].zoom = z;
                        app.ui.views[idx].fit_pending = false;
                    }
                }
                if widgets::pill_tab(ui, tl!("Fit"), false).clicked() {
                    app.ui.views[idx].fit_pending = true;
                }
            });
        });
        // The whole zoom range, and always the current zoom: a narrower slider would pull it back.
        let (lo, hi) = (crate::zoom_levels::min(v.doc_size).min(v.zoom).log2(), crate::zoom_levels::MAX.max(v.zoom).log2());
        let mut lz = v.zoom.log2();
        if lz.is_finite() && widgets::slider(ui, &mut lz, lo..=hi, None).changed() {
            app.ui.views[idx].zoom = crate::zoom_levels::clamp(2f32.powf(lz), v.doc_size);
            app.ui.views[idx].fit_pending = false;
        }
    });
    // From the preview's bottom, so the spacing above the controls counts too.
    let controls = Rect::from_min_max(pos2(frame.left(), frame.bottom()), pos2(frame.right(), controls.response.rect.bottom()));
    ui.data_mut(|d| {
        d.insert_temp(navigator_controls_id(), controls.height());
        d.insert_temp(navigator_controls_rect_id(), controls);
    });
}

/// The document's `w` × `h` fitted inside the Navigator's `frame` with an 8 pt inset, centred and
/// keeping its aspect.
fn navigator_fit(frame: Rect, w: u32, h: u32) -> Rect {
    let inner = (frame.size() - vec2(16.0, 16.0)).max(vec2(1.0, 1.0));
    let (w, h) = (w.max(1) as f32, h.max(1) as f32);
    let s = (inner.x / w).min(inner.y / h);
    Rect::from_center_size(frame.center(), vec2(w * s, h * s))
}

fn navigator_preview_id() -> egui::Id {
    egui::Id::new("navigator-preview-rect")
}

fn navigator_controls_id() -> egui::Id {
    egui::Id::new("navigator-controls-height")
}

fn navigator_controls_rect_id() -> egui::Id {
    egui::Id::new("navigator-controls-rect")
}

/// Where the Navigator drew its document preview last frame.
#[cfg(test)]
use toolbar::color_chips;
pub use toolbar::toolbar;

/// Photoshop's Shift+Alt letter position of blend mode `m`; `None` for the modes without a letter (Darker Color,
/// Lighter Color, Subtract, Divide) and for PhotoCraft's own extra modes.
pub(crate) fn blend_shortcut(m: BlendMode) -> Option<BlendShortcut> {
    Some(match m {
        BlendMode::Normal => BlendShortcut::Normal,
        BlendMode::Dissolve => BlendShortcut::Dissolve,
        BlendMode::PassThrough => BlendShortcut::PassThrough,
        BlendMode::Darken => BlendShortcut::Darken,
        BlendMode::Multiply => BlendShortcut::Multiply,
        BlendMode::ColorBurn => BlendShortcut::ColorBurn,
        BlendMode::LinearBurn => BlendShortcut::LinearBurn,
        BlendMode::Lighten => BlendShortcut::Lighten,
        BlendMode::Screen => BlendShortcut::Screen,
        BlendMode::ColorDodge => BlendShortcut::ColorDodge,
        BlendMode::LinearDodge => BlendShortcut::LinearDodge,
        BlendMode::Overlay => BlendShortcut::Overlay,
        BlendMode::SoftLight => BlendShortcut::SoftLight,
        BlendMode::HardLight => BlendShortcut::HardLight,
        BlendMode::VividLight => BlendShortcut::VividLight,
        BlendMode::LinearLight => BlendShortcut::LinearLight,
        BlendMode::PinLight => BlendShortcut::PinLight,
        BlendMode::HardMix => BlendShortcut::HardMix,
        BlendMode::Difference => BlendShortcut::Difference,
        BlendMode::Exclusion => BlendShortcut::Exclusion,
        BlendMode::Hue => BlendShortcut::Hue,
        BlendMode::Saturation => BlendShortcut::Saturation,
        BlendMode::Color => BlendShortcut::Color,
        BlendMode::Luminosity => BlendShortcut::Luminosity,
        _ => return None,
    })
}

/// The Navigator's zoom controls (below the preview) last frame.
#[cfg(test)]
pub(crate) fn last_navigator_controls(ctx: &egui::Context) -> Option<Rect> {
    ctx.data(|d| d.get_temp(navigator_controls_rect_id()))
}

#[cfg(test)]
mod navigator_wheel_tests {
    use super::*;
    use egui::{Event, Modifiers, MouseWheelUnit, RawInput, TouchPhase};

    fn frame(app: &mut PhotocraftApp, ctx: &egui::Context, pointer: Pos2, wheel: f32) {
        let mut events = vec![Event::PointerMoved(pointer)];
        if wheel != 0.0 {
            events.push(Event::MouseWheel { unit: MouseWheelUnit::Line, delta: vec2(0.0, wheel), phase: TouchPhase::Move, modifiers: Modifiers::NONE });
        }
        let mut out = ctx
            .run_ui(RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(430.0, 460.0))), events, ..Default::default() }, |ui| navigator(app, ui));
        out.textures_delta.clear();
    }

    #[test]
    fn navigator_wheel_changes_zoom_only_when_over_preview() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        app.sync_views();
        app.ui.views[0].zoom = 1.0;
        app.ui.views[0].fit_pending = false;
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
        crate::wheel_nav::configure(&ctx);
        frame(&mut app, &ctx, pos2(50.0, 55.0), 0.0);
        let center = app.ui.views[0].center;
        frame(&mut app, &ctx, pos2(50.0, 55.0), 1.0);
        assert!((app.ui.views[0].zoom - 1.1).abs() < 1e-4, "one wheel notch zooms once");
        assert_eq!(app.ui.views[0].center, center, "Navigator wheel must not pan");
        frame(&mut app, &ctx, pos2(50.0, 55.0), -1.0);
        assert!((app.ui.views[0].zoom - 1.0).abs() < 1e-4);
        // Below the preview (it now fills the group, #2726): over the zoom controls.
        let below = last_navigator_controls(&ctx).map_or(pos2(50.0, 455.0), |r| pos2(r.left() + 4.0, r.bottom() - 2.0));
        frame(&mut app, &ctx, below, 1.0);
        assert!((app.ui.views[0].zoom - 1.0).abs() < 1e-4, "wheel outside preview leaves zoom unchanged");
    }
}

fn empty(ui: &mut egui::Ui, s: &str) {
    let t = Tokens::get(ui.ctx());
    ui.add_space(6.0);
    ui.label(RichText::new(s).color(t.text_faint));
    ui.add_space(6.0);
}

/// The Color tab of the themes other than Photoshop's: the same spectrum field (chips, saturation
/// and brightness field, hue strip and readout) with HSB sliders under it (#4367).
fn color_picker(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    color_field(app, ui);
    ui.add_space(8.0);
    hsb_sliders(app, ui);
}

/// Hue, Saturation and Brightness sliders for the foreground colour.
fn hsb_sliders(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let fg = app.session.tools.foreground;
    // Keep the last edited HSB while it still gives the foreground: black and greys have no hue or
    // saturation of their own, so recomputing them from RGB would reset what was just typed to 0.
    let key = egui::Id::new("color-picker-hsva");
    let stored: Option<egui::ecolor::Hsva> = ui.data(|d| d.get_temp(key));
    let hsva0 = stored.filter(|h| srgb_bytes(hsva_srgb(*h)) == srgb_bytes(fg)).unwrap_or_else(|| srgb_hsva(fg));
    let mut h = hsva0.h * 360.0;
    let mut s = hsva0.s * 100.0;
    let mut v = hsva0.v * 100.0;
    let hue = widgets::hue_stops();
    // Preferences ▸ Interface ▸ Dynamic Color Sliders: off, the tracks are the plain ones.
    let dynamic = app.session.prefs().interface.dynamic_color_sliders;
    let mut changed = widgets::slider_row(ui, tl!("Hue"), &mut h, 0.0..=360.0, "°", dynamic.then_some(&hue)).changed();
    let sat_stops =
        [egui::ecolor::Hsva::new(hsva0.h, 0.0, hsva0.v.max(0.2), 1.0), egui::ecolor::Hsva::new(hsva0.h, 1.0, hsva0.v.max(0.2), 1.0)].map(hsva_color32);
    changed |= widgets::slider_row(ui, tl!("Saturation"), &mut s, 0.0..=100.0, "%", dynamic.then_some(&sat_stops)).changed();
    let val_stops = [Color32::BLACK, hsva_color32(egui::ecolor::Hsva::new(hsva0.h, hsva0.s, 1.0, 1.0))];
    changed |= widgets::slider_row(ui, tl!("Brightness"), &mut v, 0.0..=100.0, "%", dynamic.then_some(&val_stops)).changed();
    let hsva = egui::ecolor::Hsva::new(h / 360.0, s / 100.0, v / 100.0, 1.0);
    // Only an edit counts: the h/s/v round trip isn't exact, so comparing values would rewrite
    // the foreground (and recolour selected type) every frame.
    if changed {
        app.session.tools.foreground = hsva_srgb(hsva);
        ui.data_mut(|d| d.insert_temp(key, hsva));
        crate::type_tool::foreground_changed(app);
    }
}

// ----------------------------------------------------------------------------- layers

/// The simple themes' single lock button. On the Background it converts the layer to a normal
/// one, as clicking the Background's lock does in Photoshop. Elsewhere it clears every lock when
/// any is set, and locks all otherwise. Toggling only "lock all" would leave the Background's
/// transparency lock (still showing as locked), and a second click would lock its pixels,
/// shutting out the Eraser and every other paint tool (#76).
fn simple_lock_toggle(background: bool, l: &Layer) -> (String, Value) {
    if background {
        return ("layer.new.layerFromBackground".into(), json!({}));
    }
    let locks =
        if l.locks.any() { json!({"transparency": false, "pixels": false, "position": false, "artboard": false, "all": false}) } else { json!({"all": true}) };
    ("layer.setProps".into(), json!({"layer": l.id.0, "locks": locks}))
}

/// The Layers panel's blend modes: Photoshop's, plus the layer's own Paint.NET mode when it has one
/// (an imported `.pdn`), so the menu matches Photoshop's for every other layer.
fn blend_options(groups: bool, current: BlendMode) -> Vec<(BlendMode, &'static str)> {
    let own = Some(current).filter(|m| !m.has_psd_equivalent());
    std::iter::once(BlendMode::PassThrough).filter(|_| groups).chain(BlendMode::LAYER_MODES).chain(own).map(|m| (m, m.label())).collect()
}

/// Scroll the Layers panel while holding a layer drag over its top/bottom edge or past them.
///
/// Returns the *content* displacement in points for this frame, so positive moves the
/// list downward (reveals rows above) and negative upward (reveals rows below).
/// The speed ramps with proximity to the edge and clamps to the maximum speed when
/// dragged outside, using elapsed time instead of assuming a particular refresh rate.
fn layer_drag_edge_scroll(pointer: Option<Pos2>, viewport: Rect, dragging: bool, dt: f32) -> f32 {
    if !dragging || viewport.width() <= 0.0 || viewport.height() <= 0.0 || dt.is_nan() || dt <= 0.0 {
        return 0.0;
    }
    let Some(pointer) = pointer.filter(|p| p.x.is_finite() && p.y.is_finite() && p.x >= viewport.left() && p.x <= viewport.right()) else {
        return 0.0;
    };
    let edge = 32.0_f32.min(viewport.height() * 0.25);
    let top = ((edge - (pointer.y - viewport.top())) / edge).clamp(0.0, 1.0);
    let bottom = ((edge - (viewport.bottom() - pointer.y)) / edge).clamp(0.0, 1.0);
    let direction = top - bottom;
    if direction == 0.0 {
        return 0.0;
    }
    let velocity = 80.0 + 520.0 * direction.abs();
    direction.signum() * velocity * dt.clamp(0.0, 0.05)
}

/// Fit blend mode + Opacity into the available Layers-panel width (#2309).
/// Preserve the normal translated label when there is room; on a narrow dock,
/// keep the field interactive and give the dropdown the remaining width.
fn opacity_row_layout(available: f32, label_width: f32, gap: f32) -> (f32, bool) {
    let full_right = label_width + LAYER_PCT_W + 2.0 * gap + 16.0;
    // The label shows while the blend dropdown keeps a usable width (at the default dock width
    // with an English label it does; a long translated label on a narrow dock gives way).
    let show_label = available >= full_right + 80.0;
    let reserved = LAYER_PCT_W + gap + if show_label { label_width + gap + 16.0 } else { 0.0 };
    ((available - reserved).max(40.0), show_label)
}

fn layers(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    // A new active layer opens its parent groups and is scrolled into view (#152).
    let reveal = crate::layer_reveal::track(app, ui.ctx());
    let Some(st) = app.session.active() else {
        empty(ui, tl!("No document"));
        return;
    };
    let doc = st.doc.clone();
    let active = st.active_layer;
    let active_layer = active.and_then(|id| doc.layer(id));
    let selection = st.selected_layers();
    let isolated = st.isolated_layers.clone();
    let mut actions: Vec<(String, Value)> = Vec::new();

    let t = Tokens::get(ui.ctx());
    if t.pro {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let (r, _) = ui.allocate_exact_size(vec2(64.0, 22.0), Sense::hover());
            ui.painter().rect_filled(r, t.radius_sm, t.field);
            ui.painter().rect_stroke(r, t.radius_sm, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
            icons::paint(ui, Rect::from_center_size(r.left_center() + vec2(11.0, 0.0), vec2(14.0, 14.0)), "search", 11.0, t.text_faint);
            ui.painter().text(r.left_center() + vec2(22.0, 0.0), Align2::LEFT_CENTER, tl!("Kind"), egui::FontId::proportional(11.5), t.text_dim);
            ui.add_space(4.0);
            for (kind, icon, tip) in [
                ("pixel", "image", tl!("Filter for pixel layers")),
                ("adjustment", "adjustment-layer", tl!("Filter for adjustment layers")),
                ("type", "type", tl!("Filter for type layers")),
                ("shape", "square", tl!("Filter for shape layers")),
                ("smart", "app-window", tl!("Filter for smart objects")),
            ] {
                let on = app.ui.layer_filter.iter().any(|k| k == kind);
                if icons::button(ui, icon, 22.0, on, tip).clicked() {
                    if on {
                        app.ui.layer_filter.retain(|k| k != kind);
                    } else {
                        app.ui.layer_filter.push(kind.to_string());
                    }
                }
            }
        });
        ui.add_space(4.0);
    }
    if let Some(l) = active_layer {
        // Photoshop greys blend mode, Opacity and Fill for the Background layer.
        let bg = crate::doc_props_ui::is_background(&doc, l);
        ui.horizontal(|ui| {
            ui.add_enabled_ui(!bg, |ui| {
                let mut m = l.blend;
                // Reserve space for the numeric field first. On a narrow dock, hide the
                // redundant visible Opacity label rather than force the dropdown beyond
                // the panel's right edge (#2309). The numeric field keeps its name.
                let opacity_label = if t.pro { tl!("Opacity:") } else { tl!("Opacity") };
                let (blend_width, show_opacity_label) =
                    opacity_row_layout(ui.available_width(), body_text_width(ui, opacity_label), ui.spacing().item_spacing.x);
                let (chosen, hovered) = widgets::dropdown_wheel_hovered(ui, "blend", &mut m, &blend_options(l.is_group(), l.blend), blend_width);
                // One step per choice: a click, an arrow key or each wheel notch (#1747).
                for m in &chosen {
                    actions.push(("layer.setProps".into(), json!({"layer": l.id.0, "blend": m.label()})));
                }
                // Hovering a mode previews it on the canvas (#970).
                crate::blend_preview::hover(app, l.id, hovered.filter(|_| chosen.is_empty()));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut o = l.opacity * 100.0;
                    let field = widgets::popup_value_field(ui, opacity_label, &mut o, 0.0..=100.0, "%", LAYER_PCT_W);
                    // On a narrow dock the (scrubby) label yields its room to the controls (#2309).
                    let scrub = if show_opacity_label {
                        scrub_pct_label(ui, opacity_label, &mut o, ui.available_width())
                    } else {
                        widgets::PopupFieldResponse::default()
                    };
                    if field.changed || scrub.changed {
                        actions.push(pct_action(l, "opacity", o, scrub.drag.or(field.drag)));
                    }
                });
            });
        });
        // Docks zero the item spacing: keep the Opacity and Fill fields apart (#155).
        ui.add_space((theme::ROW_GAP - ui.spacing().item_spacing.y).max(0.0));
        ui.horizontal(|ui| {
            // A "···" button instead of the "Lock:" text: pens can't right-click, so the row
            // menu opens here for the selected layer.
            let menu_btn = icons::button(ui, "ellipsis", 20.0, false, tl!("Layer options"));
            egui::Popup::menu(&menu_btn).show(|ui| {
                crate::widgets::style_spectrum_popup_menu(ui);
                // Like right-clicking inside a multi-selection: act on every selected layer.
                let on_set = selection.len() > 1;
                if crate::layer_menu_ui::show(app, ui, l, on_set, &mut actions)
                    && let Some(done) = crate::layer_row_ui::start_rename(ui.ctx(), l.id.0, &l.name)
                {
                    actions.push(done);
                }
            });
            // Layer fill opacity (≠ Edit › Fill / path fill → Tô).
            let lang = crate::i18n::current();
            let fill_label = if t.pro { crate::i18n::tr_ctx(lang, "layers", "Fill:") } else { crate::i18n::tr_ctx(lang, "layers", "Fill") };
            if t.pro {
                // The lock icons touch, but only they: the Fill label keeps the same gap before its
                // field as Opacity's, so both stack in one right-aligned column (#2085).
                ui.scope(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let lk = l.locks;
                    for (key, icon, tip, on) in [
                        ("transparency", "grid-3x3", tl!("Lock transparent pixels"), lk.transparency),
                        ("pixels", "brush", tl!("Lock image pixels"), lk.pixels),
                        ("position", "move", tl!("Lock position"), lk.position),
                        ("artboard", "scan", tl!("Prevent auto-nesting in and out of Artboards and Frames"), lk.artboard),
                        ("all", "lock", tl!("Lock all"), lk.all),
                    ] {
                        if icons::button(ui, icon, 20.0, on, tip).clicked() {
                            actions.push(("layer.setProps".into(), json!({"layer": l.id.0, "locks": { key: !on }})));
                        }
                    }
                });
            }
            if !t.pro {
                let locked = l.locks.any();
                if icons::button(ui, if locked { "lock" } else { "lock-open" }, 22.0, locked, tl!("Lock layer")).clicked() {
                    actions.push(simple_lock_toggle(bg, l));
                }
            }
            ui.add_enabled_ui(!bg, |ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut f = l.fill_opacity * 100.0;
                    let field = widgets::popup_value_field(ui, fill_label, &mut f, 0.0..=100.0, "%", LAYER_PCT_W);
                    let scrub = scrub_pct_label(ui, fill_label, &mut f, ui.available_width());
                    if field.changed || scrub.changed {
                        actions.push(pct_action(l, "fill", f, scrub.drag.or(field.drag)));
                    }
                })
            });
        });
        ui.add_space(2.0);
    }

    // Top of the stack first, groups above their contents, closed groups' contents hidden (#126).
    let rows = crate::layer_tree_ui::panel_rows(app, &doc);
    let ctx = ui.ctx().clone();
    let footer = widgets::footer_height(ui) + ui.spacing().item_spacing.y;
    let fill = ui.available_height() > footer + 60.0;
    let rows_h = if fill { ui.available_height() - footer } else { f32::INFINITY };
    egui::ScrollArea::vertical()
        .id_salt("layer-rows")
        .max_height(rows_h)
        .min_scrolled_height(if fill { rows_h } else { 0.0 })
        .auto_shrink([false, !fill])
        .show(ui, |ui| {
            // The drag keys are set only by actual layer-row and fx drags, not clicks or
            // ordinary scrolling. The ScrollArea applies this to its own content.
            // An fx drag that never saw its release (the panel was hidden) must not outlive the button.
            if !ctx.input(|i| i.pointer.primary_down() || i.pointer.any_released()) {
                ctx.data_mut(|d| d.remove::<FxDrag>(fx_drag_key()));
                ctx.data_mut(|d| d.remove::<MaskDrag>(mask_drag_key()));
                ctx.data_mut(|d| d.remove::<crate::smart_ui::FilterDrag>(crate::smart_ui::drag_key()));
            }
            let held = ctx.data(|d| {
                d.get_temp::<u64>(egui::Id::new("layer-drag")).is_some()
                    || d.get_temp::<FxDrag>(fx_drag_key()).is_some()
                    || d.get_temp::<crate::smart_ui::FilterDrag>(crate::smart_ui::drag_key()).is_some()
            });
            let dragging = held && ctx.input(|i| i.pointer.primary_down());
            let pointer = ctx.input(|i| i.pointer.interact_pos());
            let delta = layer_drag_edge_scroll(pointer, ui.clip_rect(), dragging, ctx.input(|i| i.stable_dt));
            if delta != 0.0 {
                ui.scroll_with_delta(vec2(0.0, delta));
                ctx.request_repaint();
            }
            let filter = app.ui.layer_filter.clone();
            let fx_collapsed = app.session.active().map(|d| d.fx_collapsed.clone()).unwrap_or_default();
            crate::layer_row_ui::begin(ui.ctx());
            // The drawn rows, top to bottom (#1721): the space between them and below the last
            // one is a drop zone too.
            let mut drawn: Vec<(LayerId, Rect)> = Vec::new();
            for &(depth, l) in &rows {
                // Select › Isolate Layers.
                if !photocraft_engine::select_extra_cmds::isolation_shows(&doc, &isolated, l.id) {
                    continue;
                }
                if !filter.is_empty() {
                    let kind = match &l.content {
                        LayerContent::Raster(_) => "pixel",
                        LayerContent::Adjustment(_) | LayerContent::Fill(_) => "adjustment",
                        LayerContent::Text(_) => "type",
                        LayerContent::Shape(_) => "shape",
                        LayerContent::Smart(_) => "smart",
                        LayerContent::Group(_) => "",
                    };
                    if !filter.iter().any(|k| k == kind) {
                        continue;
                    }
                }
                let row = RowSel { selected: selection.contains(&l.id), primary: active == Some(l.id), multi: selection.len() > 1 };
                let top = ui.cursor().top();
                drawn.push((l.id, layer_row(app, &ctx, ui, &doc, l, depth, row, &mut actions)));
                if reveal == Some(l.id) {
                    crate::layer_reveal::scroll_to_row(ui, top);
                }
                if !l.effects.items.is_empty() && fx_collapsed.iter().all(|id| *id != l.id) {
                    effect_rows(app, ui, l, depth, &mut actions);
                }
                crate::smart_ui::filter_rows(app, ui, l, depth, &mut actions);
            }
            // #1721: the list takes drops everywhere, not only on a row (Photoshop): the space
            // between two rows lands above the lower one, the empty space below the bottom-most
            // row below it. Both copy with ⌥ held, move without.
            for w in drawn.windows(2) {
                let gap = Rect::from_min_max(pos2(w[0].1.left(), w[0].1.bottom()), pos2(w[0].1.right(), w[1].1.top()));
                layer_drop_zone(app, &ctx, ui, gap, (w[1].0, w[1].1), "above", &mut actions);
            }
            if let Some(&(last, last_rect)) = drawn.last() {
                let below = Rect::from_min_max(pos2(last_rect.left(), last_rect.bottom()), pos2(last_rect.right(), ui.clip_rect().bottom()));
                layer_drop_zone(app, &ctx, ui, below, (last, last_rect), "below", &mut actions);
            }
            // Tab / ⇧Tab in a rename: rename the layer below / above next, as in Photoshop. The
            // Background isn't renamed in place (double-clicking it converts it), so it ends there.
            if let Some((from, up)) = crate::layer_row_ui::take_next_rename(ui.ctx())
                && let Some(at) = drawn.iter().position(|(id, _)| id.0 == from)
                && let Some(&(next, _)) = if up { at.checked_sub(1).and_then(|i| drawn.get(i)) } else { drawn.get(at + 1) }
                && let Some(l) = doc.layer(next).filter(|l| !crate::doc_props_ui::is_background(&doc, l))
                && let Some(done) = crate::layer_row_ui::start_rename(ui.ctx(), next.0, &l.name)
            {
                actions.push(done);
            }
            // ⌥-click the line between two layers: clip / release the upper one (#967).
            crate::clip_line_ui::show(ui, &doc, &mut actions);
            // A rename whose row is gone (deleted, filtered out, inside a closed group) ends,
            // committed: nothing else could commit or cancel it.
            if let Some(layer) = crate::layer_row_ui::renaming(ui.ctx())
                && !crate::layer_row_ui::recorded(ui.ctx()).iter().any(|r| r.layer == layer)
                && let Some(done) = crate::layer_row_ui::end_rename(ui.ctx(), true)
            {
                actions.push(done);
            }
        });
    // A layer being dragged can also be dropped on the footer's Delete, New Layer and Group
    // buttons (#736); read the drag before it ends.
    let footer_drag = ctx.data(|d| d.get_temp::<u64>(egui::Id::new("layer-drag"))).map(|id| {
        let in_selection = selection.iter().any(|s| s.0 == id);
        (id, in_selection)
    });
    // An effect row can be dropped on Delete too (#3102), and a mask's thumbnail (#3113, #3119).
    let fx_footer_drag = ctx.data(|d| d.get_temp::<FxDrag>(fx_drag_key()));
    fx_drag_feedback(&ctx, &doc);
    // And a smart filter's row (#4271).
    let filter_footer_drag = ctx.data(|d| d.get_temp::<crate::smart_ui::FilterDrag>(crate::smart_ui::drag_key()));
    crate::smart_ui::drag_feedback(&ctx, &doc);
    let mask_footer_drag = ctx.data(|d| d.get_temp::<MaskDrag>(mask_drag_key()));
    if let Some((_, kind)) = mask_footer_drag
        && let Some(p) = ctx.input(|i| i.pointer.interact_pos())
    {
        let label = if kind == MaskKind::Vector { tl!("Vector Mask") } else { tl!("Layer Mask") };
        crate::layer_transfer::ghost(&ctx, p, label);
    }
    // End any layer drag after every row has had a chance to accept the drop.
    if ctx.input(|i| i.pointer.any_released()) {
        ctx.data_mut(|d| d.remove::<u64>(egui::Id::new("layer-drag")));
        ctx.data_mut(|d| d.remove::<FxDrag>(fx_drag_key()));
        ctx.data_mut(|d| d.remove::<MaskDrag>(mask_drag_key()));
        ctx.data_mut(|d| d.remove::<crate::smart_ui::FilterDrag>(crate::smart_ui::drag_key()));
    }
    widgets::panel_footer(ui, |ui| {
        let active_layer = active.and_then(|id| doc.layer(id));
        let (delete, label) = if app.ui.vector_mask_target && active_layer.is_some_and(|l| l.vector_mask.is_some()) {
            ("layer.vectorMask.delete", "Delete Vector Mask")
        } else if app.ui.mask_target && active_layer.is_some_and(|l| l.mask.is_some()) {
            ("layer.layerMask.delete", "Delete Layer Mask")
        } else {
            ("layer.delete", "Delete layer")
        };
        let trash = icons::button(ui, "trash", 26.0, false, tl!(label));
        if trash.clicked() {
            // Like Photoshop, ⌥/Alt-click deletes without asking.
            let confirm = delete == "layer.delete" && !ui.input(|i| i.modifiers.alt);
            let params = if confirm { json!({"__trash": true}) } else { json!({}) };
            actions.push((delete.into(), params));
        }
        actions.extend(footer_drop(ui, &trash, footer_drag, "layer.delete"));
        actions.extend(fx_trash_drop(ui, &trash, fx_footer_drag));
        if let Some(drag) = filter_footer_drag
            && dropped_on(ui, &trash)
        {
            actions.push(crate::smart_ui::trash_action(drag));
        }
        // Photoshop asks "Apply mask to layer before removing?" or "Delete the vector mask?"
        // (`mask_trash_prompt`).
        if let Some((layer, kind)) = mask_footer_drag
            && dropped_on(ui, &trash)
        {
            actions.push(crate::mask_trash_prompt::drop_action(layer, kind));
        }
        // A click follows the selected thumbnail; name that action for screen readers even
        // though dropping a whole layer row onto the same button always deletes the layer.
        trash.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tl!(label)));
        let new_layer = icons::button(ui, "square-plus", 26.0, false, &crate::shortcuts::tip_label(app, "Create a new layer", "layer.new.layer"));
        if new_layer.clicked() {
            // A click makes the layer; ⌥-click asks first (the New Layer dialog), as in Photoshop.
            let alt = ui.input(|i| i.modifiers.alt);
            actions.push(("layer.new.layer".into(), if alt { Value::Null } else { json!({}) }));
        }
        actions.extend(footer_drop(ui, &new_layer, footer_drag, "layer.duplicate"));
        let group = icons::button(ui, "folder", 26.0, false, tl!("Create a new group"));
        if group.clicked() {
            // Like Photoshop a plain click adds an empty group; Shift-click groups the selected
            // layers, the same as Layer › Group Layers (#2561).
            let shift = ui.input(|i| i.modifiers.shift);
            let cmd = if shift && active_layer.is_some() { "layer.groupLayers" } else { "layer.new.group" };
            actions.push((cmd.into(), json!({})));
        }
        actions.extend(footer_drop(ui, &group, footer_drag, "layer.groupLayers"));
        let adj = footer_menu_button(ui, "adjustment-layer", 26.0, tl!("Create new fill or adjustment layer"));
        // Opens on the press like Photoshop's: drag onto an item and release to choose it (#2071).
        crate::press_menu::show(&adj, |ui, menu| {
            ui.set_min_width(190.0);
            for c in photocraft_engine::command_specs().iter().filter(|c| c.id.starts_with("layer.newAdjustmentLayer.")) {
                if menu.item(ui, tl!(c.label).trim_end_matches('…')) {
                    actions.push((c.id.into(), json!({})));
                }
            }
            ui.separator();
            if menu.item(ui, tl!("Solid Color…")) {
                actions.push(("layer.newFillLayer.solidColor".into(), json!({})));
            }
            if menu.item(ui, tl!("Gradient…")) {
                actions.push(("layer.newFillLayer.gradient".into(), json!({})));
            }
        });
        // Like Photoshop the button never replaces a mask (#2075): with a layer mask it adds a
        // vector mask, and with both it is greyed.
        let alt = ui.input(|i| i.modifiers.alt);
        let mask_cmd = crate::layer_menu_ui::mask_button_command(active_layer, doc.selection.is_some(), alt);
        let mask_tip = if active_layer.is_some_and(|l| l.mask.is_some()) {
            tl!("Add vector mask").to_string()
        } else {
            crate::i18n::fmt(tl!("Add a mask  (from the selection; {key} inverts)"), &[("key", &crate::shortcuts::pretty("Alt"))])
        };
        let mask_btn = ui.add_enabled_ui(mask_cmd.is_some(), |ui| icons::button(ui, "layer-mask", 26.0, false, &mask_tip)).inner;
        mask_btn.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &mask_tip));
        if mask_btn.clicked()
            && let Some(cmd) = mask_cmd
        {
            // A path selected in the Paths panel becomes the vector mask, as in Photoshop (#4728);
            // Alt still inverts it.
            match crate::vector_ui::selected_document_path(app).filter(|_| cmd.starts_with("layer.vectorMask.")) {
                Some(name) => actions.push(("layer.vectorMask.currentPath".into(), json!({"name": name, "hide": alt}))),
                None => actions.push((cmd.into(), json!({}))),
            }
        }
        let fx = fx_button(ui, 26.0, tl!("Add a layer style"));
        crate::press_menu::show(&fx, |ui, menu| {
            ui.set_min_width(180.0);
            if menu.item(ui, tl!("Blending Options…")) {
                crate::layer_style::open(app, None);
            }
            ui.separator();
            for &(kind, label) in crate::layer_style::KINDS {
                if menu.item(ui, format!("{}…", tl!(label))) {
                    crate::layer_style::open(app, Some(kind));
                }
            }
        });
        // Photoshop's footer starts with Link Layers (enabled with two or more layers selected).
        let can_link = app.session.is_enabled("layer.linkLayers");
        if ui.add_enabled_ui(can_link, |ui| icons::button(ui, "link-2", 26.0, false, tl!("Link layers"))).inner.clicked() {
            actions.push(("layer.linkLayers".into(), json!({})));
        }
    });
    for (id, p) in actions {
        if id == "ui.maskTarget" {
            app.ui.mask_target = p.as_bool().unwrap_or(false);
            app.ui.vector_mask_target = false;
            app.sync_mask_targets();
            continue;
        }
        if id == "ui.vectorMaskTarget" {
            app.ui.vector_mask_target = p.as_bool().unwrap_or(false);
            app.ui.mask_target = false;
            app.sync_mask_targets();
            continue;
        }
        // Double-clicking a mask thumbnail opens its settings: show Properties.
        if id == "ui.properties" {
            app.ui.panels.properties = true;
            app.ui.dock_tabs.properties = 0;
            app.ui.dock.collapsed.retain(|g| *g != crate::dock::Group::Properties);
            continue;
        }
        if p.is_null() {
            // Context-menu items behave like their menu-bar twins (dialogs included).
            let ctx = ui.ctx().clone();
            if let Err(e) = crate::menus::invoke(app, &ctx, &id, json!({})) {
                app.ui.status = e;
                app.ui.status_error = true;
            }
            continue;
        }
        let _ = app.run(&id, p);
    }
}

/// A layer row dragged onto a Layers panel footer button, as in Photoshop: onto Delete deletes it,
/// onto New Layer duplicates it, onto New Group groups it. A row that is part of the selection
/// carries the whole selection; any other row goes alone. Highlights the button while over it and
/// returns the command on release.
fn footer_drop(ui: &egui::Ui, button: &egui::Response, drag: Option<(u64, bool)>, command: &str) -> Option<(String, Value)> {
    // Named for screen readers (and tests) after the command a drop runs.
    button.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, footer_label(command)));
    let (layer, in_selection) = drag?;
    dropped_on(ui, button).then(|| (command.into(), if in_selection { json!({}) } else { json!({"layer": layer}) }))
}

/// While something is dragged over a footer `button`: highlights it, and is true on the release.
pub(crate) fn dropped_on(ui: &egui::Ui, button: &egui::Response) -> bool {
    let Some(p) = ui.ctx().input(|i| i.pointer.interact_pos()) else { return false };
    if !button.rect.contains(p) {
        return false;
    }
    let t = Tokens::get(ui.ctx());
    ui.painter().rect_stroke(button.rect, t.radius_sm, Stroke::new(2.0, t.accent), StrokeKind::Inside);
    ui.ctx().input(|i| i.pointer.any_released())
}

/// Effects dragged onto the footer's Delete button, as in Photoshop (#3102): the "Effects" row
/// clears the layer style, an effect's row deletes just that effect.
fn fx_trash_drop(ui: &egui::Ui, button: &egui::Response, drag: Option<FxDrag>) -> Option<(String, Value)> {
    let (layer, effect) = drag?;
    if !dropped_on(ui, button) {
        return None;
    }
    let params = match effect {
        Some(i) => json!({"layer": layer, "effect": i}),
        None => json!({"layer": layer}),
    };
    Some(("layer.layerStyle.clear".into(), params))
}

fn footer_label(command: &str) -> &'static str {
    match command {
        "layer.delete" => tl!("Delete layer"),
        "layer.duplicate" => tl!("Create a new layer"),
        _ => tl!("Create a new group"),
    }
}

/// The small down-arrow makes it clear that the footer icon expands into a menu.
fn footer_menu_arrow(ui: &egui::Ui, rect: Rect) {
    let t = Tokens::get(ui.ctx());
    let arrow = Rect::from_min_size(rect.right_bottom() - vec2(11.0, 10.0), vec2(9.0, 9.0));
    icons::paint(ui, arrow, "chevron-down", 8.0, t.text_dim);
}

fn footer_menu_button(ui: &mut egui::Ui, icon: &str, size: f32, tip: &str) -> egui::Response {
    let response = icons::button(ui, icon, size, false, tip);
    // Named for screen readers (and tests) after its tooltip.
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tip));
    footer_menu_arrow(ui, response.rect);
    response
}

/// Photoshop's italic "fx" footer button (no icon-font equivalent).
fn fx_button(ui: &mut egui::Ui, size: f32, tip: &str) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r, t.radius_sm, t.hover);
    }
    let mut job = egui::text::LayoutJob::default();
    job.append("fx", 0.0, egui::TextFormat { font_id: egui::FontId::proportional(15.0), color: t.icon, italics: true, ..Default::default() });
    let g = ui.painter().layout_job(job);
    ui.painter().galley(r.center() - g.size() / 2.0, g, t.icon);
    footer_menu_arrow(ui, r);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tip));
    resp.on_hover_text(tip)
}

/// How a Layers panel row is selected: in the (multi-)selection, the primary/active layer, and
/// whether several layers are selected.
#[derive(Clone, Copy)]
struct RowSel {
    selected: bool,
    primary: bool,
    multi: bool,
}

/// `layer.select` mode for a click with these modifiers (Photoshop: ⌘-click toggles, ⇧-click
/// selects a range, a plain click selects one layer).
fn select_mode(m: egui::Modifiers) -> &'static str {
    if m.command {
        "toggle"
    } else if m.shift {
        "range"
    } else {
        "replace"
    }
}

/// A drag down the Layers panel's eye column (Photoshop): the first eye toggles and every row
/// swept over gets the same visibility, once per row, all in one history step.
#[derive(Clone)]
struct EyeSweep {
    visible: bool,
    /// The `coalesce` key shared by the drag's `layer.setProps` calls.
    key: u64,
    swept: Vec<u64>,
}

/// Starts an eye-column sweep from `eye`, or applies a running one to the row `row` of `l`.
fn eye_sweep(ctx: &egui::Context, l: &Layer, row: Rect, eye: &egui::Response, actions: &mut Vec<(String, Value)>) {
    let id = egui::Id::new("layer-eye-sweep");
    let set =
        |visible: bool, key: u64| ("layer.setProps".to_string(), json!({"layer": l.id.0, "visible": visible, "coalesce": format!("layer-eye-sweep:{key}")}));
    if eye.drag_started() {
        let sweep = EyeSweep { visible: !l.visible, key: ctx.cumulative_pass_nr(), swept: vec![l.id.0] };
        actions.push(set(sweep.visible, sweep.key));
        ctx.data_mut(|d| d.insert_temp(id, sweep));
        return;
    }
    let Some(mut sweep) = ctx.data(|d| d.get_temp::<EyeSweep>(id)) else { return };
    let (down, pos, delta) = ctx.input(|i| (i.pointer.primary_down(), i.pointer.interact_pos(), i.pointer.delta()));
    if !down {
        ctx.data_mut(|d| d.remove::<EyeSweep>(id));
        return;
    }
    // Everything the pointer passed since the last frame, so a fast drag skips no row.
    let Some(p) = pos else { return };
    let (y0, y1) = ((p.y - delta.y).min(p.y), (p.y - delta.y).max(p.y));
    if y1 < row.top() || y0 >= row.bottom() || sweep.swept.contains(&l.id.0) {
        return;
    }
    if l.visible != sweep.visible {
        actions.push(set(sweep.visible, sweep.key));
    }
    sweep.swept.push(l.id.0);
    ctx.data_mut(|d| d.insert_temp(id, sweep));
}

/// Marker in the clipped row's existing 12-point slot, beside its thumbnail.
fn paint_clipping_indicator(painter: &egui::Painter, at: Pos2, color: Color32) {
    let tip = at + vec2(5.0, 3.0);
    let stroke = Stroke::new(1.0, color);
    painter.add(egui::Shape::line(vec![at + vec2(9.0, -4.0), at + vec2(5.0, -4.0), tip], stroke));
    painter.add(egui::Shape::line(vec![at + vec2(2.0, 0.0), tip, at + vec2(8.0, 0.0)], stroke));
}

#[allow(clippy::too_many_arguments)]
fn layer_row(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    doc: &photocraft_doc::Document,
    l: &Layer,
    depth: usize,
    row: RowSel,
    actions: &mut Vec<(String, Value)>,
) -> Rect {
    let selected = row.selected;
    let t = Tokens::get(ctx);
    // Photoshop's default (medium) thumbnails: 32 pt rows.
    let row_h = if t.pro { 32.0 } else { 46.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click_and_drag());
    // A drag from a mask's thumbnail drags that mask, not the layer (#3113, #3119).
    let mask_drag = starts_mask_drag(ctx, l, &resp);
    if let Some(kind) = mask_drag {
        ctx.data_mut(|d| d.insert_temp::<MaskDrag>(mask_drag_key(), (l.id.0, kind)));
    }
    let mask_drag = mask_drag.is_some();
    layer_drag_and_drop(app, ctx, ui, l, rect, &resp, actions);
    fx_drop(ctx, ui, l, rect, actions);
    crate::smart_ui::drop_on_row(ctx, ui, l, rect, actions);
    if resp.drag_started() && !mask_drag {
        crate::layer_transfer::begin_from_panel(app, ctx, l.id);
    }
    // Rows are painted: name them for screen readers and UI tests.
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, &l.name));
    // A row scrolled out of view only keeps its place (#125): a layout's hundreds of rows would
    // otherwise lay out names, icons and thumbnails every frame. Group rows stay whole: their
    // disclosure triangle is a widget (accessibility, scroll-to).
    if !ui.is_rect_visible(rect) && !l.is_group() && !resp.context_menu_opened() && crate::layer_row_ui::renaming(ctx) != Some(l.id.0) {
        return rect;
    }
    let painter = ui.painter_at(rect.expand(1.0));
    let is_hovered = resp.hovered() || app.canvas_hover_layer == Some(l.id);
    if resp.hovered() {
        app.hover_layer = Some(l.id);
    }
    if t.pro {
        if selected {
            painter.rect_filled(rect, 0.0, t.row_selected);
        } else if is_hovered {
            painter.rect_filled(rect, 0.0, t.hover.gamma_multiply(0.5));
            painter.rect_filled(rect, 0.0, t.accent.gamma_multiply(0.12));
        }
    } else if selected {
        painter.rect_filled(rect, t.radius, t.hover);
        painter.rect_stroke(rect, t.radius, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
    } else if is_hovered {
        painter.rect_filled(rect, t.radius, t.hover.gamma_multiply(0.5));
        painter.rect_filled(rect, t.radius, t.accent.gamma_multiply(0.12));
    }
    let eye_cell = Rect::from_min_max(rect.left_top(), pos2((rect.left() + 30.0).min(rect.right()), rect.bottom()));
    if let Some((_, background)) = t.layer_label_colors(l.label) {
        painter.rect_filled(eye_cell, 0.0, background);
    }
    if t.pro {
        // Paint dividers over the label so the eye column stays distinct.
        painter.line_segment([pos2(rect.left() + 30.0, rect.top()), pos2(rect.left() + 30.0, rect.bottom())], Stroke::new(1.0, t.separator));
        painter.line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, t.separator));
    }
    let mut x = rect.left() + 6.0;
    let eye = Rect::from_min_size(pos2(x, rect.center().y - 11.0), vec2(22.0, 22.0));
    // The eye takes drags too (so a drag starting on it never reorders the row): dragging down
    // the eyes gives every row swept over the visibility the first eye toggled to.
    let eye_resp = ui.interact(eye, ui.id().with(("eye", l.id.0)), Sense::click_and_drag());
    // A hidden layer's eye box is left empty (still clickable).
    if l.visible {
        // Centre the drawing in the column while preserving the visibility hit area.
        icons::paint(ui, eye_cell, "eye", 15.0, t.layer_label_icon(l.label));
    }
    eye_sweep(ctx, l, rect, &eye_resp, actions);
    if eye_resp.clicked() {
        // ⌥-click shows only this layer; ⌥-click it again to restore the others.
        if ui.input(|i| i.modifiers.alt) {
            actions.push(("layer.showOnly".into(), json!({"layer": l.id.0})));
        } else {
            actions.push(("layer.setProps".into(), json!({"layer": l.id.0, "visible": !l.visible})));
        }
    }
    eye_resp.context_menu(|ui| {
        crate::widgets::style_spectrum_popup_menu(ui);
        crate::layer_menu_ui::color_choices(app, ui, l, row.multi && selected, actions);
    });
    // Everything but the thumbnails, the indentation and the name, so a narrow panel squeezes
    // the indentation first, then the thumbnails (a layer with two masks has three).
    let others = 6.0
        + 28.0
        + if l.is_group() { crate::layer_tree_ui::TRIANGLE_W } else { 0.0 }
        + if l.clipped { 12.0 } else { 0.0 }
        + crate::layer_row_ui::reserved_width(l);
    let ts = crate::mask_thumbs_ui::thumb_size(l, if t.pro { 24.0 } else { 34.0 }, rect.width() - others);
    let fixed = others + ts + 6.0 + crate::mask_thumbs_ui::width(l, ts);
    x += 28.0 + crate::layer_row_ui::indent(depth, rect.width(), fixed);
    let toggled = crate::layer_tree_ui::disclosure(ui, rect, &mut x, l, actions);
    if l.clipped {
        paint_clipping_indicator(&painter, pos2(x, rect.center().y), t.text_faint);
        x += 12.0;
    }
    let thumb = Rect::from_min_size(pos2(x, rect.center().y - ts / 2.0), vec2(ts, ts));
    draw_layer_thumb(app, ctx, ui, doc, l, thumb, row.primary);
    x += ts + 6.0;
    // Link chains and pixel / vector mask thumbnails (#153).
    let masks = crate::mask_thumbs_ui::paint(app, ctx, ui, &painter, doc, l, &mut x, rect.center().y, ts, actions);
    let mask_rect = masks.thumb(crate::mask_thumbs_ui::MaskKind::Pixel);
    let vector_rect = masks.thumb(crate::mask_thumbs_ui::MaskKind::Vector);
    // Photoshop frames the targeted thumbnail (pixels, mask or vector mask) of the active layer
    // with corner brackets.
    if row.primary {
        let target = if app.ui.vector_mask_target && vector_rect.is_some() {
            vector_rect
        } else if app.ui.mask_target {
            mask_rect.or(Some(thumb))
        } else {
            Some(thumb)
        };
        if let Some(r) = target {
            crate::mask_thumbs_ui::paint_brackets(&painter, r, t.text);
        }
    }
    // Right-hand indicators first; the name gets what is left and ends in "…" (#144).
    let fx_open = app.session.active().is_none_or(|d| !d.fx_collapsed.contains(&l.id));
    let (name_right, indicators, fx_toggled) = crate::layer_row_ui::indicators(ui, &painter, rect, x, l, fx_open, actions);
    // Dragging the fx badge drags all the layer's effects, not the layer (Photoshop). Like the eye,
    // it takes the drag from the row; clicks still reach the row.
    if let Some(&(_, badge)) = indicators.iter().find(|(k, _)| *k == crate::layer_row_ui::Indicator::Fx)
        && ui.interact(badge, ui.id().with(("fx-badge", l.id.0)), Sense::drag()).drag_started()
    {
        start_fx_drag(ctx, l.id, None);
    }
    // Clicking the lock icon unlocks the layer, and makes the Background a normal one (Photoshop,
    // #4723). It takes the click from the row, so the row isn't selected.
    if let Some(&(_, lock)) = indicators.iter().find(|(k, _)| *k == crate::layer_row_ui::Indicator::Lock)
        && ui.interact(lock, ui.id().with(("row-lock", l.id.0)), Sense::click()).clicked()
    {
        actions.push(simple_lock_toggle(doc.is_background(l.id), l));
    }
    let name_color = if l.visible { t.text } else { t.text_faint };
    let font = if selected && !t.pro { theme::medium(13.0) } else { egui::FontId::proportional(if t.pro { 12.0 } else { 13.0 }) };
    // Photoshop before 2026 set the Background layer's name in italics; 2026 sets it upright.
    let italic = !t.pro && doc.is_background(l.id);
    // Photoshop rows show only the name; the kind sub-label is a Studio-theme addition.
    let is_pixel = t.pro || matches!(l.content, LayerContent::Raster(_));
    let name_rect = crate::layer_row_ui::truncated(&painter, &l.name, font, name_color, italic, name_right - x).map(|galley| {
        let text_pos = pos2(x, rect.center().y - galley.size().y / 2.0 - if is_pixel { 0.0 } else { 7.0 });
        let r = Rect::from_min_size(text_pos, galley.size());
        painter.galley(text_pos, galley, name_color);
        r
    });
    if !is_pixel {
        let sub = match &l.content {
            LayerContent::Adjustment(a) => tl!(a.label()).to_string(),
            LayerContent::Group(g) => crate::i18n::trn(crate::i18n::current(), g.children.len() as u64, "Group · {n} layer", "Group · {n} layers"),
            other => tl!(other.kind_name()).to_string(),
        };
        crate::layer_row_ui::label(&painter, x, rect.center().y + 8.0, name_right, &sub, egui::FontId::proportional(11.0), t.text_faint);
    }
    crate::layer_row_ui::record(ctx, crate::layer_row_ui::RowRects { layer: l.id.0, row: rect, name: name_rect, indicators });
    // ⌘-click a layer, mask or vector-mask thumbnail loads its transparency / mask / path as a
    // selection (⇧ add, ⌥ subtract, ⇧⌥ intersect) instead of changing the layer selection.
    let thumb_load = resp.clicked().then(|| (ui.input(|i| i.modifiers), resp.interact_pointer_pos())).and_then(|(m, pos)| {
        let p = pos.filter(|_| m.command)?;
        if let Some(kind) = masks.hit(p) {
            return Some(crate::mask_thumbs_ui::load_params(l, kind, m));
        }
        thumb.expand(2.0).contains(p).then(|| json!({"channel": "transparency", "layer": l.id.0, "operation": crate::channels_panel::load_operation(m)}))
    });
    // ⇧-click a mask thumbnail: disable / enable that mask; ⌥-click a layer mask: view it.
    let mask_toggle = resp.clicked().then(|| (ui.input(|i| i.modifiers), resp.interact_pointer_pos())).and_then(|(m, pos)| {
        let kind = masks.hit(pos?)?;
        crate::mask_thumbs_ui::click_command(l, kind, m)
    });
    if let Some(p) = thumb_load {
        actions.push(("select.loadSelection".into(), p));
    } else if let Some(cmd) = mask_toggle {
        actions.push(cmd);
    } else if resp.clicked() && !eye_resp.clicked() && !toggled && !fx_toggled && !masks.clicked {
        let mode = select_mode(ui.input(|i| i.modifiers));
        actions.push(("layer.select".into(), json!({"layer": l.id.0, "mode": mode})));
        // A thumbnail explicitly selects content or a mask. Selecting an adjustment/fill row
        // defaults to its mask, but clicking its content thumbnail must leave the mask target.
        let pos = resp.interact_pointer_pos();
        let on_mask = mask_rect.zip(pos).is_some_and(|(r, p)| r.expand(2.0).contains(p));
        let on_thumb = pos.is_some_and(|p| thumb.expand(2.0).contains(p));
        let on_vector = pos.and_then(|p| masks.hit(p)) == Some(crate::mask_thumbs_ui::MaskKind::Vector);
        // Clicking the layer thumbnail leaves mask view (#196).
        let viewing = app.session.active().and_then(photocraft_engine::mask_view_cmds::current).is_some_and(|v| v.layer == l.id);
        if on_thumb && viewing {
            actions.push((photocraft_engine::mask_view_cmds::ID.into(), json!({"layer": l.id.0, "mode": "off"})));
        }
        let content_less = matches!(l.content, LayerContent::Adjustment(_) | LayerContent::Fill(_));
        if on_vector {
            // The vector mask thumbnail targets the vector mask: the path tools edit it and the
            // Paths panel selects the layer's path.
            app.ui.selected_path = Some("layer".into());
            actions.push(("ui.vectorMaskTarget".into(), json!(true)));
        } else if on_mask || (content_less && l.mask.is_some() && !on_thumb) {
            if on_mask {
                // The mask thumbnail also switches Properties to the mask (#3369).
                app.ui.properties_layer = None;
            }
            actions.push(("ui.maskTarget".into(), json!(true)));
        } else if on_thumb || !row.primary {
            actions.push(("ui.maskTarget".into(), json!(false)));
        }
        // ⌥-click the layer thumbnail: fit the layer on screen, as Photoshop does.
        let m = ui.input(|i| i.modifiers);
        if on_thumb && m.alt && !m.shift && !m.command {
            actions.push(("view.fitLayersOnScreen".into(), Value::Null));
        }
    }
    // Double-click: the name renames in place (over the row's full height, not just the glyphs:
    // #651); the Background, which can't be renamed while it's locked, becomes a normal layer; an
    // adjustment, fill or shape thumbnail opens its settings (a solid colour: the Color Picker,
    // #2812) and a Smart Object thumbnail its contents, and a type thumbnail edits its text;
    // anywhere else on the row opens Layer Style (#350, #537).
    // The first click already made this the active layer.
    if resp.double_clicked() {
        let pos = resp.interact_pointer_pos();
        let on = |r: Rect| pos.is_some_and(|p| r.expand(2.0).contains(p));
        if crate::doc_props_ui::is_background(doc, l) {
            actions.push(("layer.new.layerFromBackground".into(), json!({})));
        } else if name_rect.is_some_and(|n| on(Rect::from_x_y_ranges(n.x_range(), rect.y_range()))) {
            if let Some(done) = crate::layer_row_ui::start_rename(ctx, l.id.0, &l.name) {
                // One rename at a time: starting this one commits any other (#314).
                actions.push(done);
            }
        } else if let Some(kind) = pos.and_then(|p| masks.hit(p)) {
            match kind {
                crate::mask_thumbs_ui::MaskKind::Pixel => actions.push(("ui.maskTarget".into(), json!(true))),
                crate::mask_thumbs_ui::MaskKind::Vector => actions.push(("ui.vectorMaskTarget".into(), json!(true))),
            }
            if let Some((cmd, val)) = crate::mask_thumbs_ui::double_click_command(app, kind) {
                // The preference launches Select and Mask on a pixel mask.
                actions.push((cmd.into(), val));
            } else {
                // Otherwise double-click opens the mask's settings (Density/Feather).
                actions.push(("ui.properties".into(), json!(true)));
            }
        } else if pos.and_then(|p| masks.hit(p)).is_none() {
            let id = match &l.content {
                LayerContent::Adjustment(_) | LayerContent::Fill(_) | LayerContent::Shape(_) if on(thumb) => "layer.layerContentOptions",
                LayerContent::Smart(_) if on(thumb) => "layer.smartObjects.editContents",
                LayerContent::Text(_) if on(thumb) => "type.editText",
                _ => "layer.layerStyle.blendingOptions",
            };
            if crate::menus::is_enabled(app, id) {
                // Null params: run like the menu item, dialog included.
                actions.push((id.into(), Value::Null));
            }
        }
    }
    let edit_rect = Rect::from_min_max(pos2(x - 3.0, rect.center().y - 11.0), pos2(name_right.max(x + 40.0), rect.center().y + 11.0));
    if let Some(done) = crate::layer_row_ui::rename_field(ui, l.id.0, edit_rect) {
        actions.push(done);
    }
    // The visibility icon has its own color menu, and the layer-mask thumbnail its mask menu
    // (#4575); elsewhere use the full layer menu.
    let on_mask_id = ui.id().with(("mask-menu", l.id.0));
    if resp.secondary_clicked() {
        let on_mask = resp.interact_pointer_pos().and_then(|p| masks.hit(p)) == Some(crate::mask_thumbs_ui::MaskKind::Pixel);
        ctx.data_mut(|d| d.insert_temp(on_mask_id, on_mask));
    }
    if !eye_resp.secondary_clicked() && !eye_resp.context_menu_opened() {
        resp.context_menu(|ui| {
            crate::widgets::style_spectrum_popup_menu(ui);
            if ui.ctx().data(|d| d.get_temp::<bool>(on_mask_id)).unwrap_or(false) {
                crate::layer_menu_ui::show_mask(app, ui, l, actions);
                return;
            }
            // Right-clicking inside a multi-selection keeps it and acts on every selected layer.
            let on_set = row.multi && selected;
            if crate::layer_menu_ui::show(app, ui, l, on_set, actions)
                && let Some(done) = crate::layer_row_ui::start_rename(ui.ctx(), l.id.0, &l.name)
            {
                actions.push(done);
            }
        });
    }
    rect
}

/// Screen rect and UVs for a layer thumbnail: the document-shaped part of the square cell and of
/// the letterboxed square texture, so tall and wide documents don't show empty bars.
fn layer_thumb_fit(cell: Rect, w: u32, h: u32) -> (Rect, Rect) {
    crate::channels_panel::fit_thumb(cell, w, h)
}

fn draw_layer_thumb(app: &mut PhotocraftApp, ctx: &egui::Context, ui: &egui::Ui, doc: &photocraft_doc::Document, l: &Layer, rect: Rect, selected: bool) {
    let t = Tokens::get(ctx);
    let p = ui.painter();
    match &l.content {
        LayerContent::Adjustment(_) | LayerContent::Group(_) => {
            p.rect_filled(rect, 6.0, t.field);
            let icon = if l.is_group() { "folder" } else { "contrast" };
            icons::paint(ui, rect, icon, 18.0, t.icon);
        }
        LayerContent::Text(_) => {
            // Photoshop: type layers show a "T" tile rather than a pixel thumbnail.
            p.rect_filled(rect, if t.pro { 0.0 } else { 6.0 }, Color32::from_gray(if t.pro { 222 } else { 236 }));
            icons::paint(ui, rect, "type", 20.0, Color32::from_gray(40));
        }
        // Gradient fills show the gradient itself (Photoshop); solid ones their colour.
        LayerContent::Fill(f) if crate::gradient_ui::paint_thumbnail(ui, l.id, f, rect) => {}
        LayerContent::Fill(photocraft_doc::Fill::Solid(color)) => {
            crate::solid_fill_ui::paint_thumbnail(ui, crate::solid_fill_ui::thumbnail_color(app, doc.id, l.id, *color), rect);
        }
        LayerContent::Fill(_) => {
            let c = [128, 128, 128, 255];
            p.rect_filled(rect, 6.0, Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]));
        }
        _ => {
            // Keep row layout and outside thumbnail decorations when the image is clipped.
            if ui.is_rect_visible(rect) {
                // The texture is a letterboxed square: draw only the part the document fills.
                let (fitted, uv) = layer_thumb_fit(rect, doc.size.width, doc.size.height);
                widgets::checker(p, fitted, 5.0);
                let tex = app.layer_thumb(ctx, doc, l);
                p.image(tex, fitted, uv, Color32::WHITE);
            }
        }
    }
    let stroke = if t.pro {
        Stroke::new(1.0, Color32::from_gray(20))
    } else if selected {
        Stroke::new(1.5, t.text)
    } else {
        Stroke::new(1.0, t.field_border)
    };
    p.rect_stroke(rect, CornerRadius::same(if t.pro { 0 } else { 5 }), stroke, StrokeKind::Outside);
    crate::smart_ui::thumb_badge(ui, l, rect);
}

fn channels(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    crate::channels_panel::show(app, ui);
}

fn history(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(st) = app.session.active() else {
        empty(ui, tl!("No document"));
        return;
    };
    let doc = st.doc.clone();
    let entries = st.history.entries();
    let current = entries.len() - 1;
    let redo: Vec<String> = st.history.redo_labels().map(str::to_string).collect();
    // Snapshot row (Photoshop shows the document's opening state with a thumbnail).
    {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::hover());
        let thumb = Rect::from_min_size(pos2(rect.left() + 28.0, rect.center().y - 14.0), vec2(28.0, 28.0));
        let ctx = ui.ctx().clone();
        if let Some(layer) = doc.layers.first() {
            widgets::checker(ui.painter(), thumb, 4.0);
            let tex = app.layer_thumb(&ctx, &doc, layer);
            ui.painter().image(tex, thumb, Rect::from_min_max(egui::Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
        }
        ui.painter().rect_stroke(thumb, 0.0, Stroke::new(1.0, t.separator), StrokeKind::Outside);
        ui.painter().text(pos2(thumb.right() + 8.0, rect.center().y), Align2::LEFT_CENTER, &doc.name, egui::FontId::proportional(12.0), t.text);
        ui.painter().line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, t.separator));
    }
    let mut snap_cmd = None;
    let mut target: Option<isize> = None;
    let all = entries.iter().map(|e| (e.clone(), false)).chain(redo.iter().map(|e| (e.clone(), true)));
    // The dock gives History a fixed height: the rows scroll above the footer.
    let footer = if t.pro { widgets::footer_height(ui) + ui.spacing().item_spacing.y } else { 0.0 };
    let max_h = (ui.available_height() - footer).max(40.0);
    egui::ScrollArea::vertical().id_salt("history-rows").max_height(max_h).auto_shrink([false, false]).show(ui, |ui| {
        // Snapshots head the list and scroll with the states, so many of them never push the
        // footer out of a short dock.
        snap_cmd = snapshot_rows(app, ui);
        for (i, (e, is_redo)) in all.enumerate() {
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
            if i == current {
                ui.painter().rect_filled(rect, if t.pro { 0.0 } else { t.radius_sm }, if t.pro { t.row_selected } else { t.accent_soft });
            } else if resp.hovered() {
                ui.painter().rect_filled(rect, 0.0, t.hover.gamma_multiply(0.5));
            }
            let icon = match e.as_str() {
                s if s.contains("Brush") || s.contains("Eraser") => "brush",
                s if s.contains("Layer") => "layers",
                s if s.contains("Select") || s.contains("Marquee") || s.contains("Deselect") || s.contains("Lasso") => "square-dashed",
                s if s.contains("Fill") => "paint-bucket",
                "Open" => "image",
                _ => "sliders-horizontal",
            };
            let color = if is_redo { t.text_faint } else { t.text };
            icons::paint(
                ui,
                Rect::from_center_size(pos2(rect.left() + 42.0, rect.center().y), vec2(16.0, 16.0)),
                icon,
                12.0,
                if is_redo { t.text_faint } else { t.icon },
            );
            ui.painter().text(pos2(rect.left() + 58.0, rect.center().y), Align2::LEFT_CENTER, &e, egui::FontId::proportional(12.0), color);
            if resp.clicked() {
                target = Some(i as isize - current as isize);
            }
        }
    });
    let mut footer_cmd: Option<&str> = None;
    if t.pro {
        let can_delete = app.session.is_enabled("history.deleteState");
        let can_new = app.session.is_enabled("history.newDocument");
        widgets::panel_footer(ui, |ui| {
            if ui.add_enabled_ui(can_delete, |ui| icons::button(ui, "trash", 26.0, false, tl!("Delete current state"))).inner.clicked() {
                footer_cmd = Some("history.deleteState");
            }
            let can_snap = app.session.is_enabled("history.newSnapshot");
            if ui.add_enabled_ui(can_snap, |ui| icons::button(ui, "scan", 26.0, false, tl!("Create new snapshot"))).inner.clicked() {
                footer_cmd = Some("history.newSnapshot");
            }
            if ui.add_enabled_ui(can_new, |ui| icons::button(ui, "file-plus", 26.0, false, tl!("Create new document from current state"))).inner.clicked() {
                footer_cmd = Some("history.newDocument");
            }
        });
    }
    // An open Free Transform owns Undo (transform_tool::intercept): stepping the document's history under
    // its box would leave it transforming pixels that changed.
    // Selecting a snapshot changes the document, so it waits for an open transform like the rows.
    let snap_cmd = snap_cmd.filter(|(cmd, _)| *cmd != "history.selectSnapshot" || app.ui.transform.is_none());
    if let Some((cmd, p)) = footer_cmd.map(|c| (c, json!({}))).filter(|_| app.ui.transform.is_none()).or(snap_cmd)
        && let Err(e) = app.run(cmd, p)
    {
        app.ui.status = e;
        app.ui.status_error = true;
    }
    if let Some(delta) = target.filter(|_| app.ui.transform.is_none()) {
        let (cmd, n) = if delta < 0 { ("edit.undo", -delta) } else { ("edit.redo", delta) };
        for _ in 0..n {
            if app.run(cmd, json!({})).is_err() {
                break;
            }
        }
    }
}

/// The History panel's snapshot rows (#2966), under the opening row, each with its thumbnail and
/// name: a click returns to the snapshot (a new state), a double-click or right-click › Rename…
/// edits the name in place (Enter or a click elsewhere keeps it, Escape cancels), and
/// right-click › Delete removes it. Returns the command a row asks for.
fn snapshot_rows(app: &mut PhotocraftApp, ui: &mut egui::Ui) -> Option<(&'static str, Value)> {
    let t = Tokens::get(ui.ctx());
    let st = app.session.active()?;
    let (doc_id, snaps) = (st.doc.id, st.snapshots.clone());
    let live: Vec<u64> = snaps.iter().map(|s| s.id).collect();
    // Per document: snapshot ids restart in each, and a tab switch must not carry the edit over.
    let rename_id = egui::Id::new(("history-snapshot-rename", doc_id));
    let mut renaming: Option<(u64, String)> = ui.data(|d| d.get_temp(rename_id)).flatten();
    let mut cmd = None;
    let ctx = ui.ctx().clone();
    for snap in &snaps {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &snap.name));
        if resp.hovered() {
            ui.painter().rect_filled(rect, 0.0, t.hover.gamma_multiply(0.5));
        }
        let thumb = Rect::from_min_size(pos2(rect.left() + 28.0, rect.center().y - 14.0), vec2(28.0, 28.0));
        widgets::checker(ui.painter(), thumb, 4.0);
        let tex = app.snapshot_thumb(&ctx, doc_id, snap, &live);
        ui.painter().image(tex, thumb, Rect::from_min_max(egui::Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
        ui.painter().rect_stroke(thumb, 0.0, Stroke::new(1.0, t.separator), StrokeKind::Outside);
        match renaming.as_mut().filter(|(id, _)| *id == snap.id) {
            Some((_, text)) => {
                let field = Rect::from_min_max(pos2(thumb.right() + 6.0, rect.center().y - 10.0), pos2(rect.right() - 6.0, rect.center().y + 10.0));
                let r = ui.put(field, egui::TextEdit::singleline(text));
                if r.lost_focus() {
                    if !ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        cmd = Some(("history.renameSnapshot", json!({"snapshot": snap.id, "name": text.clone()})));
                    }
                    renaming = None;
                } else if !r.has_focus() {
                    // Starting to rename: focus the field with the whole name selected, so typing
                    // replaces it (Photoshop).
                    r.request_focus();
                    if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), r.id) {
                        let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(text.chars().count()));
                        state.cursor.set_char_range(Some(all));
                        state.store(ui.ctx(), r.id);
                    }
                }
            }
            None => {
                ui.painter().text(pos2(thumb.right() + 8.0, rect.center().y), Align2::LEFT_CENTER, &snap.name, egui::FontId::proportional(12.0), t.text);
            }
        }
        if resp.double_clicked() {
            renaming = Some((snap.id, snap.name.clone()));
        } else if resp.clicked() {
            cmd = Some(("history.selectSnapshot", json!({"snapshot": snap.id})));
        }
        resp.context_menu(|ui| {
            if ui.button(tl!("Rename…")).clicked() {
                renaming = Some((snap.id, snap.name.clone()));
                ui.close();
            }
            if ui.button(tl!("Delete")).clicked() {
                cmd = Some(("history.deleteSnapshot", json!({"snapshot": snap.id})));
                ui.close();
            }
        });
    }
    if !snaps.is_empty() {
        let y = ui.cursor().top();
        ui.painter().hline(ui.max_rect().x_range(), y, Stroke::new(1.0, t.separator));
    }
    ui.data_mut(|d| d.insert_temp(rename_id, renaming));
    cmd
}

// ----------------------------------------------------------------------------- properties

/// Floating Properties card anchored to the canvas' top-right corner. It shows the same content as
/// the docked panel (`properties_body`): document, layer, adjustment and mask properties.
pub fn properties_window(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if app.ui.docking.is_some() {
        return;
    }
    // Pro (Photoshop) docks Properties; Studio floats it over the canvas.
    if !app.ui.panels.properties || Tokens::get(ctx).pro {
        return;
    }
    if app.session.active().is_none() {
        return;
    }
    let t = Tokens::get(ctx);
    let canvas = app.last_canvas_rect;
    let width = 320.0;
    let pos = pos2(canvas.right() - width - 12.0, canvas.top() + 44.0);
    let frame = egui::Frame::NONE
        .fill(t.card)
        .stroke(Stroke::new(1.0, t.card_border))
        .corner_radius(CornerRadius::same(t.radius_lg as u8))
        .shadow(egui::Shadow { offset: [0, 12], blur: 36, spread: 0, color: t.shadow })
        .inner_margin(egui::Margin::same(14));
    // A floating panel (Order::Middle like other panels, so menus, popups and dialogs stay above
    // it), anchored to the canvas corner and dragged by its title off the part being worked on.
    let card_id = egui::Id::new("properties-card");
    let offset: egui::Vec2 = ctx.data(|m| m.get_temp(card_id)).unwrap_or_default();
    let mut drag = egui::Vec2::ZERO;
    let shown = egui::Area::new(card_id).order(egui::Order::Middle).fixed_pos(pos + offset).show(ctx, |ui| {
        frame.show(ui, |ui| {
            ui.set_width(width - 28.0);
            let title = ui.horizontal(|ui| {
                ui.label(RichText::new(tl!("Properties")).font(theme::semibold(13.5)).color(t.text));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icons::button(ui, "minus", 22.0, false, tl!("Hide Properties")).clicked() {
                        app.ui.panels.properties = false;
                    }
                });
            });
            let bar = title.response.rect.with_max_x(title.response.rect.right() - 28.0);
            let grip = ui.interact(bar, card_id.with("title"), Sense::drag());
            if grip.hovered() || grip.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            }
            drag = grip.drag_delta();
            ui.add_space(6.0);
            widgets::hairline(ui);
            ui.add_space(8.0);
            properties_body(app, ui);
        });
    });
    if drag != egui::Vec2::ZERO && canvas.is_positive() {
        // Keep the title bar inside the canvas.
        let r = shown.response.rect.translate(drag);
        let dx = (canvas.left() - r.left()).max(0.0) - (r.right() - canvas.right()).max(0.0);
        let dy = (canvas.top() - r.top()).max(0.0) - (r.top() + 40.0 - canvas.bottom()).max(0.0);
        ctx.data_mut(|m| m.insert_temp(card_id, offset + drag + egui::vec2(dx, dy)));
    }
}

fn adjustment_controls(app: &mut PhotocraftApp, ui: &mut egui::Ui, id: LayerId, adj: &photocraft_doc::Adjustment) {
    crate::adjust_editors::layer_editor(app, ui, id, adj);
    ui.add_space(6.0);
    let layer = app.session.active().and_then(|s| s.doc.layer(id).map(|l| (l.visible, l.clipped)));
    if let Some((visible, clipped)) = layer {
        ui.horizontal(|ui| {
            let mut vis = visible;
            if widgets::toggle(ui, &mut vis, tl!("Adjustment visible")).changed() {
                let _ = app.run("layer.setProps", json!({"layer": id.0, "visible": vis}));
            }
            let mut clip = clipped;
            if widgets::toggle(ui, &mut clip, tl!("Clip to layer below")).changed() {
                let _ = app.run("layer.setProps", json!({"layer": id.0, "clipped": clip}));
            }
        });
    }
    ui.add_space(6.0);
    if widgets::secondary_button(ui, tl!("Reset to defaults"), ui.available_width()).clicked() {
        let doc = app.session.active().map_or(0, |s| s.doc.id.0);
        // Photoshop's two-step Reset where measured (Color Balance), else the defaults.
        let params = crate::adjust_editors::properties_reset(ui.ctx(), doc, id, adj).unwrap_or_else(|| json!({"layer": id.0}));
        let _ = app.run("layer.setAdjustment", params);
        app.live_adjust = None;
    }
}

fn layer_controls(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: &Layer) {
    let t = Tokens::get(ui.ctx());
    if let Some(s) = layer.surface() {
        let b = app.cached_bounds(layer.id.0, s);
        egui::Grid::new("props-grid").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            for (k, v) in [("X", b.x0.to_string()), ("Y", b.y0.to_string()), ("W", b.width().to_string()), ("H", b.height().to_string())] {
                ui.label(RichText::new(k).color(t.text_dim));
                ui.label(RichText::new(format!("{v} px")).font(theme::mono(12.0)).color(t.text));
                ui.end_row();
            }
        });
        ui.add_space(6.0);
    }
    let mut o = layer.opacity * 100.0;
    let r = widgets::slider_row(ui, tl!("Opacity"), &mut o, 0.0..=100.0, "%", None);
    if r.drag_stopped() || (r.changed() && !r.dragged()) {
        let _ = app.run("layer.setProps", json!({"layer": layer.id.0, "opacity": o / 100.0}));
    }
    let mut f = layer.fill_opacity * 100.0;
    let r = widgets::slider_row(ui, crate::i18n::tr_ctx(crate::i18n::current(), "layers", "Fill"), &mut f, 0.0..=100.0, "%", None);
    if r.drag_stopped() || (r.changed() && !r.dragged()) {
        let _ = app.run("layer.setProps", json!({"layer": layer.id.0, "fill": f / 100.0}));
    }
}

/// Docked Properties body (Pro theme).
fn properties_body(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let identity = app.session.active().map(|st| (st.doc.id, st.active_layer, app.ui.mask_target, app.ui.vector_mask_target, app.ui.properties_layer));
    ui.push_id(("properties-target", identity), |ui| properties_body_inner(app, ui));
}

fn properties_body_inner(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(st) = app.session.active() else {
        empty(ui, tl!("No properties"));
        return;
    };
    let doc = st.doc.clone();
    if let Some(layer) = st.active_layer.and_then(|id| doc.layer(id))
        && crate::mask_props_ui::targeted(app, layer)
    {
        crate::props_layout::header(ui, layer);
        crate::mask_props_ui::properties(app, ui, layer);
        return;
    }
    // The vector mask's own controls, when its thumbnail is targeted.
    if let Some(layer) = st.active_layer.and_then(|id| doc.layer(id))
        && crate::mask_props_ui::vector_targeted(app, layer)
    {
        crate::props_layout::header(ui, layer);
        crate::mask_props_ui::vector_properties(app, ui, layer);
        return;
    }
    // Photoshop shows the Document properties when nothing or the Background layer is selected,
    // with Align and Distribute above them when the Background is one of several (#4608).
    if crate::doc_props_ui::shows_document(&st.doc, st.active_layer) {
        if t.pro && st.selected_layers().len() > 1 {
            crate::layer_props_ui::selection_align(app, ui);
        }
        crate::doc_props_ui::properties(app, ui);
        return;
    }
    let Some(id) = st.active_layer else { return };
    // Borrowed from the document snapshot: cloning the layer every frame copied whole groups.
    let Some(layer) = doc.layer(id) else { return };
    // Header: kind icon, layer name and kind (#155); sections below draw their own separators.
    crate::props_layout::header(ui, layer);
    let is_adjustment = matches!(layer.content, LayerContent::Adjustment(_));
    if is_adjustment || layer.artboard().is_some() || !t.pro {
        ui.add_space(4.0);
        widgets::hairline(ui);
        ui.add_space(6.0);
    }
    if let LayerContent::Adjustment(adj) = &layer.content {
        adjustment_controls(app, ui, id, adj);
    } else if layer.artboard().is_some() {
        crate::artboard_ui::properties(app, ui, layer);
    } else if t.pro {
        // Transform, Align, the kind's sections and Quick Actions.
        crate::layer_props_ui::properties(app, ui, layer);
    } else {
        if matches!(layer.content, LayerContent::Fill(photocraft_doc::Fill::Gradient { .. })) {
            crate::gradient_ui::properties(app, ui, layer);
            ui.add_space(6.0);
        }
        layer_controls(app, ui, layer);
        if matches!(layer.content, LayerContent::Text(_)) {
            crate::type_tool::type_properties(app, ui);
        }
        if matches!(layer.content, LayerContent::Shape(_)) {
            crate::vector_ui::shape_properties(app, ui, id);
        }
    }
}

/// Photoshop's Adjustments panel: a grid of one-click adjustment layers.
fn adjustments_grid(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    ui.label(RichText::new(tl!("Add an adjustment")).color(t.text_dim));
    ui.add_space(4.0);
    let items: [(&str, &str, &str); 16] = [
        ("brightnessContrast", "sun", tl!("Brightness/Contrast")),
        ("levels", "gauge", tl!("Levels")),
        ("curves", "pen-tool", tl!("Curves")),
        ("exposure", "scan", tl!("Exposure")),
        ("vibrance", "sparkles", tl!("Vibrance")),
        ("hueSaturation", "droplet", tl!("Hue/Saturation")),
        ("colorBalance", "blend", tl!("Color Balance")),
        ("blackWhite", "contrast", tl!("Black & White")),
        ("photoFilter", "circle-dot", tl!("Photo Filter")),
        ("channelMixer", "sliders-horizontal", tl!("Channel Mixer")),
        ("invert", "squares-subtract", tl!("Invert")),
        ("posterize", "layers", tl!("Posterize")),
        ("threshold", "square", tl!("Threshold")),
        ("gradientMap", "palette", tl!("Gradient Map")),
        ("selectiveColor", "swatch-book", tl!("Selective Color")),
        ("colorLookup", "grid-3x3", tl!("Color Lookup")),
    ];
    let mut run = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
        for (kind, icon, tip) in items {
            if icons::button(ui, icon, 30.0, false, tip).clicked() {
                run = Some(format!("layer.newAdjustmentLayer.{kind}"));
            }
        }
    });
    if let Some(id) = run {
        let _ = app.run(&id, json!({}));
        app.ui.dock_tabs.properties = 0;
    }
}

/// The Color panel's foreground and background chips. A click picks the colour the field edits,
/// framed; a double-click opens the Color Picker on it.
fn field_chips(app: &mut PhotocraftApp, ui: &mut egui::Ui, chips: Rect) {
    let t = Tokens::get(ui.ctx());
    let bgr = Rect::from_min_size(chips.min + vec2(13.0, 13.0), vec2(22.0, 22.0));
    let fgr = Rect::from_min_size(chips.min + vec2(3.0, 3.0), vec2(22.0, 22.0));
    let frame = Stroke::new(1.0, t.text_dim);
    let bg_active = app.ui.color_panel.background;
    let p = ui.painter();
    p.rect_filled(bgr, 2.0, c32(app.session.tools.background));
    p.rect_stroke(bgr, 2.0, Stroke::new(1.0, t.field_border), StrokeKind::Outside);
    if bg_active {
        p.rect_stroke(bgr.expand(2.0), 2.0, frame, StrokeKind::Outside);
    }
    p.rect_filled(fgr, 2.0, c32(app.session.tools.foreground));
    p.rect_stroke(fgr, 2.0, Stroke::new(1.0, Color32::from_gray(210)), StrokeKind::Outside);
    if !bg_active {
        p.rect_stroke(fgr.expand(2.0), 2.0, frame, StrokeKind::Outside);
    }
    // The foreground is on top, so it takes the clicks where the two overlap.
    let bg_resp = ui.interact(bgr, ui.id().with("field-bg"), Sense::click());
    let fg_resp = ui.interact(fgr, ui.id().with("field-fg"), Sense::click());
    let picked = if fg_resp.clicked() { false } else { bg_resp.clicked() || bg_active };
    app.ui.color_panel.background = picked;
    if fg_resp.double_clicked() || bg_resp.double_clicked() {
        crate::color_picker_ui::open(app, if picked { "background" } else { "foreground" });
    }
}

/// Where the dock tells the Color field how tall its group body is.
pub(crate) fn color_field_fill_id() -> egui::Id {
    egui::Id::new("color-field-fill-height")
}

/// Height of the Color field: the group body's height (when docked) less the hex/RGB readout,
/// which folds onto a second line in a narrow dock. 120 pt when nothing sets a height.
fn color_field_height(avail: Option<f32>, width: f32) -> f32 {
    let reserve = 32.0 + if width < 280.0 { 26.0 } else { 0.0 };
    avail.filter(|a| a.is_finite()).map_or(120.0, |a| (a - reserve).clamp(120.0, 2000.0))
}

/// The colour the SV field draws for `hsva`, with what the field showed last (`hue`, `saturation`)
/// filling in what the colour itself cannot carry: a grey has no hue of its own, and black has
/// neither hue nor saturation — so a drag that bottoms out into black would otherwise round-trip
/// through RGB as `s = 0` and drop the knob from the bottom-right corner to the bottom-left one
/// (#3228).
fn field_hsva(mut hsva: egui::ecolor::Hsva, remembered: (f32, f32)) -> egui::ecolor::Hsva {
    if hsva.v < 0.01 {
        hsva.h = remembered.0;
        hsva.s = remembered.1;
    } else if hsva.s < 0.01 {
        // A grey: the saturation really is zero (that is where the knob belongs), only the hue
        // is undefined.
        hsva.h = remembered.0;
    }
    hsva
}

/// Photoshop Color panel: saturation/brightness field + hue strip, drawn as shaded meshes.
fn color_field(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let bg_active = app.ui.color_panel.background;
    let key = egui::Id::new(("color-field-hue", bg_active));
    let mut hsva = srgb_hsva(if bg_active { app.session.tools.background } else { app.session.tools.foreground });
    // What the field showed last: a grey carries no hue, black neither hue nor saturation (#3228).
    let remembered: (f32, f32) = ui.data(|d| d.get_temp(key)).unwrap_or((hsva.h, hsva.s));
    hsva = field_hsva(hsva, remembered);
    let w = ui.available_width();
    let strip_w = 14.0;
    // Photoshop's Color panel field fills the panel: take the height the dock offers, less the
    // hex/RGB readout (which folds onto a second line in a narrow dock). Without a dock, 120 pt.
    // Only Photoshop's themes stretch it over the group: the other themes put sliders under it.
    let fill = if t.pro { ui.data(|d| d.get_temp::<f32>(color_field_fill_id())) } else { None };
    let h = color_field_height(fill, w);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let (chips, _) = ui.allocate_exact_size(vec2(38.0, h), Sense::hover());
        field_chips(app, ui, chips);
        // SV field.
        let field_w = w - 38.0 - strip_w - 16.0;
        let (field, fresp) = ui.allocate_exact_size(vec2(field_w, h), Sense::click_and_drag());
        let mut mesh = egui::Mesh::default();
        let n = 16;
        for j in 0..=n {
            for i in 0..=n {
                let (sx, vy) = (i as f32 / n as f32, j as f32 / n as f32);
                let c = hsva_color32(egui::ecolor::Hsva::new(hsva.h, sx, 1.0 - vy, 1.0));
                mesh.colored_vertex(pos2(field.left() + sx * field.width(), field.top() + vy * field.height()), c);
            }
        }
        for j in 0..n {
            for i in 0..n {
                let a = (j * (n + 1) + i) as u32;
                let b = a + 1;
                let c = a + (n + 1) as u32;
                let d = c + 1;
                mesh.add_triangle(a, b, d);
                mesh.add_triangle(a, d, c);
            }
        }
        ui.painter().add(mesh);
        ui.painter().rect_stroke(field, 0.0, Stroke::new(1.0, t.separator), StrokeKind::Outside);
        if (fresp.dragged() || fresp.clicked())
            && let Some(p) = fresp.interact_pointer_pos()
        {
            hsva.s = ((p.x - field.left()) / field.width()).clamp(0.0, 1.0);
            hsva.v = 1.0 - ((p.y - field.top()) / field.height()).clamp(0.0, 1.0);
        }
        let knob = pos2(field.left() + hsva.s * field.width(), field.top() + (1.0 - hsva.v) * field.height());
        ui.painter().circle_stroke(knob, 5.0, Stroke::new(1.5, Color32::WHITE));
        ui.painter().circle_stroke(knob, 6.5, Stroke::new(1.0, Color32::from_black_alpha(160)));
        // Hue strip.
        let (strip, sresp) = ui.allocate_exact_size(vec2(strip_w, h), Sense::click_and_drag());
        let mut m2 = egui::Mesh::default();
        let steps = 24;
        for k in 0..=steps {
            let f = k as f32 / steps as f32;
            let c = hsva_color32(egui::ecolor::Hsva::new(1.0 - f, 1.0, 1.0, 1.0));
            m2.colored_vertex(pos2(strip.left(), strip.top() + f * strip.height()), c);
            m2.colored_vertex(pos2(strip.right(), strip.top() + f * strip.height()), c);
        }
        for k in 0..steps {
            let a = (k * 2) as u32;
            m2.add_triangle(a, a + 1, a + 3);
            m2.add_triangle(a, a + 3, a + 2);
        }
        ui.painter().add(m2);
        if (sresp.dragged() || sresp.clicked())
            && let Some(p) = sresp.interact_pointer_pos()
        {
            hsva.h = 1.0 - ((p.y - strip.top()) / strip.height()).clamp(0.0, 0.9999);
        }
        let y = strip.top() + (1.0 - hsva.h) * strip.height();
        let tri = vec![pos2(strip.right() + 1.0, y), pos2(strip.right() + 6.0, y - 4.0), pos2(strip.right() + 6.0, y + 4.0)];
        ui.painter().add(egui::Shape::convex_polygon(tri, t.text, Stroke::NONE));
        if fresp.dragged() || fresp.clicked() || sresp.dragged() || sresp.clicked() {
            if bg_active {
                app.session.tools.background = hsva_srgb(hsva);
            } else {
                app.session.tools.foreground = hsva_srgb(hsva);
                crate::type_tool::foreground_changed(app);
            }
            ui.data_mut(|d| d.insert_temp(key, (hsva.h, hsva.s)));
        }
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let mut c = if bg_active { app.session.tools.background } else { app.session.tools.foreground };
        if color_readout(ui, ui.id().with(("color-field", bg_active)), &mut c) {
            if bg_active {
                app.session.tools.background = c;
            } else {
                app.session.tools.foreground = c;
                crate::type_tool::foreground_changed(app);
            }
            // Keep the hue for greys, so the field marker doesn't jump to red; the saturation with
            // it, so a later black lands on the corner it was last dragged to (#3228).
            let h = srgb_hsva(c);
            if h.s >= 0.01 && h.v >= 0.01 {
                ui.data_mut(|d| d.insert_temp(key, (h.h, h.s)));
            }
        }
    });
}

/// Tool colours are sRGB-encoded floats; egui's Hsva works on sRGB bytes via these helpers
/// (its `from_rgba_unmultiplied` expects *linear* RGB, which gave wrong readouts).
fn srgb_bytes(c: [f32; 4]) -> [u8; 3] {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    [q(c[0]), q(c[1]), q(c[2])]
}

/// The Color panel's HSB, as Photoshop's and the Color Picker's: on the sRGB-encoded values,
/// so a 50% grey has Brightness 50%. egui's `Hsva` conversions work in linear light, which put
/// that grey at 22% and drew the field's middle as 188 (#3323); `Hsva` only holds the numbers.
fn srgb_hsva(c: [f32; 4]) -> egui::ecolor::Hsva {
    let [h, s, v] = crate::color_picker_ui::rgb_to_hsv([c[0], c[1], c[2]].map(|v| v.clamp(0.0, 1.0)));
    egui::ecolor::Hsva::new(h / 360.0, s, v, 1.0)
}

fn hsva_srgb(h: egui::ecolor::Hsva) -> [f32; 4] {
    let [r, g, b] = crate::color_picker_ui::hsv_to_rgb(h.h * 360.0, h.s.clamp(0.0, 1.0), h.v.clamp(0.0, 1.0));
    [r, g, b, 1.0]
}

/// The colour to draw for Color panel HSB `h` (see [`srgb_hsva`]).
fn hsva_color32(h: egui::ecolor::Hsva) -> Color32 {
    let [r, g, b] = srgb_bytes(hsva_srgb(h));
    Color32::from_rgb(r, g, b)
}

/// The Color panel's editable colour readout: a hex field and R, G, B fields, all drawn like the
/// dock's [`widgets::value_field`] so they sit in the theme. Editing any of them sets `color`
/// (sRGB-encoded floats) and returns `true`.
fn color_readout(ui: &mut egui::Ui, id: egui::Id, color: &mut [f32; 4]) -> bool {
    let t = Tokens::get(ui.ctx());
    let mut changed = false;
    // Wrapped so the fields fold onto a second line in a narrow dock instead of being clipped.
    ui.horizontal_wrapped(|ui| {
        // Docks zero the item spacing; keep the fields apart.
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label(RichText::new("#").font(theme::mono(12.0)).color(t.text_faint));
        changed = hex_edit(ui, id.with("hex"), color);
        let [mut r, mut g, mut b] = srgb_bytes(*color).map(f32::from);
        let mut rgb = false;
        for (label, v) in [("R", &mut r), ("G", &mut g), ("B", &mut b)] {
            ui.label(RichText::new(label).font(theme::mono(11.0)).color(t.text_faint));
            rgb |= widgets::value_field(ui, v, 0.0..=255.0, "", 38.0).changed();
        }
        if rgb {
            *color = [r / 255.0, g / 255.0, b / 255.0, 1.0];
            changed = true;
        }
    });
    changed
}

/// The hex field of [`color_readout`]: type a colour with or without the leading `#`. Valid input
/// sets `color` and returns `true`. While it has focus it shows exactly what is typed, so partial
/// or invalid input isn't overwritten by the colour; an invalid entry snaps back when focus leaves.
fn hex_edit(ui: &mut egui::Ui, id: egui::Id, color: &mut [f32; 4]) -> bool {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(64.0, 24.0), Sense::hover());
    widgets::surface(ui, rect, t.field, false);
    if !t.bevel {
        ui.painter().rect_stroke(rect, t.radius_sm, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
    }
    let [r, g, b] = srgb_bytes(*color);
    let typing = if ui.memory(|m| m.has_focus(id)) { ui.data(|d| d.get_temp::<String>(id)) } else { None };
    let mut text = typing.unwrap_or_else(|| format!("{r:02X}{g:02X}{b:02X}"));
    let field = rect.shrink2(vec2(5.0, 2.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(field).layout(egui::Layout::left_to_right(egui::Align::Center)));
    // Room for a pasted "#rrggbb"; the parse trims it, and the field shows plain digits otherwise.
    // `Frame::NONE`: the themed surface above is this field's frame.
    let resp =
        child.add(egui::TextEdit::singleline(&mut text).id(id).char_limit(7).desired_width(field.width()).frame(egui::Frame::NONE).font(theme::mono(12.0)));
    if resp.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, text.clone()));
    } else {
        ui.data_mut(|d| d.remove::<String>(id));
    }
    if resp.changed()
        && let Some(c) = crate::color_picker_ui::parse_hex(&text)
    {
        *color = [c[0], c[1], c[2], 1.0];
        return true;
    }
    false
}

/// Photoshop's brush preset picker chip: a soft/hard round tip preview with the size underneath.
/// Draw a round brush tip preview (hard core fading to a soft edge).
fn brush_tip(p: &egui::Painter, c: egui::Pos2, rad: f32, hardness: f32, color: Color32) {
    let inner = rad * hardness.clamp(0.05, 1.0);
    let steps = 8;
    for k in (0..=steps).rev() {
        let f = k as f32 / steps as f32;
        let rr = inner + (rad - inner) * f;
        let a = if hardness >= 0.99 { 1.0 } else { (1.0 - f).powf(1.5) };
        p.circle_filled(c, rr, color.gamma_multiply(a));
    }
}

/// The Brush's or Pencil's Mode dropdown: the blend modes, with Photoshop's painting-only Clear
/// after Dissolve (#3439). Clear erases with the brush (`clear`); it shows while chosen, or while
/// `~` is held (`held`). The wheel over it steps the mode, as over the Layers panel's (#2061).
fn paint_mode_dropdown(ui: &mut egui::Ui, id: &str, b: &mut photocraft_engine::BrushSettings, clear: &mut bool, held: bool) {
    let mut opts: Vec<(Option<BlendMode>, &str)> = BlendMode::LAYER_MODES.iter().map(|m| (Some(*m), m.label())).collect();
    let at = opts.iter().position(|o| o.0 == Some(BlendMode::Dissolve)).map_or(opts.len(), |i| i + 1);
    opts.insert(at, (None, "Clear"));
    let mut mode = if *clear || held { None } else { Some(b.mode) };
    if !widgets::dropdown_wheel_hovered(ui, id, &mut mode, &opts, 96.0).0.is_empty() {
        *clear = mode.is_none();
        if let Some(m) = mode {
            b.mode = m;
        }
    }
}

/// Options-bar Smoothing % (the brush's stroke smoothing; the live stroke and the commit use it).
fn smoothing_field(ui: &mut egui::Ui, b: &mut photocraft_engine::BrushSettings, width: f32) {
    let mut sm = (b.smoothing.amount * 100.0).round();
    if widgets::value_field(ui, &mut sm, 0.0..=100.0, "%", width).changed() {
        b.smoothing.enabled = true;
        b.smoothing.amount = (sm / 100.0).clamp(0.0, 1.0);
    }
}

/// Options-bar gear beside Smoothing: Photoshop's smoothing options popup, edited on the brush
/// like the Smoothing % (so they stay per tool and reach the live stroke and the commit).
fn smoothing_options(ui: &mut egui::Ui, b: &mut photocraft_engine::BrushSettings) {
    let resp = icons::button(ui, "settings", 24.0, false, tl!("Set additional smoothing options"));
    let resp = crate::brush_picker::named(resp, "Set additional smoothing options");
    egui::Popup::from_toggle_button_response(&resp).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        let s = &mut b.smoothing;
        widgets::checkbox(ui, &mut s.pulled_string, tl!("Pulled String Mode"));
        widgets::checkbox(ui, &mut s.catch_up, tl!("Stroke Catch-up"));
        widgets::checkbox(ui, &mut s.catch_up_on_end, tl!("Catch-up on Stroke End"));
        widgets::checkbox(ui, &mut s.adjust_for_zoom, tl!("Adjust for Zoom"));
    });
}

/// Options-bar brush chip: a click shows or hides Photoshop's Brush Preset picker below it (size,
/// hardness, the preset library), the one a right-click on the canvas opens.
fn brush_preset_chip(ui: &mut egui::Ui, b: &photocraft_engine::BrushSettings, state: &mut crate::state::UiState) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(44.0, 30.0), Sense::hover());
    let resp = ui.interact(r, crate::brush_picker::chip_id(), Sense::click());
    if resp.hovered() || state.brush_picker.is_some() {
        ui.painter().rect_filled(r, t.radius_sm, t.hover);
    }
    let c = pos2(r.left() + 14.0, r.top() + 11.0);
    // Photoshop's white tip vanishes on light chrome; light themes use their icon colour.
    let tip = if t.dark() { Color32::WHITE } else { t.icon };
    brush_tip(ui.painter(), c, 7.0, b.hardness, tip);
    ui.painter().text(pos2(c.x, r.bottom() - 5.0), Align2::CENTER_CENTER, format!("{}", b.size.round() as i64), egui::FontId::proportional(9.5), t.text_dim);
    icons::paint(ui, Rect::from_center_size(pos2(r.right() - 9.0, c.y), vec2(10.0, 10.0)), "chevron-down", 9.0, t.text_faint);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tl!("Brush Preset picker")));
    if resp.on_hover_text(tl!("Brush Preset picker")).clicked() {
        if state.brush_picker.is_some() {
            crate::brush_picker::close(state);
        } else {
            crate::brush_picker::open(state, ui.ctx(), [r.left(), r.bottom() + 2.0]);
        }
    }
}

/// A drag from within a multi-selection moves the whole selection, not only the grabbed
/// row. Dragging an unselected row keeps the existing single-layer behavior.
fn layer_drop_payload(dragged: u64, target: LayerId, position: &str, selected: &[LayerId]) -> Value {
    if selected.len() > 1 && selected.contains(&LayerId(dragged)) {
        json!({"layers": selected.iter().map(|id| id.0).collect::<Vec<_>>(), "target": target.0, "position": position})
    } else {
        json!({"layer": dragged, "target": target.0, "position": position})
    }
}

/// Would this drop move the Background off the bottom, or put a layer below it? Photoshop
/// refuses both (no drop indicator); ⌥-dragging the Background still copies it above.
fn background_drop_refused(doc: &photocraft_doc::Document, payload: &Value, target: LayerId, position: &str, copy: bool) -> bool {
    let Some(bg) = doc.layers.first().filter(|b| crate::doc_props_ui::is_background(doc, b)).map(|b| b.id) else { return false };
    let moves_bg =
        !copy && (payload["layer"].as_u64() == Some(bg.0) || payload["layers"].as_array().is_some_and(|a| a.iter().any(|v| v.as_u64() == Some(bg.0))));
    moves_bg || (target == bg && position == "below")
}

/// Drag a layer row to reorder: drop above, below, or inside an existing group. With ⌥ held on
/// release the layers stay put and copies land there instead (Photoshop).
/// Multi-layer moves are atomic (one undo step), using the engine's stable document order.
fn layer_drag_and_drop(
    app: &PhotocraftApp,
    ctx: &egui::Context,
    ui: &egui::Ui,
    l: &Layer,
    rect: Rect,
    resp: &egui::Response,
    actions: &mut Vec<(String, Value)>,
) {
    let t = Tokens::get(ctx);
    let key = egui::Id::new("layer-drag");
    // A drag from the mask thumbnail (`starts_mask_drag`) moves the mask, not the layer.
    let mask_drag = ctx.data(|d| d.get_temp::<MaskDrag>(mask_drag_key())).is_some_and(|(id, _)| id == l.id.0);
    if resp.drag_started() && !mask_drag {
        ctx.data_mut(|d| d.insert_temp(key, l.id.0));
    }
    let Some(dragged) = ctx.data(|d| d.get_temp::<u64>(key)) else { return };
    let pointer = ctx.input(|i| i.pointer.interact_pos());
    let released = ctx.input(|i| i.pointer.any_released());
    let copy = ctx.input(|i| i.modifiers.alt);
    if dragged == l.id.0 {
        // Ghost label following the pointer, and the copy cursor while ⌥ is held.
        if let Some(p) = pointer {
            crate::layer_transfer::ghost(ctx, p, &l.name);
        }
        if copy {
            ctx.set_cursor_icon(egui::CursorIcon::Copy);
        }
        // #1721: with ⌥ held the row also takes the drop onto itself — the only drop target a
        // single-layer document has. The copy lands above or below it (never into a group's own
        // centre). A multi-selection is left alone: the engine copies it beside each of its own
        // layers, not onto one of them.
        let selected = app.session.active().map(|st| st.selected_layers()).unwrap_or_default();
        let in_multi = selected.len() > 1 && selected.contains(&l.id);
        if copy
            && !in_multi
            && let Some(p) = pointer.filter(|p| rect.contains(*p))
        {
            let position = if (p.y - rect.top()) / rect.height() < 0.5 { "above" } else { "below" };
            let painter = ui.painter();
            if position == "above" {
                painter.line_segment([rect.left_top(), rect.right_top()], Stroke::new(2.0, t.accent));
            } else {
                painter.line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(2.0, t.accent));
            }
            if released {
                let mut payload = layer_drop_payload(dragged, l.id, position, &selected);
                if let Some(o) = payload.as_object_mut() {
                    o.insert("copy".into(), json!(true));
                }
                actions.push(("layer.moveTo".into(), payload));
            }
        }
        return;
    }
    let Some(p) = pointer else { return };
    if !rect.contains(p) {
        return;
    }
    let f = (p.y - rect.top()) / rect.height();
    let position = if l.is_group() && (0.3..0.7).contains(&f) {
        "into"
    } else if f < 0.5 {
        "above"
    } else {
        "below"
    };
    let Some(st) = app.session.active() else { return };
    let mut payload = layer_drop_payload(dragged, l.id, position, &st.selected_layers());
    if background_drop_refused(&st.doc, &payload, l.id, position, copy) {
        return;
    }
    let painter = ui.painter();
    match position {
        "into" => {
            painter.rect_stroke(rect.shrink(1.0), t.radius_sm, Stroke::new(2.0, t.accent), StrokeKind::Inside);
        }
        "above" => {
            painter.line_segment([rect.left_top(), rect.right_top()], Stroke::new(2.0, t.accent));
        }
        _ => {
            painter.line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(2.0, t.accent));
        }
    }
    if released {
        if copy && let Some(o) = payload.as_object_mut() {
            o.insert("copy".into(), json!(true));
        }
        actions.push(("layer.moveTo".into(), payload));
    }
}

/// #1721: the drop zones no row owns — the space between two rows and the empty space below the
/// bottom-most one. The drop lands on `target` at `position` (`above` or `below` its row): a copy
/// with ⌥ held on release (Photoshop), a move without. A multi-selection dropped on the row it
/// was grabbed from is left alone: the engine can only copy it beside each of its own layers.
fn layer_drop_zone(
    app: &PhotocraftApp,
    ctx: &egui::Context,
    ui: &egui::Ui,
    zone: Rect,
    target: (LayerId, Rect),
    position: &'static str,
    actions: &mut Vec<(String, Value)>,
) {
    let Some(dragged) = ctx.data(|d| d.get_temp::<u64>(egui::Id::new("layer-drag"))) else { return };
    let selected = app.session.active().map(|st| st.selected_layers()).unwrap_or_default();
    if target.0 == LayerId(dragged) && selected.len() > 1 && selected.contains(&target.0) {
        return;
    }
    let Some(p) = ctx.input(|i| i.pointer.interact_pos()) else { return };
    if !zone.contains(p) {
        return;
    }
    let t = Tokens::get(ctx);
    let line = if position == "above" { [target.1.left_top(), target.1.right_top()] } else { [target.1.left_bottom(), target.1.right_bottom()] };
    ui.painter().line_segment(line, Stroke::new(2.0, t.accent));
    if ctx.input(|i| i.pointer.any_released()) {
        let mut payload = layer_drop_payload(dragged, target.0, position, &selected);
        if ctx.input(|i| i.modifiers.alt)
            && let Some(o) = payload.as_object_mut()
        {
            o.insert("copy".into(), json!(true));
        }
        actions.push(("layer.moveTo".into(), payload));
    }
}

/// An fx row being dragged: the layer it belongs to and the effect index (`None` = the "Effects"
/// row, all of them).
type FxDrag = (u64, Option<usize>);

/// A mask thumbnail being dragged: the layer and which of its masks (#3113, #3119).
type MaskDrag = (u64, MaskKind);

fn mask_drag_key() -> egui::Id {
    egui::Id::new("mask-drag")
}

/// Which mask's thumbnail (where it was drawn last frame) the row's drag starts on, if any.
fn starts_mask_drag(ctx: &egui::Context, l: &Layer, resp: &egui::Response) -> Option<MaskKind> {
    if !resp.drag_started() {
        return None;
    }
    let p = ctx.input(|i| i.pointer.press_origin())?;
    let (thumbs, _) = crate::mask_thumbs_ui::recorded(ctx, l.id.0)?;
    let kind = thumbs.iter().find(|(_, r)| r.contains(p))?.0;
    let present = match kind {
        MaskKind::Pixel => l.mask.is_some(),
        MaskKind::Vector => l.vector_mask.is_some(),
    };
    present.then_some(kind)
}

fn fx_drag_key() -> egui::Id {
    egui::Id::new("fx-drag")
}

/// Start dragging the effects of `layer`: one (`effect`) or all of them.
fn start_fx_drag(ctx: &egui::Context, layer: LayerId, effect: Option<usize>) {
    ctx.data_mut(|d| d.insert_temp::<FxDrag>(fx_drag_key(), (layer.0, effect)));
}

/// While effects are dragged: their name by the pointer, and the copy cursor while ⌥ is held.
fn fx_drag_feedback(ctx: &egui::Context, doc: &photocraft_doc::Document) {
    let Some((from, effect)) = ctx.data(|d| d.get_temp::<FxDrag>(fx_drag_key())) else { return };
    let Some(p) = ctx.input(|i| i.pointer.interact_pos()) else { return };
    let label = match effect {
        Some(i) => doc.layer(LayerId(from)).and_then(|l| l.effects.items.get(i)).map_or("", |e| e.label()),
        None => "Effects",
    };
    crate::layer_transfer::ghost(ctx, p, label);
    if ctx.input(|i| i.modifiers.alt) {
        ctx.set_cursor_icon(egui::CursorIcon::Copy);
    }
}

/// The command for dropping effects of layer `from` on `target`: they move there, or with ⌥ held
/// on release are copied (Photoshop). `effect` is one effect row, or all effects when `None`.
fn fx_drop_action(from: u64, effect: Option<usize>, target: LayerId, copy: bool) -> (String, Value) {
    let mut payload = json!({"from": from, "to": target.0, "copy": copy});
    if let Some(o) = payload.as_object_mut()
        && let Some(i) = effect
    {
        o.insert("effect".into(), json!(i));
    }
    ("layer.layerStyle.transferEffects".into(), payload)
}

/// A layer row accepts the fx row being dragged: outlined while the pointer is over it, dropped
/// on release.
fn fx_drop(ctx: &egui::Context, ui: &egui::Ui, l: &Layer, rect: Rect, actions: &mut Vec<(String, Value)>) {
    let Some((from, effect)) = ctx.data(|d| d.get_temp::<FxDrag>(fx_drag_key())) else { return };
    let Some(p) = ctx.input(|i| i.pointer.interact_pos()) else { return };
    if from == l.id.0 || !rect.contains(p) {
        return;
    }
    let t = Tokens::get(ctx);
    ui.painter().rect_stroke(rect.shrink(1.0), t.radius_sm, Stroke::new(2.0, t.accent), StrokeKind::Inside);
    if ctx.input(|i| i.pointer.any_released()) {
        actions.push(fx_drop_action(from, effect, l.id, ctx.input(|i| i.modifiers.alt)));
    }
}

/// Photoshop shows a layer's effects as indented sub-rows ("Effects", then each effect). The eye
/// on "Effects" shows or hides them all, the eye on an effect's row just that one (#1622).
fn effect_rows(app: &mut PhotocraftApp, ui: &mut egui::Ui, l: &Layer, depth: usize, actions: &mut Vec<(String, Value)>) {
    let t = Tokens::get(ui.ctx());
    let indent = 30.0 + depth as f32 * 14.0 + 34.0;
    let mut rows: Vec<(String, bool, Option<&'static str>)> = vec![("Effects".into(), l.effects.enabled, None)];
    for e in &l.effects.items {
        let kind = crate::layer_style::KINDS.iter().find(|k| k.1 == e.label()).map(|k| k.0);
        rows.push((e.label().to_string(), e.enabled(), kind));
    }
    for (i, (name, on, kind)) in rows.into_iter().enumerate() {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click_and_drag());
        // Drag the row onto another layer to move its effects there, ⌥-drag to copy them.
        if resp.drag_started() {
            start_fx_drag(ui.ctx(), l.id, i.checked_sub(1));
        }
        if !ui.is_rect_visible(rect) {
            continue;
        }
        if resp.hovered() {
            ui.painter().rect_filled(rect, 0.0, t.hover.gamma_multiply(0.35));
        }
        if t.pro {
            ui.painter().line_segment([pos2(rect.left() + 30.0, rect.top()), pos2(rect.left() + 30.0, rect.bottom())], Stroke::new(1.0, t.separator));
        }
        // Nested under the layer's eye column so an effect does not read as its own layer (#4572).
        let eye = Rect::from_min_size(pos2(rect.left() + 36.0, rect.center().y - 9.0), vec2(18.0, 18.0));
        // A hidden effect's eye box is left empty (still clickable), like a hidden layer's.
        if on {
            icons::paint(ui, eye, "eye", 12.0, t.icon);
        }
        // The whole eye column of the row takes the click, so it never opens the Layer Style dialog.
        let eye_cell = Rect::from_min_max(pos2(rect.left() + 30.0, rect.top()), pos2((rect.left() + 58.0).min(rect.right()), rect.bottom()));
        if ui.interact(eye_cell, ui.id().with(("fx-eye", l.id.0, i)), Sense::click()).clicked() {
            let mut params = json!({"layer": l.id.0, "visible": !on});
            if let (Some(index), Some(o)) = (i.checked_sub(1), params.as_object_mut()) {
                o.insert("index".into(), json!(index));
            }
            actions.push(("layer.setEffectsVisible".into(), params));
        }
        let x = rect.left() + indent + if i == 0 { 8.0 } else { 24.0 };
        if i == 0 {
            icons::paint(ui, Rect::from_center_size(pos2(x - 12.0, rect.center().y), vec2(14.0, 14.0)), "sparkles", 11.0, t.text_dim);
        }
        let right = rect.right() - crate::layer_row_ui::RIGHT_PAD;
        crate::layer_row_ui::label(
            ui.painter(),
            x,
            rect.center().y,
            right,
            &name,
            egui::FontId::proportional(11.5),
            if on { t.text_dim } else { t.text_faint },
        );
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name.clone()));
        if resp.double_clicked() {
            crate::layer_style::open_for_layer(app, l.id, kind);
        }
        // #2846: right-click opens the layer style menu (Create Layer, Copy Layer Style…), as in Photoshop.
        resp.context_menu(|ui| {
            crate::widgets::style_spectrum_popup_menu(ui);
            crate::layer_menu_ui::show_effects(app, ui, l, actions);
        });
    }
}

/// New / add / subtract / intersect selection buttons (shared by selection tools).
fn selection_mode_buttons(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    ui.spacing_mut().item_spacing.x = 2.0;
    for (i, (icon, tip)) in [
        ("square", tl!("New selection").to_string()),
        ("plus", crate::i18n::fmt(tl!("Add to selection  ({key})"), &[("key", &crate::shortcuts::pretty("Shift"))])),
        ("minus", crate::i18n::fmt(tl!("Subtract from selection  ({key})"), &[("key", &crate::shortcuts::pretty("Alt"))])),
        ("squares-subtract", crate::i18n::fmt(tl!("Intersect with selection  ({key})"), &[("key", &crate::shortcuts::pretty("Shift+Alt"))])),
    ]
    .iter()
    .enumerate()
    {
        if icons::button(ui, icon, 24.0, app.ui.selection_mode == i as u8, tip).clicked() {
            app.ui.selection_mode = i as u8;
        }
    }
    ui.spacing_mut().item_spacing.x = 8.0;
}

#[cfg(test)]
mod symmetry_theme_tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};

    #[test]
    fn brush_pencil_and_eraser_expose_symmetry_menu_in_every_theme() {
        for kind in crate::theme::ThemeKind::ALL {
            for tool in [Tool::Brush, Tool::Pencil, Tool::Eraser] {
                let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
                app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
                app.ui.tool = tool;
                let mut h = Harness::builder().with_size(vec2(1600.0, 300.0)).build_ui_state(|ui, app| options_bar(app, ui), app);
                PhotocraftApp::setup_context(&h.ctx, kind);
                h.run_steps(4);
                assert!(h.query_by_label("Set painting symmetry options").is_some(), "missing symmetry menu in {:?} for {:?}", kind, tool);
            }
        }
    }
}

#[cfg(test)]
mod color_tests;

#[cfg(test)]
mod fx_drag_tests;

#[cfg(test)]
mod unsupported_marker_tests;

#[cfg(test)]
mod swatch_type_tests {
    use super::*;
    use egui_kittest::Harness;

    /// The Color panel only acts on an edit. A colour's round trip through the panel's models isn't
    /// exact for every colour (#D8452E isn't), and writing it back every frame would recolour
    /// selected type the moment it was selected.
    #[test]
    fn idle_color_panel_leaves_the_foreground_and_selected_type_alone() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
        let id = app.run("type.create", json!({"text": "Hello world", "size": 40, "x": 20, "y": 100, "color": "#ffffff"})).unwrap()["layer"].as_u64().unwrap();
        app.run("tools.setColors", json!({"foreground": "#d8452e"})).unwrap();
        let fg = app.session.tools.foreground;
        app.ui.tool = Tool::Type;
        app.ui.text_edit = Some(crate::state::TextEdit { layer: id, document: None, caret: 0, anchor: 5, session: "s".into(), created: false, mask: false });
        let mut h = Harness::builder()
            .with_context(app.ctx().clone())
            .with_size(vec2(300.0, 300.0))
            .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::panels::color::panel(app, ui), app);
        h.run_steps(4);
        assert_eq!(h.state().session.tools.foreground, fg);
        let st = h.state().session.active().unwrap();
        let Some(photocraft_doc::LayerContent::Text(t)) = st.doc.layer(photocraft_doc::LayerId(id)).map(|l| &l.content) else { panic!("type layer") };
        assert_eq!(t.char_runs().len(), 1, "still one white run");
    }
}

#[cfg(test)]
mod type_flyout_tests;

#[cfg(test)]
mod clipping_indicator_tests;

#[cfg(test)]
#[path = "layer_options_button_tests.rs"]
mod layer_options_button_tests;

#[cfg(test)]
#[path = "mask_props_ui_tests.rs"]
mod mask_properties_tests;

#[cfg(test)]
mod history_panel_select_tests;
