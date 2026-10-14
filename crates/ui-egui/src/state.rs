//! UI state as plain data, so the control channel (and later MCP) can read and drive every
//! aspect of the interface: tools, zoom, panels, dialogs and windows.

use serde::{Deserialize, Serialize};

mod color_panel;
mod panel_state;
mod text_edit;
mod tool;
mod tool_options;
mod transform;

use std::collections::{BTreeSet, HashMap};

use craft::behavior::dock::DocumentWindows;
use craft::color_ui::ui::{SwatchKey, SwatchesState};
use craft::draw::ui::{BrushPickerState, GradientRef};
use craft::imaging::ui::scopes::ScopePanelState;
use craft::layers::ui::filter::LayerFilter;
use craft::layers::ui::options::LayersSettings;
use craft::ui::canvas::{SplitAt, ViewCommands, ViewRequest};
use craft::ui::contract::Tracked;
use craft::ui::patterns::presets::{PresetState, PresetView};
use photocraft_doc::DocId;

use crate::panels::brush_panel::brushes::BrushKey;

pub use color_panel::ColorPanelState;
pub use panel_state::{CurvesEditorState, PointCurveDrag, PointCurveState};
pub use text_edit::{TextEdit, TypeInsertionStyle, TypeTransform};
pub use tool::Tool;
pub use tool_options::{ObjectFinderMode, SampleAllLayers, ToolOptions};
use tool_options::{default_brush_tool, yes};
pub use transform::{CamTransform, ContentAwareOptions, Extras, FreeTransform, MadeLayer, SessionWarp};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Panels {
    pub options_bar: bool,
    pub status_bar: bool,
    /// The title bar with the in-window menus. An app that embeds PhotoCraft's UI and draws its own
    /// bar can hide it; the caption buttons of a custom title bar go with it.
    #[serde(default = "yes")]
    pub menu_bar: bool,
    /// Window › Contextual Task Bar (#10890): the small floating bar under the Crop tool's frame
    /// (`crop_task_bar`). Its ⋯ menu's Hide bar clears it too.
    #[serde(default = "yes")]
    pub context_task_bar: bool,
    /// Where that bar sits: following its frame, or dragged or pinned (the framework `TaskBar`).
    #[serde(default)]
    pub task_bar: TaskBarPlace,
}

pub use craft::ui::canvas_controls::TaskBarPlace;

impl Default for Panels {
    fn default() -> Self {
        Self { options_bar: true, status_bar: true, menu_bar: true, context_task_bar: true, task_bar: TaskBarPlace::default() }
    }
}

/// Per-document view (camera) state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct View {
    /// Screen (device) pixels per document pixel: the user-facing zoom (`100%` is `1.0`).
    /// Canvas geometry works in egui points and divides by `ctx.pixels_per_point`
    /// (`PhotocraftApp::point_zoom`), so neither the display scale nor Interface › UI Scale
    /// magnifies the image (#1943, #2121).
    pub zoom: f32,
    /// Document-space point shown at the canvas centre.
    pub center: [f32; 2],
    /// The view commands asked of this view's canvas (Fit on Screen placing a new view, a menu row, an action step),
    /// which its next pass runs in order (`craft::ui::canvas::Canvas::queued`); never persisted.
    #[serde(skip, default = "View::first_fit")]
    pub pending: Vec<ViewRequest>,
    /// Document size this view last showed; a change (Image/Canvas Size, crop) re-centres it.
    #[serde(default)]
    pub doc_size: [u32; 2],
    /// Canvas camera rotation in degrees, wrapped to (−180, 180]. Turns the view, not the pixels.
    #[serde(default)]
    pub rotation: f32,
    /// A discrete zoom command asks the canvas to ease to the camera above; never persisted.
    #[serde(skip)]
    pub(crate) zoom_animating: bool,
    /// Optional document/screen anchor for a pointer-centred Zoom-tool transition.
    #[serde(skip)]
    pub(crate) zoom_anchor: Option<([f64; 2], [f32; 2])>,
    /// The zoom and centre of the view when the discrete zoom above was asked for: the ease starts there.
    #[serde(skip)]
    pub(crate) zoom_from: Option<(f32, [f32; 2])>,
}

impl View {
    /// A new view's queue: its first pass places it fitted.
    fn first_fit() -> Vec<ViewRequest> {
        vec![ViewRequest::Place(ViewCommands::FitOnScreen)]
    }

    /// Asks this view's canvas to run `command`, a command the user gave, on its next pass, after the commands already
    /// asked: it eases under Animated Zoom.
    pub fn request(&mut self, command: ViewCommands) {
        self.pending.push(ViewRequest::Command(command));
    }

    /// Asks this view's canvas to place the view with `command` (a new or cropped document's fit) on its next pass: the
    /// view jumps there.
    pub fn place(&mut self, command: ViewCommands) {
        self.pending.push(ViewRequest::Place(command));
    }

    /// Drops the commands asked of this view's canvas: the view set now stands.
    pub fn clear_pending(&mut self) {
        self.pending.clear();
    }

    /// Whether `command` waits for this view's canvas.
    pub fn asks(&self, command: ViewCommands) -> bool {
        self.pending.iter().any(|request| request.command() == command)
    }

    /// Asks the canvas to ease the discrete zoom about to be made, from the zoom and centre the view has now, when
    /// Animated Zoom is enabled; otherwise the zoom jumps. Called before the zoom or centre changes.
    pub(crate) fn request_zoom_animation(&mut self, enabled: bool) {
        self.zoom_animating = enabled;
        self.zoom_anchor = None;
        self.zoom_from = enabled.then_some((self.zoom, self.center));
    }
}

impl Default for View {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            center: [0.0, 0.0],
            pending: View::first_fit(),
            doc_size: [0, 0],
            rotation: 0.0,
            zoom_animating: false,
            zoom_anchor: None,
            zoom_from: None,
        }
    }
}

/// An extra OS window showing a document ("Window → Arrange → New Window for Document").
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DocWindow {
    pub id: u64,
    pub document: usize,
    pub view: View,
    pub open: bool,
    /// This window owns the document's view; the document is removed from the main canvas.
    #[serde(default)]
    pub floating: bool,
    /// Transient focus edge used to activate the document once when this viewport gains focus.
    #[serde(skip)]
    pub focused: bool,
    /// Where the window opens on screen (outer top-left, in points); `None` lets the OS place it.
    /// A tab dragged off the tab strip opens its window where it was dropped (#3733).
    #[serde(default)]
    pub position: Option<[f32; 2]>,
}

