//! Tool Options of the freehand tools (double-click Pencil, Paintbrush, Smooth, Blob Brush or
//! Eraser): the options each tool keeps, which last across tool switches and are saved with the
//! preferences. OK sets them with `tool.setOption {tool, values}`. The Pencil also has `liveFit`
//! (Live curve fitting, on: its preview fits the stroke while it is drawn).
//!
//! Fields: `tool`; `fidelity` (points; Pencil, Paintbrush and Smooth), `fill` (Pencil and
//! Paintbrush), `keepSelected` (Pencil, Paintbrush and Blob Brush), `mergeOnlySelection` (Blob
//! Brush), `closeWithin` and `editWithin` (screen pixels, 0 turns it off; Pencil and Paintbrush),
//! `angle` (degrees) and `roundness` (percent; Eraser), `livePreview` (Pencil: the stroke shows
//! with its fill and stroke while it is drawn, or as a thin line until the release) and `size`
//! (points; Blob Brush and Eraser).

use serde_json::{Map, Value, json};

use super::document_setup::check;
use super::{DialogSpec, form, run_and_close};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of the freehand tools' Tool Options.
pub const KIND: &str = "freehandOptions";

pub(super) const SPEC: DialogSpec = DialogSpec { heading, body, confirm, min_width: CONTENT_W, ..DialogSpec::FORM };

/// The tools the dialog serves.
pub const TOOLS: [&str; 5] = ["pencil", "paintbrush", "smooth", "blobBrush", "eraser"];

/// The options the dialog edits, in the order it shows them (a tool has those it reports).
const KEYS: [&str; 12] = [
    "fidelity",
    "angle",
    "roundness",
    "size",
    "fill",
    "altSmooth",
    "keepSelected",
    "mergeOnlySelection",
    "closeWithin",
    "editWithin",
    "livePreview",
    "liveFit",
];

/// Width of the dialog's rows: every number field ends at its right edge.
const CONTENT_W: f32 = 330.0;

/// Width of the number fields, Fidelity's among them.
const FIELD_W: f32 = 72.0;

/// Width of the label column of a row with a number field: what the field leaves of the row. The
/// Fidelity slider takes the same width under its label.
fn label_w(ui: &egui::Ui) -> f32 {
    CONTENT_W - FIELD_W - ui.spacing().item_spacing.x
}

/// The top of the Fidelity slider (points); it starts at the lowest Fidelity the tool takes, and
/// the field takes up to 100.
const FIDELITY_MAX: f64 = 20.0;

/// Fidelity `v` stepped `n` steps by the slider's and the field's keys: an arrow steps hundredths up to 0.1 pt,
/// tenths up to 1 pt and whole points above (a step down from 0.1 or 1 takes the finer step below
/// it); ten steps at once (Shift with an arrow, Page Up/Down) multiply or divide by ten, so they go
/// 0.01 → 0.1 → 1 → 10.
fn fidelity_step(v: f64, n: f64) -> f64 {
    if n.abs() >= 10.0 {
        let times = 10f64.powf((n / 10.0).round());
        return ((v * times) * 100.0).round() / 100.0;
    }
    // The step at `v` going up, or going down from it.
    let unit = |v: f64, up: bool| {
        let at = |edge: f64| if up { v < edge - 1e-9 } else { v <= edge + 1e-9 };
        if at(0.1) {
            0.01
        } else if at(1.0) {
            0.1
        } else {
            1.0
        }
    };
    let up = n > 0.0;
    let mut v = v;
    for _ in 0..(n.abs().min(1000.0) as usize) {
        let u = unit(v, up);
        let q = v / u;
        let k = if up { (q + 1e-6).floor() + 1.0 } else { (q - 1e-6).ceil() - 1.0 };
        v = ((k * u) * 100.0).round() / 100.0;
    }
    v
}

/// Where Fidelity `v` sits on a slider over `(min, max)`, from 0 to 1. The rail is logarithmic, so
/// the low values, where the stroke is followed closely, get as much of it as the high ones
/// (0.01–1 pt takes most of a 0.01–20 rail).
fn fidelity_at(v: f64, (min, max): (f64, f64)) -> f32 {
    if !(v.is_finite() && min > 0.0 && max > min) {
        return 0.0;
    }
    ((v.clamp(min, max) / min).ln() / (max / min).ln()) as f32
}

