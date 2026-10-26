//! Transform panel: reference point, X/Y/W/H with the constrain link, rotate and shear, flips,
//! live-shape properties and the Scale Corners / Scale Strokes & Effects options. While the
//! Artboard tool is in use it shows and edits the active artboard's X/Y/W/H instead (a new W or H
//! resizes every selected artboard), and with only ruler guides selected their X (vertical guides)
//! and Y (horizontal ones).

use egui::Ui;
use serde_json::json;
use vectorcraft_doc::{LiveShape, Node, NodeKind, OrientedBox};
use vectorcraft_geom::shapes::CornerKind;
use vectorcraft_geom::{Point, Rect, Vec2};

use super::{corner_radius_row, first_selected, pstate, set_pstate};
use crate::dialogs::corners::kind_label as corner_kind;
use crate::theme::Tokens;
use crate::widgets::{self, menu_item};
use crate::{VectorcraftApp, icons};

pub const ANGLE_PRESETS: [f64; 9] = [-180.0, -135.0, -90.0, -45.0, 0.0, 45.0, 90.0, 135.0, 180.0];
/// The shear angle's presets: steep enough to be useful, short of the degenerate ±90°.
pub const SHEAR_PRESETS: [f64; 9] = [-60.0, -45.0, -30.0, -15.0, 0.0, 15.0, 30.0, 45.0, 60.0];

/// Proportional size: the other dimension when one changes with the link on.
pub fn constrained(w: f64, h: f64, new_w: Option<f64>, new_h: Option<f64>) -> (f64, f64) {
    match (new_w, new_h) {
        (Some(nw), _) if w.abs() > 1e-9 => (nw, h * nw / w),
        (_, Some(nh)) if h.abs() > 1e-9 => (w * nh / h, nh),
        (a, b) => (a.unwrap_or(w), b.unwrap_or(h)),
    }
}

/// The artboard the Artboard tool has active, while that tool is in use.
fn tool_artboard(app: &VectorcraftApp) -> Option<(usize, Rect)> {
    if app.session.tool_id() != "artboard" {
        return None;
    }
    let i = usize::try_from(app.session.tool_options().get("active")?.as_u64()?).ok()?;
    app.session.active()?.doc.artboards.get(i).map(|a| (i, a.rect))
}

/// Where an artboard goes when the panel sets its X, Y, W or H: the reference point `refi` moves to
/// the new X/Y or stays put while the size changes (both sides together when `link` is on).
pub fn artboard_rect(r: Rect, refi: usize, x: Option<f64>, y: Option<f64>, w: Option<f64>, h: Option<f64>, link: bool) -> Rect {
    let rp = vectorcraft_geom::reference_point(r, refi);
    let (w0, h0) = (r.width(), r.height());
    let (nw, nh) = if link { constrained(w0, h0, w, h) } else { (w.unwrap_or(w0), h.unwrap_or(h0)) };
    // Where the reference point sits across the box (0, ½ or 1 of each side).
    let fx = if w0 > 1e-9 { (rp.x - r.x0) / w0 } else { 0.0 };
    let fy = if h0 > 1e-9 { (rp.y - r.y0) / h0 } else { 0.0 };
    let (x0, y0) = (x.unwrap_or(rp.x) - fx * nw, y.unwrap_or(rp.y) - fy * nh);
    Rect::new(x0, y0, x0 + nw, y0 + nh)
}

/// The reference point the Transform panel and the Control bar share (0 to 8, the center by default).
pub fn reference_index(ui: &Ui) -> usize {
    ui.data(|d| d.get_temp(egui::Id::unique("refpt"))).unwrap_or(4)
}

/// Where the Transform panel and the Properties panel's Transform section measure the selection
/// from and turn, shear and flip it about ([`panel_reference`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanelReference {
    /// The reference point (0 to 8) of the selection's box that `object.setBounds` keeps in place
    /// or moves to a new X or Y, and that the locator shows.
    pub index: usize,
    /// The point on the page: that reference point, or a symbol instance's registration point.
    pub point: Point,
    /// `point` less the box's reference point `index` (zero unless the registration point lies on
    /// none of the box's nine points, as on a sheared instance).
    shift: Vec2,
}

impl PanelReference {
    /// The `x` and `y` of `object.setBounds` that put [`Self::point`] at `x`, `y` on the page.
    pub fn bounds_xy(&self, x: f64, y: f64) -> (f64, f64) {
        (x - self.shift.x, y - self.shift.y)
    }

    /// The `origin` param of the rotations, shears and reflections: [`Self::point`].
    pub fn origin(&self) -> serde_json::Value {
        json!([self.point.x, self.point.y])
    }
}

/// The reference of box `bx` of the selection for the Transform panel and the Properties panel's
/// Transform section: the shared reference point ([`reference_index`]); with Use Registration Point
/// for Symbol on and one symbol instance selected, its symbol's registration point (Symbol Options
/// › Registration) where the instance's transform puts it, with the box's reference point nearest
/// it: after a flip the art's top-left corner is the box's top-right one, after a rotation the
/// box turns with the art.
pub fn panel_reference(app: &VectorcraftApp, ui: &Ui, bx: &OrientedBox) -> PanelReference {
    let Some(q) = symbol_registration(app) else {
        let index = reference_index(ui);
        return PanelReference { index, point: bx.reference_point(index), shift: Vec2::ZERO };
    };
    // The nearest of the box's nine points, measured in the box's own (turned) frame.
    let (local, r) = (bx.to_local(q), bx.rect);
    let third = |v: f64, start: f64, len: f64| if len > 1e-9 { ((v - start) / len * 2.0).round().clamp(0.0, 2.0) as usize } else { 1 };
    let index = third(local.y, r.y0, r.height()) * 3 + third(local.x, r.x0, r.width());
    PanelReference { index, point: q, shift: q - bx.reference_point(index) }
}

/// Where the registration point of the one selected symbol instance is on the page, while Use
/// Registration Point for Symbol is on: the point of the symbol's art box on the 9-point grid,
/// placed by the instance's transform.
fn symbol_registration(app: &VectorcraftApp) -> Option<Point> {
    if !app.session.prefs.use_registration_point_for_symbol {
        return None;
    }
    let st = app.session.active()?;
    let [id] = st.selection.objects.as_slice() else { return None };
    let NodeKind::SymbolInstance { symbol, xf } = &st.doc.node(*id)?.kind else { return None };
    let own = Node::new(*id, NodeKind::SymbolInstance { symbol: symbol.clone(), xf: vectorcraft_geom::Affine::IDENTITY });
    let art = own.geometric_bounds_with(&st.doc.symbol_boxes())?;
    Some(*xf * vectorcraft_geom::reference_point(art, usize::from(st.doc.symbol_registration(symbol).0)))
}

/// The Transform panel menu's choices for what the panel transforms: the value of the
/// `transformPanelPatterns` preference each sets, and its label.
const PATTERN_MODES: [(&str, &str); 3] = [("object", "Transform Object Only"), ("pattern", "Transform Pattern Only"), ("both", "Transform Both")];