/// Options-bar state for tools (Photoshop keeps these per tool).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolOptions {
    pub tolerance: f32,
    pub contiguous: bool,
    pub anti_alias: bool,
    pub sample_all_layers: bool,
    /// Object Selection › Object Finder: hovering highlights the object a click selects (#4087).
    pub object_finder: bool,
    /// Remove tool: the fill goes to a new layer above the target (Photoshop's Create a new layer).
    pub remove_new_layer: bool,
    /// Remove tool: each stroke is filled when released (Photoshop's default). Off, the strokes
    /// collect as a tint on the canvas and ↩ applies them together (Esc drops them).
    pub remove_after_each_stroke: bool,
    pub feather: f32,
    /// linear | radial | angle | reflected | diamond
    pub gradient_style: String,
    pub gradient_reverse: bool,
    /// Gradient tool: dither to reduce 8-bit banding (Photoshop default on).
    pub gradient_dither: bool,
    /// Gradient tool mode: false = "Gradient" (live: a Gradient Fill layer, editable on canvas),
    /// true = "Classic gradient" (paints the pixels).
    pub gradient_classic: bool,
    /// Blend mode used by both live and classic gradient drags.
    pub gradient_blend_mode: photocraft_color::BlendMode,
    pub fill_opacity: f32,
    /// Paint Bucket fill source: false = Foreground colour, true = Pattern (Patterns panel selection).
    pub bucket_fill_pattern: bool,
    /// Paint Bucket "Expand" (px): grow the sampled boundary, 0..50.
    pub bucket_expand: f32,
    /// Paint Bucket "Close gaps" (px): gaps up to that wide count as closed, 0..20.
    pub bucket_close_gap: f32,
    /// Paint Bucket sampling: "active" (current layer), "all" (visible composite) or "layer"
    /// (one named layer, see `bucket_sample_layer`).
    pub bucket_sample: String,
    /// The sampled layer's id when `bucket_sample` is "layer" (0 = none chosen yet).
    pub bucket_sample_layer: u64,
    /// Type tool: family, style name, size (pt), anti-aliasing and paragraph alignment.
    pub type_font: String,
    pub type_style: String,
    pub type_size: f32,
    pub type_aa: String,
    pub type_align: String,
    /// Clone Stamp / Healing Brush.
    pub clone_aligned: bool,
    /// Pattern Stamp: lock the tile origin in document space across strokes.
    pub pattern_stamp_aligned: bool,
    /// Pattern Stamp: jitter each dab's phase (seeded, replayable).
    pub pattern_stamp_impressionist: bool,
    /// Pattern Stamp scale %, 1..1000 (100 = the tile's native size).
    pub pattern_stamp_scale: f32,
    /// Pattern Stamp rotation in degrees (counter-clockwise).
    pub pattern_stamp_angle: f32,
    /// current | currentAndBelow | all
    pub clone_sample: String,
    /// Spot Healing: contentAware | createTexture | proximityMatch
    pub spot_type: String,
    /// Patch: source (repair the selection) | destination (repair where it is dragged).
    pub patch_mode: String,
    /// Patch: Content-Aware instead of Normal, with its Structure 1..7 and Color 0..10.
    pub patch_content_aware: bool,
    pub patch_structure: f32,
    pub patch_color: f32,
    /// Content-Aware Move: move | extend, Structure 1..7, Color 0..10.
    pub cam_mode: String,
    pub cam_structure: f32,
    pub cam_color: f32,
    /// Content-Aware Move: Transform On Drop (a transform box on the dropped patch).
    pub cam_transform_on_drop: bool,
    /// Dodge/Burn: shadows | midtones | highlights, exposure %, protect tones.
    pub tone_range: String,
    pub exposure: f32,
    pub protect_tones: bool,
    /// Sponge: desaturate | saturate, vibrance.
    pub sponge_mode: String,
    pub vibrance: bool,
    /// Blur/Sharpen/Smudge strength %, Sharpen protect detail, Smudge finger painting.
    pub strength: f32,
    pub protect_detail: bool,
    pub finger_painting: bool,
    /// Quick Selection enhance edge.
    pub enhance_edge: bool,
    /// Pen: "path" (work path) or "shape" (shape layer).
    pub vector_mode: String,
    /// Shape tools: fill with the foreground colour, stroke width (0 = none), rectangle corner
    /// radius, polygon sides, line weight.
    pub shape_fill: bool,
    pub stroke_width: f32,
    #[serde(default)]
    pub shape_stroke: crate::shape_stroke_ui::StrokeOptions,
    pub corner_radius: f32,
    pub polygon_sides: u32,
    pub line_weight: f32,
    /// Freeform Pen › Curve Fit: how far (px) the fitted path may stray from the pointer trace.
    #[serde(default = "default_curve_fit")]
    pub curve_fit: f32,
    /// Pen › Auto Add/Delete: a Pen click on a segment of the active path adds an anchor there,
    /// on one of its anchors deletes it (#4144).
    #[serde(default = "yes")]
    pub pen_auto_add_delete: bool,
    /// Marquee options bar: "normal" | "fixedRatio" | "fixedSize", with the ratio or size.
    #[serde(default = "default_marquee_style")]
    pub marquee_style: String,
    #[serde(default = "one")]
    pub marquee_width: f32,
    #[serde(default = "one")]
    pub marquee_height: f32,
    /// Move tool: Auto-Select (with "layer" or "group" target) and Show Transform Controls.
    /// Auto-Select is on with the "group" target by default, as in current Photoshop;
    /// ⌘/Ctrl-click inverts it.
    #[serde(default = "yes")]
    pub move_auto_select: bool,
    #[serde(default = "default_move_target")]
    pub move_target: String,
    #[serde(default)]
    pub move_show_transform: bool,
    /// Crop options bar: aspect ratio ("" = Ratio/unconstrained, "w:h", or "original") and
    /// Delete Cropped Pixels.
    #[serde(default)]
    pub crop_ratio: String,
    /// In-progress custom crop ratio fields. Kept separately because entering one side before the
    /// other must survive a frame while the ratio itself remains unconstrained.
    #[serde(default)]
    pub crop_custom_ratio_width: f32,
    #[serde(default)]
    pub crop_custom_ratio_height: f32,
    #[serde(default = "yes")]
    pub crop_delete: bool,
    /// Crop W x H x Resolution (`crop_ratio` = "whr", #2443): width and height as typed lengths
    /// with their unit ("4 in", "1024 px"; empty = unset), the resolution ("" = the document's)
    /// and its unit ("px/in" or "px/cm"). See `crop_size`.
    #[serde(default)]
    pub crop_width: String,
    #[serde(default)]
    pub crop_height: String,
    #[serde(default)]
    pub crop_resolution: String,
    #[serde(default = "default_crop_resolution_unit")]
    pub crop_resolution_unit: String,
    /// Perspective Crop's Show Grid (`perspective_crop_ui`); it shares W, H and the resolution above.
    #[serde(default = "yes")]
    pub perspective_crop_grid: bool,
    /// Crop overlay (#1919): the guide in the crop box, when it shows and its orientation
    /// (Photoshop's defaults: Rule of Thirds, Auto Show Overlay). See `crop_overlay`.
    #[serde(default)]
    pub crop_overlay: crate::crop_overlay::CropOverlay,
    #[serde(default)]
    pub crop_overlay_show: crate::crop_overlay::OverlayShow,
    #[serde(default)]
    pub crop_overlay_orientation: u8,
    /// Crop gear menu (#1919): Show Cropped Area and the crop shield. See `crop_shield`.
    #[serde(default)]
    pub crop_shield: crate::crop_shield::CropShield,
    /// Magic Eraser opacity % (tolerance, anti-alias, contiguous and sample-all are shared with the
    /// Magic Wand and Paint Bucket).
    pub magic_eraser_opacity: f32,
    /// Background Eraser: sampling (continuous | once | backgroundSwatch), limits (discontiguous |
    /// contiguous | findEdges), tolerance % and Protect Foreground Color.
    pub bg_sampling: String,
    pub bg_limits: String,
    pub bg_tolerance: f32,
    pub bg_protect_fg: bool,
    /// Color Replacement: mode (hue | saturation | color | luminosity), sampling (continuous |
    /// once | backgroundSwatch), limits (discontiguous | contiguous | findEdges), tolerance % and
    /// anti-alias, as `paint.colorReplacement` takes them.
    pub cr_mode: String,
    pub cr_sampling: String,
    pub cr_limits: String,
    pub cr_tolerance: f32,
    pub cr_anti_alias: bool,
    /// Zoom tool › Scrubby Zoom: dragging left/right zooms continuously (else a zoom rectangle).
    #[serde(default = "yes")]
    pub zoom_scrubby: bool,
    /// Pencil › Auto Erase: a stroke that starts on the foreground colour paints the background colour.
    #[serde(default)]
    pub pencil_auto_erase: bool,
    /// Brush › Mode: Clear, which erases with the Brush (Photoshop's painting-only mode, #3439).
    /// Not a blend mode, so it is kept beside the brush's `mode` rather than in it.
    #[serde(default)]
    pub brush_clear: bool,
    /// Pencil › Mode: Clear (see `brush_clear`).
    #[serde(default)]
    pub pencil_clear: bool,
    /// Eraser › Mode: `brush` (soft, anti-aliased) or `pencil` (hard, aliased pixels, through
    /// `paint.pencil`), as in Photoshop's options bar (#2662).
    #[serde(default = "default_eraser_mode")]
    pub eraser_mode: String,
    /// Magnetic Lasso: detection width (px, 1..256), edge contrast (%, 1..100), how often it
    /// fastens points by itself (0..100), and whether pen pressure narrows the width.
    pub magnetic_width: f32,
    pub magnetic_contrast: f32,
    pub magnetic_frequency: f32,
    pub magnetic_pressure: bool,
    /// Red Eye: pupil search size (1–100) and how far corrected pixels darken (0–100).
    #[serde(default = "fifty")]
    pub red_eye_pupil_size: f32,
    #[serde(default = "fifty")]
    pub red_eye_darken: f32,
    /// Eyedropper › Sample Size: 1 = Point Sample, else the side of the averaged square (3, 5,
    /// 11, 31, 51, 101; `photocraft_engine::sample_cmds::SAMPLE_SIZES`). Shared, as in Photoshop,
    /// with the ⌥-click eyedropper of the painting tools, the dialog eyedroppers and the Info panel.
    #[serde(default = "one_u32")]
    pub eyedropper_size: u32,
    /// Eyedropper › Sample: current | currentAndBelow | all | allNoAdjustments |
    /// currentAndBelowNoAdjustments (`document.sampleColor`'s `sampleLayer`).
    #[serde(default = "default_eyedropper_sample")]
    pub eyedropper_sample: String,
    /// Eyedropper › Show Sampling Ring.
    #[serde(default = "yes")]
    pub eyedropper_ring: bool,
}