/// The Fidelity at `x` (0 to 1) on a slider over `(min, max)`: hundredths below 1 pt, tenths above.
fn fidelity_from(x: f32, (min, max): (f64, f64)) -> f64 {
    let v = min * (max / min).powf(f64::from(x.clamp(0.0, 1.0)));
    let step = if v < 1.0 { 100.0 } else { 10.0 };
    ((v * step).round() / step).clamp(min, max)
}

fn heading(d: &Dialog) -> String {
    let label = vectorcraft_tools::tool_info(&d.str("tool")).map_or("", |t| t.label);
    crate::i18n::fmt(tl!("{tool} Options"), &[("tool", tl!(label))])
}

/// Open the options of freehand tool `tool` with their current values.
pub fn open(app: &mut VectorcraftApp, tool: &str) -> Result<Value, String> {
    if !TOOLS.contains(&tool) {
        return Err(format!("`{tool}` isn't a freehand tool"));
    }
    let o = app.session.tool_options_of(tool);
    let mut fields: Map<String, Value> = KEYS.iter().filter_map(|key| Some((key.to_string(), o.get(*key)?.clone()))).collect();
    fields.insert("tool".into(), json!(tool));
    app.ui.dialog = Some(Dialog { kind: KIND.into(), fields });
    Ok(json!({ "dialog": KIND }))
}

/// A row with a number field for `d.fields[key]` (`suffix` is its unit), kept within `range`.
fn number(ui: &mut egui::Ui, d: &mut Dialog, (key, label): (&str, &str), suffix: &str, decimals: usize, range: (f64, f64)) {
    let label_w = label_w(ui);
    let v = d.f64(key, 0.0);
    widgets::label_row(ui, label, label_w, |ui| {
        if let Some(x) = widgets::range_field(ui, ("freehand-field", key), v, range.0..=range.1, suffix, decimals, FIELD_W) {
            d.fields.insert(key.into(), json!(x));
        }
    });
}

/// The Fidelity rows: the label on a line of its own, then a logarithmic slider from the tool's
/// lowest Fidelity to [`FIDELITY_MAX`] across the row with the field at its end, both to two
/// decimals, and Accurate and Smooth under the slider's ends.
fn fidelity(ui: &mut egui::Ui, d: &mut Dialog) {
    let t = Tokens::get(ui.ctx());
    let v = d.f64("fidelity", 1.5);
    let rail = (vectorcraft_tools::draw2::min_fidelity(&d.str("tool")), FIDELITY_MAX);
    let rail_w = label_w(ui);
    let mut new = None;
    ui.label(egui::RichText::new(tl!("Fidelity:")).size(12.5).color(t.text));
    ui.horizontal(|ui| {
        let shown = widgets::SliderValue::field(v, rail.0..=rail.1, "pt", 2).with_step(fidelity_step);
        let (x, _, keyed) =
            widgets::color_slider_stepped(ui, "freehand-fidelity", tl!("Fidelity:"), Some(shown), fidelity_at(v, rail), rail_w, &|_| t.input_border);
        new = x.map(|x| fidelity_from(x, rail)).or(keyed);
        if let Some(x) = widgets::range_field_stepped(ui, "freehand-fidelity-field", v, rail.0..=100.0, "pt", 2, FIELD_W, fidelity_step) {
            new = Some(x);
        }
    });
    if let Some(n) = new.filter(|n| *n != v) {
        d.fields.insert("fidelity".into(), json!(n));
    }
    form::slider_ends_below(ui, rail_w, (tl!("Accurate"), tl!("Smooth")));
    ui.add_space(4.0);
    // Wrapped to the rows' width, so the sentence doesn't widen the dialog.
    ui.allocate_ui(egui::vec2(CONTENT_W, 0.0), |ui| {
        ui.add(
            egui::Label::new(
                egui::RichText::new(tl!("A lower value follows the stroke closely; a higher one gives fewer points and smoother curves."))
                    .size(11.5)
                    .color(t.text_dim),
            )
            .wrap(),
        );
    });
}

