//! Freehand gesture tools: Pencil (N), Paintbrush (B), Blob Brush (Shift+B), Eraser (Shift+E),
//! Knife, Smooth, Path Eraser and Join.
//!
//! Pencil and Paintbrush preview the fitted path live (`path.freehand` re-applied on the snapshot)
//! and commit on release; with Live curve fitting off, the Pencil's preview joins the stroke's
//! points with straight segments and the release fits them; Alt on release (or ending near the start) closes the path, and starting
//! near an end of a selected open path continues it. Shift held at the press draws a straight
//! Pencil line at 45° steps from the Constrain Angle instead, and Alt held at the press (with Alt
//! key toggles to Smooth Tool off) draws a straight line at any angle. The Pencil's new path starts and ends on
//! Smart Guides ([`DrawSnap`]: an anchor, a path, in line with the art), hovering too; the points
//! between follow the hand. The Paintbrush records the pen pressure of each point (a stroke a pen
//! pressed less than fully on sends `[x, y, pressure]` points) for pressure-sensitive brushes. The
//! other tools collect the drag polyline (shown as an overlay) and run
//! one command on release. With Shift held, the Blob Brush's points stay on the horizontal or
//! vertical line through the press point.

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_geom::Point;

use super::{FEEDBACK, eraser_area, points_json, polyline};
use crate::guides::DrawSnap;
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

pub struct GestureTool {
    /// How many points the last preview of the stroke had ([`crate::previews_at`]).
    previewed: usize,
    id: &'static str,
    points: Vec<Point>,
    /// The pen pressure of each of `points` (the Paintbrush only).
    pressures: Vec<f32>,
    active: bool,
    began: bool,
    /// A release where the pointer last dragged to would close the stroke because it ends near
    /// its start (Close Paths When Ends Are Within): the cursor shows the closing ring.
    near_start: bool,
    /// Pencil/Paintbrush continuing a selected open path: (path, continue from its start).
    extend: Option<(NodeId, bool)>,
    /// A Pencil stroke continuing a selected open path: the other paths selected at the press,
    /// whose open ends it connects to.
    connect_to: Vec<NodeId>,
    /// Pencil/Paintbrush starting on a selected path away from its ends: (path, Edit Selected
    /// Paths' distance in points); the stroke redraws the section it ends on (#2850).
    redraw: Option<(NodeId, f64)>,
    /// Fit tolerance in points (Pencil/Paintbrush/Smooth "Fidelity").
    pub fidelity: f64,
    /// Brush / eraser diameter in points (the Eraser's across its tip's long axis).
    pub size: f64,
    /// The angle of the Eraser tip's long axis, in degrees counter-clockwise from horizontal.
    pub angle: f64,
    /// The Eraser tip's short axis in percent of its long one (100: round).
    pub roundness: f64,
    /// Fill new pencil/brush strokes.
    pub fill: bool,
    /// "Keep selected": select the path a Pencil, Paintbrush or Blob Brush stroke draws, continues
    /// or merges into (on for the Pencil and Paintbrush, off for the Blob Brush).
    pub keep_selected: bool,
    /// "Alt key toggles to Smooth Tool": holding Alt with the Pencil tool smooths selected paths
    /// instead of closing the path (Pencil only, off by default).
    pub alt_smooth: bool,
    /// "Live preview" (Pencil only, on by default): the stroke shows with its fill and stroke as it
    /// is drawn. Off, only a thin line follows the pen while drawing, and the path takes its fill
    /// and stroke on release.
    pub live_preview: bool,
    /// "Live curve fitting": the Pencil's preview fits the stroke while it is drawn; off, the
    /// preview shows the points joined by straight segments and the release fits them (Pencil
    /// only, on by default).
    pub live_fit: bool,
    /// Active drag is smoothing with the Pencil tool.
    smoothing: bool,
    /// Active Pencil drag draws a straight line (Shift or Alt held at the press): the stroke is
    /// the press point and the pointer.
    straight: bool,
    /// The straight line is Alt's: the pointer is not constrained to 45° steps, and the line's end
    /// snaps to Smart Guides as a stroke's end does.
    unconstrained: bool,
    /// Where the pointer was for each Blob Brush point dragged since Shift went down: the last
    /// `shift_raw.len()` of `points`.
    shift_raw: Vec<Point>,
    /// Where the pointer was for the Shift-constrained point the next [`Self::push`] adds.
    shift_next: Option<Point>,
    /// The angle of the line the dragged points were last moved onto: while it holds they are on
    /// it, and the next drag event need not move them again.
    shift_line: Option<f64>,
    /// "Merge only with selection": a Blob Brush stroke merges only with selected paths.
    pub merge_only_selection: bool,
    /// "Edit selected paths within N pixels".
    pub edit_within: f64,
    /// "Close paths when ends are within N pixels".
    pub close_within: f64,
    /// Smart Guides for the Pencil's ends.
    snap: DrawSnap,
}

/// The paths a stroke can redraw for selected object `id`: the path itself, or a compound path's
/// member paths.
fn redraw_targets(cx: &ToolContext, id: NodeId) -> Vec<NodeId> {
    let Some(n) = cx.doc.node(id) else { return vec![] };
    match &n.kind {
        NodeKind::Path { .. } => vec![id],
        NodeKind::Compound { children, .. } => children.iter().filter(|c| c.path_data().is_some()).map(|c| c.id).collect(),
        _ => vec![],
    }
}

impl GestureTool {
    pub fn new(id: &str) -> Self {
        let id: &'static str = match id {
            "paintbrush" => "paintbrush",
            "blobBrush" => "blobBrush",
            "eraser" => "eraser",
            "knife" => "knife",
            "smooth" => "smooth",
            "pathEraser" => "pathEraser",
            "join" => "join",
            _ => "pencil",
        };
        let fidelity = if id == "smooth" { 2.5 } else { 1.5 };
        Self {
            id,
            previewed: 0,
            points: vec![],
            pressures: vec![],
            active: false,
            began: false,
            near_start: false,
            extend: None,
            connect_to: vec![],
            redraw: None,
            fidelity,
            size: 10.0,
            angle: 0.0,
            roundness: 100.0,
            fill: false,
            keep_selected: id != "blobBrush",
            // Off, as Pencil Tool Options ships it: Alt keeps drawing a closed path until it is on.
            alt_smooth: false,
            live_preview: true,
            live_fit: true,
            smoothing: false,
            straight: false,
            unconstrained: false,
            shift_raw: vec![],
            shift_next: None,
            shift_line: None,
            merge_only_selection: false,
            edit_within: 12.0,
            close_within: 15.0,
            snap: DrawSnap::default(),
        }
    }

    /// Do the ends of its strokes snap (the Pencil)?
    fn snaps(&self) -> bool {
        self.id == "pencil"
    }

    fn draws_path(&self) -> bool {
        matches!(self.id, "pencil" | "paintbrush")
    }

    /// Does a stroke show as the path it draws while it is drawn? Not with the Pencil's Live preview
    /// off: then the drag shows as a thin line (an overlay) until the release.
    fn previews(&self) -> bool {
        self.id != "pencil" || self.live_preview
    }

    /// Add a stroke sample.
    fn push(&mut self, p: Point, pressure: f32) {
        let raw = self.shift_next.take();
        if self.points.is_empty() {
            self.shift_raw.clear();
        } else if let Some(raw) = raw {
            self.shift_raw.push(raw);
        }
        self.points.push(p);
        if self.id == "paintbrush" {
            self.pressures.push(pressure);
        }
    }

    /// Would a press with `m` held draw a straight Pencil line? Shift or Alt held, unless Alt lends
    /// the Smooth tool: the rule `PointerKind::Down` sets `straight` by.
    fn straight_at_press(&self, m: Mods) -> bool {
        self.id == "pencil" && !(self.alt_smooth && m.alt) && (m.shift || m.alt)
    }

    /// Where the stroke takes a pointer at `p`. For the Blob Brush with Shift held, that is the
    /// horizontal or vertical line through the press point that is nearer in angle to `p` (90°
    /// steps from the Constrain Angle), at `p`'s distance from the press point. The points dragged
    /// since Shift went down move to that line too, each at its pointer position's distance and on
    /// that position's side of the press point; a pointer at the press point leaves them on their
    /// line. Without Shift the stroke takes `p`.
    fn constrained(&mut self, cx: &ToolContext, p: Point, m: Mods) -> Point {
        self.shift_next = None;
        if self.straight && self.unconstrained {
            return p;
        }
        if self.straight
            && let Some(&start) = self.points.first()
        {
            return start + vectorcraft_geom::constrain_angle_from(p - start, 45.0, cx.constrain_angle);
        }
        let Some(&start) = self.points.first().filter(|_| self.id == "blobBrush" && m.shift) else {
            self.shift_raw.clear();
            self.shift_line = None;
            return p;
        };
        self.shift_next = Some(p);
        // The press point is on both lines: the points dragged before stay on theirs.
        if (p - start).hypot() < vectorcraft_geom::EPS {
            return start;
        }
        let d = vectorcraft_geom::constrain_angle_from(p - start, 90.0, cx.constrain_angle);
        // `line` is the line's angle, measured counterclockwise on screen as the Constrain Angle is measured.
        let line = (-d.y).atan2(d.x).to_degrees();
        let on_line = |q: Point| start + vectorcraft_geom::constrain_angle_from(q - start, 180.0, line);
        if self.shift_line != Some(line) {
            for (q, raw) in self.points.iter_mut().skip(1).rev().zip(self.shift_raw.iter().rev()) {
                *q = on_line(*raw);
            }
            self.shift_line = Some(line);
        }
        on_line(p)
    }

    /// Forget the stroke.
    fn clear(&mut self) {
        self.points.clear();
        self.pressures.clear();
        self.previewed = 0;
        self.shift_line = None;
    }

    /// The stroke's points, each `[x, y, pressure]` when a pen pressed less than fully anywhere
    /// along it (a mouse always presses fully).
    fn stroke_json(&self) -> Value {
        if self.pressures.len() == self.points.len() && self.pressures.iter().any(|p| *p < 1.0) {
            Value::Array(self.points.iter().zip(&self.pressures).map(|(p, f)| json!([p.x, p.y, (f64::from(*f) * 1000.0).round() / 1000.0])).collect())
        } else {
            points_json(&self.points)
        }
    }

    fn label(&self) -> &'static str {
        match self.id {
            "paintbrush" => "Paintbrush",
            "blobBrush" => "Blob Brush",
            "eraser" => "Eraser",
            "knife" => "Knife",
            "smooth" => "Smooth",
            "pathEraser" => "Path Eraser",
            "join" => "Join",
            _ => "Pencil",
        }
    }

    /// The selected open path whose end is near `p` (Edit Selected Paths, off at 0 pixels), and
    /// that end.
    fn find_extend(&self, cx: &ToolContext, p: Point) -> Option<(NodeId, bool, Point)> {
        self.find_end(cx, p, &cx.selection.objects)
    }

    /// [`Self::find_extend`] among `ids`.
    fn find_end(&self, cx: &ToolContext, p: Point, ids: &[NodeId]) -> Option<(NodeId, bool, Point)> {
        if self.edit_within <= 0.0 {
            return None;
        }
        let tol = cx.tol(self.edit_within);
        let editable = cx.doc.editable();
        for id in ids {
            let Some(pd) = cx.doc.node(*id).and_then(|n| n.path_data()) else { continue };
            if !editable(*id) {
                continue;
            }
            let Some(sp) = pd.subpaths.iter().rev().find(|s| !s.closed && !s.anchors.is_empty()) else { continue };
            let (first, last) = (sp.anchors[0].p, sp.anchors[sp.anchors.len() - 1].p);
            if p.distance(last) <= tol {
                return Some((*id, false, last));
            }
            if p.distance(first) <= tol {
                return Some((*id, true, first));
            }
        }
        None
    }

    /// The selected path whose outline is near `p` (Edit Selected Paths, off at 0 pixels).
    fn find_redraw(&self, cx: &ToolContext, p: Point) -> Option<(NodeId, f64)> {
        let tol = cx.tol(self.edit_within);
        if self.edit_within <= 0.0 || !tol.is_finite() {
            return None;
        }
        // A selected compound path (a shape with a hole) redraws the member path the stroke starts
        // on, the nearest of them: a compound path has no path of its own.
        cx.selection.objects.iter().find_map(|id| {
            redraw_targets(cx, *id)
                .into_iter()
                .filter(|m| cx.doc.is_editable(*m))
                .filter_map(|m| Some((m, cx.doc.node(m)?.path_data()?.nearest(p)?.4)))
                .filter(|(_, d)| *d <= tol)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(m, _)| (m, tol))
        })
    }

    /// Does a stroke that ends at `end` end near its start (Close Paths When Ends Are Within, off
    /// at 0 pixels)?
    fn ends_near_start(&self, cx: &ToolContext, end: Point) -> bool {
        self.close_within > 0.0 && self.points.len() > 2 && self.points[0].distance(end) <= cx.tol(self.close_within)
    }

    /// Does a release now, with the stroke ending at `end`, close a freehand stroke because it ends
    /// near its start (`finish`)? A stroke that redraws a selected path closes only when the engine
    /// finds it does not end on that path, so it counts as not closing.
    fn release_closes_near_start(&self, cx: &ToolContext, end: Point) -> bool {
        self.draws_path() && !self.smoothing && !self.straight && self.extend.is_none() && self.redraw.is_none() && self.ends_near_start(cx, end)
    }

    /// Parameters of `path.freehand` for the current stroke.
    pub fn freehand_params(&self, closed: bool) -> Value {
        let mut v = json!({
            "points": self.stroke_json(),
            "fidelity": self.fidelity,
            "closed": closed,
            "style": if self.id == "paintbrush" { "brush" } else { "pencil" },
            "fill": self.fill,
            "keepSelected": self.keep_selected,
        });
        if let Some((id, start)) = self.extend {
            v["extend"] = json!({"id": id.0, "end": if start { "start" } else { "end" }});
        } else if let Some((id, within)) = self.redraw {
            v["redraw"] = json!({"id": id.0, "within": within});
        }
        v
    }

    fn finish(&mut self, cx: &ToolContext, m: Mods) -> Vec<Action> {
        let pts = &self.points;
        let has_sel = !cx.selection.is_empty();
        let tol = cx.tol(self.edit_within);
        if self.smoothing {
            if has_sel && pts.len() >= 2 {
                return vec![Action::Exec("path.smoothRegion".into(), json!({"points": points_json(pts), "radius": tol, "fidelity": self.fidelity}))];
            }
            return vec![];
        }
        match self.id {
            "pencil" | "paintbrush" => {
                if !self.began {
                    return vec![];
                }
                // Close Paths When Ends Are Within is off at 0 pixels.
                let near = pts.last().is_some_and(|&end| self.ends_near_start(cx, end));
                // A straight line has two points: closing it would draw it back over itself.
                let closed = !self.straight && ((m.alt && !self.alt_smooth) || (self.extend.is_none() && near));
                let mut params = self.freehand_params(closed);
                // A Pencil stroke that continues a selected path and ends on an open end of another
                // path selected at the press connects the two.
                let connect = pts.last().and_then(|&end| self.find_end(cx, end, &self.connect_to));
                if let Some((other, at_start, _)) = connect {
                    params["connect"] = json!({"id": other.0, "end": if at_start { "start" } else { "end" }});
                }
                vec![Action::Preview("path.freehand".into(), params), Action::Commit]
            }
            "blobBrush" => vec![Action::Exec(
                "path.blob".into(),
                json!({
                    "points": points_json(pts),
                    "size": self.size,
                    "keepSelected": self.keep_selected,
                    "mergeOnlySelection": self.merge_only_selection
                }),
            )],
            "eraser" => vec![Action::Exec(
                "path.eraseRegion".into(),
                json!({"points": points_json(pts), "size": self.size, "angle": self.angle, "roundness": self.roundness}),
            )],
            "knife" if pts.len() >= 2 => vec![Action::Exec("path.knife".into(), json!({"points": points_json(pts)}))],
            "smooth" if has_sel && pts.len() >= 2 => {
                vec![Action::Exec("path.smoothRegion".into(), json!({"points": points_json(pts), "radius": tol, "fidelity": self.fidelity}))]
            }
            "pathEraser" if has_sel => vec![Action::Exec("path.eraseSegments".into(), json!({"points": points_json(pts), "width": cx.tol(8.0)}))],
            "join" if pts.len() >= 2 => vec![Action::Exec("path.joinScrub".into(), json!({"points": points_json(pts), "tolerance": tol}))],
            _ => vec![],
        }
    }
}