fn yes() -> bool {
    true
}

fn default_crop_resolution_unit() -> String {
    crate::crop_size::PX_PER_IN.into()
}

fn default_move_target() -> String {
    "group".into()
}

fn default_curve_fit() -> f32 {
    2.0
}

fn default_eraser_mode() -> String {
    "brush".into()
}

fn default_marquee_style() -> String {
    "normal".into()
}

fn one() -> f32 {
    1.0
}

fn fifty() -> f32 {
    50.0
}

/// The session brush starts out as the Brush's (the tool the app opens with).
fn default_brush_tool() -> Tool {
    Tool::Brush
}

fn one_u32() -> u32 {
    1
}

fn default_eyedropper_sample() -> String {
    "all".into()
}

impl Default for ToolOptions {
    fn default() -> Self {
        Self {
            tolerance: 32.0,
            contiguous: true,
            anti_alias: true,
            sample_all_layers: false,
            object_finder: true,
            remove_new_layer: false,
            remove_after_each_stroke: true,
            feather: 0.0,
            gradient_style: "linear".into(),
            gradient_reverse: false,
            gradient_dither: true,
            gradient_classic: false,
            gradient_blend_mode: photocraft_color::BlendMode::Normal,
            fill_opacity: 100.0,
            bucket_fill_pattern: false,
            bucket_expand: 0.0,
            bucket_close_gap: 0.0,
            bucket_sample: "active".into(),
            bucket_sample_layer: 0,
            type_font: "Inter".into(),
            type_style: "Regular".into(),
            type_size: 48.0,
            type_aa: "sharp".into(),
            type_align: "left".into(),
            clone_aligned: true,
            pattern_stamp_aligned: true,
            pattern_stamp_impressionist: false,
            pattern_stamp_scale: 100.0,
            pattern_stamp_angle: 0.0,
            clone_sample: "current".into(),
            spot_type: "contentAware".into(),
            patch_mode: "source".into(),
            patch_content_aware: false,
            patch_structure: 4.0,
            patch_color: 0.0,
            cam_mode: "move".into(),
            cam_structure: 4.0,
            cam_color: 0.0,
            cam_transform_on_drop: false,
            tone_range: "midtones".into(),
            exposure: 50.0,
            protect_tones: true,
            sponge_mode: "desaturate".into(),
            vibrance: true,
            strength: 50.0,
            protect_detail: true,
            finger_painting: false,
            enhance_edge: false,
            vector_mode: "path".into(),
            shape_fill: true,
            stroke_width: 0.0,
            shape_stroke: Default::default(),
            corner_radius: 0.0,
            polygon_sides: 5,
            line_weight: 3.0,
            curve_fit: default_curve_fit(),
            pen_auto_add_delete: true,
            marquee_style: default_marquee_style(),
            marquee_width: 1.0,
            marquee_height: 1.0,
            move_auto_select: true,
            move_target: default_move_target(),
            move_show_transform: false,
            crop_ratio: String::new(),
            crop_custom_ratio_width: 0.0,
            crop_custom_ratio_height: 0.0,
            crop_delete: true,
            crop_width: String::new(),
            crop_height: String::new(),
            crop_resolution: String::new(),
            crop_resolution_unit: default_crop_resolution_unit(),
            perspective_crop_grid: true,
            crop_overlay: Default::default(),
            crop_overlay_show: Default::default(),
            crop_overlay_orientation: 0,
            crop_shield: Default::default(),
            magic_eraser_opacity: 100.0,
            bg_sampling: "continuous".into(),
            bg_limits: "contiguous".into(),
            bg_tolerance: 50.0,
            bg_protect_fg: false,
            cr_mode: "color".into(),
            cr_sampling: "continuous".into(),
            cr_limits: "contiguous".into(),
            cr_tolerance: 30.0,
            cr_anti_alias: true,
            zoom_scrubby: true,
            pencil_auto_erase: false,
            brush_clear: false,
            pencil_clear: false,
            eraser_mode: default_eraser_mode(),
            magnetic_width: 10.0,
            magnetic_contrast: 10.0,
            magnetic_frequency: 57.0,
            magnetic_pressure: false,
            red_eye_pupil_size: 50.0,
            red_eye_darken: 50.0,
            eyedropper_size: 1,
            eyedropper_sample: default_eyedropper_sample(),
            eyedropper_ring: true,
        }
    }
}