/// Run transform command `cmd` from the Transform panel or the Properties panel's Transform
/// section with `params` and the `objects` and `patterns` switches of the panel menu's Transform
/// Object Only (the object; its pattern fills and strokes stay as they are), Transform Pattern
/// Only (the pattern tiles alone) or Transform Both. Until one is chosen (`default`, or any other
/// value) neither is passed: patterns follow General › Transform Pattern Tiles. Errors are
/// dropped, as the panels' fields have always done.
pub fn run_transform(app: &mut VectorcraftApp, cmd: &str, mut params: serde_json::Value) {
    let switches = match app.session.prefs.transform_panel_patterns.as_str() {
        "object" => Some((true, false)),
        "pattern" => Some((false, true)),
        "both" => Some((true, true)),
        _ => None,
    };
    if let Some((objects, patterns)) = switches {
        params["objects"] = json!(objects);
        params["patterns"] = json!(patterns);
    }
    app.run(cmd, params).ok();
}

/// The panel menu's checked choice of [`PATTERN_MODES`]: the one chosen, else (`default`) what
/// General › Transform Pattern Tiles makes the panel do, Transform Both when it is on and
/// Transform Object Only when it is off.
fn pattern_mode(app: &VectorcraftApp) -> &'static str {
    let prefs = &app.session.prefs;
    PATTERN_MODES.iter().map(|(v, _)| *v).find(|v| *v == prefs.transform_panel_patterns).unwrap_or(if prefs.transform_pattern_tiles {
        "both"
    } else {
        "object"
    })
}

/// The shared reference point's locator; a click on it chooses another point.
pub fn reference_locator(ui: &mut Ui, refi: usize) {
    if let Some(i) = widgets::reference_point(ui, refi) {
        ui.data_mut(|d| d.insert_temp(egui::Id::unique("refpt"), i));
    }
}

/// Set artboard `index` to a new X, Y, W or H from reference point `refi` ([`artboard_rect`]) with
/// `artboard.setProps`, passing the Artboard tool's Move and Scale Artwork with Artboard options as
/// `moveArt` and `scaleArt`; an error is shown in the status bar. A new X or Y moves artboard
/// `index` alone; a new W or H resizes every selected artboard about its own reference point
/// ([`crate::panels::artboards::resized_with`]), in one undo step.
pub fn set_artboard(app: &mut VectorcraftApp, index: usize, refi: usize, xywh: [Option<f64>; 4], link: bool) {
    let [x, y, w, h] = xywh;
    let scale_art = crate::panels::artboards::scale_art(app);
    let move_art = crate::panels::artboards::move_art(app);
    let indices = if x.is_none() && y.is_none() { crate::panels::artboards::resized_with(app, index) } else { vec![index] };
    let common = json!({"scaleArt": scale_art, "moveArt": move_art});
    let r = crate::panels::artboards::set_props(app, &indices, common, |i, r| {
        let n = artboard_rect(r, refi, x, y, w, h, link);
        Some(json!({"index": i, "x": n.x0, "y": n.y0, "width": n.width(), "height": n.height()}))
    });
    if let Err(e) = r {
        app.status(e);
    }
}

/// The Artboard tool's view of the panel: the active artboard's position and size ([`set_artboard`]).
fn artboard_fields(app: &mut VectorcraftApp, ui: &mut Ui, index: usize, r: Rect) {
    let units = app.session.general_unit();
    let origin = app.ruler_origin();
    let refi = reference_index(ui);
    let link = proportional(app, ui);
    let rp = vectorcraft_geom::reference_point(r, refi);
    let set = |app: &mut VectorcraftApp, x, y, w, h| set_artboard(app, index, refi, [x, y, w, h], link);
    ui.horizontal(|ui| {
        reference_locator(ui, refi);
        ui.add_space(4.0);
        egui::Grid::new("xfp-ab-grid").num_columns(4).spacing([4.0, 6.0]).min_col_width(0.0).show(ui, |ui| {
            widgets::dim_label(ui, "X:");
            if let Some(v) = widgets::num_field_named(ui, "xfp-ab-x", Some(crate::i18n::t("X:")), Some(rp.x - origin.x), units, 80.0) {
                set(app, Some(v + origin.x), None, None, None);
            }
            widgets::dim_label(ui, "W:");
            if let Some(v) = widgets::size_field_named(ui, "xfp-ab-w", Some(tl!("W:")), Some(r.width()), units, 80.0) {
                set(app, None, None, Some(v), None);
            }
            ui.end_row();
            widgets::dim_label(ui, "Y:");
            if let Some(v) = widgets::num_field_named(ui, "xfp-ab-y", Some(crate::i18n::t("Y:")), Some(rp.y - origin.y), units, 80.0) {
                set(app, None, Some(v + origin.y), None, None);
            }
            widgets::dim_label(ui, "H:");
            if let Some(v) = widgets::size_field_named(ui, "xfp-ab-h", Some(tl!("H:")), Some(r.height()), units, 80.0) {
                set(app, None, None, None, Some(v));
            }
            ui.end_row();
        });
        constrain_link(app, ui);
    });
}

/// With ruler guides selected and no art: the X of the vertical ones and the Y of the horizontal
/// ones (the least of each; none where no such guide is selected).
fn selected_guides(app: &VectorcraftApp) -> Option<(Option<f64>, Option<f64>)> {
    let st = app.session.active()?;
    if !st.selection.objects.is_empty() || st.selection.guides.is_empty() {
        return None;
    }
    let least = |vertical: bool| {
        st.selection.guides.iter().filter_map(|i| st.doc.guides.get(*i)).filter(|g| g.vertical == vertical).map(|g| g.pos).min_by(f64::total_cmp)
    };
    Some((least(true), least(false)))
}