impl Tool for GestureTool {
    fn id(&self) -> &'static str {
        self.id
    }
    fn busy(&self) -> bool {
        self.active
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let p = ev.pos;
        match ev.kind {
            PointerKind::Move if self.snaps() => {
                self.snap.hover(cx, p, &[], None);
                vec![]
            }
            PointerKind::Down => {
                self.active = true;
                self.began = false;
                self.near_start = false;
                self.smoothing = self.id == "pencil" && self.alt_smooth && ev.mods.alt;
                // Alt draws a straight line only while it doesn't lend the Smooth tool; with Shift
                // too, the line keeps Shift's 45° steps.
                self.straight = self.id == "pencil" && !self.smoothing && (ev.mods.shift || ev.mods.alt);
                self.unconstrained = self.straight && !ev.mods.shift;
                let extend = if self.draws_path() && !self.smoothing { self.find_extend(cx, p) } else { None };
                self.extend = extend.map(|(id, at_start, _)| (id, at_start));
                // The stroke's previews select the path it continues, so the others are kept now.
                self.connect_to = match self.extend {
                    Some((id, _)) if self.id == "pencil" => cx.selection.objects.iter().copied().filter(|o| *o != id).collect(),
                    _ => vec![],
                };
                self.redraw = if self.draws_path() && !self.smoothing && self.extend.is_none() { self.find_redraw(cx, p) } else { None };
                // A path continued starts at its end, one redrawn on it; a new one where it snaps.
                // A straight line is constrained from the end it continues, so it starts there.
                let start = if let Some((_, _, end)) = extend.filter(|_| self.straight) {
                    end
                } else if self.snaps() && !self.smoothing && self.extend.is_none() && self.redraw.is_none() {
                    self.snap.press(cx, p, &[], None)
                } else {
                    p
                };
                self.clear();
                self.push(start, ev.pressure);
                vec![]
            }
            PointerKind::Drag => {
                if !self.active {
                    return vec![];
                }
                let p = self.constrained(cx, p, ev.mods);
                if !self.began && !self.straight && self.id == "pencil" && self.alt_smooth && ev.mods.alt {
                    self.smoothing = true;
                }
                let mut end = p;
                if self.snaps() && !self.smoothing && !self.straight {
                    // Where the stroke would end: shown, taken on release.
                    end = self.snap.drag(cx, p, None);
                }
                let p = if self.straight && self.unconstrained && self.snaps() {
                    // Alt's line ends where a stroke would: shown, taken on release.
                    self.snap.drag(cx, p, None)
                } else {
                    p
                };
                if self.straight {
                    if !self.unconstrained {
                        // The press's Smart Guides would stay on screen: the line's end doesn't snap.
                        self.snap.clear();
                    }
                    // The line runs from the press point to the pointer, wherever the drag went.
                    if self.points.first().is_some_and(|s| s.distance(p) < cx.tol(1.0)) {
                        // Back at the press point there is no line: the one shown goes.
                        self.points.truncate(1);
                        return if std::mem::take(&mut self.began) { vec![Action::Cancel] } else { vec![] };
                    }
                    self.points.truncate(1);
                } else if self.points.last().is_some_and(|l| l.distance(p) < cx.tol(1.0)) || self.points.len() >= crate::MAX_STROKE_SAMPLES {
                    // A release here keeps the last point unless it snaps (`Up`), so the ring
                    // follows the point `finish` will test.
                    let kept = self.points.last().copied().filter(|l| l.distance(p) < cx.tol(1.0) && !self.snaps());
                    let release_end = kept.unwrap_or(end);
                    self.near_start = self.release_closes_near_start(cx, release_end);
                    return vec![];
                }
                self.push(p, ev.pressure);
                self.near_start = self.release_closes_near_start(cx, end);
                if !self.draws_path() || self.smoothing {
                    return vec![];
                }
                let mut out = vec![];
                if !self.began {
                    self.began = true;
                    out.push(Action::Begin(self.label().into()));
                }
                // With the Pencil's Live preview off a thin line shows instead (`overlays`).
                if self.previews() && crate::previews_at(self.points.len()) {
                    self.previewed = self.points.len();
                    let mut v = self.freehand_params(false);
                    if !self.live_fit {
                        // Live curve fitting off: the release fits the stroke (`finish`).
                        v["fit"] = json!(false);
                    }
                    out.push(Action::Preview("path.freehand".into(), v));
                }
                out
            }
            PointerKind::Up => {
                if !self.active {
                    return vec![];
                }
                let p = self.constrained(cx, p, ev.mods);
                let end = if self.snaps() && !self.smoothing && (!self.straight || self.unconstrained) { self.snap.drag(cx, p, None) } else { p };
                self.snap.clear();
                let mut cancel = false;
                if self.straight {
                    self.points.truncate(1);
                    if self.points.first().is_some_and(|s| s.distance(end) >= cx.tol(1.0)) {
                        self.push(end, ev.pressure);
                    } else {
                        cancel = self.began;
                    }
                } else if self.points.last().is_some_and(|l| l.distance(p) >= cx.tol(1.0)) {
                    self.push(end, ev.pressure);
                } else if self.snaps()
                    && !self.smoothing
                    && self.points.len() > 1
                    && let Some(last) = self.points.last_mut()
                {
                    // The last step was where the button went up: it moves where that snaps.
                    *last = end;
                }
                let out = if cancel { vec![Action::Cancel] } else { self.finish(cx, ev.mods) };
                self.active = false;
                self.began = false;
                self.near_start = false;
                self.smoothing = false;
                self.straight = false;
                self.unconstrained = false;
                self.clear();
                self.extend = None;
                self.connect_to.clear();
                self.redraw = None;
                out
            }
            _ => vec![],
        }
    }
    /// The freehand tools build their stroke from every position the pointer passed; the Join tool
    /// only needs where its scribble crosses the ends.
    fn wants_samples(&self) -> bool {
        matches!(self.id, "pencil" | "paintbrush" | "blobBrush" | "eraser" | "knife" | "smooth" | "pathEraser")
    }
    /// `[` and `]` are the Blob Brush's and the Eraser's: their size (#1661).
    fn claims_key(&self, _cx: &ToolContext, key: ToolKey) -> bool {
        matches!(key, ToolKey::BracketLeft | ToolKey::BracketRight) && matches!(self.id, "blobBrush" | "eraser")
    }
    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _m: Mods) -> Vec<Action> {
        match key {
            ToolKey::Escape if self.active => {
                let began = self.began;
                self.snap.clear();
                self.active = false;
                self.began = false;
                self.near_start = false;
                self.smoothing = false;
                self.straight = false;
                self.unconstrained = false;
                self.clear();
                if began { vec![Action::Cancel] } else { vec![] }
            }
            ToolKey::BracketLeft if matches!(self.id, "blobBrush" | "eraser") => {
                self.size = (self.size - 1.0).max(1.0);
                vec![]
            }
            ToolKey::BracketRight if matches!(self.id, "blobBrush" | "eraser") => {
                self.size = (self.size + 1.0).min(1000.0);
                vec![]
            }
            _ => vec![],
        }
    }
    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
        let began = self.began;
        // Drags past the last preview are in the stroke kept, too, and with Live curve fitting
        // off the stroke kept is fitted.
        let stale = (began && (self.previewed != self.points.len() || !self.live_fit) && self.draws_path() && !self.smoothing)
            .then(|| Action::Preview("path.freehand".into(), self.freehand_params(false)));
        self.snap.clear();
        self.active = false;
        self.near_start = false;
        self.began = false;
        self.straight = false;
        self.unconstrained = false;
        self.clear();
        if began { stale.into_iter().chain([Action::Commit]).collect() } else { vec![] }
    }
    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let mut o = self.snap.guides().to_vec();
        // A path being drawn shows itself, except with the Pencil's Live preview off.
        if !self.active || self.points.len() < 2 || (self.began && self.previews()) {
            return o;
        }
        let width = match self.id {
            "blobBrush" | "eraser" => (self.size * cx.zoom) as f32,
            _ => 1.0,
        };
        let color = if self.id == "eraser" { [0x80, 0x80, 0x80] } else { FEEDBACK };
        // A tip that is not round shows the outline of the area it covers.
        if self.id == "eraser" && self.roundness < 100.0 {
            let area = eraser_area(&self.points, self.size, self.angle, self.roundness);
            o.push(Overlay::Path { path: area.to_bezpath(), color, width: 1.0, dashed: false });
            return o;
        }
        o.push(Overlay::Path { path: polyline(&self.points), color, width, dashed: self.id == "knife" });
        if cx.measurement_labels && self.id == "blobBrush" {
            let last = self.points[self.points.len() - 1];
            let length = self.points.windows(2).map(|w| w[0].distance(w[1])).sum();
            o.push(Overlay::Measure { p: last + vectorcraft_geom::Vec2::new(cx.tol(12.0), cx.tol(12.0)), text: format!("L: {}", cx.len(length)) });
        }
        o
    }
    fn cursor(&self, cx: &ToolContext, p: Point, m: Mods) -> Cursor {
        // A drag that smooths, or Alt before a Pencil stroke has begun to draw (a stroke that
        // has begun keeps drawing with Alt).
        if self.smoothing || (self.id == "pencil" && self.alt_smooth && m.alt && !self.began) {
            return Cursor::Smooth;
        }
        // Before a press that would draw a straight line: over an end of a selected open path,
        // the line continues that path from the end (`PointerKind::Down`).
        if !self.active && self.straight_at_press(m) {
            return if self.find_extend(cx, p).is_some() { Cursor::PencilContinue } else { Cursor::PencilStraight };
        }
        // Alt held at the release closes the path (`finish`).
        // A straight line stays open, so its release has nothing to close, and before a press Alt
        // gives the Pencil a straight line.
        if self.draws_path() && !self.alt_smooth && m.alt && !self.straight && (self.active || self.id != "pencil") {
            return if self.id == "paintbrush" { Cursor::PaintbrushClose } else { Cursor::PencilClose };
        }
        // A release near a freehand stroke's start closes it too (`finish`): the ring shows while
        // the pointer is there.
        if self.draws_path() && self.active && self.near_start {
            return if self.id == "paintbrush" { Cursor::PaintbrushClose } else { Cursor::PencilClose };
        }
        match self.id {
            "paintbrush" => Cursor::Paintbrush,
            "blobBrush" => Cursor::BlobBrush,
            "eraser" => Cursor::Eraser,
            "knife" => Cursor::Knife,
            "smooth" => Cursor::Smooth,
            "pathEraser" => Cursor::PathEraser,
            "join" => Cursor::Join,
            // Before a press that draws a new path the Pencil shows its star; over a selected path
            // that the press continues from an end or redraws (`PointerKind::Down`), it does not.
            _ if !self.active && self.find_extend(cx, p).is_none() && self.find_redraw(cx, p).is_none() => Cursor::PencilNew,
            _ => Cursor::Pencil,
        }
    }
    fn options(&self) -> Value {
        match self.id {
            "pencil" => {
                json!({
                    "fidelity": self.fidelity,
                    "fill": self.fill,
                    "altSmooth": self.alt_smooth,
                    "livePreview": self.live_preview,
                    "liveFit": self.live_fit,
                    "keepSelected": self.keep_selected,
                    "editWithin": self.edit_within,
                    "closeWithin": self.close_within
                })
            }
            "paintbrush" => {
                json!({
                    "fidelity": self.fidelity,
                    "fill": self.fill,
                    "keepSelected": self.keep_selected,
                    "editWithin": self.edit_within,
                    "closeWithin": self.close_within
                })
            }
            "smooth" => json!({"fidelity": self.fidelity}),
            "blobBrush" => json!({"size": self.size, "keepSelected": self.keep_selected, "mergeOnlySelection": self.merge_only_selection}),
            "eraser" => json!({"size": self.size, "angle": self.angle, "roundness": self.roundness}),
            _ => Value::Null,
        }
    }
    fn set_option(&mut self, key: &str, v: &Value) {
        match key {
            "fidelity" => self.fidelity = v.as_f64().unwrap_or(self.fidelity).clamp(super::min_fidelity(self.id), 100.0),
            "size" => self.size = v.as_f64().unwrap_or(self.size).clamp(0.1, 1000.0),
            "angle" => self.angle = v.as_f64().unwrap_or(self.angle).clamp(-180.0, 180.0),
            "roundness" => self.roundness = v.as_f64().unwrap_or(self.roundness).clamp(1.0, 100.0),
            "fill" => self.fill = v.as_bool().unwrap_or(self.fill),
            "altSmooth" => self.alt_smooth = v.as_bool().unwrap_or(self.alt_smooth),
            "livePreview" if self.id == "pencil" => self.live_preview = v.as_bool().unwrap_or(self.live_preview),
            "liveFit" if self.id == "pencil" => self.live_fit = v.as_bool().unwrap_or(self.live_fit),
            "keepSelected" => self.keep_selected = v.as_bool().unwrap_or(self.keep_selected),
            "mergeOnlySelection" => self.merge_only_selection = v.as_bool().unwrap_or(self.merge_only_selection),
            "editWithin" => self.edit_within = v.as_f64().unwrap_or(self.edit_within).clamp(0.0, 100.0),
            "closeWithin" => self.close_within = v.as_f64().unwrap_or(self.close_within).clamp(0.0, 100.0),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_doc::Selection;

    fn drag(t: &mut GestureTool, cx: &ToolContext, pts: &[(f64, f64)], m: Mods) -> Vec<Vec<Action>> {
        let mut v = vec![t.pointer(cx, &PointerEvent::new(PointerKind::Down, pts[0].0, pts[0].1))];
        for &(x, y) in &pts[1..] {
            v.push(t.pointer(cx, &PointerEvent::new(PointerKind::Drag, x, y)));
        }
        let l = pts[pts.len() - 1];
        v.push(t.pointer(cx, &PointerEvent::new(PointerKind::Up, l.0, l.1).with_mods(m)));
        v
    }

    fn drag_held(t: &mut GestureTool, cx: &ToolContext, pts: &[(f64, f64)], m: Mods) -> Vec<Vec<Action>> {
        let mut v = vec![t.pointer(cx, &PointerEvent::new(PointerKind::Down, pts[0].0, pts[0].1).with_mods(m))];
        for &(x, y) in &pts[1..] {
            v.push(t.pointer(cx, &PointerEvent::new(PointerKind::Drag, x, y).with_mods(m)));
        }
        let l = pts[pts.len() - 1];
        v.push(t.pointer(cx, &PointerEvent::new(PointerKind::Up, l.0, l.1).with_mods(m)));
        v
    }

    /// The Pencil's new path starts and ends on Smart Guides (#506), the points between where
    /// the hand went.
    #[test]
    fn pencil_ends_snap_to_smart_guides() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = GestureTool::new("pencil");
        let acts = drag(&mut t, &cx, &[(102.0, 98.0), (151.0, 300.0), (250.0, 320.0), (198.0, 203.0)], Mods::default());
        let Some(Action::Preview(_, v)) = acts.last().and_then(|a| a.first()) else { panic!("{acts:?}") };
        assert_eq!(v["points"], json!([[100.0, 100.0], [151.0, 300.0], [250.0, 320.0], [200.0, 200.0]]));
        // The Paintbrush follows the hand all the way.
        let mut t = GestureTool::new("paintbrush");
        let acts = drag(&mut t, &cx, &[(102.0, 98.0), (151.0, 300.0)], Mods::default());
        let Some(Action::Preview(_, v)) = acts.last().and_then(|a| a.first()) else { panic!("{acts:?}") };
        assert_eq!(v["points"][0], json!([102.0, 98.0]));
    }

    /// #852: the Paintbrush sends each point's pen pressure; a mouse (always fully pressed) and
    /// the Pencil send plain points.
    #[test]
    fn paintbrush_sends_pen_pressure() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let stroke = |id: &str, pressures: [f32; 3]| {
            let mut t = GestureTool::new(id);
            let kinds = [PointerKind::Down, PointerKind::Drag, PointerKind::Up];
            let mut last = vec![];
            for ((k, x), f) in kinds.into_iter().zip([0.0, 40.0, 80.0]).zip(pressures) {
                last = t.pointer(&cx, &PointerEvent { pressure: f, ..PointerEvent::new(k, x, 0.0) });
            }
            let Some(Action::Preview(_, v)) = last.first() else { panic!("{last:?}") };
            v["points"].clone()
        };
        assert_eq!(stroke("paintbrush", [0.25, 0.5, 1.0]), json!([[0.0, 0.0, 0.25], [40.0, 0.0, 0.5], [80.0, 0.0, 1.0]]));
        assert_eq!(stroke("paintbrush", [1.0; 3]), json!([[0.0, 0.0], [40.0, 0.0], [80.0, 0.0]]));
        assert_eq!(stroke("pencil", [0.25, 0.5, 1.0]), json!([[0.0, 0.0], [40.0, 0.0], [80.0, 0.0]]));
    }

    #[test]
    fn pencil_previews_and_commits() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = GestureTool::new("pencil");
        let acts = drag(&mut t, &cx, &[(0.0, 0.0), (10.0, 5.0), (20.0, 0.0)], Mods::default());
        assert!(acts[0].is_empty());
        assert_eq!(acts[1][0], Action::Begin("Pencil".into()));
        assert!(matches!(&acts[1][1], Action::Preview(c, v) if c == "path.freehand" && v["points"].as_array().unwrap().len() == 2));
        let last = acts.last().unwrap();
        assert!(matches!(&last[0], Action::Preview(_, v) if v["closed"] == false && v["style"] == "pencil"));
        assert_eq!(last[1], Action::Commit);
        assert!(!t.busy());
    }

    /// Live curve fitting is the Pencil's, on by default. Off, the previews while a stroke is drawn
    /// ask for the points unfitted (`fit: false`), and the release's preview is fitted as before.
    #[test]
    fn pencil_with_live_curve_fitting_off_previews_the_stroke_unfitted() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let stroke = [(0.0, 0.0), (10.0, 5.0), (20.0, 0.0), (30.0, 6.0), (40.0, 0.0)];
        // The `fit` each preview sends: those while drawn, then the release's.
        let fits = |t: &mut GestureTool| {
            let acts = drag(t, &cx, &stroke, Mods::default());
            let fit = |a: &Action| if let Action::Preview(_, v) = a { Some(v.get("fit").cloned()) } else { None };
            let (drawn, release) = acts.split_at(acts.len() - 1);
            (drawn.iter().flatten().filter_map(fit).collect::<Vec<_>>(), release[0].iter().filter_map(fit).collect::<Vec<_>>())
        };
        let mut t = GestureTool::new("pencil");
        assert_eq!(t.options()["liveFit"], json!(true));
        let (drawn, release) = fits(&mut t);
        assert!(!drawn.is_empty() && drawn.iter().all(Option::is_none), "{drawn:?}");
        assert_eq!(release, [None]);
        t.set_option("liveFit", &json!(false));
        assert_eq!(t.options()["liveFit"], json!(false));
        let (drawn, release) = fits(&mut t);
        assert!(!drawn.is_empty() && drawn.iter().all(|f| *f == Some(json!(false))), "{drawn:?}");
        assert_eq!(release, [None], "the release fits the stroke");
        // Choosing another tool while the stroke is drawn keeps it fitted, too.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0));
        for &(x, y) in &stroke[1..] {
            t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, x, y));
        }
        let acts = t.deactivate(&cx);
        assert!(matches!(acts.as_slice(), [Action::Preview(_, v), Action::Commit] if v.get("fit").is_none()), "{acts:?}");
        // The Paintbrush has no such option, and its previews are fitted.
        let mut t = GestureTool::new("paintbrush");
        t.set_option("liveFit", &json!(false));
        assert!(t.options().get("liveFit").is_none());
        let (drawn, _) = fits(&mut t);
        assert!(!drawn.is_empty() && drawn.iter().all(Option::is_none), "{drawn:?}");
    }

    /// Keep selected is on by default, and every Pencil and Paintbrush preview sends it.
    #[test]
    fn pencil_and_paintbrush_send_keep_selected() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        for id in ["pencil", "paintbrush"] {
            let mut t = GestureTool::new(id);
            assert_eq!(t.options()["keepSelected"], json!(true), "{id}");
            for keep in [true, false] {
                t.set_option("keepSelected", &json!(keep));
                assert_eq!(t.options()["keepSelected"], json!(keep), "{id}");
                let acts = drag(&mut t, &cx, &[(0.0, 0.0), (10.0, 5.0), (20.0, 0.0)], Mods::default());
                assert!(acts[1..].iter().all(|a| a.iter().all(|a| !matches!(a, Action::Preview(_, v) if v["keepSelected"] != keep))), "{id}");
                assert!(matches!(&acts.last().unwrap()[0], Action::Preview(_, v) if v["keepSelected"] == keep), "{id}");
            }
        }
    }

    /// With the Pencil's Live preview off, a stroke previews nothing while it is drawn: a thin line
    /// follows the pen (an overlay), and the release draws the path with its fill and stroke. On (the
    /// default), every step previews the path and no line is shown. The Paintbrush has no such option.
    #[test]
    fn pencil_with_live_preview_off_shows_a_thin_line_until_the_release() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let previews = |acts: &[Action]| acts.iter().filter(|a| matches!(a, Action::Preview(..))).count();
        let line = |t: &GestureTool| t.overlays(&cx).iter().any(|o| matches!(o, Overlay::Path { width, .. } if *width == 1.0));
        let pts = [(0.0, 0.0), (10.0, 5.0), (20.0, 0.0), (30.0, 5.0)];
        for live in [true, false] {
            let mut t = GestureTool::new("pencil");
            assert_eq!(t.options()["livePreview"], json!(true), "on by default");
            t.set_option("livePreview", &json!(live));
            assert_eq!(t.options()["livePreview"], json!(live));
            t.pointer(&cx, &PointerEvent::new(PointerKind::Down, pts[0].0, pts[0].1));
            let mut during = vec![];
            for &(x, y) in &pts[1..] {
                during.extend(t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, x, y)));
            }
            assert!(matches!(during.first(), Some(Action::Begin(_))), "live {live}: {during:?}");
            assert_eq!(previews(&during), if live { 3 } else { 0 }, "live {live}: {during:?}");
            assert_eq!(line(&t), !live, "live {live}: the thin line shows only with Live preview off");
            let up = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 30.0, 5.0));
            assert!(matches!(up.as_slice(), [Action::Preview(c, _), Action::Commit] if c == "path.freehand"), "live {live}: {up:?}");
            assert!(!line(&t), "live {live}: no line after the release");
        }
        // Escape mid-stroke with Live preview off cancels the stroke it began.
        let mut t = GestureTool::new("pencil");
        t.set_option("livePreview", &json!(false));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 10.0, 5.0));
        assert_eq!(t.key(&cx, ToolKey::Escape, Mods::default()), vec![Action::Cancel]);
        // The Paintbrush keeps previewing: Live preview isn't one of its options.
        let mut b = GestureTool::new("paintbrush");
        assert!(b.options().get("livePreview").is_none());
        b.set_option("livePreview", &json!(false));
        b.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0));
        let acts = b.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 10.0, 5.0));
        assert_eq!(previews(&acts), 1, "{acts:?}");
    }

    /// Holding Alt shows the Paintbrush's closing cursor before the press, and the Pencil's and the
    /// Paintbrush's while drawing (before a press, Alt gives the Pencil its straight-line cursor); a release
    /// with Alt still held closes the path, and Alt let go before the release leaves it open.
    /// The freehand tools take every position the pointer passed (`wants_samples`); the Join tool
    /// doesn't need them.
    #[test]
    fn the_freehand_tools_want_every_sample() {
        for id in ["pencil", "paintbrush", "blobBrush", "eraser", "knife", "smooth", "pathEraser"] {
            assert!(GestureTool::new(id).wants_samples(), "{id}");
        }
        assert!(!GestureTool::new("join").wants_samples());
    }

    #[test]
    fn alt_shows_the_closing_cursor_and_a_release_with_alt_closes() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let alt = Mods { alt: true, ..Default::default() };
        let at = Point::new(10.0, 5.0);
        for (id, own, closing, style) in
            [("pencil", Cursor::Pencil, Cursor::PencilClose, "pencil"), ("paintbrush", Cursor::Paintbrush, Cursor::PaintbrushClose, "brush")]
        {
            let mut t = GestureTool::new(id);
            assert_eq!(t.cursor(&cx, at, Mods::default()), if id == "pencil" { Cursor::PencilNew } else { own }, "{id}");
            assert_eq!(t.cursor(&cx, at, alt), if id == "pencil" { Cursor::PencilStraight } else { closing }, "{id}");
            for alt_at_release in [true, false] {
                t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0));
                t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 10.0, 5.0).with_mods(alt));
                assert_eq!(t.cursor(&cx, at, alt), closing, "{id} while drawing");
                assert_eq!(t.cursor(&cx, at, Mods::default()), own, "{id} while drawing");
                let up = Mods { alt: alt_at_release, ..Default::default() };
                let acts = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 20.0, 0.0).with_mods(up));
                assert!(
                    matches!(&acts[0], Action::Preview(_, v) if v["closed"] == alt_at_release && v["style"] == style),
                    "{id}, Alt at the release {alt_at_release}: {acts:?}"
                );
            }
        }
        // With Alt key toggles to Smooth Tool on, Alt lends the Pencil the Smooth tool instead, until
        // a stroke has begun to draw: Alt then neither smooths nor closes, and the cursor stays the Pencil's.
        let mut t = GestureTool::new("pencil");
        t.set_option("altSmooth", &json!(true));
        assert_eq!(t.cursor(&cx, at, alt), Cursor::Smooth);
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 10.0, 5.0));
        assert_eq!(t.cursor(&cx, at, alt), Cursor::Pencil, "a stroke that has begun");
        let acts = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 20.0, 0.0).with_mods(alt));
        assert!(matches!(&acts[0], Action::Preview(_, v) if v["closed"] == false), "{acts:?}");
        // A smoothing drag keeps the Smooth cursor when Alt is let go.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0).with_mods(alt));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 10.0, 5.0).with_mods(alt));
        assert_eq!(t.cursor(&cx, at, Mods::default()), Cursor::Smooth, "a smoothing drag");
    }

    #[test]
    fn blob_brush_drag_shows_scrub_length_measurement_labels() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let on = ToolContext { measurement_labels: true, ..cx(&d, &s, &p) };
        let off = ToolContext { measurement_labels: false, ..cx(&d, &s, &p) };
        let mut t = GestureTool::new("blobBrush");
        t.pointer(&on, &PointerEvent::new(PointerKind::Down, 50.0, 150.0));
        t.pointer(&on, &PointerEvent::new(PointerKind::Drag, 150.0, 150.0));
        let want = format!("L: {}", on.len(100.0));
        assert!(t.overlays(&on).iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text == &want)));
        let mut quiet = GestureTool::new("blobBrush");
        quiet.pointer(&off, &PointerEvent::new(PointerKind::Down, 50.0, 150.0));
        quiet.pointer(&off, &PointerEvent::new(PointerKind::Drag, 150.0, 150.0));
        assert!(!quiet.overlays(&off).iter().any(|o| matches!(o, Overlay::Measure { .. })));
    }

    /// The Blob Brush has Keep selected and Merge only with selection, both off by default, and its
    /// stroke sends both.
    #[test]
    fn blob_brush_sends_keep_selected_and_merge_only_with_selection() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = GestureTool::new("blobBrush");
        assert_eq!((t.options()["keepSelected"].clone(), t.options()["mergeOnlySelection"].clone()), (json!(false), json!(false)));
        for (keep, only) in [(false, false), (true, false), (false, true)] {
            t.set_option("keepSelected", &json!(keep));
            t.set_option("mergeOnlySelection", &json!(only));
            let acts = drag(&mut t, &cx, &[(50.0, 150.0), (250.0, 150.0)], Mods::default());
            assert!(
                matches!(&acts[2][0], Action::Exec(c, v) if c == "path.blob" && v["keepSelected"] == keep && v["mergeOnlySelection"] == only),
                "{acts:?}"
            );
        }
        // The Eraser keeps Size, Angle and Roundness: a round tip by default.
        assert_eq!(GestureTool::new("eraser").options(), json!({"size": 10.0, "angle": 0.0, "roundness": 100.0}));
    }

    /// Shift keeps a Blob Brush stroke on the horizontal or vertical line through the press point,
    /// whichever is nearer to the pointer, at 90° steps from the Constrain Angle; the points
    /// dragged with Shift held move to the other line together, and each point follows the Shift
    /// state of its own event.
    #[test]
    fn blob_brush_shift_keeps_the_stroke_horizontal_or_vertical() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let mut cx = cx(&d, &s, &p);
        let shift = Mods { shift: true, ..Default::default() };
        let blob = |acts: &[Vec<Action>]| -> Vec<(f64, f64)> {
            let Some(Action::Exec(c, v)) = acts.last().and_then(|a| a.first()) else { panic!("{acts:?}") };
            assert_eq!(c, "path.blob");
            v["points"].as_array().unwrap().iter().map(|q| (q[0].as_f64().unwrap(), q[1].as_f64().unwrap())).collect()
        };
        let mut t = GestureTool::new("blobBrush");
        let pts = blob(&drag_held(&mut t, &cx, &[(0.0, 0.0), (30.0, 5.0), (50.0, 8.0)], shift));
        assert_eq!(pts.len(), 3);
        assert!(pts.iter().all(|q| q.1 == 0.0 && q.0 >= 0.0), "{pts:?}");
        let pts = blob(&drag_held(&mut t, &cx, &[(0.0, 0.0), (5.0, 30.0), (8.0, 50.0)], shift));
        assert!(pts.iter().all(|q| q.0.abs() < 1e-9 && q.1 >= 0.0), "{pts:?}");
        // A pointer that crosses to the vertical line takes the points before it along, each on
        // its own side of the press point and at its own distance.
        let pts = blob(&drag_held(&mut t, &cx, &[(0.0, 0.0), (-40.0, -2.0), (100.0, 99.0), (90.0, 120.0)], shift));
        assert_eq!(pts.len(), 4, "{pts:?}");
        assert!(pts.iter().all(|q| q.0.abs() < 1e-9), "{pts:?}");
        assert!((pts[1].1 + 1604f64.sqrt()).abs() < 1e-9 && (pts[2].1 - 19801f64.sqrt()).abs() < 1e-9 && (pts[3].1 - 150.0).abs() < 1e-9, "{pts:?}");
        // Without Shift the stroke follows the pointer.
        let pts = blob(&drag_held(&mut t, &cx, &[(0.0, 0.0), (30.0, 5.0), (50.0, 8.0)], Mods::default()));
        assert_eq!(pts, vec![(0.0, 0.0), (30.0, 5.0), (50.0, 8.0)]);
        // Shift pressed and let go mid-stroke constrains only the points dragged while it is down,
        // and a later press of Shift leaves the points of the earlier one where they are.
        let ev = |k, x, y, m| PointerEvent::new(k, x, y).with_mods(m);
        t.pointer(&cx, &ev(PointerKind::Down, 0.0, 0.0, Mods::default()));
        t.pointer(&cx, &ev(PointerKind::Drag, 20.0, 10.0, Mods::default()));
        t.pointer(&cx, &ev(PointerKind::Drag, 40.0, 10.0, shift));
        t.pointer(&cx, &ev(PointerKind::Drag, 60.0, 10.0, Mods::default()));
        let pts = blob(&[t.pointer(&cx, &ev(PointerKind::Up, 10.0, 80.0, shift))]);
        assert_eq!((pts.len(), pts[1], pts[3]), (5, (20.0, 10.0), (60.0, 10.0)), "{pts:?}");
        assert!(pts[2].1 == 0.0 && (pts[2].0 - 1700f64.sqrt()).abs() < 1e-9, "{pts:?}");
        assert!(pts[4].0.abs() < 1e-9 && (pts[4].1 - 6500f64.sqrt()).abs() < 1e-9, "{pts:?}");
        // The axes turn with the Constrain Angle.
        cx.constrain_angle = 30.0;
        let pts = blob(&drag_held(&mut t, &cx, &[(0.0, 0.0), (40.0, -20.0)], shift));
        let a = (-pts[1].1).atan2(pts[1].0).to_degrees();
        assert!((a - 30.0).abs() < 1e-9, "{pts:?}");
        // A pointer back at the press point leaves the stroke on its line.
        let pts = blob(&drag_held(&mut t, &cx, &[(0.0, 0.0), (40.0, -20.0), (0.0, 0.0)], shift));
        let a = (-pts[1].1).atan2(pts[1].0).to_degrees();
        assert!(pts.len() == 3 && pts[2] == (0.0, 0.0) && (a - 30.0).abs() < 1e-9, "{pts:?}");
    }

    /// #3799: Shift held at the press draws a straight Pencil line from the press point, at 45°
    /// steps from the Constrain Angle, previewed as it is dragged; the Paintbrush keeps following
    /// the hand.
    #[test]
    fn pencil_shift_draws_a_straight_line_at_45_degree_steps() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let mut cx = cx(&d, &s, &p);
        cx.smart_guides = false;
        let shift = Mods { shift: true, ..Default::default() };
        let pts = |acts: &[Vec<Action>]| -> Vec<Vec<(f64, f64)>> {
            acts.iter()
                .filter_map(|a| a.iter().find_map(|a| if let Action::Preview(_, v) = a { Some(v["points"].clone()) } else { None }))
                .map(|v| v.as_array().unwrap().iter().map(|q| (q[0].as_f64().unwrap(), q[1].as_f64().unwrap())).collect())
                .collect()
        };
        let angle = |q: (f64, f64), from: (f64, f64)| (-(q.1 - from.1)).atan2(q.0 - from.0).to_degrees();
        let mut t = GestureTool::new("pencil");
        // A wavy drag heading up and right at about 40°: a line at 45°, as long as the drag.
        let acts = drag_held(&mut t, &cx, &[(0.0, 0.0), (30.0, -10.0), (50.0, -60.0), (100.0, -84.0)], shift);
        let all = pts(&acts);
        assert!(all.iter().all(|v| v.len() == 2 && v[0] == (0.0, 0.0)), "{all:?}");
        let end = *all.last().unwrap().last().unwrap();
        assert!((angle(end, (0.0, 0.0)) - 45.0).abs() < 1e-9, "{end:?}");
        assert!((end.0.hypot(end.1) - 100f64.hypot(84.0)).abs() < 1e-9, "{end:?}");
        let last = acts.last().unwrap();
        assert!(matches!(&last[0], Action::Preview(_, v) if v["closed"] == false) && last[1] == Action::Commit, "{last:?}");
        // The steps turn with the Constrain Angle: at 30°, a drag at 40° gives 30°, one at 60° gives 75°.
        cx.constrain_angle = 30.0;
        for (to, want) in [((100.0, -84.0), 30.0), ((50.0, -86.6), 75.0)] {
            let all = pts(&drag_held(&mut t, &cx, &[(10.0, 10.0), (to.0 + 10.0, to.1 + 10.0)], shift));
            let end = *all.last().unwrap().last().unwrap();
            assert!((angle(end, (10.0, 10.0)) - want).abs() < 1e-9, "{to:?}: {all:?}");
        }
        // Without Shift, and with the Paintbrush, the stroke follows the hand.
        for (id, m) in [("pencil", Mods::default()), ("paintbrush", shift)] {
            let mut t = GestureTool::new(id);
            let all = pts(&drag_held(&mut t, &cx, &[(0.0, 0.0), (30.0, -10.0), (50.0, -60.0)], m));
            assert_eq!(all.last().unwrap().len(), 3, "{id}: {all:?}");
        }
    }

    /// #3799: a Shift line leaves no Smart Guides of its press on screen while it is dragged
    /// (its end doesn't snap), and Alt at its release leaves it open.
    #[test]
    fn pencil_shift_line_drops_the_press_guides_and_stays_open_with_alt() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let shift = Mods { shift: true, ..Default::default() };
        let mut t = GestureTool::new("pencil");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 102.0, 98.0).with_mods(shift));
        assert!(!t.overlays(&cx).is_empty(), "the press snaps to the corner and shows it");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 300.0, 140.0).with_mods(shift));
        assert!(t.overlays(&cx).is_empty(), "{:?}", t.overlays(&cx));
        let shift_alt = Mods { shift: true, alt: true, ..Default::default() };
        let acts = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 300.0, 140.0).with_mods(shift_alt));
        assert!(matches!(&acts[0], Action::Preview(_, v) if v["closed"] == false && v["points"].as_array().unwrap().len() == 2), "{acts:?}");
    }

    /// Before a press, the Pencil shows its straight-line cursor exactly when the press would draw a
    /// straight line; once the press is down, and with the Paintbrush, Shift leaves the tool's own
    /// cursor.
    #[test]
    fn pencil_shows_the_straight_line_cursor_when_a_press_draws_a_line() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let mut cx = cx(&d, &s, &p);
        cx.smart_guides = false;
        let at = Point::new(0.0, 0.0);
        let shift = Mods { shift: true, ..Default::default() };
        let alt = Mods { alt: true, ..Default::default() };
        let shift_alt = Mods { shift: true, alt: true, ..Default::default() };
        // The last drag's preview: two points for a line, all three for a freehand stroke, none
        // for a smoothing drag.
        let straight = |acts: &[Vec<Action>]| {
            acts[acts.len() - 2].iter().any(|a| matches!(a, Action::Preview(_, v) if v["points"].as_array().is_some_and(|q| q.len() == 2)))
        };
        for alt_smooth in [false, true] {
            for m in [Mods::default(), shift, alt, shift_alt] {
                let mut t = GestureTool::new("pencil");
                t.set_option("altSmooth", &json!(alt_smooth));
                let shown = t.cursor(&cx, at, m) == Cursor::PencilStraight;
                let drawn = straight(&drag_held(&mut t, &cx, &[(0.0, 0.0), (30.0, -10.0), (50.0, -60.0)], m));
                assert_eq!(shown, drawn, "altSmooth {alt_smooth}, {m:?}");
            }
        }
        let mut t = GestureTool::new("pencil");
        assert_eq!(t.cursor(&cx, at, Mods::default()), Cursor::PencilNew);
        assert_eq!(t.cursor(&cx, at, shift), Cursor::PencilStraight);
        assert_eq!(t.cursor(&cx, at, alt), Cursor::PencilStraight, "Alt alone draws a line at any angle");
        assert_eq!(t.cursor(&cx, at, shift_alt), Cursor::PencilStraight, "Alt at the release leaves a straight line open");
        // While the line is drawn, the Pencil's own cursor; after the release, Shift shows it again.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0).with_mods(shift));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 40.0, -10.0).with_mods(shift));
        assert_eq!(t.cursor(&cx, at, shift), Cursor::Pencil, "while the line is drawn");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 40.0, -10.0).with_mods(shift));
        assert_eq!(t.cursor(&cx, at, shift), Cursor::PencilStraight, "after the release");
        // Shift pressed during a freehand stroke doesn't make it a line.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 40.0, -10.0).with_mods(shift));
        assert_eq!(t.cursor(&cx, at, shift), Cursor::Pencil, "a freehand stroke");
        assert_eq!(GestureTool::new("paintbrush").cursor(&cx, at, shift), Cursor::Paintbrush);
        assert_eq!(Cursor::PencilStraight.precise(), Cursor::Crosshair, "Use Precise Cursors");
    }

    /// Before a press, the Pencil shows its new-path cursor (the star) exactly when the press draws
    /// a new path: over an end of a selected open path, which the stroke continues, and over a
    /// selected path, which it redraws, it does not. With nothing selected, with Edit Selected Paths
    /// off and away from the paths, it does; while a stroke is drawn, it does not.
    #[test]
    fn pencil_shows_the_star_before_a_press_that_draws_a_new_path() {
        let (mut d, _) = doc_with_rect();
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let line = vectorcraft_geom::PathData::from_bezpath(&vectorcraft_geom::BezPath::from_vec(vec![
            vectorcraft_geom::PathEl::MoveTo(Point::new(300.0, 300.0)),
            vectorcraft_geom::PathEl::LineTo(Point::new(400.0, 300.0)),
        ]));
        d.insert(Some(l), 1, vectorcraft_doc::Node::path(id, line, vectorcraft_doc::Appearance::default_art())).unwrap();
        let mut s = Selection::default();
        s.set([id]);
        let p = paint();
        let mut cx = cx(&d, &s, &p);
        cx.smart_guides = false;
        // The stroke edits the selected path when its previews continue or redraw it.
        let edits = |acts: &[Vec<Action>]| {
            acts.iter().flatten().any(|a| matches!(a, Action::Preview(_, v) if v.get("extend").is_some() || v.get("redraw").is_some()))
        };
        let (end, middle, away) = (Point::new(405.0, 303.0), Point::new(350.0, 304.0), Point::new(350.0, 360.0));
        for edit_within in [12.0, 0.0] {
            for at in [end, Point::new(296.0, 302.0), middle, away, Point::new(200.0, 200.0)] {
                let mut t = GestureTool::new("pencil");
                t.set_option("editWithin", &json!(edit_within));
                let shown = t.cursor(&cx, at, Mods::default()) == Cursor::PencilNew;
                let acts = drag(&mut t, &cx, &[(at.x, at.y), (360.0, 330.0), (420.0, 350.0)], Mods::default());
                assert_eq!(shown, !edits(&acts), "editWithin {edit_within} at {at:?}");
            }
        }
        let mut t = GestureTool::new("pencil");
        assert_eq!(t.cursor(&cx, end, Mods::default()), Cursor::Pencil, "over an end the stroke continues");
        assert_eq!(t.cursor(&cx, middle, Mods::default()), Cursor::Pencil, "over the path the stroke redraws");
        assert_eq!(t.cursor(&cx, away, Mods::default()), Cursor::PencilNew, "away from the path");
        // While a stroke is drawn, the Pencil's own cursor.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, away.x, away.y));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 380.0, 380.0));
        assert_eq!(t.cursor(&cx, Point::new(380.0, 380.0), Mods::default()), Cursor::Pencil, "while a stroke is drawn");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 380.0, 380.0));
        assert_eq!(t.cursor(&cx, away, Mods::default()), Cursor::PencilNew, "after the release");
        // With nothing selected, every press draws a new path.
        let none = Selection::default();
        let cx_none = crate::testutil::cx(&d, &none, &p);
        assert_eq!(GestureTool::new("pencil").cursor(&cx_none, middle, Mods::default()), Cursor::PencilNew, "nothing selected");
        // Shift held: a straight line, with its own cursor; the Paintbrush keeps its cursor.
        let shift = Mods { shift: true, ..Default::default() };
        assert_eq!(GestureTool::new("pencil").cursor(&cx, away, shift), Cursor::PencilStraight);
        assert_eq!(GestureTool::new("paintbrush").cursor(&cx, away, Mods::default()), Cursor::Paintbrush);
    }

    /// #3799: a Shift line started on an end of a selected open path continues the path from that
    /// end, constrained from the end itself.
    #[test]
    fn pencil_shift_line_continues_a_selected_path_from_its_end() {
        let (mut d, _) = doc_with_rect();
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let line = vectorcraft_geom::PathData::from_bezpath(&vectorcraft_geom::BezPath::from_vec(vec![
            vectorcraft_geom::PathEl::MoveTo(Point::new(300.0, 300.0)),
            vectorcraft_geom::PathEl::LineTo(Point::new(400.0, 300.0)),
        ]));
        d.insert(Some(l), 1, vectorcraft_doc::Node::path(id, line, vectorcraft_doc::Appearance::default_art())).unwrap();
        let mut s = Selection::default();
        s.set([id]);
        let p = paint();
        let mut cx = cx(&d, &s, &p);
        cx.smart_guides = false;
        let mut t = GestureTool::new("pencil");
        // Pressed 5 pt off the end, released 100 pt right and 4 pt above it.
        let acts = drag_held(&mut t, &cx, &[(405.0, 303.0), (450.0, 290.0), (500.0, 296.0)], Mods { shift: true, ..Default::default() });
        let Some(Action::Preview(_, v)) = acts.last().and_then(|a| a.first()) else { panic!("{acts:?}") };
        assert_eq!(v["extend"], json!({"id": id.0, "end": "end"}));
        assert_eq!(v["points"], json!([[400.0, 300.0], [400.0 + 100f64.hypot(4.0), 300.0]]));
    }

    /// Before a press over an end of a selected open path, the Pencil shows its continuation cursor
    /// exactly when the press would draw a straight line that continues the path; away from the
    /// ends, with nothing selected, with Edit Selected Paths off, once the press is down and with
    /// the Paintbrush, it does not.
    #[test]
    fn pencil_shows_the_continuation_cursor_over_an_end_a_line_continues() {
        let (mut d, _) = doc_with_rect();
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let line = vectorcraft_geom::PathData::from_bezpath(&vectorcraft_geom::BezPath::from_vec(vec![
            vectorcraft_geom::PathEl::MoveTo(Point::new(300.0, 300.0)),
            vectorcraft_geom::PathEl::LineTo(Point::new(400.0, 300.0)),
        ]));
        d.insert(Some(l), 1, vectorcraft_doc::Node::path(id, line, vectorcraft_doc::Appearance::default_art())).unwrap();
        let mut s = Selection::default();
        s.set([id]);
        let p = paint();
        let mut cx = cx(&d, &s, &p);
        cx.smart_guides = false;
        let shift = Mods { shift: true, ..Default::default() };
        let alt = Mods { alt: true, ..Default::default() };
        let shift_alt = Mods { shift: true, alt: true, ..Default::default() };
        // The release's stroke: a straight line that continues the path.
        let continues = |acts: &[Vec<Action>]| {
            acts.last().and_then(|a| a.first()).is_some_and(
                |a| matches!(a, Action::Preview(_, v) if v.get("extend").is_some() && v["points"].as_array().is_some_and(|q| q.len() == 2)),
            )
        };
        for alt_smooth in [false, true] {
            for m in [Mods::default(), shift, alt, shift_alt] {
                for (at, other_end) in [(Point::new(405.0, 303.0), (500.0, 296.0)), (Point::new(296.0, 302.0), (200.0, 310.0))] {
                    let mut t = GestureTool::new("pencil");
                    t.set_option("altSmooth", &json!(alt_smooth));
                    let shown = t.cursor(&cx, at, m) == Cursor::PencilContinue;
                    let acts = drag_held(&mut t, &cx, &[(at.x, at.y), (450.0, 250.0), other_end], m);
                    assert_eq!(shown, continues(&acts), "altSmooth {alt_smooth}, {m:?} at {at:?}");
                }
            }
        }
        let mut t = GestureTool::new("pencil");
        let end = Point::new(405.0, 303.0);
        assert_eq!(t.cursor(&cx, end, shift), Cursor::PencilContinue);
        assert_eq!(t.cursor(&cx, end, alt), Cursor::PencilContinue, "Alt alone continues at any angle");
        assert_eq!(t.cursor(&cx, end, Mods::default()), Cursor::Pencil, "without Shift or Alt the stroke is freehand");
        assert_eq!(t.cursor(&cx, Point::new(350.0, 360.0), shift), Cursor::PencilStraight, "away from the ends");
        // While the line is drawn, the Pencil's own cursor.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, end.x, end.y).with_mods(shift));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 450.0, 303.0).with_mods(shift));
        assert_eq!(t.cursor(&cx, end, shift), Cursor::Pencil, "while the line is drawn");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 450.0, 303.0).with_mods(shift));
        // Edit Selected Paths off: the press draws a new line.
        t.set_option("editWithin", &json!(0.0));
        assert_eq!(t.cursor(&cx, end, shift), Cursor::PencilStraight, "Edit Selected Paths off");
        let none = Selection::default();
        let cx_none = crate::testutil::cx(&d, &none, &p);
        assert_eq!(GestureTool::new("pencil").cursor(&cx_none, end, shift), Cursor::PencilStraight, "nothing selected");
        assert_eq!(GestureTool::new("paintbrush").cursor(&cx, end, shift), Cursor::Paintbrush);
        assert_eq!(Cursor::PencilContinue.precise(), Cursor::Crosshair, "Use Precise Cursors");
    }

    /// #3846: Alt during a Shift line doesn't promise a close the release can't do, and with Alt
    /// Toggles to Smooth, Alt pressed after the press doesn't turn the line into a smoothing drag.
    #[test]
    fn pencil_shift_line_ignores_alt_for_cursor_and_smoothing() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let mut cx = cx(&d, &s, &p);
        cx.smart_guides = false;
        let shift = Mods { shift: true, ..Default::default() };
        let shift_alt = Mods { shift: true, alt: true, ..Default::default() };
        let mut t = GestureTool::new("pencil");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0).with_mods(shift));
        assert_eq!(t.cursor(&cx, Point::new(0.0, 0.0), shift_alt), Cursor::Pencil);
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 0.0, 0.0).with_mods(shift));

        let mut t = GestureTool::new("pencil");
        t.alt_smooth = true;
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0).with_mods(shift));
        let acts = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 80.0, 0.0).with_mods(shift_alt));
        assert!(matches!(acts.as_slice(), [Action::Begin(_), Action::Preview(..)]), "{acts:?}");
        let acts = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 80.0, 0.0).with_mods(shift_alt));
        assert!(matches!(&acts[0], Action::Preview(_, v) if v["points"].as_array().unwrap().len() == 2), "{acts:?}");
    }

    /// #3846: a Shift line dragged back to its press point has no line: the preview is cancelled
    /// and the release commits nothing (not the previous end of the line).
    #[test]
    fn pencil_shift_line_back_at_the_press_point_draws_nothing() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let mut cx = cx(&d, &s, &p);
        cx.smart_guides = false;
        let shift = Mods { shift: true, ..Default::default() };
        // Released back at the press point, and dragged back there first.
        for back_in_drag in [false, true] {
            let mut t = GestureTool::new("pencil");
            t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0).with_mods(shift));
            t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 80.0, 0.0).with_mods(shift));
            let mut acts = vec![];
            if back_in_drag {
                // The shown line goes as soon as the pointer is back, not only at the release.
                let back = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 0.2, 0.0).with_mods(shift));
                assert_eq!(back, vec![Action::Cancel]);
                acts.extend(back);
            }
            acts.extend(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 0.2, 0.0).with_mods(shift)));
            assert!(acts.contains(&Action::Cancel), "{back_in_drag}: {acts:?}");
            assert!(!acts.contains(&Action::Commit) && !acts.iter().any(|a| matches!(a, Action::Preview(..))), "{back_in_drag}: {acts:?}");
        }
    }

    /// #3799: Alt held at the press draws a straight Pencil line at any angle from the press point
    /// to the pointer, previewed as it is dragged, and left open by a release with Alt still held;
    /// with Shift too it keeps Shift's 45° steps. Alt pressed after the press keeps the stroke
    /// freehand and closes it at the release, and the Paintbrush keeps following the hand.
    #[test]
    fn pencil_alt_at_the_press_draws_a_straight_line_at_any_angle() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let mut cx = cx(&d, &s, &p);
        cx.smart_guides = false;
        let alt = Mods { alt: true, ..Default::default() };
        let pts = |acts: &[Vec<Action>]| -> Vec<Vec<(f64, f64)>> {
            acts.iter()
                .filter_map(|a| a.iter().find_map(|a| if let Action::Preview(_, v) = a { Some(v["points"].clone()) } else { None }))
                .map(|v| v.as_array().unwrap().iter().map(|q| (q[0].as_f64().unwrap(), q[1].as_f64().unwrap())).collect())
                .collect()
        };
        let wavy = [(0.0, 0.0), (30.0, -10.0), (50.0, -60.0), (100.0, -84.0)];
        let mut t = GestureTool::new("pencil");
        assert_eq!(t.cursor(&cx, Point::new(0.0, 0.0), alt), Cursor::PencilStraight, "before the press");
        let acts = drag_held(&mut t, &cx, &wavy, alt);
        let all = pts(&acts);
        assert_eq!(all.len(), 4, "{all:?}");
        assert!(all.iter().all(|v| v.len() == 2 && v[0] == (0.0, 0.0)), "{all:?}");
        assert_eq!(all[1][1], (50.0, -60.0), "the preview follows the pointer at its own angle");
        assert_eq!(all[3][1], (100.0, -84.0), "{all:?}");
        let last = acts.last().unwrap();
        assert!(matches!(&last[0], Action::Preview(_, v) if v["closed"] == false) && last[1] == Action::Commit, "{last:?}");
        // While the line is drawn, Alt shows no closing ring.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0).with_mods(alt));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 40.0, 10.0).with_mods(alt));
        assert_eq!(t.cursor(&cx, Point::new(40.0, 10.0), alt), Cursor::Pencil);
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 40.0, 10.0).with_mods(alt));
        // Shift with Alt: 45° steps.
        let all = pts(&drag_held(&mut t, &cx, &wavy, Mods { shift: true, alt: true, ..Default::default() }));
        let end = *all.last().unwrap().last().unwrap();
        assert!(((-end.1).atan2(end.0).to_degrees() - 45.0).abs() < 1e-9, "{end:?}");
        // Alt pressed after the press: freehand, closed at the release.
        let acts = drag(&mut t, &cx, &wavy, alt);
        assert_eq!(pts(&acts).last().unwrap().len(), 4, "{acts:?}");
        assert!(matches!(&acts.last().unwrap()[0], Action::Preview(_, v) if v["closed"] == true), "{acts:?}");
        // The Paintbrush with Alt held follows the hand and closes.
        let mut b = GestureTool::new("paintbrush");
        let acts = drag_held(&mut b, &cx, &wavy, alt);
        assert_eq!(pts(&acts).last().unwrap().len(), 4, "{acts:?}");
        assert!(matches!(&acts.last().unwrap()[0], Action::Preview(_, v) if v["closed"] == true), "{acts:?}");
    }

    /// #3799: with Alt key toggles to Smooth Tool on, Alt held at the press smooths as before and
    /// draws no line.
    #[test]
    fn pencil_alt_with_alt_to_smooth_on_smooths_instead_of_drawing_a_line() {
        let (d, id) = doc_with_rect();
        let mut s = Selection::default();
        s.set([id]);
        let p = paint();
        let mut cx = cx(&d, &s, &p);
        cx.smart_guides = false;
        let alt = Mods { alt: true, ..Default::default() };
        let mut t = GestureTool::new("pencil");
        t.set_option("altSmooth", &json!(true));
        let acts = drag_held(&mut t, &cx, &[(50.0, 150.0), (150.0, 150.0), (250.0, 160.0)], alt);
        assert!(acts[..acts.len() - 1].iter().all(|a| a.is_empty()), "{acts:?}");
        assert!(matches!(&acts.last().unwrap()[0], Action::Exec(c, v) if c == "path.smoothRegion" && v["points"].as_array().unwrap().len() == 3));
    }

    /// #3799: Alt's line starts and ends on Smart Guides as a stroke does, and pressed on an end of
    /// a selected open path it continues the path from that end to the pointer.
    #[test]
    fn pencil_alt_line_snaps_its_ends_and_continues_a_selected_path() {
        let (mut d, _) = doc_with_rect();
        let alt = Mods { alt: true, ..Default::default() };
        let p = paint();
        {
            let s = Selection::default();
            let cx = cx(&d, &s, &p);
            let mut t = GestureTool::new("pencil");
            let acts = drag_held(&mut t, &cx, &[(102.0, 98.0), (151.0, 300.0), (198.0, 203.0)], alt);
            let Some(Action::Preview(_, v)) = acts.last().and_then(|a| a.first()) else { panic!("{acts:?}") };
            assert_eq!(v["points"], json!([[100.0, 100.0], [200.0, 200.0]]));
        }
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let line = vectorcraft_geom::PathData::from_bezpath(&vectorcraft_geom::BezPath::from_vec(vec![
            vectorcraft_geom::PathEl::MoveTo(Point::new(300.0, 300.0)),
            vectorcraft_geom::PathEl::LineTo(Point::new(400.0, 300.0)),
        ]));
        d.insert(Some(l), 1, vectorcraft_doc::Node::path(id, line, vectorcraft_doc::Appearance::default_art())).unwrap();
        let mut s = Selection::default();
        s.set([id]);
        let mut cx = cx(&d, &s, &p);
        cx.smart_guides = false;
        let mut t = GestureTool::new("pencil");
        let acts = drag_held(&mut t, &cx, &[(405.0, 303.0), (450.0, 340.0), (470.0, 330.0)], alt);
        let Some(Action::Preview(_, v)) = acts.last().and_then(|a| a.first()) else { panic!("{acts:?}") };
        assert_eq!(v["extend"], json!({"id": id.0, "end": "end"}));
        assert_eq!(v["points"], json!([[400.0, 300.0], [470.0, 330.0]]));
    }

    #[test]
    fn pencil_alt_closes_and_continues_selected_path() {
        let (d, id) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = GestureTool::new("paintbrush");
        let acts = drag(&mut t, &cx, &[(0.0, 0.0), (10.0, 5.0), (20.0, 0.0)], Mods { alt: true, ..Default::default() });
        assert!(matches!(&acts.last().unwrap()[0], Action::Preview(_, v) if v["closed"] == true && v["style"] == "brush"));
        // A selected closed rect is not continued; an open path would be (see engine tests).
        let mut s2 = Selection::default();
        s2.set([id]);
        let cx2 = crate::testutil::cx(&d, &s2, &p);
        let acts = drag(&mut t, &cx2, &[(100.0, 100.0), (90.0, 90.0)], Mods::default());
        assert!(matches!(&acts[1][1], Action::Preview(_, v) if v.get("extend").is_none()));
    }

    /// Edit selected paths within 0 turns editing off: a stroke that starts exactly on an end of a
    /// selected open path draws a new path; at 12 px it continues the path from that end.
    #[test]
    fn edit_selected_paths_within_0_draws_a_new_path_from_an_end() {
        let (mut d, _) = doc_with_rect();
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let line = vectorcraft_geom::PathData::from_bezpath(&vectorcraft_geom::BezPath::from_vec(vec![
            vectorcraft_geom::PathEl::MoveTo(Point::new(300.0, 300.0)),
            vectorcraft_geom::PathEl::LineTo(Point::new(400.0, 300.0)),
        ]));
        d.insert(Some(l), 1, vectorcraft_doc::Node::path(id, line, vectorcraft_doc::Appearance::default_art())).unwrap();
        let mut s = Selection::default();
        s.set([id]);
        let p = paint();
        let cx = cx(&d, &s, &p);
        for tool in ["pencil", "paintbrush"] {
            let mut t = GestureTool::new(tool);
            for (within, extends) in [(12.0, true), (0.0, false)] {
                t.set_option("editWithin", &json!(within));
                for (x, end) in [(400.0, "end"), (300.0, "start")] {
                    let acts = drag(&mut t, &cx, &[(x, 300.0), (x + 20.0, 350.0), (x, 380.0)], Mods::default());
                    let Some(Action::Preview(_, v)) = acts.last().and_then(|a| a.first()) else { panic!("{tool}: {acts:?}") };
                    if extends {
                        assert_eq!(v["extend"], json!({"id": id.0, "end": end}), "{tool} from the {end}");
                    } else {
                        assert!(v.get("extend").is_none() && v.get("redraw").is_none(), "{tool} from the {end}: {v}");
                    }
                }
            }
        }
    }

    /// A Pencil stroke from an end of a selected open path to an end of another selected open path
    /// connects the two; it does not when the other path is not selected, and the Paintbrush does
    /// not connect.
    #[test]
    fn pencil_connects_a_stroke_from_one_selected_path_to_an_end_of_another() {
        let (mut d, _) = doc_with_rect();
        let l = d.layers[0].id;
        let add = |d: &mut vectorcraft_doc::Document, a: (f64, f64), b: (f64, f64)| {
            let id = d.alloc_id();
            let line = vectorcraft_geom::PathData::from_bezpath(&vectorcraft_geom::BezPath::from_vec(vec![
                vectorcraft_geom::PathEl::MoveTo(Point::new(a.0, a.1)),
                vectorcraft_geom::PathEl::LineTo(Point::new(b.0, b.1)),
            ]));
            d.insert(Some(l), 1, vectorcraft_doc::Node::path(id, line, vectorcraft_doc::Appearance::default_art())).unwrap();
            id
        };
        let a = add(&mut d, (300.0, 300.0), (400.0, 300.0));
        let b = add(&mut d, (500.0, 400.0), (600.0, 400.0));
        let p = paint();
        let stroke = [(400.0, 300.0), (450.0, 350.0), (499.0, 401.0)];
        let mut both = Selection::default();
        both.set([a, b]);
        let mut one = Selection::default();
        one.set([a]);
        // The previews select the path continued: after the press only it is selected.
        let last = |tool: &str, at_press: &Selection| {
            let mut t = GestureTool::new(tool);
            t.pointer(&cx(&d, at_press, &p), &PointerEvent::new(PointerKind::Down, stroke[0].0, stroke[0].1));
            let after = cx(&d, &one, &p);
            t.pointer(&after, &PointerEvent::new(PointerKind::Drag, stroke[1].0, stroke[1].1));
            t.pointer(&after, &PointerEvent::new(PointerKind::Drag, stroke[2].0, stroke[2].1));
            let acts = t.pointer(&after, &PointerEvent::new(PointerKind::Up, stroke[2].0, stroke[2].1));
            let Some(Action::Preview(_, v)) = acts.first() else { panic!("{acts:?}") };
            v.clone()
        };
        let v = last("pencil", &both);
        assert_eq!((v["extend"].clone(), v["connect"].clone()), (json!({"id": a.0, "end": "end"}), json!({"id": b.0, "end": "start"})));
        assert!(last("paintbrush", &both).get("connect").is_none(), "the Paintbrush");
        let v = last("pencil", &one);
        assert!(v.get("extend").is_some() && v.get("connect").is_none(), "{v}");
    }

    /// Close paths when ends are within 0 turns closing off: a stroke that ends exactly where it
    /// started stays open; at 15 px it is closed.
    #[test]
    fn close_paths_within_0_leaves_a_stroke_that_ends_at_its_start_open() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        for tool in ["pencil", "paintbrush"] {
            let mut t = GestureTool::new(tool);
            for (within, closes) in [(15.0, true), (0.0, false)] {
                t.set_option("closeWithin", &json!(within));
                let acts = drag(&mut t, &cx, &[(310.0, 330.0), (360.0, 350.0), (330.0, 390.0), (310.0, 330.0)], Mods::default());
                let Some(Action::Preview(_, v)) = acts.last().and_then(|a| a.first()) else { panic!("{tool}: {acts:?}") };
                assert_eq!(v["points"][0], v["points"][3], "{tool}");
                assert_eq!(v["closed"], json!(closes), "{tool} at {within} px");
            }
        }
    }

    /// While a stroke is drawn back near its start, the cursor shows the closing ring when a
    /// release there closes the path (Close Paths When Ends Are Within), and the tool's own cursor
    /// away from the start, at 0 pixels and after the release.
    #[test]
    fn a_stroke_near_its_start_shows_the_closing_cursor() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let none = Mods::default();
        let tools = [("pencil", Cursor::Pencil, Cursor::PencilClose), ("paintbrush", Cursor::Paintbrush, Cursor::PaintbrushClose)];
        for (tool, own, closing) in tools {
            for within in [15.0, 0.0] {
                let near = if within > 0.0 { closing } else { own };
                let mut t = GestureTool::new(tool);
                t.set_option("closeWithin", &json!(within));
                let mut at = |k, x, y| {
                    let acts = t.pointer(&cx, &PointerEvent::new(k, x, y));
                    (acts, t.cursor(&cx, Point::new(x, y), none))
                };
                assert_eq!(at(PointerKind::Down, 310.0, 330.0).1, own, "{tool} at the press");
                assert_eq!(at(PointerKind::Drag, 360.0, 350.0).1, own, "{tool} away from the start");
                assert_eq!(at(PointerKind::Drag, 330.0, 390.0).1, own, "{tool} away from the start");
                assert_eq!(at(PointerKind::Drag, 318.0, 335.0).1, near, "{tool} near the start at {within} px");
                assert_eq!(at(PointerKind::Drag, 345.0, 340.0).1, own, "{tool} away from the start again");
                assert_eq!(at(PointerKind::Drag, 318.0, 335.0).1, near, "{tool} back near the start at {within} px");
                let (acts, after) = at(PointerKind::Up, 318.0, 335.0);
                let Some(Action::Preview(_, v)) = acts.first() else { panic!("{tool}: {acts:?}") };
                assert_eq!(v["closed"], json!(within > 0.0), "{tool}: the release does what the cursor showed");
                assert_eq!(after, if tool == "pencil" { Cursor::PencilNew } else { own }, "{tool} after the release");
            }
        }
        // A straight Pencil line (Shift or Alt held at the press) stays open, so it shows no ring
        // near its start.
        for held in [Mods { shift: true, ..Mods::default() }, Mods { alt: true, ..Mods::default() }] {
            let mut t = GestureTool::new("pencil");
            t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 310.0, 330.0).with_mods(held));
            for (x, y) in [(360.0, 330.0), (330.0, 390.0), (318.0, 335.0)] {
                t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, x, y).with_mods(held));
            }
            for m in [held, none] {
                assert_eq!(t.cursor(&cx, Point::new(318.0, 335.0), m), Cursor::Pencil, "a straight line, {held:?}, {m:?}");
            }
        }
    }

    /// A Paintbrush sample under a pixel from the last point is dropped, and a release there keeps
    /// the last point: the closing cursor follows that point, not the pointer.
    #[test]
    fn the_paintbrush_closing_cursor_follows_the_point_a_release_keeps() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let none = Mods::default();
        let mut t = GestureTool::new("paintbrush");
        t.set_option("closeWithin", &json!(15.0));
        for (k, x, y) in [(PointerKind::Down, 310.0, 330.0), (PointerKind::Drag, 360.0, 350.0), (PointerKind::Drag, 330.0, 390.0)] {
            t.pointer(&cx, &PointerEvent::new(k, x, y));
        }
        // The last point kept is 15.5 from the start, outside Close Paths When Ends Are Within.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 325.5, 330.0));
        assert_eq!(t.cursor(&cx, Point::new(325.5, 330.0), none), Cursor::Paintbrush, "outside the closing distance");
        // The pointer moves 0.7 to 14.8 from the start; the sample is dropped.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 324.8, 330.0));
        assert_eq!(t.cursor(&cx, Point::new(324.8, 330.0), none), Cursor::Paintbrush, "the dropped sample shows no ring");
        let acts = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 324.8, 330.0));
        let Some(Action::Preview(_, v)) = acts.first() else { panic!("{acts:?}") };
        assert_eq!(v["closed"], json!(false), "the release keeps the last point and leaves the path open");
    }

    #[test]
    fn eraser_and_knife_exec_on_release() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = GestureTool::new("eraser");
        let acts = drag(&mut t, &cx, &[(50.0, 150.0), (250.0, 150.0)], Mods::default());
        assert!(acts[1].is_empty());
        assert!(matches!(&acts[2][0], Action::Exec(c, v) if c == "path.eraseRegion" && v["size"] == 10.0));
        let mut k = GestureTool::new("knife");
        k.pointer(&cx, &PointerEvent::new(PointerKind::Down, 50.0, 150.0));
        k.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 150.0));
        assert_eq!(k.overlays(&cx).len(), 1);
        let a = k.pointer(&cx, &PointerEvent::new(PointerKind::Up, 250.0, 150.0));
        assert!(matches!(&a[0], Action::Exec(c, v) if c == "path.knife" && v["points"].as_array().unwrap().len() == 3));
    }

    /// The Eraser's Angle and Roundness (Eraser Tool Options) go with its stroke, kept within
    /// −180–180° and 1–100%; while it drags, a tip that is not round shows the outline of the area
    /// it covers, and a round one the drag as wide as the tip.
    #[test]
    fn the_eraser_sends_its_tip_and_shows_the_area_it_covers() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = GestureTool::new("eraser");
        t.set_option("size", &json!(20));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 50.0, 150.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 150.0));
        let round = t.overlays(&cx);
        assert!(matches!(&round[..], [Overlay::Path { width, .. }] if *width == 20.0), "{round:?}");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 250.0, 150.0));
        for (angle, roundness, sent) in [(90.0, 25.0, (90.0, 25.0)), (400.0, 0.0, (180.0, 1.0)), (-400.0, 250.0, (-180.0, 100.0))] {
            t.set_option("angle", &json!(angle));
            t.set_option("roundness", &json!(roundness));
            assert_eq!((t.options()["angle"].as_f64(), t.options()["roundness"].as_f64()), (Some(sent.0), Some(sent.1)));
            let acts = drag(&mut t, &cx, &[(50.0, 150.0), (250.0, 150.0)], Mods::default());
            assert!(
                matches!(&acts[2][0], Action::Exec(c, v) if c == "path.eraseRegion" && v["size"] == 20.0 && v["angle"] == sent.0 && v["roundness"] == sent.1),
                "{acts:?}"
            );
        }
        // A tip 20 pt long and 5 pt wide, standing upright: along a horizontal drag from x 50 to
        // x 150 the outline reaches 2.5 pt either side of the drag and 10 pt above and below
        // its ends.
        t.set_option("angle", &json!(90));
        t.set_option("roundness", &json!(25));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 50.0, 150.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 150.0));
        let o = t.overlays(&cx);
        let [Overlay::Path { path, width, .. }] = &o[..] else { panic!("{o:?}") };
        assert_eq!(*width, 1.0);
        let b = vectorcraft_geom::Shape::bounding_box(path);
        assert!((b.x0 - 47.5).abs() < 0.1 && (b.x1 - 152.5).abs() < 0.1, "{b:?}");
        assert!((b.y0 - 140.0).abs() < 0.1 && (b.y1 - 160.0).abs() < 0.1, "{b:?}");
    }

    #[test]
    fn smooth_needs_selection_and_brackets_resize() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = GestureTool::new("smooth");
        let acts = drag(&mut t, &cx, &[(50.0, 150.0), (250.0, 150.0)], Mods::default());
        assert!(acts.iter().all(|a| a.is_empty()));
        let mut b = GestureTool::new("blobBrush");
        b.key(&cx, ToolKey::BracketRight, Mods::default());
        assert_eq!(b.options()["size"], 11.0);
    }

    #[test]
    fn pencil_alt_toggles_to_smooth_when_enabled_and_closes_when_disabled() {
        let (d, id) = doc_with_rect();
        let mut s = Selection::default();
        s.set([id]);
        let p = paint();
        let cx = cx(&d, &s, &p);

        // Off by default, as Pencil Tool Options ships it; on, Alt lends the Smooth tool.
        let mut t = GestureTool::new("pencil");
        assert_eq!(t.options()["altSmooth"], json!(false));
        t.set_option("altSmooth", &json!(true));

        // When alt is pressed, cursor shows smooth tool.
        let alt_mod = Mods { alt: true, ..Default::default() };
        assert_eq!(t.cursor(&cx, Point::new(50.0, 50.0), alt_mod), Cursor::Smooth);
        assert_eq!(t.cursor(&cx, Point::new(50.0, 50.0), Mods::default()), Cursor::PencilNew);

        // Alt-drag over selected path smooths it.
        let acts = drag_held(&mut t, &cx, &[(50.0, 150.0), (150.0, 150.0), (250.0, 150.0)], alt_mod);
        assert!(acts[1].is_empty(), "drag does not preview freehand path while smoothing");
        let last = acts.last().unwrap();
        assert!(
            matches!(&last[0], Action::Exec(c, v) if c == "path.smoothRegion" && v["points"].as_array().unwrap().len() == 3),
            "expected path.smoothRegion, got: {last:?}"
        );

        // When altSmooth is disabled:
        t.set_option("altSmooth", &json!(false));
        assert_eq!(t.options()["altSmooth"], json!(false));
        // Before a press, Alt gives a straight line rather than a closed path.
        assert_eq!(t.cursor(&cx, Point::new(50.0, 50.0), alt_mod), Cursor::PencilStraight);

        // A drag released with Alt held draws a closed pencil path.
        let empty_sel = Selection::default();
        let cx_empty = crate::testutil::cx(&d, &empty_sel, &p);
        let acts = drag(&mut t, &cx_empty, &[(0.0, 0.0), (10.0, 5.0), (20.0, 0.0)], alt_mod);
        assert_eq!(acts[1][0], Action::Begin("Pencil".into()));
        let last = acts.last().unwrap();
        assert!(matches!(&last[0], Action::Preview(_, v) if v["closed"] == true && v["style"] == "pencil"));
        assert_eq!(last[1], Action::Commit);
    }
}