/// Edit › Transform's mode: what a handle drag does with no modifier keys held.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TransformMode {
    /// Free Transform and Scale: handles scale, drag outside rotates.
    #[default]
    Free,
    /// Edit › Transform › Rotate: handle drags rotate about the reference point (#3941).
    Rotate,
    /// Edge and corner handles skew: each moves along one axis.
    Skew,
    /// Corner handles move freely, one at a time; nothing else moves them (no rotating, no
    /// edges) and nothing snaps.
    Distort,
    /// Corner handles move in pairs, mirrored (one-point perspective).
    Perspective,
}

impl TransformMode {
    /// The mode an `edit.transform.*` / `edit.freeTransform` menu id starts.
    pub fn for_command(id: &str) -> Self {
        match id {
            "edit.transform.rotate" => Self::Rotate,
            "edit.transform.skew" => Self::Skew,
            "edit.transform.distort" => Self::Distort,
            "edit.transform.perspective" => Self::Perspective,
            _ => Self::Free,
        }
    }
}

/// Free Transform in progress: the source frame `rect` and where its corners currently are.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransformSession {
    pub session: u64,
    pub layer: u64,
    pub rect: [f64; 4],
    /// Corners clockwise from top-left (document px).
    pub quad: [[f64; 2]; 4],
    /// Reference point (rotation / ⌥-scale centre).
    pub pivot: [f64; 2],
    pub interpolation: String,
    /// Warp mode (Edit › Transform › Warp): the warp being edited over `rect`, in document px.
    #[serde(default)]
    pub warp: Option<photocraft_geom::warp::Warp>,
    /// Select › Transform Selection: the box transforms the selection outline, not pixels.
    #[serde(default)]
    pub selection: bool,
    /// `edit.transform`'s `"target"` when the box moves an unlinked layer mask, an alpha channel or
    /// the Quick Mask by itself (`None`: the layer, with its linked masks).
    #[serde(default)]
    pub target: Option<serde_json::Value>,
    /// Free Transform Path: `path.transform`'s `name` (and `layer`) when the box moves a
    /// path's anchors and handles instead of pixels.
    #[serde(default)]
    pub path: Option<serde_json::Value>,
    /// The layer was made for this session (⌥⌘T's copy, #352; a file dropped on the canvas), so
    /// Cancel takes it back and OK folds it into one history step with the transform.
    #[serde(default)]
    pub made: Option<MadeLayer>,
    /// Edit › Transform › Rotate / Skew / Distort / Perspective (`Free` for Free Transform and Scale).
    #[serde(default)]
    pub mode: TransformMode,
    /// Edit › Content-Aware Scale (#3096): the box only scales and moves, and OK runs
    /// `edit.contentAwareScale` with these options instead of `edit.transform`.
    #[serde(default)]
    pub content_aware: Option<ContentAwareOptions>,
    /// The options bar's V (vertical skew, degrees) last typed. Any box can be read as scale,
    /// skew and rotation with any vertical skew, so the bar keeps showing the typed one.
    #[serde(default)]
    pub skew_v: f64,
}

/// Content-Aware Scale's options bar: Amount, Protect and Protect Skin Tones.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentAwareOptions {
    /// How much is content-aware, 0..100 % (the rest is a plain resample).
    pub amount: f32,
    /// The alpha channel whose white areas keep their shape (`None`: no protection).
    #[serde(default)]
    pub protect: Option<String>,
    #[serde(default)]
    pub protect_skin_tones: bool,
}

impl Default for ContentAwareOptions {
    fn default() -> Self {
        Self { amount: 100.0, protect: None, protect_skin_tones: false }
    }
}

/// Why a Free Transform session's layer was made for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MadeLayer {
    /// Free Transform on a copy (⌥⌘T): OK makes the copy and the transform one Free Transform step.
    Copy,
    /// A file dropped on the canvas: OK makes the place and the transform one Place Embedded step.
    Place,
}

/// In-progress inline type editing (Type tool). Offsets are character indices.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextEdit {
    pub layer: u64,
    pub caret: usize,
    pub anchor: usize,
    /// History coalescing key: the whole editing session is one "Edit Type Layer" step.
    pub session: String,
    /// The layer was created by this session (its name follows the text; empty on commit = delete).
    pub created: bool,
    /// A Type Mask tool session: the layer is temporary and commits as a selection.
    #[serde(default)]
    pub mask: bool,
    #[serde(skip)]
    pub dragging: bool,
    /// Paragraph-box handle being dragged (0-3 corners from top-left clockwise, 4-7 top/right/bottom/left edges).
    #[serde(skip)]
    pub resize: Option<u8>,
    /// IME composition in progress: (start, length) in characters. The preedit text lives in the
    /// layer so it lays out like typed text; each IME update replaces it.
    #[serde(skip)]
    pub preedit: Option<(usize, usize)>,
}

/// Temporary Type-tool drag. The document is unchanged until release; this frame is view state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TypeTransform {
    pub document: u64,
    pub revision: u64,
    pub original: photocraft_geom::Affine,
    pub frame: TransformSession,
    pub(crate) gesture: crate::transform_tool::Gesture,
}

