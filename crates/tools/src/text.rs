//! The Type tools: Type (T), Area Type and Type on a Path.
//!
//! Type: click places point type, drag draws an area-type frame, clicking into existing text
//! places the caret. Area Type / Type on a Path: click a path to turn it into a text frame or a
//! baseline (`text.createInPath`). While type is being edited, a click that lands on no type only
//! ends the editing, and the next click makes new type. Point type placed by a click and left
//! empty is discarded when editing ends (`text.discardEmpty`); frames keep their shape. Where new
//! type goes (the click, the frame's corners) snaps to Smart Guides ([`DrawSnap`]), hovering too.
//!
//! While editing: caret movement by character / word (Cmd or Alt) / line (Up/Down) / line ends
//! (Home/End; Cmd = whole text), Shift extends the selection, drag selects, double-click selects a
//! word and triple-click a paragraph, typing replaces the selection. Edits are coalesced into one
//! undo step per typing session: the tool keeps the styled runs locally and previews a single
//! `text.editRange` (the changed span, with its styled runs) against the session snapshot.
//! Styling a selected range goes through `text.setRangeStyle` (the Character panel reads the
//! selection from [`Tool::options`]); Alt+arrows step it by the Preferences › Type increments
//! (`type.step`). New type starts with placeholder text, selected, when Fill New Type Objects With
//! Placeholder Text is on. A space typed on its own after "1.", "1)", "A.", "A)", "*", "#", ">" or
//! "+" at the start of a paragraph in no list makes that paragraph a list (`text.startList`).
//!
//! IME: the marked text of a composition is part of the typing session (so it lays out in place),
//! underlined, until the IME commits it (it then goes through [`Tool::text_input`]) or clears it.

use std::ops::Range;
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Value, json};
use vectorcraft_doc::hit::hit_test;
use vectorcraft_doc::{NodeId, NodeKind, TextKind, TextObject, TextRun};
use vectorcraft_geom::{BezPath, Point, Rect, Shape};
use vectorcraft_text::{FontDb, TextLayout, edit};

use crate::guides::DrawSnap;
use crate::{Action, Cursor, ImeSurrounding, Mods, Overlay, PointerEvent, PointerKind, SurroundingUnit, Tool, ToolContext, ToolKey};

/// Which of the Type tools this is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Mode {
    #[default]
    Type,
    Area,
    OnPath,
}

/// A coalesced typing session: the runs at its start and the runs now.
struct Typing {
    base: Vec<TextRun>,
    cur: Vec<TextRun>,
    /// The first byte any edit of the session touched (the text before it is unchanged): the
    /// previewed span starts no later, so Return at a paragraph's end splits that paragraph (its
    /// style continues), not the next one.
    lo: usize,
}

/// IME marked text inside the typing session, not yet committed.
#[derive(Clone, Debug, PartialEq)]
struct Preedit {
    /// Byte range of the marked text in the session's runs.
    range: Range<usize>,
    /// The clause being converted, in bytes relative to `range.start` (empty: the IME's cursor).
    active: Option<Range<usize>>,
}

/// A list that typed characters started ([`TypeTool::start_typed_list`]).
struct TypedList {
    /// The type object.
    id: NodeId,
    /// The bytes of the typed characters and the space: from the paragraph's start to the caret.
    start: usize,
    end: usize,
    /// The plain text with the typed characters, before the list started.
    text: String,
    list: vectorcraft_doc::text::ParaList,
}

/// The edited text's layout, keyed by the font database's generation and the text.
type CachedLayout = ((u64, TextObject), Arc<TextLayout>);

/// Process-global Type-tool edit-session counter: the IME target's replacement epoch.
///
/// A per-tool counter restarts at zero when the tool is recreated (a document switch, File ›
/// Revert), while the document keeps its uid and node ids. The same `(document, node)` would then
/// receive an epoch a still-cached native IME context already used, violating the replacement
/// contract that an old context can never edit the new owner. Sessions are therefore numbered for
/// the process lifetime; the counter only stops changing after 2^64 sessions (unreachable).
/// A mutex (not an `AtomicU64`) keeps this available on wasm targets without 64-bit atomics.
static NEXT_EDIT_SESSION: Mutex<u64> = Mutex::new(0);

/// Return the next process-wide edit-session id. It saturates at `u64::MAX` instead of wrapping to
/// zero; the final id can repeat only after 2^64 sessions.
fn next_edit_session() -> u64 {
    let mut next = NEXT_EDIT_SESSION.lock().unwrap_or_else(PoisonError::into_inner);
    let session = next.checked_add(1).unwrap_or(u64::MAX);
    *next = session;
    session
}

#[derive(Default)]
pub struct TypeTool {
    vertical: bool,
    mode: Mode,
    /// The tool to return to on Escape (e.g. when double-clicked from a selection tool).
    prev_tool: Option<String>,
    /// Text object being edited.
    editing: Option<NodeId>,
    /// Monotonic identity of this edit session (the IME target's replacement epoch): it changes
    /// whenever editing starts afresh, including on the same text object, and never on ordinary
    /// edits or caret moves. Numbered process-wide ([`NEXT_EDIT_SESSION`]), so recreating the tool
    /// (File › Revert, a document switch) cannot reuse an epoch of the same document and node.
    edit_session: u64,
    /// The point type a click just placed (edited now): discarded if empty when editing ends.
    fresh: Option<NodeId>,
    /// Caret and selection anchor (byte offsets into the plain text).
    caret: usize,
    anchor: usize,
    /// Horizontal position kept by Up/Down (text space).
    goal_x: Option<f64>,
    typing: Option<Typing>,
    preedit: Option<Preedit>,
    /// A command from outside the tool (`settle_interaction`, an external edit while typing) ended
    /// the typing session, while the text stays open for editing. A commit over that edit is
    /// application-origin unless an IME deletion has already begun a fresh typing session.
    typing_ended: bool,
    /// Where the button went down, and where new type goes from there (snapped to Smart Guides).
    press: Option<(Point, Point)>,
    /// The other corner of an area dragged out (snapped).
    drag: Option<Point>,
    /// Smart Guides for where new type goes.
    snap: DrawSnap,
    /// The press landed in the edited text: dragging selects.
    selecting: bool,
    /// Click counting for double/triple click (position of the last click, count).
    clicks: (Option<Point>, u8),
    /// Styled copy of the last copied range (pasting the same plain text keeps its styles).
    clipboard: Vec<TextRun>,
    /// The list typed characters last started (`text.startList`), so that after Undo puts them back
    /// the caret goes on after them, and after Redo it goes back.
    listed: Option<TypedList>,
    cache: Mutex<Option<CachedLayout>>,
}

impl TypeTool {
    pub fn new(id: &str) -> Self {
        let mode = match id {
            "areaType" | "verticalAreaType" => Mode::Area,
            "typeOnPath" | "verticalTypeOnPath" => Mode::OnPath,
            _ => Mode::Type,
        };
        Self { mode, vertical: id.starts_with("vertical"), ..Default::default() }
    }

