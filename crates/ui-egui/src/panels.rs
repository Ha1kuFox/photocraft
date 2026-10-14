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

/// The entry of blend mode `m` under the app's `key`, for every blend list the framework builds.
pub(crate) fn blend_entry<K>(key: K, m: BlendMode) -> BlendKeyEntry<K> {
    match blend_shortcut(m) {
        Some(s) => BlendKeyEntry::new(key, s),
        None => BlendKeyEntry::unlettered(key),
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
mod mask_properties_tests;

#[cfg(test)]
mod history_panel_select_tests;