/// The selected ruler guides' view of the panel (#1597): X moves the vertical ones and Y the
/// horizontal ones, together.
fn guide_fields(app: &mut VectorcraftApp, ui: &mut Ui, (x, y): (Option<f64>, Option<f64>)) {
    let units = app.session.general_unit();
    let origin = app.ruler_origin();
    egui::Grid::new("xfp-guide-grid").num_columns(2).spacing([4.0, 6.0]).min_col_width(0.0).show(ui, |ui| {
        for (label, id, pos, origin, by) in [("X:", "xfp-guide-x", x, origin.x, "dx"), ("Y:", "xfp-guide-y", y, origin.y, "dy")] {
            widgets::dim_label(ui, label);
            ui.add_enabled_ui(pos.is_some(), |ui| {
                if let (Some(v), Some(pos)) = (widgets::num_field_named(ui, id, Some(tl!(label)), pos.map(|p| p - origin), units, 80.0), pos)
                    && let Err(e) = app.run("guide.move", json!({ by: v + origin - pos }))
                {
                    app.status(e);
                }
            });
            ui.end_row();
        }
    });
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    if app.session.active().is_none() {
        widgets::dim_label(ui, tl!("No document"));
        return;
    }
    if let Some((i, r)) = tool_artboard(app) {
        artboard_fields(app, ui, i, r);
        return;
    }
    if let Some(guides) = selected_guides(app) {
        guide_fields(app, ui, guides);
        return;
    }
    let units = app.session.general_unit();
    let origin = app.ruler_origin();
    // The bounding box, rotated with rotated objects: W/H are its own sides, X/Y its reference
    // point on the page.
    let bx = app.selection_box();
    let bounds = bx.map(|b| b.rect);
    let reference = bx.map(|b| panel_reference(app, ui, &b));
    let refi = reference.map_or_else(|| reference_index(ui), |r| r.index);
    // `object.setBounds`'s x and y that put the reference (the registration point) at x, y.
    let to_box = |x: f64, y: f64| reference.map_or((x, y), |r| r.bounds_xy(x, y));
    let link = proportional(app, ui);
    let copy = alt_enter_copies(app, ui);
    let has = bounds.is_some();
    let rp = reference.map(|r| r.point);
    ui.horizontal(|ui| {
        if let Some(i) = widgets::reference_point(ui, refi) {
            ui.data_mut(|d| d.insert_temp(egui::Id::unique("refpt"), i));
        }
        ui.add_space(4.0);
        ui.add_enabled_ui(has, |ui| {
            egui::Grid::new("xfp-grid").num_columns(4).spacing([4.0, 6.0]).min_col_width(0.0).show(ui, |ui| {
                widgets::dim_label(ui, "X:");
                if let Some(v) = widgets::num_field_named(ui, "xfp-x", Some(crate::i18n::t("X:")), rp.map(|p| p.x - origin.x), units, 80.0) {
                    run_transform(app, "object.setBounds", json!({"x": to_box(v + origin.x, 0.0).0, "reference": refi, "copy": copy}));
                }
                widgets::dim_label(ui, "W:");
                if let Some(v) = widgets::size_field_named(ui, "xfp-w", Some(tl!("W:")), bounds.map(|b| b.width()), units, 80.0) {
                    run_transform(app, "object.setBounds", json!({"width": v, "reference": refi, "proportional": link, "copy": copy}));
                }
                ui.end_row();
                widgets::dim_label(ui, "Y:");
                if let Some(v) = widgets::num_field_named(ui, "xfp-y", Some(crate::i18n::t("Y:")), rp.map(|p| p.y - origin.y), units, 80.0) {
                    run_transform(app, "object.setBounds", json!({"y": to_box(0.0, v + origin.y).1, "reference": refi, "copy": copy}));
                }
                widgets::dim_label(ui, "H:");
                if let Some(v) = widgets::size_field_named(ui, "xfp-h", Some(tl!("H:")), bounds.map(|b| b.height()), units, 80.0) {
                    run_transform(app, "object.setBounds", json!({"height": v, "reference": refi, "proportional": link, "copy": copy}));
                }
                ui.end_row();
            });
        });
        constrain_link(app, ui);
    });
    ui.add_space(4.0);
    let origin = reference.map(|r| r.origin());
    ui.horizontal(|ui| {
        ui.add_enabled_ui(has, |ui| {
            let icon = icons::icon(ui, "rotate-ccw", 16.0, t.icon).on_hover_text(tl!("Rotate"));
            crate::scrub::note_label(ui, icon.rect);
            // The bounding box's angle: a new value turns the selection to it.
            let angle = bx.map_or(0.0, |b| b.angle);
            if let Some(a) = widgets::spin_plain_named(ui, "xfp-rot", Some(tl!("Rotate")), angle, "°", 2, 96.0, 15.0, -360.0, &ANGLE_PRESETS)
                && a != angle
            {
                run_transform(app, "object.rotate", json!({"angle": a, "absolute": true, "origin": origin, "copy": copy}));
            }
            ui.add_space(4.0);
            let icon = icons::icon(ui, "dc-shear", 16.0, t.icon).on_hover_text(tl!("Shear"));
            crate::scrub::note_label(ui, icon.rect);
            // Shear is relative: the field rests at 0° and a new value shears by it.
            if let Some(a) = widgets::spin_plain_named(ui, "xfp-shear", Some(tl!("Shear")), 0.0, "°", 2, 96.0, 15.0, -89.0, &SHEAR_PRESETS)
                && a != 0.0
                && a.abs() < 90.0
            {
                run_transform(app, "object.shear", json!({"angle": a, "axis": "horizontal", "origin": origin, "copy": copy}));
            }
        });
    });
    ui.horizontal(|ui| {
        // Lucide's flip icons are named for their mirror line: flip-vertical-2 draws a vertical
        // line (the left-right flip), flip-horizontal-2 a horizontal one (top-bottom).
        if widgets::icon_button_enabled(ui, "flip-vertical-2", tl!("Flip Horizontal"), false, has, 24.0).clicked() {
            run_transform(app, "object.reflect", json!({"axis": "vertical", "origin": origin}));
        }
        if widgets::icon_button_enabled(ui, "flip-horizontal-2", tl!("Flip Vertical"), false, has, 24.0).clicked() {
            run_transform(app, "object.reflect", json!({"axis": "horizontal", "origin": origin}));
        }
    });
    // Live shape properties (a path with live corners is no shape).
    if let Some(n) = first_selected(app)
        && let NodeKind::Path { live: Some(live), .. } = &n.kind
        && !matches!(**live, vectorcraft_doc::LiveShape::Path { .. })
    {
        widgets::divider(ui);
        match &**live {
            vectorcraft_doc::LiveShape::Rectangle { .. } => {
                widgets::subheader(ui, tl!("Rectangle Properties:"));
                corner_radius_row(app, ui, &n, "xfp-radius");
            }
            vectorcraft_doc::LiveShape::Polygon { .. } => {
                widgets::subheader(ui, tl!("Polygon Properties:"));
                polygon_rows(app, ui, &n, live, "xfp-polygon");
            }
            vectorcraft_doc::LiveShape::Ellipse { pie, .. } => {
                widgets::subheader(ui, tl!("Ellipse Properties:"));
                pie_rows(app, ui, *pie, "xfp-pie");
            }
            _ => {
                widgets::subheader(ui, tl!("Shape Properties:"));
                widgets::dim_label(ui, tl!(n.kind_label()));
            }
        }
    }
    if pstate::<bool>(ui.ctx(), "xf-hide-options") {
        return;
    }
    widgets::divider(ui);
    let (sc, ss) = (app.session.prefs.scale_corners, app.session.prefs.scale_strokes);
    if widgets::check(ui, tl!("Scale Corners"), sc, true) {
        set_pref(app, "scaleCorners", !sc);
    }
    if widgets::check(ui, tl!("Scale Strokes & Effects"), ss, true) {
        set_pref(app, "scaleStrokes", !ss);
    }
}

/// An ellipse's Pie Start and End Angle (degrees, counterclockwise from 3 o'clock; 0 to 360 is the
/// whole ellipse) and Invert Pie, which shows the other part of it (Transform and Properties panels).
pub(crate) fn pie_rows(app: &mut VectorcraftApp, ui: &mut Ui, pie: (f64, f64), id: &str) {
    for (label, key, v) in [(tl!("Pie Start Angle:"), "pieStart", pie.0), (tl!("Pie End Angle:"), "pieEnd", pie.1)] {
        ui.horizontal(|ui| {
            widgets::dim_label(ui, label);
            if let Some(a) = widgets::plain_field(ui, (id, key), v, "°", 1, 60.0) {
                app.run("object.setLiveShape", json!({ key: a })).ok();
            }
        });
    }
    if widgets::flat_button(ui, tl!("Invert Pie"), 90.0).clicked() {
        app.run("object.setLiveShape", json!({"invertPie": true})).ok();
    }
}