    fn text<'a>(cx: &'a ToolContext, id: NodeId) -> Option<&'a TextObject> {
        match &cx.doc.node(id)?.kind {
            NodeKind::Text(t) => Some(t),
            _ => None,
        }
    }

    /// Layout of `t`, cached by content (overlays run every frame) and by the fonts available (the
    /// web build adds fonts after startup).
    fn layout(&self, t: &TextObject) -> Arc<TextLayout> {
        let fonts = FontDb::global().generation();
        let mut c = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(((g, k), l)) = c.as_ref()
            && *g == fonts
            && k == t
        {
            return l.clone();
        }
        let l = Arc::new(vectorcraft_text::layout(FontDb::global(), t));
        *c = Some(((fonts, t.clone()), l.clone()));
        l
    }

    /// The edited text used to find word and paragraph boundaries for selection: the typing session's
    /// runs, or the document's current text when there is no live session or it has gone stale.
    fn plain(&self, cx: &ToolContext) -> String {
        let document = self.editing.and_then(|id| Self::text(cx, id));
        match &self.typing {
            // A session the document changed under is not the edited text any more: use the
            // document's own runs (word/paragraph selection must not index stale text).
            Some(ty) if document.is_some_and(|t| ty.cur != t.runs) => document.map(|t| t.plain_text()).unwrap_or_default(),
            Some(ty) => ty.cur.iter().map(|r| r.text.as_str()).collect(),
            None => document.map(|t| t.plain_text()).unwrap_or_default(),
        }
    }

    /// The edited plain text as borrowed run slices: the IME query copies only the bounded window
    /// it needs, never the whole document text. A typing session whose runs no longer match the
    /// document (an external edit) is not used; the document's runs are.
    fn plain_segments<'a>(&'a self, cx: &'a ToolContext) -> Option<Vec<&'a str>> {
        let id = self.editing?;
        let t = Self::text(cx, id)?;
        match &self.typing {
            Some(ty) if ty.cur == t.runs => Some(ty.cur.iter().map(|r| r.text.as_str()).collect()),
            _ => Some(t.runs.iter().map(|r| r.text.as_str()).collect()),
        }
    }

    /// The exact text [`Tool::text_input`] would insert for `s`: control characters removed, `\r`
    /// normalized to `\n`, and Typographer's Quotes applied to a straight quote. `None` when
    /// nothing would be inserted. Shared with [`Tool::text_input_is_verbatim`] so the IME receipt
    /// and the insertion cannot disagree about what was transformed.
    fn input_text(&self, cx: &ToolContext, s: &str) -> Option<String> {
        let s: String = s.chars().filter(|c| !c.is_control() || matches!(c, '\n' | '\t')).map(|c| if c == '\r' { '\n' } else { c }).collect();
        if s.is_empty() {
            return None;
        }
        // Use Typographer's Quotes (Document Setup): a typed straight quote becomes the document's
        // opening or closing quote, by the character before it.
        let setup = &cx.doc.setup;
        if setup.typographers_quotes && matches!(s.as_str(), "\"" | "'") {
            let mut prev = self.char_before(cx, self.caret.min(self.anchor));
            return Some(setup.quotes.apply(&s, &mut prev));
        }
        Some(s)
    }

    /// The character just before byte `at` in the edited plain text, walked run by run (the quote
    /// direction needs one character, not a copy of the whole text).
    fn char_before(&self, cx: &ToolContext, at: usize) -> Option<char> {
        let segments = self.plain_segments(cx)?;
        let mut acc = 0usize;
        let mut prev = None;
        for segment in &segments {
            let end = acc + segment.len();
            if at <= acc {
                break;
            }
            let local = at.min(end).saturating_sub(acc);
            if let Some((_, c)) = segment.char_indices().take_while(|(i, _)| *i < local).last() {
                prev = Some(c);
            }
            acc = end;
        }
        prev
    }

    /// The edited object as it currently is (with the typing session's runs). A session the
    /// document changed under is dropped: the document's own runs win.
    fn current(&self, cx: &ToolContext) -> Option<TextObject> {
        let mut t = Self::text(cx, self.editing?)?.clone();
        if let Some(ty) = &self.typing
            && ty.cur == t.runs
        {
            t.runs = ty.cur.clone();
        }
        Some(t)
    }

    fn sel(&self) -> (usize, usize) {
        (self.caret.min(self.anchor), self.caret.max(self.anchor))
    }

    /// End the typing session (one undo step). Marked text left by an IME stays as typed (as
    /// clicking away from a composition does in macOS text views); the UI interrupts the IME.
    fn commit(&mut self) -> Vec<Action> {
        self.preedit = None;
        if self.typing.take().is_some() { vec![Action::Commit] } else { vec![] }
    }

    /// Stop editing. Point type a click placed that is still empty is discarded, with the steps
    /// since its click (`text.discardEmpty`).
    fn finish(&mut self, cx: &ToolContext) -> Vec<Action> {
        let mut out = self.commit();
        if let Some(id) = self.fresh.take()
            && self.editing == Some(id)
            && Self::text(cx, id).is_some_and(|t| matches!(t.kind, TextKind::Point) && edit::runs_len(&t.runs) == 0)
        {
            out.push(Action::Exec("text.discardEmpty".into(), json!({"id": id.0})));
        }
        self.editing = None;
        self.selecting = false;
        self.goal_x = None;
        out
    }

    /// Replace the selection with `insert` (styled `runs` when given) inside the typing session.
    fn replace(&mut self, cx: &ToolContext, insert: &str, styled: Option<Vec<TextRun>>) -> Vec<Action> {
        let Some(id) = self.editing else { return vec![] };
        let Some(t) = Self::text(cx, id) else {
            self.editing = None;
            return vec![];
        };
        let mut out = vec![];
        // The document changed under us (undo, a panel edit): start a new session.
        if self.typing.as_ref().is_some_and(|ty| ty.cur != t.runs) {
            out.push(Action::Commit);
            self.typing = None;
            self.preedit = None;
        }
        if self.typing.is_none() {
            out.push(Action::Begin("Typing".into()));
            self.typing_ended = false;
        }
        let ty = self.typing.get_or_insert_with(|| Typing { base: t.runs.clone(), cur: t.runs.clone(), lo: usize::MAX });
        let len = edit::runs_len(&ty.cur);
        let (a, b) = (self.caret.min(self.anchor).min(len), self.caret.max(self.anchor).min(len));
        ty.lo = ty.lo.min(a);
        let caret = match &styled {
            Some(r) => edit::replace_range_styled(&mut ty.cur, a, b, r),
            None => edit::replace_range(&mut ty.cur, a, b, insert),
        };
        self.caret = caret;
        self.anchor = caret;
        self.goal_x = None;
        // One preview for the whole session: the changed span of base → cur, with its runs.
        let base: String = ty.base.iter().map(|r| r.text.as_str()).collect();
        let cur: String = ty.cur.iter().map(|r| r.text.as_str()).collect();
        let (p, s) = common_affixes(&base, &cur, ty.lo);
        let runs = edit::slice_runs(&ty.cur, p, cur.len() - s);
        let runs = if runs.is_empty() { json!([]) } else { serde_json::to_value(&runs).unwrap_or(json!([])) };
        out.push(Action::Preview("text.editRange".into(), json!({"id": id.0, "start": p, "end": base.len() - s, "runs": runs})));
        out
    }

    /// Move the caret (Shift keeps the anchor). Moving ends the typing session.
    fn move_to(&mut self, byte: usize, extend: bool) -> Vec<Action> {
        let out = self.commit();
        self.caret = byte;
        if !extend {
            self.anchor = byte;
        }
        self.listed = None;
        out
    }

    /// Begin a new edit session for `id` at `caret`, assigning a fresh session id and clearing typing,
    /// pre-edit and goal state.
    fn start_editing(&mut self, id: NodeId, caret: usize) {
        self.edit_session = next_edit_session();
        self.editing = Some(id);
        self.fresh = None;
        self.listed = None;
        self.caret = caret;
        self.anchor = caret;
        self.goal_x = None;
        self.typing = None;
        self.preedit = None;
        self.typing_ended = false;
    }

    /// Typed list detection, after a space typed on its own at the caret: when the characters
    /// from the start of the caret's paragraph up to it start a list ([`TextObject::typed_list_at`]),
    /// the typing session ends as one undo step with those characters, and `text.startList`
    /// removes them and makes the paragraph a list as the next one, so one Undo puts them back.
    fn start_typed_list(&mut self, cx: &ToolContext) -> Vec<Action> {
        let (Some(id), Some(ty)) = (self.editing, &self.typing) else { return vec![] };
        let Some(mut t) = Self::text(cx, id).cloned() else { return vec![] };
        // The document does not have the space yet (its preview is applied after this input).
        t.runs = ty.cur.clone();
        let at = self.caret;
        let Some((start, list)) = t.typed_list_at(at) else { return vec![] };
        let mut out = self.commit();
        out.push(Action::Exec("text.startList".into(), json!({"id": id.0, "at": at})));
        self.caret = start;
        self.anchor = start;
        self.listed = Some(TypedList { id, start, end: at, text: t.plain_text(), list });
        out
    }

    /// After Undo of a list that typed characters started (and of what was typed after it), the
    /// text is as it was typed: the caret goes on after the characters. After Redo the text is as
    /// the list left it: the caret goes back to the paragraph's start.
    fn after_typed_list(&mut self, cx: &ToolContext) {
        let Some(l) = &self.listed else { return };
        if self.editing != Some(l.id) || self.typing.is_some() || self.caret != self.anchor {
            return;
        }
        let Some(t) = Self::text(cx, l.id) else { return };
        let (text, list) = (t.plain_text(), t.para_at(t.paragraphs_in(l.start, l.start).start).list);
        let listed_text = l.text.get(..l.start).zip(l.text.get(l.end..)).map(|(a, b)| format!("{a}{b}"));
        if text == l.text && list.is_none() {
            self.caret = l.end;
        } else if self.caret == l.end && Some(&text) == listed_text.as_ref() && list == Some(l.list) {
            self.caret = l.start;
        }
        self.anchor = self.caret;
    }

    /// Byte under `p` in the edited text if `p` is inside it (frame, path band or layout bounds).
    fn hit_edited(&self, cx: &ToolContext, p: Point) -> Option<usize> {
        let t = self.current(cx)?;
        let lay = self.layout(&t);
        let local = t.xf.inverse() * p;
        let tol = cx.tol(4.0);
        let inside = match &t.kind {
            TextKind::Area { frame } => frame.bounds().is_some_and(|b| b.inflate(tol, tol).contains(local)),
            _ => lay.bounds.inflate(tol, tol).contains(local),
        };
        inside.then(|| vectorcraft_text::hit_byte(&lay, local))
    }

    fn select_word_or_para(&mut self, cx: &ToolContext, para: bool) {
        let s = self.plain(cx);
        let r = if para { edit::paragraph_at(&s, self.caret) } else { edit::word_at(&s, self.caret) };
        self.anchor = r.start;
        self.caret = r.end;
    }

    /// Path under `p` for the Area Type / Type on a Path tools.
    fn path_at(cx: &ToolContext, p: Point, closed: bool) -> Option<NodeId> {
        let h = hit_test(cx.doc, p, vectorcraft_doc::hit::HitOptions { tol: cx.tol(4.0), outline: true, ..Default::default() })?;
        match &cx.doc.node(h.leaf)?.kind {
            NodeKind::Path { path, .. } if !closed || path.is_closed() => Some(h.leaf),
            _ => None,
        }
    }

    /// Start editing the type under `p` with the caret there → the actions (ending the previous
    /// edit, selecting the type); None when no type is under `p`. A click among the characters
    /// edits them, whatever Type Object Selection by Path Only says.
    fn edit_at(&mut self, cx: &ToolContext, p: Point) -> Option<Vec<Action>> {
        let h = hit_test(cx.doc, p, vectorcraft_doc::hit::HitOptions { type_path_only: false, ..cx.hit_options() })?;
        let Some(NodeKind::Text(t)) = cx.doc.node(h.leaf).map(|n| &n.kind) else { return None };
        let lay = self.layout(t);
        let byte = vectorcraft_text::hit_byte(&lay, t.xf.inverse() * p);
        let mut out = self.finish(cx);
        self.start_editing(h.leaf, byte);
        self.clicks = (Some(p), 1);
        out.push(Action::Exec("select.set".into(), json!({"ids": [h.leaf.0]})));
        Some(out)
    }

    /// The button released after a press at `start` (new type goes at `at`). While type is being
    /// edited, a click that lands on no type only ends the editing; the next click makes new type
    /// (#2871). A drag that draws an area-type frame makes it either way.
    fn on_up(&mut self, cx: &ToolContext, start: Point, at: Point) -> Vec<Action> {
        // Click into existing text: place the caret.
        if let Some(out) = self.edit_at(cx, start) {
            return out;
        }
        let was_editing = self.editing.is_some_and(|id| Self::text(cx, id).is_some());
        let mut out = self.finish(cx);
        let drag = self.drag.take();
        // New type is set along General › Constrain Angle's axes (none at 0° or a whole turn): an
        // area's box measured along them, turned about its top-left corner, point type about its
        // first baseline's start.
        let axes = vectorcraft_geom::constrain_axes(cx.constrain_angle);
        let back = axes.map(|a| a.inverse());
        let area = drag
            .map(|d| match back {
                Some(b) => Rect::from_points(b * at, b * d),
                None => Rect::from_points(at, d),
            })
            .filter(|r| r.width() > cx.tol(6.0) && r.height() > cx.tol(6.0));
        if was_editing && area.is_none() {
            return out;
        }
        // Area Type / Type on a Path: click a path.
        if self.mode != Mode::Type && drag.is_none() {
            let on_path = self.mode == Mode::OnPath;
            if let Some(pid) = Self::path_at(cx, start, !on_path) {
                let mode = if on_path { "onPath" } else { "area" };
                out.push(Action::Exec(
                    "text.createInPath".into(),
                    json!({"path": pid.0, "mode": mode, "text": "", "at": [start.x, start.y], "vertical": self.vertical, "placeholder": cx.placeholder_text}),
                ));
                out.push(Action::Notify("text.editNew".into()));
                return out;
            }
        }
        let mut params = match area {
            Some(r) => {
                let o = axes.map_or(r.origin(), |a| a * r.origin());
                json!({"x": o.x, "y": o.y, "text": "", "area": {"width": r.width(), "height": r.height()}})
            }
            None => json!({"x": at.x, "y": at.y, "text": ""}),
        };
        if let Some(rotation) = vectorcraft_geom::constrain_rotation(cx.constrain_angle) {
            params["rotation"] = json!(rotation);
        }
        params["vertical"] = json!(self.vertical);
        params["placeholder"] = json!(cx.placeholder_text);
        out.push(Action::Exec("text.create".into(), params));
        out.push(Action::Notify("text.editNew".into()));
        out
    }

    /// Delete plain-text byte ranges in one typing session, shifting the caret, anchor and any
    /// pre-edit range so the selection's direction and the marked text are preserved.
    fn delete_ranges(&mut self, cx: &ToolContext, ranges: &[(usize, usize)]) -> Vec<Action> {
        let Some(id) = self.editing else { return vec![] };
        let Some(t) = Self::text(cx, id) else {
            self.editing = None;
            return vec![];
        };
        let mut out = vec![];
        // The document changed under us (undo, a panel edit): start a new session.
        if self.typing.as_ref().is_some_and(|ty| ty.cur != t.runs) {
            out.push(Action::Commit);
            self.typing = None;
            // The marked text belonged to the replaced session; it must not be sliced out of the
            // new document text by a stale range.
            self.preedit = None;
        }
        if self.typing.is_none() {
            out.push(Action::Begin("Typing".into()));
            self.typing_ended = false;
        }
        let len = match &self.typing {
            Some(ty) => edit::runs_len(&ty.cur),
            None => edit::runs_len(&t.runs),
        };
        let mut sorted: Vec<(usize, usize)> = ranges.iter().map(|&(a, b)| (a.min(b).min(len), a.max(b).min(len))).filter(|(a, b)| a < b).collect();
        sorted.sort_unstable();
        if sorted.is_empty() {
            return out;
        }
        // Shift the selection ends and the pre-edit by the bytes removed before them. The ranges
        // lie outside the selection/pre-edit, so both ends move by the same preceding amount and
        // a reversed selection stays reversed.
        let shift = |at: usize| -> usize {
            let removed: usize = sorted.iter().map(|&(a, b)| if at >= b { b - a } else { at.saturating_sub(a) }).sum();
            at.saturating_sub(removed)
        };
        let (caret, anchor) = (self.caret.min(len), self.anchor.min(len));
        self.caret = shift(caret);
        self.anchor = shift(anchor);
        if let Some(p) = &mut self.preedit {
            p.range.start = shift(p.range.start.min(len));
            p.range.end = shift(p.range.end.min(len));
        }
        self.goal_x = None;
        let ty = self.typing.get_or_insert_with(|| Typing { base: t.runs.clone(), cur: t.runs.clone(), lo: usize::MAX });
        // Delete from the end so earlier offsets stay valid; each call floors to a char boundary
        // and the one Preview below still makes the whole session one undo step.
        for &(a, b) in sorted.iter().rev() {
            ty.lo = ty.lo.min(a);
            edit::replace_range(&mut ty.cur, a, b, "");
        }
        // One preview for the whole session, exactly as `replace` does.
        let base: String = ty.base.iter().map(|r| r.text.as_str()).collect();
        let cur: String = ty.cur.iter().map(|r| r.text.as_str()).collect();
        let (p, s) = common_affixes(&base, &cur, ty.lo);
        let runs = edit::slice_runs(&ty.cur, p, cur.len() - s);
        let runs = if runs.is_empty() { json!([]) } else { serde_json::to_value(&runs).unwrap_or(json!([])) };
        out.push(Action::Preview("text.editRange".into(), json!({"id": id.0, "start": p, "end": base.len() - s, "runs": runs})));
        out
    }
}