/// Content-Aware Move with Transform On Drop: the box on the dropped patch. The document is
/// unchanged until it is committed; this is view state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CamTransform {
    pub document: u64,
    pub revision: u64,
    /// How far the selection was dragged; the box's `rect` is the selection's bounds there.
    pub offset: [i32; 2],
    pub frame: TransformSession,
    #[serde(skip)]
    pub(crate) gesture: Option<crate::transform_tool::Gesture>,
}

/// View-menu overlays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Extras {
    pub rulers: bool,
    pub grid: bool,
    pub guides: bool,
    pub snap: bool,
    pub lock_guides: bool,
}

impl Default for Extras {
    fn default() -> Self {
        Self { rulers: false, grid: false, guides: true, snap: true, lock_guides: false }
    }
}

/// Selected tab per dock card.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DockTabs {
    pub properties: usize,
    pub color: usize,
    pub layers: usize,
    /// Navigator | Histogram | Info.
    pub navigator: usize,
    /// History | Actions.
    pub history: usize,
    /// Character | Paragraph.
    pub character: usize,
}

/// Color panel state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorPanelState {
    /// The panel edits the background colour (its chip was clicked), not the foreground.
    pub background: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UiState {
    pub tool: Tool,
    /// The tools used so far, most recent first and each one once. The first tool of a shortcut
    /// group (the tools sharing a key, [`Tool::key`]) is the one its key brings back, and the first
    /// tool of a toolbar slot is the one the slot shows, as in Photoshop (#2608, #3569). One list
    /// answers both because they don't line up: the Marquee slot holds tools with and without a
    /// key, the Hand slot two keys. Kept between launches in `Preferences::group_tools`.
    #[serde(default)]
    pub group_tools: Vec<Tool>,
    /// A tool letter still held (spring-loaded tool, #2672): the tool to give back on release.
    /// Exposed to automation, never restored from settings.
    #[serde(default, skip_deserializing)]
    pub spring: Option<crate::spring_tools::Spring>,
    /// Recently opened file paths, most-recent first (File › Open Recent). Capped; de-duplicated.
    #[serde(default)]
    pub recent_files: craft::ui::contract::Tracked<Vec<String>>,
    /// Recently picked type families, most-recent first (the font menu's Recent group). Capped by
    /// Preferences ▸ Type ▸ Recent Fonts; de-duplicated.
    #[serde(default)]
    pub recent_fonts: Vec<String>,
    /// Inline type editing session, if any.
    #[serde(default)]
    pub text_edit: Option<TextEdit>,
    /// Whether the text being edited overflows its paragraph box (the box's lower-right handle
    /// shows the overflow "+").
    /// Read by automation while a session edits that box; never written to settings.
    #[serde(skip)]
    pub type_box_overflow: bool,
    /// Font for subsequent typing at this caret, separate from existing character runs.
    #[serde(default)]
    pub type_insertion_style: Option<TypeInsertionStyle>,
    /// Tool to restore when an inline type edit session commits or cancels (e.g. Move tool, #1132).
    #[serde(skip)]
    pub text_edit_return_tool: Option<Tool>,
    /// Ctrl/Cmd Type-tool gesture; never saved or restored.
    #[serde(skip)]
    pub type_transform: Option<TypeTransform>,
    /// Reference point for the current type editing session (document coordinates).
    #[serde(skip)]
    pub type_transform_pivot: Option<[f64; 2]>,
    /// Content-Aware Move's Transform On Drop box, waiting for ↩ / Esc.
    #[serde(default, skip_deserializing)]
    pub cam_transform: Option<CamTransform>,
    /// Free Transform session, if any. Written as its record (`FreeTransform::record`) for
    /// automation; never restored.
    #[serde(default, skip_deserializing)]
    pub transform: Option<FreeTransform>,
    /// Temporary symmetry-axis editing, exposed to automation but never restored from settings.
    #[serde(default, skip_deserializing)]
    pub symmetry_transform: Option<crate::symmetry_ui::Transform>,
    /// The Properties panel's armed Levels/Curves eyedropper on an adjustment layer (`layer_picker`),
    /// exposed to automation but never restored from settings.
    #[serde(default, skip_deserializing)]
    pub layer_picker: Option<crate::layer_picker::LayerPicker>,
    /// Clone Stamp / Healing source point (⌥-click) and the aligned offset once a stroke started.
    #[serde(default)]
    pub clone_source: Option<[f64; 2]>,
    #[serde(default)]
    pub clone_offset: Option<[f64; 2]>,
    /// Pattern Stamp aligned origin in document pixels, kept across strokes.
    #[serde(default)]
    pub pattern_stamp_phase: Option<[f32; 2]>,
    /// Painting targets the active layer's mask instead of its pixels.
    #[serde(default)]
    pub mask_target: bool,
    /// The active layer's vector mask is targeted (its Layers thumbnail is bracketed): the path
    /// tools edit it (#196). Never set together with `mask_target`.
    #[serde(default)]
    pub vector_mask_target: bool,
    /// The smart object (layer id) whose filter mask is targeted (its thumbnail on the Smart
    /// Filters row clicked): painting, fills, filters and adjustments edit that mask
    /// (FILE-215-3). Never set together with `mask_target` or `vector_mask_target`.
    #[serde(default)]
    pub filter_mask_target: Option<u64>,
    /// The adjustment or fill layer (id) whose own settings Properties shows although its mask
    /// is targeted: set when that layer is created, as in Photoshop (#3369). Targeting the mask
    /// thumbnail, the layer's pixels or another layer clears it.
    #[serde(default)]
    pub properties_layer: Option<u64>,
    /// Brush Preset picker opened by a right-click on the canvas or the options-bar brush chip:
    /// its screen position (points).
    #[serde(default)]
    pub brush_picker: Option<[f32; 2]>,
    /// The Brush Preset picker's list: search, collapsed groups, view, size and the parts shown.
    #[serde(skip)]
    pub(crate) brush_picker_state: Tracked<BrushPickerState<BrushKey>>,
    /// Photoshop's HUD Color Picker (#6098), open while Shift+Alt+right-click is held on the
    /// canvas with a painting tool (`crate::color_hud`).
    #[serde(default)]
    pub color_hud: Option<crate::color_hud::Hud>,
    /// Selection-tool context menu opened by a plain canvas right-click.
    #[serde(default)]
    pub canvas_tool_menu: Option<crate::canvas_tool_menu::CanvasToolMenu>,
    /// The guide a Move-tool click selected (`guide_select`): drawn blue, Delete removes it.
    #[serde(default)]
    pub selected_guide: Option<crate::guide_select::SelectedGuide>,
    /// The brush is a per-tool option (the Brush, the Eraser and every retouching tool each keep
    /// their own size, hardness, tip and dynamics, as in Photoshop): the tool whose brush the
    /// session brush holds, and the other tools' saved brushes.
    #[serde(default = "default_brush_tool")]
    pub brush_tool: Tool,
    #[serde(default)]
    pub tool_brushes: Vec<(Tool, photocraft_engine::paint::BrushSettings)>,
    /// Pen path under construction. Not saved: its `EditSubpath` has no serde.
    #[serde(skip)]
    pub pen: Option<crate::vector_ui::PenPath>,
    /// The document the pen path is drawn on; switching away commits it to that Work Path.
    #[serde(default)]
    pub pen_doc: Option<photocraft_doc::DocId>,
    /// The shape options' Stroke Options popover is open.
    #[serde(default)]
    pub shape_stroke_open: bool,
    /// The folders open in the shape Fill / Stroke swatch lists; kept for the session.
    #[serde(skip)]
    pub paint_folders: PresetState<SwatchKey>,
    /// The shape Fill / Stroke gradient lists' view and open groups, by their place in this
    /// session's lists; kept for the session.
    #[serde(skip)]
    pub shape_gradient_state: PresetState<GradientRef>,
    /// The arrow line (document id, layer id) whose arrowheads are the Line tool's default
    /// (Properties › Set as default): later edits to it are copied to the tool defaults too.
    #[serde(skip)]
    pub arrow_default: Option<(u64, u64)>,
    /// The path alignment popover's Align To choice (an index into its list).
    #[serde(skip)]
    pub path_align_to: usize,
    /// Direct Selection tool: selected anchors and the drag in progress (#790).
    #[serde(default)]
    pub direct_selection: crate::direct_select::DirectSelection,
    #[serde(skip)]
    pub type_path_geometry_cache: crate::type_path_placement::GeometryCache,
    /// Selected row in the Paths panel ("work" or a saved path name).
    #[serde(default)]
    pub selected_path: Option<String>,
    /// The document and Work Path deselected by making a selection from it with the Pen or Path
    /// Selection tool (#6079): the canvas hides that path until a path is selected again or the
    /// Work Path changes.
    #[serde(default)]
    pub hidden_work_path: Option<(u64, photocraft_doc::Path)>,
    /// The Layers panel filter: what it filters by (kind, name, effect, mode, attribute, colour,
    /// smart object) and each type's value (#5048, #5898, #10880, #10881).
    #[serde(default)]
    pub layer_filter: LayerFilter,
    /// The Layers panel options: thumbnail size and contents, the filter row and the other
    /// Panel Options. Kept with the preferences, not here.
    #[serde(skip)]
    pub layers_settings: LayersSettings,
    /// The Actions panel state, of which only Button Mode and the collapsed sets are saved. The
    /// list lives on the session.
    #[serde(default)]
    pub actions: crate::actions::ActionsUi,
    /// Layer Comps panel: the selected comp (by comp id).
    #[serde(default)]
    pub layer_comp_selected: Option<u32>,
    /// Preset panels (Gradients, Patterns, Styles, Shapes, Tool Presets, Clone Source).
    #[serde(default)]
    pub presets_ui: crate::preset_panels::PresetUi,
    /// Character/Paragraph Styles, Glyphs and Check Spelling (see `type_panels_ui`).
    #[serde(default)]
    pub type_panels: crate::type_panels_ui::TypePanelsUi,
    /// The font menus' filter bar (classes, favourites, variable, similar); kept for the session.
    #[serde(skip)]
    pub(crate) font_filters: craft::font_manager::picker::FontFilters,
    /// Ruler/Count/Note tools, Measurement Log and Notes panels (see `analysis`).
    #[serde(default)]
    pub analysis: crate::analysis::AnalysisUi,
    pub timeline: crate::timeline_ui::TimelineUi,
    /// Slice and Slice Select tools (see `slice_ui`).
    #[serde(default)]
    pub slices: crate::slice_ui::SliceUi,
    /// The open Save for Web dialog's colour table shifts (see `file_ui::web`).
    #[serde(skip)]
    pub web_table: crate::dialogs::file_ui::WebTable,
    /// Frame tool (see `frame_tool`).
    #[serde(default)]
    pub frame_tool: crate::frame_tool::FrameToolUi,
    /// The contextual task bar as drawn in the last frame: its screen rect `[x0, y0, x1, y1]` in
    /// points, `None` while no bar shows (`ui.inspect` › `taskBar`).
    #[serde(skip)]
    pub task_bar_shown: Option<[f32; 4]>,
    /// Artboard tool (see `artboard_tool`).
    pub artboard_tool: crate::artboard_tool::ArtboardToolUi,
    /// Modifier Keys panel, custom pixel aspect ratios, workspace dialogs (see `workspace_ui`).
    #[serde(default)]
    pub shell: crate::workspace_ui::ShellUi,
    /// View extras: rulers (⌘R), grid (⌘'), guides (⌘;), snapping (⇧⌘;), locked guides (⌥⌘;).
    #[serde(default)]
    pub extras: Extras,
    /// View / Window / Type preferences: screen mode, Extras, Show and Snap To, flip, arrangement.
    #[serde(default)]
    pub view: crate::view_cmds::ViewOptions,
    pub panels: Panels,
    /// Views per open document (index-aligned with the session's documents).
    pub views: Vec<View>,
    pub windows: Vec<DocWindow>,
    /// The document groups: the docked ones the document area tiles and the floating windows,
    /// each with its tabs and the document it shows.
    #[serde(default)]
    pub document_windows: DocumentWindows<DocId>,
    /// The divider positions of the document area's tiled groups.
    #[serde(default)]
    pub split_at: SplitAt,
    /// A New Window's own view, keyed by its group id; a group without one shows its document's view.
    #[serde(default)]
    pub window_views: HashMap<u64, View>,
    /// The theme on screen, taken from the preferences and the suite each frame.
    #[serde(skip)]
    pub theme: crate::theme::AppTheme,
    /// The command palette: whether it is open and its recent picks.
    #[serde(default)]
    pub palette: crate::palette::PaletteState,
    /// The panel layout: every panel's place, the docks, the floating panels and where each one
    /// docks back to.
    /// One `Arc` across frames: the dock keys what it derives on the workspace's revision.
    #[serde(default = "default_docking")]
    pub docking: std::sync::Arc<craft::behavior::dock::Workspace<String>>,
    /// The panel open in the flyout of a column collapsed to icons (`Layout::opens_in_flyout`); the dock area shows it.
    /// Not saved: a flyout is closed at launch.
    #[serde(skip)]
    pub flyout: Option<String>,
    /// Which chip the Color panel edits, and how it shows the colour.
    #[serde(default)]
    pub color_panel: ColorPanelState,
    /// The Color panel options as last synced with the preferences: a stored value that differs
    /// from it was changed outside the panel (`prefs.set`, a reset) and is taken, not overwritten.
    #[serde(skip)]
    pub color_panel_synced: Option<serde_json::Value>,
    /// The Swatches panel's selection, open folders and view; kept for the session.
    #[serde(skip)]
    pub swatches: SwatchesState,
    /// Histogram panel options (view, channel, statistics, colours, source) and the tonal range
    /// dragged across its graph. The options are kept with the preferences
    /// (`histogram_panel::persist_options`), not here.
    #[serde(skip)]
    pub histogram: ScopePanelState,
    /// The History panel's selected snapshot: its document, the snapshot id and where the history
    /// stood when it was clicked (`history::selected_snapshot`).
    #[serde(skip)]
    pub history_snapshot: Option<(DocId, u64, crate::panels::history::HistorySpot)>,
    /// The History row to scroll into view on the next pass (`history::reveal`).
    #[serde(skip)]
    pub history_reveal: Option<craft::ui::patterns::panels::HistoryKey>,
    /// The toolbar's Edit Toolbar drawer is open.
    #[serde(skip)]
    pub toolbar_drawer: bool,
    /// The Properties panel's Transform link: W and H change together. On at start; not saved.
    #[serde(skip, default = "yes")]
    pub properties_linked: bool,
    /// Brush Settings: selected section (0 = Brush Tip Shape).
    #[serde(default)]
    pub brush_section: usize,
    /// The Brush Settings dynamic controls whose curve editor is open.
    #[serde(skip)]
    pub brush_curves_open: BTreeSet<&'static str>,
    /// Brushes panel: list or grid, zoom, collapsed groups and the search filter. Starts as a list.
    #[serde(default = "brushes_list")]
    pub(crate) brushes_panel: Tracked<PresetState<BrushKey>>,
    /// Marquee options-bar mode: 0 new, 1 add, 2 subtract, 3 intersect (modifier keys override).
    #[serde(default)]
    pub selection_mode: u8,
    #[serde(default)]
    pub tool_options: ToolOptions,
    /// The shape Fill / Stroke swatches' recently used gradients, newest first (the Gradient
    /// tab's Recent row).
    #[serde(default)]
    pub recent_gradients: Vec<photocraft_doc::Fill>,
    pub next_id: u64,
    /// Last status message (errors from commands, hints).
    pub status: String,
    /// The status message is an error (shown in the warning colour).
    #[serde(default)]
    pub status_error: bool,
    /// Notices (import/export warnings, files that couldn't open) posted since the last pass and
    /// not yet in the window's toast queue, newest last.
    #[serde(default)]
    pub notices: Vec<crate::notices::Notice>,
    /// Non-modal release-check window; readable and controllable through `ui.inspect`/`ui.set`.
    #[serde(default)]
    pub updates_open: bool,
    /// Pending GPU fallback warning, visible to automation.
    #[serde(default)]
    pub gpu_fallback_notice: Option<String>,
    /// Documents (ids) whose slow full refresh on the CPU compositor has had its notice.
    #[serde(default)]
    pub slow_refresh_noticed: Vec<u64>,
    /// Status bar info field, Home screen (see `chrome_ui`).
    #[serde(default)]
    pub chrome: crate::chrome_ui::ChromeState,
}