/// A live polygon's Polygon Properties (Transform and Properties panels): its side count, angle
/// (counterclockwise), corner type and radius, radius (centre to vertex) and side length, and Make
/// Sides Equal, which an uneven scale enables.
pub(crate) fn polygon_rows(app: &mut VectorcraftApp, ui: &mut Ui, n: &Node, live: &LiveShape, id: &str) {
    let LiveShape::Polygon { sides, .. } = live else { return };
    let (units, fw) = (app.session.general_unit(), super::field_width(ui));
    let radius = live.polygon_radius();
    let side = radius.map(|r| 2.0 * r * (std::f64::consts::PI / f64::from((*sides).max(3))).sin());
    let kind = super::corner_style(app, n).1;
    let kinds: Vec<&str> = CornerKind::ALL.iter().map(|k| corner_kind(*k)).collect();
    // The `object.setLiveShape` param an edited field sets.
    let mut set = None;
    egui::Grid::new((id, "grid")).num_columns(2).spacing([6.0, 6.0]).show(ui, |ui| {
        widgets::dim_label(ui, tl!("Sides:"));
        if let Some(s) = widgets::range_field_named(ui, (id, "sides"), Some(tl!("Sides:")), f64::from(*sides), 3.0..=f64::INFINITY, "", 0, fw) {
            set = Some(("sides", json!(s as u64)));
        }
        ui.end_row();
        widgets::dim_label(ui, tl!("Angle:"));
        if let Some(a) = widgets::plain_field_named(ui, (id, "angle"), Some(tl!("Angle:")), live.polygon_angle().unwrap_or(0.0), "°", 1, fw) {
            set = Some(("polygonAngle", json!(a)));
        }
        ui.end_row();
        widgets::dim_label(ui, tl!("Corner:"));
        if let Some(k) = widgets::dropdown_names_named(ui, (id, "kind"), Some(tl!("Corner:")), kind.map_or("", corner_kind), &kinds, fw)
            .and_then(|i| CornerKind::ALL.get(i))
        {
            set = Some(("kind", json!(k)));
        }
        ui.end_row();
        widgets::dim_label(ui, tl!("Corner Radius:"));
        super::corner_radius_field(app, ui, n, (id, "cornerRadius"), fw);
        ui.end_row();
        widgets::dim_label(ui, tl!("Radius:"));
        if let Some(r) = widgets::num_field_named(ui, (id, "radius"), Some(tl!("Radius:")), radius, units, fw) {
            set = Some(("polygonRadius", json!(r)));
        }
        ui.end_row();
        widgets::dim_label(ui, tl!("Side Length:"));
        if let Some(l) = widgets::num_field_named(ui, (id, "side"), Some(tl!("Side Length:")), side, units, fw) {
            set = Some(("sideLength", json!(l)));
        }
        ui.end_row();
    });
    if ui.add_enabled_ui(!live.polygon_sides_equal(), |ui| widgets::flat_button(ui, tl!("Make Sides Equal"), 130.0)).inner.clicked() {
        set = Some(("makeSidesEqual", json!(true)));
    }
    if let Some((key, v)) = set {
        app.run("object.setLiveShape", json!({ key: v })).ok();
    }
}

/// The Control bar's underlined "Transform" link: a click toggles the Transform panel in a
/// popover under it, as the Stroke link does with the Stroke panel.
pub fn link(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let resp = ui.link(egui::RichText::new(tl!("Transform")).size(12.0).color(t.text).underline()).on_hover_text(tl!("Transform options"));
    widgets::popover(&resp, resp.clicked(), |ui| {
        ui.set_width(300.0);
        show(app, ui);
    });
}

/// The link between W and H (Transform panel, Properties panel, Control bar): one toggle, the
/// `constrainProportions` preference, which the size fields pass on as `proportional`.
pub fn constrain_link(app: &mut VectorcraftApp, ui: &mut Ui) {
    let on = app.session.prefs.constrain_proportions;
    if widgets::icon_button(ui, if on { "link" } else { "link-2-off" }, tl!("Constrain Width and Height Proportions"), on, 22.0).clicked() {
        set_pref(app, "constrainProportions", !on);
    }
}

/// The `copy` param of a command that a field of the Transform panel, the Properties panel's
/// Transform section or the Control bar runs: true when Alt+Enter (Option+Return) applied the value
/// and no anchors are selected with Direct Selection.
pub fn alt_enter_copies(app: &VectorcraftApp, ui: &Ui) -> bool {
    widgets::enter_modifiers(ui).is_some_and(|m| m.alt) && app.session.active().is_some_and(|st| st.selection.anchors.is_empty())
}

/// The `proportional` param of the W and H fields of the Transform panel (the artboard's too), the
/// Properties panel's Transform section and the Control bar: the Constrain Width and Height
/// Proportions link, or true when Ctrl/Cmd+Enter applied the value.
pub fn proportional(app: &VectorcraftApp, ui: &Ui) -> bool {
    app.session.prefs.constrain_proportions || widgets::enter_modifiers(ui).is_some_and(|m| m.command)
}