/// How far (screen px) the ports of area type sit from the frame's corners, along its sides: the
/// out port above the bottom-right corner on the right side, the in port below the top-left corner
/// on the left side, clear of the bounding box's corner handles, so both can be grabbed.
pub const PORT_RISE_PX: f64 = 12.0;
/// Half a port's side (screen px).
const PORT_HALF_PX: f64 = 4.5;
/// The red of the out port of text that doesn't fit.
const OVERFLOW_RED: [u8; 3] = [230, 30, 30];

/// One of the two ports of area type or type on a path: its story flows in through the in port
/// and on through the out port.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Port {
    In,
    Out,
}

/// What a port shows: empty when the frame is not threaded on that side (and, for the out port,
/// all its text is seen), an arrow when it is threaded to another frame there, a red "+" in the out
/// port when its text doesn't fit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortState {
    Empty,
    Threaded,
    Overflow,
}

/// Centre of `port` of `t`, `px` document units to the screen pixel: area type's on its frame's
/// sides by [`PORT_RISE_PX`] from the corners; type on a path's along its path, as far past its end
/// bracket (out) or before its start bracket (in), clear of the brackets, which drag.
pub fn port_centre(t: &TextObject, port: Port, px: f64) -> Option<Point> {
    let rise = PORT_RISE_PX * px;
    match &t.kind {
        TextKind::Area { frame } => {
            let b = frame.bounds()?;
            // The corner the port sits by, and the far end of the side it sits on.
            let (corner, far) = match port {
                Port::Out => (Point::new(b.x1, b.y1), Point::new(b.x1, b.y0)),
                Port::In => (Point::new(b.x0, b.y0), Point::new(b.x0, b.y1)),
            };
            let (corner, side) = (t.xf * corner, t.xf * far - t.xf * corner);
            let len = side.hypot();
            Some(if len > 1e-9 && len.is_finite() { corner + side * (rise.min(len / 2.0) / len) } else { corner })
        }
        TextKind::OnPath { path, .. } => {
            let arc = vectorcraft_geom::ArcPath::new(path);
            let (start, end) = t.kind.path_span()?;
            if arc.is_empty() {
                return None;
            }
            let (at, away) = match port {
                Port::Out => (end, rise),
                Port::In => (start, -rise),
            };
            let (p, dir) = arc.at(at * arc.len());
            let (at, ahead) = (t.xf * p, t.xf * (p + dir) - t.xf * p);
            let len = ahead.hypot();
            Some(if len > 1e-9 && len.is_finite() { at + ahead * (away / len) } else { at })
        }
        TextKind::Point => None,
    }
}