fn body(_app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    if d.fields.contains_key("fidelity") {
        fidelity(ui, d);
        ui.add_space(10.0);
    }
    if d.fields.contains_key("angle") {
        number(ui, d, ("angle", tl!("Angle:")), "°", 0, (-180.0, 180.0));
    }
    if d.fields.contains_key("roundness") {
        number(ui, d, ("roundness", tl!("Roundness:")), "%", 0, (1.0, 100.0));
    }
    if d.fields.contains_key("size") {
        number(ui, d, ("size", tl!("Size:")), "pt", 1, (0.1, 1000.0));
    }
    if d.fields.contains_key("fill") {
        check(ui, d, "fill", tl!("Fill new strokes"));
    }
    if d.fields.contains_key("altSmooth") {
        check(ui, d, "altSmooth", tl!("Alt key toggles to Smooth Tool"));
    }
    if d.fields.contains_key("keepSelected") {
        check(ui, d, "keepSelected", tl!("Keep selected"));
        if d.fields.contains_key("mergeOnlySelection") {
            check(ui, d, "mergeOnlySelection", tl!("Merge only with selection"));
        }
        ui.add_space(6.0);
    }
    if d.fields.contains_key("closeWithin") {
        number(ui, d, ("closeWithin", tl!("Close paths when ends are within:")), "px", 0, (0.0, 100.0));
    }
    if d.fields.contains_key("editWithin") {
        number(ui, d, ("editWithin", tl!("Edit selected paths within:")), "px", 0, (0.0, 100.0));
    }
    if d.fields.contains_key("livePreview") {
        ui.add_space(6.0);
        check(ui, d, "livePreview", tl!("Live preview"));
    }
    if d.fields.contains_key("liveFit") {
        if !d.fields.contains_key("livePreview") {
            ui.add_space(6.0);
        }
        check(ui, d, "liveFit", tl!("Live curve fitting"));
    }
    false
}