/// The panel layout at start and for a saved state without one.
fn default_docking() -> std::sync::Arc<craft::behavior::dock::Workspace<String>> {
    std::sync::Arc::new(crate::panels::registry::default_workspace())
}

/// The Brushes panel's state at start and for a saved state without one: the list view.
fn brushes_list() -> Tracked<PresetState<BrushKey>> {
    let mut state = PresetState::new();
    state.view = PresetView::List;
    Tracked::new(state)
}

impl Default for UiState {
    /// Creates the default UI state with default tools and empty drag state.
    fn default() -> Self {
        Self {
            tool: Tool::Move,
            group_tools: Vec::new(),
            spring: None,
            recent_files: Default::default(),
            recent_fonts: Vec::new(),
            text_edit: None,
            type_box_overflow: false,
            type_insertion_style: None,
            text_edit_return_tool: None,
            type_transform: None,
            type_transform_pivot: None,
            cam_transform: None,
            transform: None,
            symmetry_transform: None,
            layer_picker: None,
            mask_target: false,
            vector_mask_target: false,
            filter_mask_target: None,
            properties_layer: None,
            brush_picker: None,
            brush_picker_state: Tracked::default(),
            color_hud: None,
            canvas_tool_menu: None,
            selected_guide: None,
            brush_tool: Tool::Brush,
            tool_brushes: Vec::new(),
            clone_source: None,
            clone_offset: None,
            pattern_stamp_phase: None,
            extras: Extras::default(),
            view: Default::default(),
            actions: Default::default(),
            layer_comp_selected: None,
            presets_ui: Default::default(),
            type_panels: Default::default(),
            font_filters: Default::default(),
            analysis: Default::default(),
            timeline: Default::default(),
            slices: Default::default(),
            web_table: Default::default(),
            frame_tool: Default::default(),
            task_bar_shown: None,
            artboard_tool: Default::default(),
            shell: Default::default(),
            layer_filter: LayerFilter::default(),
            layers_settings: LayersSettings::default(),
            pen: None,
            pen_doc: None,
            shape_stroke_open: false,
            paint_folders: PresetState::new(),
            shape_gradient_state: PresetState::new(),
            arrow_default: None,
            path_align_to: 0,
            direct_selection: Default::default(),
            type_path_geometry_cache: Default::default(),
            selected_path: None,
            hidden_work_path: None,
            panels: Panels::default(),
            views: Vec::new(),
            windows: Vec::new(),
            document_windows: DocumentWindows::default(),
            split_at: SplitAt::default(),
            window_views: HashMap::new(),
            theme: crate::theme::AppTheme::default(),
            palette: Default::default(),
            docking: default_docking(),
            flyout: None,
            color_panel: Default::default(),
            color_panel_synced: None,
            swatches: SwatchesState::default(),
            histogram: ScopePanelState::default(),
            history_snapshot: None,
            history_reveal: None,
            toolbar_drawer: false,
            properties_linked: true,
            brush_section: 0,
            brush_curves_open: BTreeSet::new(),
            brushes_panel: brushes_list(),
            selection_mode: 0,
            tool_options: ToolOptions::default(),
            recent_gradients: Vec::new(),
            next_id: 1,
            status: String::new(),
            status_error: false,
            notices: Vec::new(),
            updates_open: false,
            gpu_fallback_notice: None,
            slow_refresh_noticed: Vec::new(),
            chrome: Default::default(),
        }
    }
}