/// Toggle a boolean preference (through `prefs.set`, as agents do).
pub fn set_pref(app: &mut VectorcraftApp, key: &str, on: bool) {
    if let Err(e) = app.run("prefs.set", json!({ "key": key, "value": on })) {
        app.status(e);
    }
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let hidden: bool = pstate(ui.ctx(), "xf-hide-options");
    let has = super::selection_len(app) > 0;
    // The flips mirror across the panel's reference point, as its flip buttons do.
    let origin = app.selection_box().map(|b| panel_reference(app, ui, &b).origin());
    if menu_item(ui, if hidden { tl!("Show Options") } else { tl!("Hide Options") }, true, false) {
        set_pstate(ui.ctx(), "xf-hide-options", !hidden);
    }
    ui.separator();
    if menu_item(ui, tl!("Flip Horizontal"), has, false) {
        run_transform(app, "object.reflect", json!({"axis": "vertical", "origin": origin}));
    }
    if menu_item(ui, tl!("Flip Vertical"), has, false) {
        run_transform(app, "object.reflect", json!({"axis": "horizontal", "origin": origin}));
    }
    ui.separator();
    let ss = app.session.prefs.scale_strokes;
    if menu_item(ui, tl!("Scale Strokes & Effects"), true, ss) {
        set_pref(app, "scaleStrokes", !ss);
    }
    ui.separator();
    // What the panel transforms (Transform Object Only / Pattern Only / Both), one checked.
    let current = pattern_mode(app);
    for (value, label) in PATTERN_MODES {
        if menu_item(ui, label, true, value == current)
            && let Err(e) = app.run("prefs.set", json!({"key": "transformPanelPatterns", "value": value}))
        {
            app.status(e);
        }
    }
    ui.separator();
    let reg = app.session.prefs.use_registration_point_for_symbol;
    if menu_item(ui, tl!("Use Registration Point for Symbol"), true, reg) {
        set_pref(app, "useRegistrationPointForSymbol", !reg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One headless frame of the panel (or the Align panel's menu): the texts drawn.
    fn texts(app: &mut VectorcraftApp, draw: fn(&mut VectorcraftApp, &mut Ui)) -> Vec<String> {
        fn walk(s: &egui::Shape, out: &mut Vec<String>) {
            match s {
                egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| draw(app, ui));
        out.textures_delta.clear();
        let mut v = vec![];
        out.shapes.iter().for_each(|c| walk(&c.shape, &mut v));
        v
    }

    #[test]
    fn use_preview_bounds_shows_the_visual_size_and_checks_the_align_flyout() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 50})).unwrap();
        app.run("stroke.set", json!({"weight": 10})).unwrap();
        assert!(texts(&mut app, show).iter().any(|t| t == "100 pt"));
        assert!(texts(&mut app, crate::panels::align::menu).iter().any(|t| t == "   Use Preview Bounds"));
        set_pref(&mut app, "usePreviewBounds", true);
        let shown = texts(&mut app, show);
        assert!(shown.iter().any(|t| t == "110 pt") && shown.iter().any(|t| t == "60 pt"), "{shown:?}");
        assert!(texts(&mut app, crate::panels::align::menu).iter().any(|t| t == "✓ Use Preview Bounds"));
    }

    #[test]
    fn use_preview_bounds_shows_the_size_after_geometry_effects() {
        // A 100 × 50 rectangle without a stroke, with Offset Path 10: 120 × 70 as drawn.
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("shape.rectangle", json!({"x": 50, "y": 50, "width": 100, "height": 50})).unwrap();
        app.run("paint.setStroke", json!({"none": true})).unwrap();
        app.run("effect.apply", json!({"effect": "path.offsetPath", "params": {"offset": 10}, "item": null})).unwrap();
        let shown = texts(&mut app, show);
        assert!(shown.iter().any(|t| t == "100 pt") && shown.iter().any(|t| t == "50 pt"), "{shown:?}");
        set_pref(&mut app, "usePreviewBounds", true);
        let shown = texts(&mut app, show);
        assert!(shown.iter().any(|t| t == "120 pt") && shown.iter().any(|t| t == "70 pt"), "{shown:?}");
    }

    #[test]
    fn rotation_field_shows_the_angle_the_box_keeps() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 50})).unwrap();
        app.run("object.rotate", json!({"angle": 45, "absolute": true})).unwrap();
        let shown = texts(&mut app, show);
        // The angle stays, and W/H are the rectangle's own sides.
        assert!(shown.iter().any(|t| t == "45°"), "{shown:?}");
        assert!(shown.iter().any(|t| t == "100 pt") && shown.iter().any(|t| t == "50 pt"), "{shown:?}");
        app.run("object.resetBoundingBox", json!({})).unwrap();
        let shown = texts(&mut app, show);
        assert!(shown.iter().any(|t| t == "0°") && !shown.iter().any(|t| t == "100 pt"), "{shown:?}");
    }

    /// #812: a selected live polygon shows its Polygon Properties in the Transform panel and the
    /// Properties panel's Transform section, and Make Sides Equal evens it out after a stretch.
    #[test]
    fn a_polygon_shows_its_polygon_properties() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("shape.polygon", json!({"cx": 150, "cy": 150, "radius": 50, "sides": 6})).unwrap();
        for draw in [show as fn(&mut VectorcraftApp, &mut Ui), crate::panels::properties::transform_section] {
            let shown = texts(&mut app, draw);
            // A hexagon's side is as long as its radius.
            for t in ["Sides:", "6", "Angle:", "0°", "Corner:", "Round", "Corner Radius:", "Radius:", "Side Length:", "50 pt", "Make Sides Equal"] {
                assert!(shown.iter().any(|s| s == t), "{t} in {shown:?}");
            }
        }
        assert!(texts(&mut app, show).iter().any(|s| s == "Polygon Properties:"));
        app.run("object.scale", json!({"sx": 200, "sy": 100})).unwrap();
        let live = |app: &VectorcraftApp| match &first_selected(app).unwrap().kind {
            NodeKind::Path { live: Some(l), .. } => l.clone(),
            k => panic!("{k:?}"),
        };
        assert!(!live(&app).polygon_sides_equal());
        app.run("object.setLiveShape", json!({"makeSidesEqual": true})).unwrap();
        assert!(live(&app).polygon_sides_equal());
    }

    /// #3451: a live ellipse shows pie start/end angles and Invert Pie in the Transform panel and
    /// the Properties panel's Transform section; typed angles update the shape.
    #[test]
    fn an_ellipse_shows_its_pie_properties() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("shape.ellipse", json!({"x": 50, "y": 50, "width": 120, "height": 80})).unwrap();
        for draw in [show as fn(&mut VectorcraftApp, &mut Ui), crate::panels::properties::transform_section] {
            let shown = texts(&mut app, draw);
            for t in ["Ellipse Properties:", "Pie Start Angle:", "0°", "Pie End Angle:", "360°", "Invert Pie"] {
                assert!(shown.iter().any(|s| s == t), "{t} in {shown:?}");
            }
        }
        app.run("object.setLiveShape", json!({"pieStart": 30, "pieEnd": 120})).unwrap();
        let shown = texts(&mut app, show);
        assert!(shown.iter().any(|s| s == "30°") && shown.iter().any(|s| s == "120°"), "{shown:?}");
    }

    #[test]
    fn the_artboard_tool_shows_the_active_artboard() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 40, "height": 30})).unwrap();
        assert!(texts(&mut app, show).iter().any(|t| t == "40 pt"), "the selection first");
        app.run("tool.select", json!({"tool": "artboard"})).unwrap();
        let shown = texts(&mut app, show);
        // Centre reference point: X 150, Y 100, W 300, H 200; no rotate/shear for artboards.
        for v in ["150 pt", "100 pt", "300 pt", "200 pt"] {
            assert!(shown.iter().any(|t| t == v), "{v} in {shown:?}");
        }
        assert!(!shown.iter().any(|t| t == "40 pt" || t == "0°"), "{shown:?}");
    }

    /// #1597: with only ruler guides selected the panel shows their X or Y, and nothing of the art.
    #[test]
    fn selected_guides_show_their_position() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        app.run("guide.add", json!({"vertical": true, "pos": 120})).unwrap();
        app.run("guide.add", json!({"vertical": false, "pos": 45})).unwrap();
        app.run("guide.select", json!({"indexes": [0]})).unwrap();
        assert_eq!(selected_guides(&app), Some((Some(120.0), None)));
        let shown = texts(&mut app, show);
        assert!(shown.iter().any(|t| t == "120 pt") && !shown.iter().any(|t| t == "W:" || t == "45 pt"), "{shown:?}");
        app.run("guide.select", json!({"indexes": [0, 1]})).unwrap();
        let shown = texts(&mut app, show);
        assert!(shown.iter().any(|t| t == "120 pt") && shown.iter().any(|t| t == "45 pt"), "{shown:?}");
        // The field's edit: the vertical guide moves, the horizontal one stays.
        app.run("guide.move", json!({"dx": 30})).unwrap();
        assert_eq!(selected_guides(&app), Some((Some(150.0), Some(45.0))));
        // Art selected with them: the art's box, as before.
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 40, "height": 30})).unwrap();
        assert_eq!(selected_guides(&app), None);
    }

    /// #1824: with an anchor selected with Direct Selection the panel shows that anchor's position,
    /// and a new X moves it alone.
    #[test]
    fn an_anchor_selection_shows_and_moves_its_own_box() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        let square = json!({"anchors": [{"x": 20, "y": 30}, {"x": 120, "y": 30}, {"x": 120, "y": 130}, {"x": 20, "y": 130}], "closed": true});
        let id = app.run("path.create", square).unwrap()["id"].as_u64().unwrap();
        let shown = texts(&mut app, show);
        assert!(shown.iter().any(|t| t == "70 pt") && shown.iter().any(|t| t == "100 pt"), "the whole path: {shown:?}");
        app.run("select.anchors", json!({"id": id, "anchors": [[0, 1]]})).unwrap();
        let shown = texts(&mut app, show);
        for v in ["120 pt", "30 pt", "0 pt"] {
            assert!(shown.iter().any(|t| t == v), "{v} in {shown:?}");
        }
        assert!(!shown.iter().any(|t| t == "70 pt" || t == "100 pt"), "{shown:?}");
        app.run("object.setBounds", json!({"x": 150, "reference": 4})).unwrap();
        let n = first_selected(&app).unwrap();
        let Some(path) = n.path_data() else { panic!("a path") };
        let xs: Vec<f64> = path.subpaths[0].anchors.iter().map(|a| a.p.x).collect();
        assert_eq!(xs, [20.0, 150.0, 120.0, 20.0]);
    }

    /// #2398: the panel menu's Flip Horizontal and Flip Vertical mirror the selection across the
    /// reference point, as the panel's flip buttons do, not across its centre.
    #[test]
    fn menu_flips_keep_the_reference_point() {
        use crate::panels::tests_appearance::{click, frame_events};
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("shape.rectangle", json!({"x": 20, "y": 30, "width": 100, "height": 50})).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        // Top-right reference point: (120, 30).
        ctx.data_mut(|d| d.insert_temp(egui::Id::unique("refpt"), 2usize));
        let flip = |app: &mut VectorcraftApp, item: &str| {
            let texts = frame_events(&ctx, app, vec![], menu);
            let r = texts.iter().find(|(t, _)| t.trim_start() == item).unwrap_or_else(|| panic!("{item} in {texts:?}")).1;
            click(&ctx, app, r.center(), menu);
            app.selection_box().unwrap().rect
        };
        assert_eq!(flip(&mut app, "Flip Horizontal"), Rect::new(120.0, 30.0, 220.0, 80.0));
        assert_eq!(flip(&mut app, "Flip Vertical"), Rect::new(120.0, -20.0, 220.0, 30.0));
    }

    /// Clicks the item of the panel menu labelled `item` (with or without its check mark).
    fn menu_click(ctx: &egui::Context, app: &mut VectorcraftApp, item: &str) {
        use crate::panels::tests_appearance::{click, frame_events};
        let texts = frame_events(ctx, app, vec![], menu);
        let r = texts.iter().find(|(t, _)| t.trim_start_matches('✓').trim_start() == item).unwrap_or_else(|| panic!("{item} in {texts:?}")).1;
        click(ctx, app, r.center(), menu);
    }

    /// The panel menu's items as drawn, check marks included.
    fn menu_texts(ctx: &egui::Context, app: &mut VectorcraftApp) -> Vec<String> {
        crate::panels::tests_appearance::frame_events(ctx, app, vec![], menu).into_iter().map(|(t, _)| t).collect()
    }

    /// Types `value` into the field of `draw` that shows `shown`, and presses Enter.
    fn type_into(ctx: &egui::Context, app: &mut VectorcraftApp, draw: fn(&mut VectorcraftApp, &mut Ui), shown: &str, value: &str) {
        use crate::panels::tests_appearance::{click, frame_events, text_rect};
        let texts = frame_events(ctx, app, vec![], draw);
        click(ctx, app, text_rect(&texts, shown).center(), draw);
        let key = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
        frame_events(ctx, app, vec![key(egui::Key::A, egui::Modifiers::COMMAND)], draw);
        frame_events(ctx, app, vec![egui::Event::Text(value.into())], draw);
        frame_events(ctx, app, vec![key(egui::Key::Enter, egui::Modifiers::NONE)], draw);
        frame_events(ctx, app, vec![], draw);
    }

    /// The panel menu's Transform Object Only, Transform Pattern Only and Transform Both, one
    /// checked: a new X typed in the panel (or the Properties panel's Transform section) moves the
    /// pattern-filled rectangle and leaves its tiles, moves its tiles alone, or moves both. Until
    /// one is chosen the panels do as General › Transform Pattern Tiles says, as they did before
    /// the menu acted, and the menu checks Transform Both while it is on, Transform Object Only
    /// while it is off; a choice overrides it, and resetting the preferences returns to it.
    #[test]
    fn the_menu_chooses_whether_the_object_its_pattern_or_both_transform() {
        let tiles = |app: &VectorcraftApp, id: u64| {
            let n = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().clone();
            let vectorcraft_color::Paint::Pattern { xf, .. } = n.appearance.fill_paint() else { panic!("a pattern fill") };
            xf.as_coeffs()
        };
        let labels = ["Transform Object Only", "Transform Pattern Only", "Transform Both"];
        let modes = ["object", "pattern", "both"];
        let checked = |shown: &[String]| -> Vec<usize> { (0..3).filter(|i| shown.contains(&format!("✓ {}", labels[*i]))).collect() };
        for draw in [show as fn(&mut VectorcraftApp, &mut Ui), crate::panels::properties::transform_section] {
            for general in [false, true] {
                for pick in [None, Some(0), Some(1), Some(2)] {
                    let case = format!("General › Transform Pattern Tiles {general}, menu choice {pick:?}");
                    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
                    app.run("prefs.set", json!({"key": "transformPatternTiles", "value": general})).unwrap();
                    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
                    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
                    app.run("object.pattern.make", json!({"name": "Dots", "width": 20, "height": 20})).unwrap();
                    app.run("object.pattern.done", json!({})).unwrap();
                    let id = app.run("shape.rectangle", json!({"x": 100, "y": 40, "width": 100, "height": 60})).unwrap()["id"].as_u64().unwrap();
                    app.run("paint.setFill", json!({"ids": [id], "swatch": "Dots"})).unwrap();
                    app.run("select.set", json!({"ids": [id]})).unwrap();
                    let ctx = egui::Context::default();
                    crate::theme::install_fonts(&ctx);
                    let following = if general { 2 } else { 0 };
                    assert_eq!(checked(&menu_texts(&ctx, &mut app)), [following], "{case}: before a choice");
                    if let Some(item) = pick {
                        menu_click(&ctx, &mut app, labels[item]);
                    }
                    let item = pick.unwrap_or(following);
                    assert_eq!(app.session.prefs.transform_panel_patterns, pick.map_or("default", |i| modes[i]), "{case}");
                    assert_eq!(checked(&menu_texts(&ctx, &mut app)), [item], "{case}");
                    // The centre reference point: X 150 becomes 170.
                    type_into(&ctx, &mut app, draw, "150 pt", "170");
                    let x0 = app.selection_box().unwrap().rect.x0;
                    assert_eq!(x0, if item == 1 { 100.0 } else { 120.0 }, "{case}: the object");
                    assert_eq!(tiles(&app, id), [1.0, 0.0, 0.0, 1.0, if item == 0 { 0.0 } else { 20.0 }, 0.0], "{case}: the tiles");
                    // Reset (the General category): the menu follows General › Transform Pattern
                    // Tiles again.
                    app.run("prefs.reset", json!({"category": "General"})).unwrap();
                    assert_eq!(app.session.prefs.transform_panel_patterns, "default", "{case}: reset");
                    let following = if app.session.prefs.transform_pattern_tiles { 2 } else { 0 };
                    assert_eq!(checked(&menu_texts(&ctx, &mut app)), [following], "{case}: reset");
                }
            }
        }
    }

    /// Transform Object Only, Transform Pattern Only and Transform Both act on the rotation field
    /// of both panels and on the Transform panel's shear field as they do on X: the pattern-filled
    /// rectangle turns or shears with its tiles left as they are, its tiles alone, or both.
    #[test]
    fn the_menu_choice_also_applies_to_rotation_and_shear() {
        let tiles = |app: &VectorcraftApp, id: u64| {
            let n = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().clone();
            let vectorcraft_color::Paint::Pattern { xf, .. } = n.appearance.fill_paint() else { panic!("a pattern fill") };
            xf.as_coeffs()
        };
        let labels = ["Transform Object Only", "Transform Pattern Only", "Transform Both"];
        // (panel, which "0°" field: 0 the rotation, 1 the shear)
        let fields = [(show as fn(&mut VectorcraftApp, &mut Ui), 0), (show, 1), (crate::panels::properties::transform_section, 0)];
        for (draw, field) in fields {
            for (item, label) in labels.iter().enumerate() {
                let case = format!("{label}, {}", if field == 0 { "rotation" } else { "shear" });
                let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
                app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
                app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
                app.run("object.pattern.make", json!({"name": "Dots", "width": 20, "height": 20})).unwrap();
                app.run("object.pattern.done", json!({})).unwrap();
                let id = app.run("shape.rectangle", json!({"x": 100, "y": 40, "width": 100, "height": 60})).unwrap()["id"].as_u64().unwrap();
                app.run("paint.setFill", json!({"ids": [id], "swatch": "Dots"})).unwrap();
                app.run("select.set", json!({"ids": [id]})).unwrap();
                let ctx = egui::Context::default();
                crate::theme::install_fonts(&ctx);
                menu_click(&ctx, &mut app, label);
                let before = (app.selection_box().unwrap(), tiles(&app, id));
                // Click the field showing "0°" and type 30 over it.
                use crate::panels::tests_appearance::{click, frame_events};
                let texts = frame_events(&ctx, &mut app, vec![], draw);
                let at = texts.iter().filter(|(t, _)| t == "0°").nth(field).unwrap_or_else(|| panic!("{case}: {texts:?}")).1.center();
                click(&ctx, &mut app, at, draw);
                let key = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
                frame_events(&ctx, &mut app, vec![key(egui::Key::A, egui::Modifiers::COMMAND)], draw);
                frame_events(&ctx, &mut app, vec![egui::Event::Text("30".into())], draw);
                frame_events(&ctx, &mut app, vec![key(egui::Key::Enter, egui::Modifiers::NONE)], draw);
                frame_events(&ctx, &mut app, vec![], draw);
                let after = (app.selection_box().unwrap(), tiles(&app, id));
                assert_eq!(after.0 != before.0, item != 1, "{case}: the object {:?} → {:?}", before.0, after.0);
                // Turned or sheared tiles change their matrix's linear part; moved ones only its offset.
                assert_eq!(after.1[..4] != before.1[..4], item != 0, "{case}: the tiles {:?} → {:?}", before.1, after.1);
            }
        }
    }

    /// Use Registration Point for Symbol: with one symbol instance selected, the panel and the
    /// Properties panel's Transform section show and set X and Y at its symbol's registration point
    /// (here the top-left one) instead of the reference point (the centre).
    #[test]
    fn use_registration_point_for_symbol_measures_an_instance_from_its_registration_point() {
        for draw in [show as fn(&mut VectorcraftApp, &mut Ui), crate::panels::properties::transform_section] {
            let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
            app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
            app.run("shape.rectangle", json!({"x": 100, "y": 50, "width": 80, "height": 40})).unwrap();
            let id = app.run("symbol.new", json!({"name": "Corner", "registration": 0})).unwrap()["id"].as_u64().unwrap();
            app.run("select.set", json!({"ids": [id]})).unwrap();
            let ctx = egui::Context::default();
            crate::theme::install_fonts(&ctx);
            let shown = |app: &mut VectorcraftApp| {
                crate::panels::tests_appearance::frame_events(&ctx, app, vec![], draw).into_iter().map(|(t, _)| t).collect::<Vec<_>>()
            };
            let off = shown(&mut app);
            assert!(off.iter().any(|t| t == "140 pt") && off.iter().any(|t| t == "70 pt"), "the centre: {off:?}");
            assert!(menu_texts(&ctx, &mut app).contains(&"   Use Registration Point for Symbol".to_string()));
            menu_click(&ctx, &mut app, "Use Registration Point for Symbol");
            assert!(app.session.prefs.use_registration_point_for_symbol);
            assert!(menu_texts(&ctx, &mut app).contains(&"✓ Use Registration Point for Symbol".to_string()));
            let on = shown(&mut app);
            assert!(on.iter().any(|t| t == "100 pt") && on.iter().any(|t| t == "50 pt") && !on.iter().any(|t| t == "140 pt"), "the top-left: {on:?}");
            type_into(&ctx, &mut app, draw, "100 pt", "30");
            assert_eq!(app.selection_box().unwrap().rect, Rect::new(30.0, 50.0, 110.0, 90.0));
            // Any other selection: the reference point.
            app.run("shape.rectangle", json!({"x": 200, "y": 200, "width": 20, "height": 20})).unwrap();
            let rect = shown(&mut app);
            assert!(rect.iter().any(|t| t == "210 pt") && !rect.iter().any(|t| t == "200 pt"), "the centre: {rect:?}");
        }
    }

    /// Use Registration Point for Symbol on a reflected, a rotated and a sheared symbol instance:
    /// the panel and the Properties panel's Transform section show X and Y at the registration
    /// point where the instance's transform put it (not at the box's point of the same index), a
    /// new X moves it there, and their rotation field and flip buttons, and the panel menu's
    /// flips, turn and mirror the instance about it.
    #[test]
    fn use_registration_point_for_symbol_follows_a_transformed_instance() {
        let panels = [
            (show as fn(&mut VectorcraftApp, &mut Ui), "Flip Horizontal", "Flip Vertical"),
            (crate::panels::properties::transform_section, "Flip Along Horizontal Axis", "Flip Along Vertical Axis"),
        ];
        // How each instance is transformed after it is made: (command, params).
        let shapes = [
            ("object.reflect", json!({"axis": "vertical"})),
            ("object.rotate", json!({"angle": 90})),
            // About the registration point (the top-left corner), which then lies on the box's
            // top edge but on none of its nine reference points.
            ("object.shear", json!({"angle": 30, "axis": "horizontal", "origin": [100, 50]})),
        ];
        // The registration point (the art's top-left) where the instance's transform puts it.
        let registration = |app: &VectorcraftApp, id: u64| match &app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
            NodeKind::SymbolInstance { xf, .. } => *xf * vectorcraft_geom::Point::new(-vectorcraft_doc::SYMBOL_HALF, -vectorcraft_doc::SYMBOL_HALF),
            k => panic!("not an instance: {k:?}"),
        };
        let pt = |v: f64| format!("{} pt", (v * 1000.0).round() / 1000.0);
        let near = |a: vectorcraft_geom::Point, b: vectorcraft_geom::Point| (a - b).hypot() < 1e-6;
        for (draw, flip_h, flip_v) in panels {
            for (cmd, params) in &shapes {
                let case = format!("{cmd} {params}");
                let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
                app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
                app.run("shape.rectangle", json!({"x": 100, "y": 50, "width": 80, "height": 40})).unwrap();
                let id = app.run("symbol.new", json!({"name": "Corner", "registration": 0})).unwrap()["id"].as_u64().unwrap();
                app.run("select.set", json!({"ids": [id]})).unwrap();
                app.run(cmd, params.clone()).unwrap();
                app.run("prefs.set", json!({"key": "useRegistrationPointForSymbol", "value": true})).unwrap();
                let ctx = egui::Context::default();
                crate::theme::install_fonts(&ctx);
                ctx.enable_accesskit();
                let q = registration(&app, id);
                let shown: Vec<String> =
                    crate::panels::tests_appearance::frame_events(&ctx, &mut app, vec![], draw).into_iter().map(|(t, _)| t).collect();
                assert!(shown.contains(&pt(q.x)) && shown.contains(&pt(q.y)), "{case}: X/Y at {q:?} in {shown:?}");
                // A new X moves the registration point there.
                type_into(&ctx, &mut app, draw, &pt(q.x), &format!("{}", q.x - 40.0));
                let q = vectorcraft_geom::Point::new(q.x - 40.0, q.y);
                assert!(near(registration(&app, id), q), "{case}: moved to {q:?}, now {:?}", registration(&app, id));
                // The rotation field turns it about the registration point.
                let angle = app.selection_box().unwrap().angle;
                let texts = crate::panels::tests_appearance::frame_events(&ctx, &mut app, vec![], draw);
                let degrees = texts.iter().find(|(t, _)| t.ends_with('°')).unwrap_or_else(|| panic!("{case}: no angle in {texts:?}")).0.clone();
                let before = app.session.active().unwrap().revision;
                type_into(&ctx, &mut app, draw, &degrees, &format!("{}", angle + 30.0));
                assert_ne!(app.session.active().unwrap().revision, before, "{case}: turned");
                assert!(near(registration(&app, id), q), "{case}: rotated about {q:?}, now {:?}", registration(&app, id));
                // The flip buttons mirror it about the registration point.
                for label in [flip_h, flip_v] {
                    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| draw(&mut app, ui));
                    out.textures_delta.clear();
                    let nodes = &out.platform_output.accesskit_update.as_ref().expect("an accessibility tree").nodes;
                    let (_, n) = nodes
                        .iter()
                        .find(|(_, n)| n.label() == Some(label) && n.role() == egui::accesskit::Role::Button)
                        .unwrap_or_else(|| panic!("no button named {label:?}"));
                    let b = n.bounds().expect("bounds");
                    let at = egui::pos2((b.x0 + b.x1) as f32 * 0.5, (b.y0 + b.y1) as f32 * 0.5);
                    let before = app.session.active().unwrap().revision;
                    crate::panels::tests_appearance::click(&ctx, &mut app, at, draw);
                    assert_ne!(app.session.active().unwrap().revision, before, "{case}: {label} flipped");
                    assert!(near(registration(&app, id), q), "{case}: {label} about {q:?}, now {:?}", registration(&app, id));
                }
                // So do the panel menu's flips.
                for item in ["Flip Horizontal", "Flip Vertical"] {
                    menu_click(&ctx, &mut app, item);
                    assert!(near(registration(&app, id), q), "{case}: menu {item} about {q:?}, now {:?}", registration(&app, id));
                }
            }
        }
    }

    #[test]
    fn artboard_fields_keep_the_reference_point() {
        let r = Rect::new(0.0, 0.0, 300.0, 200.0);
        // Centre: a new X centres it there, a new W grows it both ways.
        assert_eq!(artboard_rect(r, 4, Some(200.0), None, None, None, false), Rect::new(50.0, 0.0, 350.0, 200.0));
        assert_eq!(artboard_rect(r, 4, None, None, Some(400.0), None, false), Rect::new(-50.0, 0.0, 350.0, 200.0));
        // Top-left: a new W keeps the left edge; linked, H follows.
        assert_eq!(artboard_rect(r, 0, None, None, Some(600.0), None, true), Rect::new(0.0, 0.0, 600.0, 400.0));
        // Bottom-right: a new Y puts the bottom edge there.
        assert_eq!(artboard_rect(r, 8, None, Some(500.0), None, None, false), Rect::new(0.0, 300.0, 300.0, 500.0));
    }

    #[test]
    fn constrain_proportions() {
        assert_eq!(constrained(100.0, 50.0, Some(200.0), None), (200.0, 100.0));
        assert_eq!(constrained(100.0, 50.0, None, Some(25.0)), (50.0, 25.0));
        assert_eq!(constrained(0.0, 50.0, Some(10.0), None), (10.0, 50.0));
    }

    /// The Transform panel names its Rotate and Shear inputs.
    #[test]
    fn rotate_and_shear_fields_are_named() {
        use egui_kittest::kittest::Queryable;
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 100, "height": 50})).unwrap();
        let mut fonts = false;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(280.0, 500.0)).with_accessibility_check(false).build_ui_state(
            move |ui, app: &mut VectorcraftApp| {
                if !std::mem::replace(&mut fonts, true) {
                    crate::theme::install_fonts(ui.ctx());
                    return;
                }
                show(app, ui);
            },
            app,
        );
        h.run_steps(3);
        let _ = h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Rotate");
        let _ = h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Shear");
    }
}