/// What the in and out ports of frame `id` of `d` show, its text `overflow`ing or not: an arrow
/// where a frame of its thread comes before (in) or after (out) it, else a red "+" in the out port
/// of text that doesn't fit, else nothing.
pub fn port_states(d: &vectorcraft_doc::Document, id: NodeId, overflow: bool) -> (PortState, PortState) {
    let thread = d.text_threads.iter().find(|t| t.contains(&id));
    let at = thread.and_then(|t| t.iter().position(|x| *x == id));
    let input = if at.is_some_and(|i| i > 0) { PortState::Threaded } else { PortState::Empty };
    let output = match (thread, at) {
        (Some(t), Some(i)) if i + 1 < t.len() => PortState::Threaded,
        _ if overflow => PortState::Overflow,
        _ => PortState::Empty,
    };
    (input, output)
}

/// A port at `c` showing `state`, `px` document units to the screen pixel: a white box outlined in
/// `color` (the frame's layer colour; red for text that doesn't fit), with a red "+" or an arrow
/// pointing the way the story flows.
pub fn push_port(o: &mut Vec<Overlay>, c: Point, state: PortState, color: [u8; 3], px: f64) {
    let h = PORT_HALF_PX * px;
    let r = Rect::new(c.x - h, c.y - h, c.x + h, c.y + h);
    let color = if state == PortState::Overflow { OVERFLOW_RED } else { color };
    let corners = [Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)];
    o.push(Overlay::Highlight { quad: corners, color: [255, 255, 255, 255] });
    o.push(Overlay::Path { path: r.to_path(0.1), color, width: 1.0, dashed: false });
    let k = 2.5 * px;
    let line = |a: Point, b: Point| Overlay::Line { a, b, color, dashed: false };
    match state {
        PortState::Empty => {}
        PortState::Overflow => {
            o.push(line(Point::new(c.x - k, c.y), Point::new(c.x + k, c.y)));
            o.push(line(Point::new(c.x, c.y - k), Point::new(c.x, c.y + k)));
        }
        PortState::Threaded => {
            let tip = Point::new(c.x + k, c.y);
            o.push(line(Point::new(c.x - k, c.y), tip));
            o.push(line(Point::new(c.x, c.y - k), tip));
            o.push(line(Point::new(c.x, c.y + k), tip));
        }
    }
}

/// The in and out ports of area type or type on a path `t`, frame `id`, its text `overflow`ing or
/// not ([`port_states`]).
fn ports(cx: &ToolContext, id: NodeId, t: &TextObject, overflow: bool, o: &mut Vec<Overlay>) {
    let (input, output) = port_states(cx.doc, id, overflow);
    let (color, px) = (cx.doc.layer_color(id), cx.tol(1.0));
    for (port, state) in [(Port::In, input), (Port::Out, output)] {
        if let Some(c) = port_centre(t, port, px) {
            push_port(o, c, state, color, px);
        }
    }
}

/// The most selected area type and type on a path whose ports are drawn and take clicks: selecting
/// all of an opened PDF can select tens of thousands of type objects, whose ports would only bury
/// the canvas (and the GPU's buffers) in boxes.
pub const PORTS_MAX: usize = 1_000;

/// The text of `id` in `d` when it has ports: area type or type on a path a thread can hold, not in
/// perspective ([`crate::select::is_threadable`]). The one test for drawing ports, clicking them and
/// counting them toward [`PORTS_MAX`].
pub fn ported(d: &vectorcraft_doc::Document, id: NodeId) -> Option<&TextObject> {
    match d.node(id).filter(|n| crate::select::is_threadable(n)).map(|n| &n.kind) {
        Some(NodeKind::Text(t)) => Some(t),
        _ => None,
    }
}

/// Are at most [`PORTS_MAX`] of `selected` [`ported`] (counting stops past it), so that their
/// ports show?
pub fn ports_shown(d: &vectorcraft_doc::Document, selected: &[NodeId]) -> bool {
    selected.iter().filter(|id| ported(d, **id).is_some()).take(PORTS_MAX + 1).count() <= PORTS_MAX
}

/// The selected area type or type on a path with a port under `p`, and which port: a click on the
/// out port loads the Selection tool with its text to thread into another frame after it, a click
/// on the in port to thread into one before it. Every such frame has both ports, threaded,
/// overflowing or not, while their ports show ([`PORTS_MAX`]).
pub(crate) fn port_at(cx: &ToolContext, p: Point) -> Option<(NodeId, Port)> {
    if !ports_shown(cx.doc, &cx.selection.objects) {
        return None;
    }
    let h = cx.tol(PORT_HALF_PX + 1.0);
    let near = |c: Point| (c.x - p.x).abs() <= h && (c.y - p.y).abs() <= h;
    cx.selection.objects.iter().find_map(|id| {
        let t = ported(cx.doc, *id)?;
        [Port::Out, Port::In].into_iter().find(|port| port_centre(t, *port, cx.tol(1.0)).is_some_and(near)).map(|port| (*id, port))
    })
}

/// Which selected text overflows, for [`thread_ports`]: each text object's layout verdict kept
/// with the object and the fonts it was made from, as overlays run every frame.
#[derive(Default)]
pub struct PortMarks {
    seen: std::collections::HashMap<NodeId, ((u64, TextObject), bool)>,
}

/// The in and out ports of the selected area type and type on a path, for every tool but the Type
/// tools (they draw their own): each tells whether the frame is threaded on that side and the out
/// port whether text is hidden by the size of its frame, whatever tool is active. None when more
/// than [`PORTS_MAX`] of them are selected.
pub fn thread_ports(marks: &mut PortMarks, cx: &ToolContext) -> Vec<Overlay> {
    let fonts = FontDb::global().generation();
    let mut o = Vec::new();
    let mut kept = std::collections::HashMap::new();
    if !ports_shown(cx.doc, &cx.selection.objects) {
        marks.seen = kept;
        return o;
    }
    for id in &cx.selection.objects {
        let Some(t) = ported(cx.doc, *id) else { continue };
        let over = match marks.seen.remove(id) {
            Some(((g, was), over)) if g == fonts && &was == t => over,
            _ => vectorcraft_text::layout(FontDb::global(), t).overflow,
        };
        ports(cx, *id, t, over, &mut o);
        kept.insert(*id, ((fonts, t.clone()), over));
    }
    marks.seen = kept;
    o
}