impl UiState {
    /// Records `tool` as the last one used in its shortcut group and toolbar slot.
    pub fn remember_group_tool(&mut self, tool: Tool) {
        if self.group_tools.first() != Some(&tool) {
            self.group_tools.retain(|t| *t != tool);
            self.group_tools.insert(0, tool);
        }
    }

    /// The last tool used in the shortcut group of `key`, if any.
    #[cfg(test)]
    pub fn group_tool(&self, key: char) -> Option<Tool> {
        self.group_tools.iter().copied().find(|t| t.key() == key)
    }

    pub fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_all_layers_is_per_tool_and_old_prefs_load() {
        // The single shared flag of older preferences is dropped: every tool starts off (#5524).
        let o: ToolOptions = serde_json::from_str(r#"{"sample_all_layers": true, "feather": 3.0}"#).unwrap();
        assert_eq!((o.sample_all, o.feather), (SampleAllLayers::default(), 3.0));
        let mut o = ToolOptions::default();
        o.sample_all.magic_wand = true;
        let back: ToolOptions = serde_json::from_value(serde_json::to_value(&o).unwrap()).unwrap();
        assert!(back.sample_all.magic_wand && !back.sample_all.get(Tool::Blur));
        assert!(!back.sample_all.get(Tool::Brush), "a tool without the option reads false");
        let mut s = SampleAllLayers::default();
        assert!(s.slot(Tool::Brush).is_none());
        for t in [Tool::MagicWand, Tool::PaintBucket, Tool::Blur, Tool::Sharpen, Tool::Smudge, Tool::Remove] {
            *s.slot(t).unwrap() = true;
            assert!(s.get(t), "{t:?}");
        }
        assert!(!s.get(Tool::QuickSelection));
    }

    #[test]
    fn tool_names_parse() {
        basic_and_retouch_tool_names_parse();
        vector_tool_names_parse();
    }

    fn basic_and_retouch_tool_names_parse() {
        assert_eq!(Tool::from_name("brush"), Some(Tool::Brush));
        assert_eq!(Tool::from_name("Rect"), Some(Tool::RectMarquee));
        assert_eq!(Tool::from_name("RectMarquee"), Some(Tool::RectMarquee));
        assert_eq!(Tool::from_name("Eraser Tool"), Some(Tool::Eraser));
        assert_eq!(Tool::from_name("mixerBrush"), Some(Tool::MixerBrush));
        assert_eq!(Tool::from_name("Mixer Brush Tool"), Some(Tool::MixerBrush));
        assert_eq!(Tool::from_name("colorReplacement"), Some(Tool::ColorReplacement));
        assert_eq!(Tool::from_name("Color Replacement Tool"), Some(Tool::ColorReplacement));
        assert_eq!(Tool::from_name("redEye"), Some(Tool::RedEye));
        assert_eq!(Tool::from_name("Red Eye Tool"), Some(Tool::RedEye));
        assert_eq!(Tool::from_name("artHistoryBrush"), Some(Tool::ArtHistoryBrush));
        assert_eq!(Tool::from_name("Art History Brush"), Some(Tool::ArtHistoryBrush));
        assert_eq!(Tool::from_name("Art History Brush Tool"), Some(Tool::ArtHistoryBrush));
        assert_eq!(Tool::RedEye.key(), 'J');
        assert!(!Tool::RedEye.is_brushlike());
        assert_eq!(Tool::from_name("patternStamp"), Some(Tool::PatternStamp));
        assert_eq!(Tool::from_name("Pattern Stamp Tool"), Some(Tool::PatternStamp));
        assert_eq!(Tool::ALL.len(), 70);
    }

    fn vector_tool_names_parse() {
        assert_eq!(Tool::from_name("Vertical Type Mask Tool"), Some(Tool::VerticalTypeMask));
        assert_eq!(Tool::from_name("addAnchorPoint"), Some(Tool::AddAnchorPoint));
        assert_eq!(Tool::from_name("Delete Anchor Point Tool"), Some(Tool::DeleteAnchorPoint));
        assert_eq!(Tool::DeleteAnchorPoint.key(), ' ');
        assert_eq!(Tool::from_name("convertPoint"), Some(Tool::ConvertPoint));
        assert_eq!(Tool::from_name("Convert Point Tool"), Some(Tool::ConvertPoint));
        assert_eq!(Tool::ConvertPoint.key(), ' ', "P never reaches it");
        assert!(!Tool::ConvertPoint.is_brushlike());
        assert_eq!(Tool::from_name("Remove Tool"), Some(Tool::Remove));
        assert_eq!(Tool::Remove.key(), 'J');
        assert!(Tool::Remove.is_brushlike());
        assert_eq!(Tool::from_name("RotateView"), Some(Tool::RotateView));
        assert_eq!(Tool::from_name("Rotate View Tool"), Some(Tool::RotateView));
        assert_eq!(Tool::RotateView.key(), 'R');
        assert_eq!(Tool::ArtHistoryBrush.key(), 'Y');
        assert!(Tool::ArtHistoryBrush.is_brushlike());
        assert_eq!(Tool::from_name("nope"), None);
    }

    /// The README's "Everything in the box" tool heading and list name exactly the tools a user can
    /// pick, `Tool::ALL` (#996).
    #[test]
    fn readme_tool_list_matches_tool_all() {
        let readme = include_str!("../../../README.md");
        let (_, rest) = readme.split_once("<h4>🧰 ").unwrap();
        let (count, rest) = rest.split_once(" tools</h4>").unwrap();
        assert_eq!(count.parse::<usize>().unwrap(), Tool::ALL.len(), "README.md tool heading");
        let list = rest.trim_start().lines().next().unwrap();
        // "Rectangular and Elliptical Marquee" names two tools: "Rectangular Marquee", "Elliptical Marquee".
        let mut named: Vec<String> = Vec::new();
        for entry in list.split(" · ") {
            match entry.split_once(" and ").and_then(|(first, rest)| Some((first, rest.split_once(' ')?))) {
                Some((first, (second, noun))) => named.extend([format!("{first} {noun}"), format!("{second} {noun}")]),
                None => named.push(entry.to_owned()),
            }
        }
        named.sort();
        let mut labels: Vec<String> = Tool::ALL.iter().map(|tool| tool.label().trim_end_matches(" Tool").to_owned()).collect();
        labels.sort();
        assert_eq!(named, labels, "README.md tool list");
    }

    #[test]
    fn ui_state_serializes() {
        let s = UiState::default();
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["tool"], "Move");
        let back: UiState = serde_json::from_value(v).unwrap();
        assert_eq!(back, s);
    }
}