/// OK: set the tool's options.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let values: Map<String, Value> = KEYS.iter().filter_map(|k| Some((k.to_string(), d.fields.get(*k)?.clone()))).collect();
    run_and_close(app, "tool.setOption", json!({ "tool": d.str("tool"), "values": values }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    fn app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app
    }

    /// The labels the open dialog paints.
    fn painted(app: &mut VectorcraftApp) -> String {
        crate::tests_labels::painted_text(app, |app, ui| super::super::show(app, ui.ctx()))
    }

    #[test]
    fn the_fidelity_rail_is_logarithmic_and_round_trips_two_decimals() {
        let rail = (0.01, FIDELITY_MAX);
        assert_eq!((fidelity_at(0.01, rail), fidelity_at(20.0, rail)), (0.0, 1.0));
        assert_eq!((fidelity_from(0.0, rail), fidelity_from(1.0, rail)), (0.01, 20.0));
        // 0.01–1 pt gets more than half of the rail.
        assert!(fidelity_at(1.0, rail) > 0.5, "{}", fidelity_at(1.0, rail));
        // The values a user picks come back as they were, and the rail only ever rises.
        for v in [0.01, 0.02, 0.05, 0.1, 0.25, 0.5, 1.5, 6.5, 12.0, 20.0] {
            assert_eq!(fidelity_from(fidelity_at(v, rail), rail), v, "{v}");
        }
        let steps: Vec<f64> = (0..=200).map(|i| fidelity_from(i as f32 / 200.0, rail)).collect();
        assert!(steps.windows(2).all(|w| w[0] <= w[1]), "{steps:?}");
        // Past the ends and on bad input it stays on the rail.
        assert_eq!((fidelity_at(0.0, rail), fidelity_at(50.0, rail), fidelity_at(f64::NAN, rail)), (0.0, 1.0, 0.0));
        assert_eq!((fidelity_from(-1.0, rail), fidelity_from(2.0, rail)), (0.01, 20.0));
    }

    #[test]
    fn the_fidelity_keys_step_finely_at_the_low_end() {
        let up: Vec<f64> = std::iter::successors(Some(0.01), |&v| Some(fidelity_step(v, 1.0))).take(21).collect();
        assert_eq!(up, [0.01, 0.02, 0.03, 0.04, 0.05, 0.06, 0.07, 0.08, 0.09, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 2.0, 3.0]);
        // Down retraces the same steps.
        let down: Vec<f64> = std::iter::successors(Some(3.0), |&v| Some(fidelity_step(v, -1.0))).take(21).collect();
        assert_eq!(down, up.iter().rev().copied().collect::<Vec<_>>());
        // Ten steps at once (Shift) multiply or divide by ten.
        assert_eq!((fidelity_step(0.01, 10.0), fidelity_step(0.1, 10.0), fidelity_step(1.0, 10.0)), (0.1, 1.0, 10.0));
        assert_eq!(
            (fidelity_step(10.0, -10.0), fidelity_step(1.0, -10.0), fidelity_step(0.1, -10.0), fidelity_step(0.03, 10.0)),
            (1.0, 0.1, 0.01, 0.3)
        );
        // A value between steps goes to the next one in its direction.
        assert_eq!(
            (fidelity_step(0.035, 1.0), fidelity_step(0.035, -1.0), fidelity_step(1.5, 1.0), fidelity_step(1.5, -1.0)),
            (0.04, 0.03, 2.0, 1.0)
        );
    }

    #[test]
    fn the_fidelity_field_takes_a_hundredth_for_the_pencil() {
        let mut app = app();
        app.run("tool.options", json!({"tool": "pencil"})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("fidelity".into(), json!(0.02));
        let text = painted(&mut app);
        assert!(text.contains("0.02"), "{text}");
        super::super::confirm(&mut app).unwrap();
        assert_eq!(app.session.tool_options_of("pencil")["fidelity"].as_f64(), Some(0.02));
    }

    #[test]
    fn each_tool_shows_its_options_and_ok_sets_them() {
        let mut app = app();
        // Pencil: Fidelity, fill, the two tolerances; no Size.
        app.run("tool.options", json!({"tool": "pencil"})).unwrap();
        let text = painted(&mut app);
        for label in [
            "Pencil Tool Options",
            "Fidelity:",
            "Accurate",
            "Smooth",
            "Fill new strokes",
            "Alt key toggles to Smooth Tool",
            "Close paths when ends are within:",
            "Edit selected paths within:",
            "Live preview",
        ] {
            assert!(text.contains(label), "{label} in {text}");
        }
        assert!(!text.contains("Size:"), "{text}");
        let d = app.ui.dialog.as_mut().unwrap();
        assert_eq!(
            (d.f64("fidelity", 0.0), d.bool("fill"), d.bool("altSmooth"), d.f64("closeWithin", 0.0), d.f64("editWithin", 0.0)),
            (1.5, false, false, 15.0, 12.0)
        );
        for (k, v) in
            [("fidelity", json!(6.5)), ("fill", json!(true)), ("altSmooth", json!(true)), ("closeWithin", json!(0)), ("editWithin", json!(20))]
        {
            d.fields.insert(k.into(), v);
        }
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let o = app.session.tool_options_of("pencil");
        assert_eq!(
            (o["fidelity"].as_f64(), o["fill"].as_bool(), o["altSmooth"].as_bool(), o["closeWithin"].as_f64(), o["editWithin"].as_f64()),
            (Some(6.5), Some(true), Some(true), Some(0.0), Some(20.0))
        );
        // The Paintbrush keeps its own: the Pencil's change left it alone.
        app.run("tool.options", json!({"tool": "paintbrush"})).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.f64("fidelity", 0.0), d.bool("fill")), (1.5, false));
        let pb_text = painted(&mut app);
        assert!(pb_text.contains("Paintbrush Tool Options"));
        assert!(!pb_text.contains("Alt key toggles to Smooth Tool") && !pb_text.contains("Live preview"), "{pb_text}");
        // Smooth has Fidelity only, the Eraser Size only, and the Blob Brush Size, Keep selected and Merge only with selection.
        app.run("tool.options", json!({"tool": "smooth"})).unwrap();
        let text = painted(&mut app);
        assert!(text.contains("Smooth Tool Options") && text.contains("Fidelity:"), "{text}");
        assert!(
            !text.contains("Fill new strokes")
                && !text.contains("Keep selected")
                && !text.contains("Alt key toggles to Smooth Tool")
                && !text.contains("Size:"),
            "{text}"
        );
        for tool in ["blobBrush", "eraser"] {
            app.run("tool.options", json!({"tool": tool})).unwrap();
            let text = painted(&mut app);
            assert!(text.contains("Size:") && !text.contains("Fidelity:"), "{tool}: {text}");
            // The Blob Brush also has Keep selected and Merge only with selection.
            let blob = tool == "blobBrush";
            assert_eq!((text.contains("Keep selected"), text.contains("Merge only with selection")), (blob, blob), "{tool}: {text}");
            // The Eraser also has Angle and Roundness.
            assert_eq!((text.contains("Angle:"), text.contains("Roundness:")), (!blob, !blob), "{tool}: {text}");
            app.ui.dialog.as_mut().unwrap().fields.insert("size".into(), json!(24));
            super::super::confirm(&mut app).unwrap();
            assert_eq!(app.session.tool_options_of(tool)["size"].as_f64(), Some(24.0), "{tool}");
        }
    }

    /// Live curve fitting (the Pencil's, on by default): on, the path shown while a stroke is
    /// drawn is fitted; off, it joins the stroke's points with straight segments, and the release
    /// fits it as with the option on.
    #[test]
    fn live_curve_fitting_off_shows_the_points_until_the_release_fits_them() {
        let mut app = app();
        // The anchors of the newest path while a wave around height `y0` is drawn and after the
        // release, with nothing selected (a stroke starting at the end of a selected path would
        // continue it).
        let draw = |app: &mut VectorcraftApp, y0: f64| {
            app.run("select.set", json!({"ids": []})).unwrap();
            app.select_tool("pencil");
            let view = app.view_info();
            let pts: Vec<_> = (0..=40).map(|i| (20.0 + i as f64 * 8.0, y0 + 30.0 * (i as f64 * 0.25).sin())).collect();
            let pointer = |kind, (x, y)| vectorcraft_tools::PointerEvent::new(kind, x, y);
            let newest = |app: &VectorcraftApp| {
                let doc = &app.session.doc().unwrap().doc;
                doc.layers[0].children().unwrap().last().unwrap().path_data().unwrap().subpaths[0].anchors.clone()
            };
            crate::canvas::dispatch(app, &pointer(vectorcraft_tools::PointerKind::Down, pts[0]), view);
            for p in &pts[1..] {
                crate::canvas::dispatch(app, &pointer(vectorcraft_tools::PointerKind::Drag, *p), view);
            }
            let drawing = newest(app);
            crate::canvas::dispatch(app, &pointer(vectorcraft_tools::PointerKind::Up, pts[40]), view);
            (drawing, newest(app))
        };
        let corners = |anchors: &[vectorcraft_geom::Anchor]| anchors.iter().all(|a| a.h_in == a.p && a.h_out == a.p);
        // On by default: the Pencil's dialog shows it and the Paintbrush's does not.
        app.run("tool.options", json!({"tool": "pencil"})).unwrap();
        assert!(painted(&mut app).contains("Live curve fitting"));
        assert!(app.ui.dialog.as_ref().unwrap().bool("liveFit"));
        super::super::confirm(&mut app).unwrap();
        let (drawing, on) = draw(&mut app, 100.0);
        assert!(!corners(&drawing) && drawing.len() < 20, "fitted while drawn: {} anchors", drawing.len());
        // Off: the points while the stroke is drawn, fitted at the release.
        app.run("tool.options", json!({"tool": "pencil"})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("liveFit".into(), json!(false));
        super::super::confirm(&mut app).unwrap();
        app.select_tool("selection");
        assert_eq!(app.session.tool_options_of("pencil")["liveFit"].as_bool(), Some(false), "it lasts while another tool is chosen");
        let (drawing, off) = draw(&mut app, 200.0);
        assert!(corners(&drawing) && drawing.len() > 30, "the points while drawn: {} anchors", drawing.len());
        assert_eq!(off.len(), on.len(), "the release fits the stroke as with the option on");
        assert!(off.iter().zip(&on).all(|(a, b)| (a.p.x - b.p.x).abs() < 1e-9 && (a.p.y - b.p.y - 100.0).abs() < 1e-9), "{off:?}");
        app.run("tool.options", json!({"tool": "paintbrush"})).unwrap();
        assert!(!painted(&mut app).contains("Live curve fitting"));
    }

    #[test]
    fn a_new_fidelity_changes_what_the_pencil_draws() {
        let mut app = app();
        // The anchors of a wave drawn around height `y0`, with nothing selected (a stroke starting
        // at the end of a selected path would continue it).
        let anchors = |app: &mut VectorcraftApp, y0: f64| {
            app.run("select.set", json!({"ids": []})).unwrap();
            app.select_tool("pencil");
            let view = app.view_info();
            let pts: Vec<_> = (0..=40).map(|i| (20.0 + i as f64 * 8.0, y0 + 30.0 * (i as f64 * 0.25).sin())).collect();
            let pointer = |kind, (x, y)| vectorcraft_tools::PointerEvent::new(kind, x, y);
            crate::canvas::dispatch(app, &pointer(vectorcraft_tools::PointerKind::Down, pts[0]), view);
            for p in &pts[1..] {
                crate::canvas::dispatch(app, &pointer(vectorcraft_tools::PointerKind::Drag, *p), view);
            }
            crate::canvas::dispatch(app, &pointer(vectorcraft_tools::PointerKind::Up, pts[40]), view);
            let doc = &app.session.doc().unwrap().doc;
            doc.layers[0].children().unwrap().last().unwrap().path_data().map_or(0, |p| p.subpaths.iter().map(|s| s.anchors.len()).sum::<usize>())
        };
        let precise = anchors(&mut app, 100.0);
        app.run("tool.options", json!({"tool": "pencil"})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("fidelity".into(), json!(12));
        super::super::confirm(&mut app).unwrap();
        let smooth = anchors(&mut app, 200.0);
        assert!(smooth < precise, "{smooth} anchors at fidelity 12, {precise} at the default");
    }

    #[test]
    fn the_new_path_is_selected_only_with_keep_selected_on() {
        let mut app = app();
        // A stroke of `tool` along height `y0`; the ids selected afterwards and the new path's id.
        let stroke = |app: &mut VectorcraftApp, tool: &str, y0: f64| {
            app.select_tool(tool);
            let view = app.view_info();
            let pointer = |kind, x| vectorcraft_tools::PointerEvent::new(kind, x, y0);
            crate::canvas::dispatch(app, &pointer(vectorcraft_tools::PointerKind::Down, 20.0), view);
            for x in [60.0, 100.0, 140.0] {
                crate::canvas::dispatch(app, &pointer(vectorcraft_tools::PointerKind::Drag, x), view);
            }
            crate::canvas::dispatch(app, &pointer(vectorcraft_tools::PointerKind::Up, 180.0), view);
            let st = app.session.doc().unwrap();
            (st.selection.objects.clone(), st.doc.layers[0].children().unwrap().last().unwrap().id)
        };
        for tool in ["pencil", "paintbrush"] {
            app.run("select.set", json!({"ids": []})).unwrap();
            // On by default: the new path is selected.
            app.run("tool.options", json!({"tool": tool})).unwrap();
            assert!(painted(&mut app).contains("Keep selected"), "{tool}");
            let d = app.ui.dialog.as_mut().unwrap();
            assert!(d.bool("keepSelected"), "{tool}");
            super::super::confirm(&mut app).unwrap();
            let (selected, id) = stroke(&mut app, tool, 100.0);
            assert_eq!(selected, vec![id], "{tool}");
            // Off: the next stroke leaves the selection as it was.
            app.run("tool.options", json!({"tool": tool})).unwrap();
            app.ui.dialog.as_mut().unwrap().fields.insert("keepSelected".into(), json!(false));
            super::super::confirm(&mut app).unwrap();
            // It lasts while another tool is chosen.
            app.select_tool("selection");
            assert_eq!(app.session.tool_options_of(tool)["keepSelected"].as_bool(), Some(false), "{tool}");
            let (selected, new) = stroke(&mut app, tool, 200.0);
            assert_ne!(new, id, "{tool} drew a new path");
            assert_eq!(selected, vec![id], "{tool}");
        }
    }

    #[test]
    fn the_blob_brush_options_set_what_a_stroke_merges_with_and_selects() {
        let mut app = app();
        // A Blob Brush stroke across the canvas at height 100; the ids selected afterwards.
        let stroke = |app: &mut VectorcraftApp| {
            app.select_tool("blobBrush");
            let view = app.view_info();
            let pointer = |kind, x| vectorcraft_tools::PointerEvent::new(kind, x, 100.0);
            crate::canvas::dispatch(app, &pointer(vectorcraft_tools::PointerKind::Down, 20.0), view);
            for x in [80.0, 140.0, 200.0, 260.0] {
                crate::canvas::dispatch(app, &pointer(vectorcraft_tools::PointerKind::Drag, x), view);
            }
            crate::canvas::dispatch(app, &pointer(vectorcraft_tools::PointerKind::Up, 300.0), view);
            app.session.doc().unwrap().selection.objects.clone()
        };
        let ids = |app: &VectorcraftApp| app.session.doc().unwrap().doc.layers[0].children().unwrap().iter().map(|n| n.id).collect::<Vec<_>>();
        // Two blobs the stroke crosses, drawn with the same color.
        let blob = |app: &mut VectorcraftApp, x: f64| {
            let r = app.run("path.blob", json!({"points": [[x, 60], [x, 140]], "size": 20})).unwrap();
            vectorcraft_doc::NodeId(r["id"].as_u64().unwrap())
        };
        let (a, b) = (blob(&mut app, 100.0), blob(&mut app, 240.0));
        app.run("tool.options", json!({"tool": "blobBrush"})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        // Both are off by default.
        assert!(!d.bool("keepSelected") && !d.bool("mergeOnlySelection"));
        d.fields.insert("keepSelected".into(), json!(true));
        d.fields.insert("mergeOnlySelection".into(), json!(true));
        super::super::confirm(&mut app).unwrap();
        let o = app.session.tool_options_of("blobBrush");
        assert_eq!((o["keepSelected"].as_bool(), o["mergeOnlySelection"].as_bool()), (Some(true), Some(true)));
        // With `b` selected, the stroke merges into `b` alone, and leaves it selected.
        app.run("select.set", json!({"ids": [b.0]})).unwrap();
        assert_eq!(stroke(&mut app), vec![b]);
        assert_eq!(ids(&app), vec![a, b], "merged into b only");
    }

    /// Eraser Tool Options' Angle and Roundness shape the tip the next Eraser drag erases with,
    /// and the tool keeps them while another tool is chosen.
    #[test]
    fn the_eraser_options_set_the_tips_angle_and_roundness() {
        let mut app = app();
        // The width of the gap a vertical Eraser drag cuts through the middle of a 100 pt square.
        let erase = |app: &mut VectorcraftApp| {
            app.run("select.set", json!({"ids": []})).unwrap();
            let r = app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap();
            let id = r["id"].as_u64().unwrap();
            app.run("select.set", json!({"ids": [id]})).unwrap();
            app.select_tool("eraser");
            let view = app.view_info();
            let pointer = |kind, y| vectorcraft_tools::PointerEvent::new(kind, 150.0, y);
            crate::canvas::dispatch(app, &pointer(vectorcraft_tools::PointerKind::Down, 50.0), view);
            for y in [100.0, 150.0, 200.0] {
                crate::canvas::dispatch(app, &pointer(vectorcraft_tools::PointerKind::Drag, y), view);
            }
            crate::canvas::dispatch(app, &pointer(vectorcraft_tools::PointerKind::Up, 250.0), view);
            let st = app.session.doc().unwrap();
            let bounds: Vec<_> = st.selection.objects.iter().filter_map(|id| st.doc.node(*id)?.path_data()?.control_bounds()).collect();
            let [a, b] = bounds[..] else { panic!("two halves: {bounds:?}") };
            let (l, r) = if a.x0 < b.x0 { (a, b) } else { (b, a) };
            r.x0 - l.x1
        };
        app.run("tool.options", json!({"tool": "eraser"})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        // A round tip by default.
        assert_eq!((d.f64("angle", -1.0), d.f64("roundness", -1.0)), (0.0, 100.0));
        d.fields.insert("size".into(), json!(20));
        super::super::confirm(&mut app).unwrap();
        let round = erase(&mut app);
        assert!((round - 20.0).abs() < 0.2, "a 20 pt round tip cuts {round} pt");
        // Upright and a quarter as wide: the band it erases is 5 pt wide.
        app.run("tool.options", json!({"tool": "eraser"})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("angle".into(), json!(90));
        d.fields.insert("roundness".into(), json!(25));
        super::super::confirm(&mut app).unwrap();
        app.select_tool("selection");
        let o = app.session.tool_options_of("eraser");
        assert_eq!((o["angle"].as_f64(), o["roundness"].as_f64(), o["size"].as_f64()), (Some(90.0), Some(25.0), Some(20.0)));
        let narrow = erase(&mut app);
        assert!((narrow - 5.0).abs() < 0.2, "a 20 by 5 pt upright tip cuts {narrow} pt");
    }

    #[test]
    fn the_other_tools_have_no_freehand_options() {
        let mut app = app();
        assert!(open(&mut app, "pen").is_err());
        assert!(app.ui.dialog.is_none());
    }
}