/// Byte range in `s` of the characters `r` (IME ranges count characters; carets count bytes).
/// `None` when `r` runs past the end of `s` or backwards.
/// The `type.step` an Alt+arrow asks for, by one step: ←/→ kerning at a `caret`, else tracking;
/// ↑/↓ leading (down opens it up); Shift+↑/↓ baseline shift.
pub fn step_key(key: ToolKey, shift: bool, caret: bool) -> Option<(&'static str, f64)> {
    let sign = |up: bool| if up { 1.0 } else { -1.0 };
    Some(match (key, shift) {
        (ToolKey::Left | ToolKey::Right, false) => (if caret { "kerning" } else { "tracking" }, sign(key == ToolKey::Right)),
        (ToolKey::Up | ToolKey::Down, false) => ("leading", sign(key == ToolKey::Down)),
        (ToolKey::Up | ToolKey::Down, true) => ("baselineShift", sign(key == ToolKey::Up)),
        _ => return None,
    })
}

fn char_range_to_bytes(s: &str, r: Range<usize>) -> Option<Range<usize>> {
    let byte = |c: usize| if c == s.chars().count() { Some(s.len()) } else { s.char_indices().nth(c).map(|(i, _)| i) };
    let (a, b) = (byte(r.start)?, byte(r.end)?);
    (a <= b).then_some(a..b)
}

/// Lengths of the common prefix (at most `max_prefix`) and suffix of `a` and `b` (on char
/// boundaries, not overlapping).
fn common_affixes(a: &str, b: &str, max_prefix: usize) -> (usize, usize) {
    let mut p = 0;
    for ((i, x), y) in a.char_indices().zip(b.chars()) {
        if x != y || i + x.len_utf8() > max_prefix {
            break;
        }
        p = i + x.len_utf8();
    }
    let max_s = (a.len() - p).min(b.len() - p);
    let mut s = 0;
    for (x, y) in a[p..].chars().rev().zip(b[p..].chars().rev()) {
        if x != y || s + x.len_utf8() > max_s {
            break;
        }
        s += x.len_utf8();
    }
    (p, s)
}

/// Byte length of the borrowed plain-text segments.
fn plain_len(segments: &[&str]) -> usize {
    segments.iter().map(|s| s.len()).sum()
}

/// Is `byte` a UTF-8 boundary in the concatenation of `segments`? Run edges are boundaries.
fn plain_is_boundary(segments: &[&str], byte: usize) -> bool {
    let mut at = 0usize;
    for segment in segments {
        if byte == at {
            return true;
        }
        if byte < at + segment.len() {
            return segment.is_char_boundary(byte - at);
        }
        at += segment.len();
    }
    byte == at
}

/// Append the bytes `start..end` of the concatenation of `segments` to `out`.
fn copy_plain_range(segments: &[&str], start: usize, end: usize, out: &mut String) {
    let mut at = 0usize;
    for segment in segments {
        let segment_end = at + segment.len();
        let from = start.max(at);
        let to = end.min(segment_end);
        if from < to {
            let (a, b) = (from - at, to - at);
            if segment.is_char_boundary(a) && segment.is_char_boundary(b) {
                out.push_str(&segment[a..b]);
            }
        }
        if segment_end >= end {
            break;
        }
        at = segment_end;
    }
}

/// Move `i` back to a character boundary in the concatenation of `segments` (deleting fewer
/// bytes rather than splitting UTF-8).
fn plain_floor_boundary(segments: &[&str], mut i: usize) -> usize {
    i = i.min(plain_len(segments));
    while i > 0 && !plain_is_boundary(segments, i) {
        i -= 1;
    }
    i
}

/// Move `i` forward to a character boundary in the concatenation of `segments`.
fn plain_ceil_boundary(segments: &[&str], mut i: usize) -> usize {
    let len = plain_len(segments);
    i = i.min(len);
    while i < len && !plain_is_boundary(segments, i) {
        i += 1;
    }
    i
}

/// The byte offset `n` characters before `at` (a boundary) in `segments`, stopping at the start of
/// the text. Walks the runs, so a large IME deletion count never copies the whole text.
fn plain_chars_before(segments: &[&str], at: usize, n: usize) -> usize {
    if n == 0 {
        return at;
    }
    let mut remaining = n;
    let mut end = plain_len(segments);
    for segment in segments.iter().rev() {
        let start = end - segment.len();
        let local_end = at.min(end).saturating_sub(start);
        if local_end > 0 && segment.is_char_boundary(local_end) {
            let before = &segment[..local_end];
            let count = before.chars().count();
            if count >= remaining {
                return start + before.char_indices().rev().nth(remaining.saturating_sub(1)).map_or(0, |(i, _)| i);
            }
            remaining -= count;
        }
        end = start;
    }
    0
}

/// The byte offset `n` characters after `at` (a boundary) in `segments`, stopping at the end of
/// the text.
fn plain_chars_after(segments: &[&str], at: usize, n: usize) -> usize {
    if n == 0 {
        return at;
    }
    let len = plain_len(segments);
    let mut remaining = n;
    let mut acc = 0usize;
    for segment in segments {
        let end = acc + segment.len();
        let local_start = at.saturating_sub(acc);
        if end > at && segment.is_char_boundary(local_start) {
            let after = &segment[local_start..];
            let count = after.chars().count();
            if count >= remaining {
                return acc + local_start + after.char_indices().nth(remaining).map_or(after.len(), |(i, _)| i);
            }
            remaining -= count;
        }
        acc = end;
    }
    len
}

impl Tool for TypeTool {
    fn id(&self) -> &'static str {
        match (self.mode, self.vertical) {
            (Mode::Type, false) => "type",
            (Mode::Type, true) => "verticalType",
            (Mode::Area, false) => "areaType",
            (Mode::Area, true) => "verticalAreaType",
            (Mode::OnPath, false) => "typeOnPath",
            (Mode::OnPath, true) => "verticalTypeOnPath",
        }
    }
    fn busy(&self) -> bool {
        self.press.is_some() || self.selecting
    }
    fn wants_text(&self) -> bool {
        self.editing.is_some()
    }
    /// Escape while editing is the tool's: it goes on to the Selection tool (#1686).
    fn claims_key(&self, _: &ToolContext, key: ToolKey) -> bool {
        key == ToolKey::Escape && self.editing.is_some()
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        match ev.kind {
            PointerKind::Down => {
                if let Some(byte) = self.hit_edited(cx, ev.pos) {
                    let same = self.clicks.0.is_some_and(|q| (q - ev.pos).hypot() <= cx.tol(4.0));
                    let n = if same { self.clicks.1 + 1 } else { 1 };
                    self.clicks = (Some(ev.pos), n);
                    let out = self.move_to(byte, ev.mods.shift);
                    self.goal_x = None;
                    if n >= 3 {
                        self.select_word_or_para(cx, true);
                        self.clicks.1 = 0;
                    } else {
                        self.selecting = true;
                    }
                    return out;
                }
                self.clicks = (None, 0);
                // A press on other type edits it from there, so a drag selects its text at once,
                // as it does in the type being edited.
                if let Some(out) = self.edit_at(cx, ev.pos) {
                    self.selecting = true;
                    return out;
                }
                self.press = Some((ev.pos, self.snap.press(cx, ev.pos, self.editing.as_slice(), None)));
                self.drag = None;
                vec![]
            }
            PointerKind::Drag => {
                if self.selecting {
                    if let Some(t) = self.current(cx) {
                        let lay = self.layout(&t);
                        self.caret = vectorcraft_text::hit_byte(&lay, t.xf.inverse() * ev.pos);
                        self.clicks = (None, 0);
                    }
                } else if self.press.is_some() {
                    self.drag = Some(self.snap.drag(cx, ev.pos, None));
                }
                vec![]
            }
            PointerKind::Up => {
                if self.selecting {
                    self.selecting = false;
                    return vec![];
                }
                self.snap.clear();
                let Some((start, at)) = self.press.take() else { return vec![] };
                self.on_up(cx, start, at)
            }
            PointerKind::DoubleClick => {
                if self.editing.is_some() && self.hit_edited(cx, ev.pos).is_some() {
                    self.select_word_or_para(cx, false);
                    self.clicks = (Some(ev.pos), 2);
                }
                vec![]
            }
            PointerKind::Move => {
                // The type being edited is no target: its guides would only point at itself.
                self.snap.hover(cx, ev.pos, self.editing.as_slice(), None);
                vec![]
            }
        }
    }
    fn text_input(&mut self, cx: &ToolContext, s: &str) -> Vec<Action> {
        if self.editing.is_none() {
            return vec![];
        }
        // An IME commit replaces its marked text (an empty commit just removes it).
        let committing = self.preedit.is_some();
        // A space typed on its own at a caret may start a list.
        let list_space = !committing && s == " " && self.caret == self.anchor;
        if committing && s.is_empty() {
            return self.ime_preedit(cx, "", None);
        }
        if let Some(p) = self.preedit.take() {
            self.anchor = p.range.start;
            self.caret = p.range.end;
        }
        let Some(s) = self.input_text(cx, s) else { return vec![] };
        self.clicks = (None, 0);
        // Pasting what we copied keeps its formatting (unless pasted text is kept plain).
        let clip: String = self.clipboard.iter().map(|r| r.text.as_str()).collect();
        if !committing && s.len() > 1 && s == clip && !cx.paste_plain_text {
            let runs = self.clipboard.clone();
            return self.replace(cx, "", Some(runs));
        }
        let mut out = self.replace(cx, &s, None);
        if list_space {
            out.extend(self.start_typed_list(cx));
        }
        out
    }
    /// Would [`Tool::text_input`] change `s` before inserting it? Control characters are dropped,
    /// `\r` becomes `\n`, Typographer's Quotes rewrites a straight quote, and copied text keeps
    /// its run styles. An IME commit the application rewrites is an application edit, not an
    /// accepted IME insertion (its receipt must stay `Other`).
    fn text_input_is_verbatim(&self, cx: &ToolContext, s: &str) -> bool {
        if self.editing.is_none() {
            return true;
        }
        if s.is_empty() {
            return true;
        }
        // A straight quote's direction depends on the character before the insertion. During a
        // commit this check still sees the pre-edit at the caret, while `text_input` replaces it
        // from its start; with a custom pair whose opening quote is straight, that can look
        // verbatim here and then be rewritten. Never claim verbatim for one.
        if cx.doc.setup.typographers_quotes && matches!(s, "\"" | "'") {
            return false;
        }
        let Some(normalized) = self.input_text(cx, s) else { return false };
        if normalized != s {
            return false;
        }
        let clip: String = self.clipboard.iter().map(|r| r.text.as_str()).collect();
        !(self.preedit.is_none() && normalized.len() > 1 && normalized == clip && !cx.paste_plain_text)
    }
    /// Whether the typing session still describes the edited document; a session ended by an outside
    /// command reports false until a replacement commit starts a new one.
    fn ime_session_matches_document(&self, cx: &ToolContext) -> bool {
        match (self.editing, &self.typing) {
            (Some(id), Some(ty)) => Self::text(cx, id).is_none_or(|t| ty.cur == t.runs),
            // An outside command settled the session: the document may have changed under it, and
            // the commit that starts a new session cannot claim the IME made that change.
            (Some(_), None) => !self.typing_ended,
            _ => true,
        }
    }
    /// Handle an editing key: ignore keys while an IME composes, remap arrows in vertical text, step
    /// type attributes with Alt, and otherwise edit, navigate or end the session.
    fn key(&mut self, cx: &ToolContext, key: ToolKey, mods: Mods) -> Vec<Action> {
        // The IME owns the keys while it composes (the UI doesn't send them; this is a guard).
        if self.preedit.is_some() {
            return vec![];
        }
        let Some(t) = self.current(cx) else {
            self.editing = None;
            return vec![];
        };
        let text = t.plain_text();
        let len = text.len();
        self.caret = self.caret.min(len);
        self.anchor = self.anchor.min(len);
        let (a, b) = self.sel();
        // Alt+arrows step the type by the Preferences › Type increments (Cmd/Ctrl too: five steps).
        if mods.alt
            && let Some(id) = self.editing
            && let Some((attribute, by)) = step_key(key, mods.shift, a == b)
        {
            let mut out = self.commit();
            let by = if mods.cmd { by * 5.0 } else { by };
            out.push(Action::Exec("type.step".into(), json!({"id": id.0, "start": a, "end": b, "attribute": attribute, "by": by})));
            return out;
        }
        let word = mods.cmd || mods.alt;
        let lay = self.layout(&t);
        let key = if lay.vertical {
            match key {
                ToolKey::Up => ToolKey::Left,
                ToolKey::Down => ToolKey::Right,
                ToolKey::Right => ToolKey::Up,
                ToolKey::Left => ToolKey::Down,
                other => other,
            }
        } else {
            key
        };
        let vertical = matches!(key, ToolKey::Up | ToolKey::Down);
        if !vertical {
            self.goal_x = None;
        }
        match key {
            // Escape ends editing and goes back to the tool used before double-clicking into type
            // (or to the Selection tool by default), the type selected: the keyboard is free for
            // tool shortcuts again (#1686).
            ToolKey::Escape => {
                let target = self.prev_tool.take().unwrap_or_else(|| "selection".into());
                let mut out = self.finish(cx);
                out.push(Action::SwitchTool(target));
                out
            }
            ToolKey::Enter => self.replace(cx, "\n", None),
            ToolKey::Tab => self.replace(cx, "\t", None),
            ToolKey::Backspace | ToolKey::Delete if a != b => self.replace(cx, "", None),
            ToolKey::Backspace if self.caret > 0 => {
                self.anchor = if word { edit::prev_word(&text, self.caret) } else { edit::prev_char(&text, self.caret) };
                self.replace(cx, "", None)
            }
            // Backspace takes back one character (a mistyped Thai tone mark, say); Delete, the
            // Left and Right arrows a whole cluster.
            ToolKey::Delete if self.caret < len => {
                self.anchor = if word { edit::next_word(&text, self.caret) } else { edit::next_cluster(&text, self.caret) };
                self.replace(cx, "", None)
            }
            ToolKey::Left => {
                let to = if a != b && !mods.shift {
                    if lay.glyphs.iter().any(|g| g.rtl)
                        && vectorcraft_text::caret_position(&lay, a).0.x > vectorcraft_text::caret_position(&lay, b).0.x
                    {
                        b
                    } else {
                        a
                    }
                } else if word {
                    edit::prev_word(&text, self.caret)
                } else {
                    if lay.glyphs.iter().any(|g| g.rtl) {
                        vectorcraft_text::caret_horizontal_cluster(&lay, &text, self.caret, false)
                    } else {
                        edit::prev_cluster(&text, self.caret)
                    }
                };
                self.move_to(to, mods.shift)
            }
            ToolKey::Right => {
                let to = if a != b && !mods.shift {
                    if lay.glyphs.iter().any(|g| g.rtl)
                        && vectorcraft_text::caret_position(&lay, a).0.x > vectorcraft_text::caret_position(&lay, b).0.x
                    {
                        a
                    } else {
                        b
                    }
                } else if word {
                    edit::next_word(&text, self.caret)
                } else {
                    if lay.glyphs.iter().any(|g| g.rtl) {
                        vectorcraft_text::caret_horizontal_cluster(&lay, &text, self.caret, true)
                    } else {
                        edit::next_cluster(&text, self.caret)
                    }
                };
                self.move_to(to, mods.shift)
            }
            ToolKey::Up | ToolKey::Down => {
                let d = if key == ToolKey::Up { -1 } else { 1 };
                let from = if a != b && !mods.shift { if d < 0 { a } else { b } } else { self.caret };
                let x = self.goal_x.unwrap_or_else(|| lay.logical_point(vectorcraft_text::caret_position(&lay, from).0).x);
                let to = if mods.cmd {
                    // Cmd+Up/Down: paragraph start / end.
                    let p = edit::paragraph_at(&text, from);
                    if d < 0 {
                        if from == p.start { edit::paragraph_at(&text, p.start.saturating_sub(1)).start } else { p.start }
                    } else if from == p.end {
                        edit::paragraph_at(&text, (p.end + 1).min(len)).end
                    } else {
                        p.end
                    }
                } else {
                    vectorcraft_text::caret_vertical(&lay, from, d, x)
                };
                let out = self.move_to(to, mods.shift);
                self.goal_x = Some(x);
                out
            }
            ToolKey::Home => {
                let to = if mods.cmd { 0 } else { vectorcraft_text::line_home(&lay, self.caret) };
                self.move_to(to, mods.shift)
            }
            ToolKey::End => {
                let to = if mods.cmd { len } else { vectorcraft_text::line_end_of(&lay, self.caret) };
                self.move_to(to, mods.shift)
            }
            _ => vec![],
        }
    }
    fn ime_preedit(&mut self, cx: &ToolContext, text: &str, active_chars: Option<Range<usize>>) -> Vec<Action> {
        if self.editing.is_none() {
            return vec![];
        }
        // A clear with nothing marked (some integrations send one when the IME is enabled) must
        // not delete the selection.
        if text.is_empty() && self.preedit.is_none() {
            return vec![];
        }
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        // The new marked text replaces the previous one, or the selection when it starts.
        if let Some(p) = self.preedit.take() {
            self.anchor = p.range.start;
            self.caret = p.range.end;
        }
        self.clicks = (None, 0);
        let mut out = self.replace(cx, &text, None);
        if text.is_empty() {
            // Cancelled before anything else was typed: no empty undo step.
            if self.typing.as_ref().is_some_and(|ty| ty.cur == ty.base) {
                self.typing = None;
                out.push(Action::Cancel);
            }
            return out;
        }
        let end = self.caret;
        let start = end.saturating_sub(text.len());
        self.preedit = Some(Preedit { range: start..end, active: active_chars.and_then(|r| char_range_to_bytes(&text, r)) });
        out
    }
    /// Delete the requested bytes or characters before and after the selection or marked text,
    /// preserving the selection direction and composition and refusing a stale session; byte counts
    /// round inward to UTF-8 boundaries.
    fn ime_delete_surrounding(&mut self, cx: &ToolContext, before: usize, after: usize, unit: SurroundingUnit) -> Vec<Action> {
        let Some(id) = self.editing else { return vec![] };
        // The document changed under the session (undo, a panel edit): the ranges below would index
        // text that is no longer there. The commit that follows resets the session; do not delete
        // from the wrong text in the meantime.
        if let Some(t) = Self::text(cx, id)
            && self.typing.as_ref().is_some_and(|ty| ty.cur != t.runs)
        {
            return vec![];
        }
        // Walk the runs in place: joining the whole edited text here would copy a large document
        // for every IME deletion event, though only the characters next to the selection matter.
        let Some(segments) = self.plain_segments(cx) else { return vec![] };
        let len = plain_len(&segments);
        // The text the IME's following commit replaces (the selection, or the marked text while
        // composing). The surrounding deletion never removes it: it deletes only the bytes/characters
        // before and after it, preserving the selection direction and the composition.
        let (raw_start, raw_end) = match &self.preedit {
            Some(p) => (p.range.start.min(len), p.range.end.min(len)),
            None => (self.caret.min(self.anchor).min(len), self.caret.max(self.anchor).min(len)),
        };
        let (before_start, start, end, after_end) = match unit {
            SurroundingUnit::Bytes => {
                // Inward rounding: a count ending inside a scalar leaves that scalar whole. The
                // preceding side moves its start forward (ceil), the following side moves its end
                // backward (floor). Do not reverse these: that would delete part of a character.
                let start = plain_floor_boundary(&segments, raw_start);
                let end = plain_ceil_boundary(&segments, raw_end);
                let before_start = plain_ceil_boundary(&segments, start.saturating_sub(before));
                let after_end = plain_floor_boundary(&segments, end.saturating_add(after));
                (before_start, start, end, after_end)
            }
            SurroundingUnit::Chars => {
                let start = plain_floor_boundary(&segments, raw_start);
                let end = plain_ceil_boundary(&segments, raw_end);
                let before_start = plain_chars_before(&segments, start, before);
                let after_end = plain_chars_after(&segments, end, after);
                (before_start, start, end, after_end)
            }
        };
        let mut ranges = Vec::with_capacity(2);
        if before_start < start {
            ranges.push((before_start, start));
        }
        if end < after_end {
            ranges.push((end, after_end));
        }
        if ranges.is_empty() {
            return vec![];
        }
        self.delete_ranges(cx, &ranges)
    }
    /// Return the active Type tool's stable IME owner (document uid, target id, replacement epoch),
    /// or `None` when nothing is being edited.
    fn ime_target(&self, cx: &ToolContext) -> Option<(u64, u64, u64)> {
        let id = self.editing?;
        Self::text(cx, id)?;
        Some((cx.revision.0, id.0, self.edit_session))
    }
    /// Return a bounded logical text window around the caret, anchor and optional pre-edit; a stale
    /// typing session publishes the document's current text, and unusable or non-boundary offsets
    /// return `Unavailable` rather than stale data.
    fn ime_surrounding(&self, cx: &ToolContext, max_bytes: usize) -> ImeSurrounding {
        let Some(id) = self.editing else {
            return ImeSurrounding::Unavailable("no text object is being edited");
        };
        let Some(current) = Self::text(cx, id) else {
            return ImeSurrounding::Unavailable("the edited text object is gone");
        };
        // The document changed under the typing session (undo, a panel or control edit): the
        // session's runs, marked-text range and caret offsets no longer describe the document.
        // Publish the document's own text (never the stale session's, which the IME cannot see);
        // the marked text has no range in it, so it is excluded from the logical view. The
        // commit-time check (`ime_session_matches_document`) reports the reset as application
        // origin instead of letting a receipt describe the wrong text.
        let stale = self.typing.as_ref().is_some_and(|ty| ty.cur != current.runs);
        if self.preedit.is_some() && self.typing.is_none() {
            return ImeSurrounding::Unavailable("the IME pre-edit has no typing session");
        }
        let Some(segments) = self.plain_segments(cx) else {
            return ImeSurrounding::Unavailable("the edited text object is gone");
        };
        let len = plain_len(&segments);
        let caret = self.caret.min(len);
        let anchor = self.anchor.min(len);
        if !plain_is_boundary(&segments, caret) || !plain_is_boundary(&segments, anchor) {
            return ImeSurrounding::Unavailable("the caret or selection anchor is not a UTF-8 boundary");
        }
        let preedit = match self.preedit.as_ref().filter(|_| !stale) {
            Some(p) => {
                let start = p.range.start.min(len);
                let end = p.range.end.min(len);
                if start > end || !plain_is_boundary(&segments, start) || !plain_is_boundary(&segments, end) {
                    return ImeSurrounding::Unavailable("the IME pre-edit is not on UTF-8 boundaries");
                }
                Some(start..end)
            }
            None => None,
        };
        ime_surrounding_window(&segments, preedit, len, caret, anchor, max_bytes)
    }
    /// Whether an IME pre-edit is currently shown for this Type-tool session.
    fn composing(&self) -> bool {
        self.preedit.is_some()
    }
    fn ime_caret(&self, cx: &ToolContext) -> Option<(Point, Point)> {
        let t = self.current(cx)?;
        let lay = self.layout(&t);
        let len = t.plain_text().len();
        // The candidate window follows the clause being converted, else the caret.
        let at = match &self.preedit {
            Some(p) => p.range.start + p.active.as_ref().map_or(0, |r| r.start),
            None => self.caret.min(self.anchor),
        };
        let (a, b) = vectorcraft_text::caret_position(&lay, at.min(len));
        Some((t.xf * a, t.xf * b))
    }
    fn deactivate(&mut self, cx: &ToolContext) -> Vec<Action> {
        self.prev_tool = None;
        self.finish(cx)
    }
    /// Editing ends when a command takes the text out of the selection (Deselect, another object
    /// picked in the Layers panel) or removes it. After an Undo that takes back a list typed
    /// characters started, the caret goes on after the characters, and after its Redo the caret
    /// goes before the paragraph's text.
    fn after_command(&mut self, cx: &ToolContext) -> Vec<Action> {
        self.after_typed_list(cx);
        let Some(id) = self.editing else { return vec![] };
        let mut cur = Some(id).filter(|i| cx.doc.node(*i).is_some());
        while let Some(c) = cur {
            if cx.selection.contains(c) {
                return vec![];
            }
            cur = cx.doc.parent_of(c);
        }
        self.finish(cx)
    }
    /// After `text.create` / `text.createInPath` the engine selects the new object; edit it.
    fn notify(&mut self, cx: &ToolContext, what: &str) {
        if what == "text.editNew"
            && let Some(id) = cx.selection.objects.first().copied()
        {
            self.start_editing(id, 0);
            let t = Self::text(cx, id);
            self.fresh = t.filter(|t| matches!(t.kind, TextKind::Point)).map(|_| id);
            // Placeholder text comes selected: typing replaces it.
            self.caret = t.map_or(0, |t| edit::runs_len(&t.runs));
        }
    }
    /// `{editing, start, end, caret, anchor, typing, composing}` — the Character panel styles
    /// `start..end`.
    fn options(&self) -> Value {
        match self.editing {
            Some(id) => {
                // Offsets may exceed the text after Select All; callers clamp to the text length.
                let (a, b) = self.sel();
                json!({
                    "editing": id.0,
                    "start": a,
                    "end": b,
                    "caret": self.caret,
                    "anchor": self.anchor,
                    "typing": self.typing.is_some(),
                    "composing": self.preedit.is_some(),
                    "prevTool": self.prev_tool,
                })
            }
            None => json!({"editing": null, "prevTool": self.prev_tool}),
        }
    }
    /// `select {start, end}` / `selectAll` / `copy` (with `runs` of the selection) /
    /// `commitTyping` (the caller commits the engine interaction).
    fn set_option(&mut self, key: &str, v: &Value) {
        match key {
            // Transient selection transferred between Type and Eyedropper; never a saved setting.
            "textSelection" => {
                if let (Some(id), Some(a), Some(b)) = (
                    v.get("editing").and_then(Value::as_u64),
                    v.get("anchor").or_else(|| v.get("start")).and_then(Value::as_u64),
                    v.get("caret").or_else(|| v.get("end")).and_then(Value::as_u64),
                ) && let (Ok(a), Ok(b)) = (usize::try_from(a), usize::try_from(b))
                {
                    self.start_editing(NodeId(id), b);
                    self.anchor = a;
                }
            }
            "prevTool" | "previousTool" => {
                self.prev_tool = v.as_str().map(String::from);
            }
            "commitTyping" | "endTyping" => {
                self.typing = None;
                self.preedit = None;
                self.typing_ended = true;
            }
            "selectAll" if self.editing.is_some() => {
                self.anchor = 0;
                self.caret = usize::MAX / 4;
            }
            "select" if self.editing.is_some() => {
                let g = |k: &str| v.get(k).and_then(Value::as_u64).map(|x| x as usize);
                if let (Some(a), Some(b)) = (g("start"), g("end")) {
                    self.anchor = a;
                    self.caret = b;
                }
            }
            "copy" => {
                if let Ok(runs) = serde_json::from_value::<Vec<TextRun>>(v.clone()) {
                    self.clipboard = runs;
                }
            }
            "stopEditing" => {
                self.typing = None;
                self.preedit = None;
                self.editing = None;
                self.prev_tool = None;
            }
            _ => {}
        }
    }
    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let mut o = self.snap.guides().to_vec();
        if let (Some((_, s)), Some(d)) = (self.press, self.drag) {
            // The area being drawn (along the Constrain Angle's axes when one is set, as `on_up`
            // makes it) and its size.
            let size = match vectorcraft_geom::constrain_axes(cx.constrain_angle) {
                None => {
                    let r = Rect::from_points(s, d);
                    o.push(Overlay::Marquee(r));
                    r.size()
                }
                Some(axes) => {
                    let back = axes.inverse();
                    let r = Rect::from_points(back * s, back * d);
                    let mut path = r.to_path(0.1);
                    path.apply_affine(axes);
                    o.push(Overlay::Path { path, color: [79, 128, 255], width: 1.0, dashed: true });
                    r.size()
                }
            };
            if cx.measurement_labels {
                o.push(Overlay::Measure { p: d, text: cx.size_label(size.width, size.height) });
            }
        }
        // The ports of selected area type and type on a path, as many as show ([`PORTS_MAX`]), the
        // edited one's included.
        let shown = ports_shown(cx.doc, &cx.selection.objects);
        if shown {
            for id in cx.selection.objects.iter().filter(|id| Some(**id) != self.editing) {
                if let Some(t) = ported(cx.doc, *id) {
                    let lay = self.layout(t);
                    ports(cx, *id, t, lay.overflow, &mut o);
                }
            }
        }
        let Some(t) = self.current(cx) else { return o };
        let lay = self.layout(&t);
        let len = t.plain_text().len();
        // Frame / baseline path of the edited object.
        let frame: Option<BezPath> = match &t.kind {
            TextKind::Area { frame } => Some(frame.to_bezpath()),
            TextKind::OnPath { path, .. } => Some(path.to_bezpath()),
            TextKind::Point => None,
        };
        if let Some(mut f) = frame {
            f.apply_affine(t.xf);
            o.push(Overlay::Path { path: f, color: [79, 128, 255], width: 1.0, dashed: false });
        }
        let (a, b) = (self.caret.min(self.anchor).min(len), self.caret.max(self.anchor).min(len));
        if let Some(pe) = &self.preedit {
            // Marked text: a thin underline, a thick one under the clause being converted.
            let under = |o: &mut Vec<Overlay>, r: Range<usize>, width: f32| {
                for q in vectorcraft_text::selection_quads(&lay, r.start.min(len), r.end.min(len)) {
                    let (l, rr) = (t.xf * q[3], t.xf * q[2]);
                    o.push(Overlay::Path { path: vectorcraft_geom::Line::new(l, rr).to_path(0.1), color: [0, 0, 0], width, dashed: false });
                }
            };
            under(&mut o, pe.range.clone(), 1.0);
            match &pe.active {
                Some(r) if !r.is_empty() => under(&mut o, pe.range.start + r.start..pe.range.start + r.end, 2.5),
                _ => {
                    let at = pe.range.start + pe.active.as_ref().map_or(pe.range.len(), |r| r.start);
                    let (p, q) = vectorcraft_text::caret_position(&lay, at.min(len));
                    o.push(Overlay::Line { a: t.xf * p, b: t.xf * q, color: [0, 0, 0], dashed: false });
                }
            }
        } else if a == b {
            let (p, q) = vectorcraft_text::caret_position(&lay, self.caret.min(len));
            o.push(Overlay::Line { a: t.xf * p, b: t.xf * q, color: [0, 0, 0], dashed: false });
        } else {
            for q in vectorcraft_text::selection_quads(&lay, a, b) {
                o.push(Overlay::Highlight { quad: q.map(|p| t.xf * p), color: [60, 120, 255, 90] });
            }
        }
        if let Some(id) = self.editing.filter(|id| shown && ported(cx.doc, *id).is_some()) {
            ports(cx, id, &t, lay.overflow, &mut o);
        }
        o
    }
    fn cursor(&self, cx: &ToolContext, p: Point, _m: Mods) -> Cursor {
        if self.mode != Mode::Type && Self::path_at(cx, p, self.mode == Mode::Area).is_some() {
            return Cursor::Crosshair;
        }
        if self.vertical { Cursor::TypeVertical } else { Cursor::Text }
    }
}

/// The logical IME surrounding window: the pre-edit is removed and the caret/anchor offsets
/// are remapped around it. `preedit` has already been validated against `segments`.
fn ime_surrounding_window(
    segments: &[&str],
    preedit: Option<Range<usize>>,
    len: usize,
    caret: usize,
    anchor: usize,
    max_bytes: usize,
) -> ImeSurrounding {
    let removed = preedit.as_ref().map_or(0, |r| r.end - r.start);
    let logical_len = len - removed;
    let to_logical = |byte: usize| -> usize {
        match &preedit {
            Some(r) if byte > r.start && byte < r.end => r.start,
            Some(r) if byte >= r.end => byte - removed,
            _ => byte,
        }
    };
    let cursor = to_logical(caret);
    let anchor = to_logical(anchor);
    let selection_start = cursor.min(anchor);
    let selection_end = cursor.max(anchor);
    let selection_len = selection_end - selection_start;
    if selection_len > max_bytes {
        return ImeSurrounding::Unavailable("the selection is wider than the IME surrounding-text limit");
    }
    let budget = max_bytes - selection_len;
    let before_available = selection_start;
    let after_available = logical_len - selection_end;
    let mut before = (budget / 2).min(before_available);
    let mut after = (budget - before).min(after_available);
    before = (budget - after).min(before_available);
    after = (budget - before).min(after_available);
    let mut window_start = selection_start - before;
    let mut window_end = selection_end + after;
    let logical_boundary = |byte: usize| -> bool {
        match &preedit {
            Some(r) if byte == r.start => true,
            Some(r) if byte < r.start => plain_is_boundary(segments, byte),
            Some(_) => plain_is_boundary(segments, byte + removed),
            None => plain_is_boundary(segments, byte),
        }
    };
    while window_start < selection_start && !logical_boundary(window_start) {
        window_start += 1;
    }
    while selection_end < window_end && !logical_boundary(window_end) {
        window_end -= 1;
    }
    let to_original = |byte: usize| -> usize {
        match &preedit {
            Some(r) if byte > r.start => byte + removed,
            _ => byte,
        }
    };
    let original_start = to_original(window_start);
    let original_end = to_original(window_end);
    let mut text = String::with_capacity(window_end - window_start);
    match &preedit {
        Some(r) => {
            if window_start < r.start {
                let first_end = original_end.min(r.start);
                if original_start < first_end {
                    copy_plain_range(segments, original_start, first_end, &mut text);
                }
            }
            if r.start < window_end {
                let second_start = r.end.max(original_start);
                if second_start < original_end {
                    copy_plain_range(segments, second_start, original_end, &mut text);
                }
            }
        }
        None => {
            if original_start < original_end {
                copy_plain_range(segments, original_start, original_end, &mut text);
            }
        }
    }
    debug_assert_eq!(text.len(), window_end - window_start, "removing the pre-edit must preserve the logical window length");
    ImeSurrounding::Available { text, cursor: cursor - window_start, anchor: anchor - window_start }
}

#[cfg(test)]
#[path = "text_tests.rs"]
mod tests;
