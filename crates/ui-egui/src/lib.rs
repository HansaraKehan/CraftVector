//! VectorCraft's egui frontend: an Illustrator-style UI over `vectorcraft-engine`.
//!
//! The UI is thin: every action goes through [`VectorcraftApp::run`], which dispatches UI commands
//! (view/window) here and everything else to the engine. The same entry point serves menus,
//! shortcuts, the ⌘K palette and the control channel ([`control`]).
#![forbid(unsafe_code)]

/// Translate a UI string literal into the language the UI is drawn in (see [`i18n`]).
#[macro_export]
macro_rules! tl {
    ($s:expr) => {
        $crate::i18n::t($s)
    };
}

pub mod background;
mod brand;
pub mod canvas;
pub mod chrome;
mod clipboard_probe;
pub mod community;
pub mod control;
pub mod credits;
pub mod cursors;
pub mod dialogs;
pub mod dock;
mod edit_toolbar;
pub mod export_progress;
pub mod find_font;
pub mod font_menu;
mod frame_rate;
mod free_transform;
pub mod graphics;
pub mod i18n;
pub mod icon_data;
pub mod icons;
pub(crate) mod ime;
pub mod io;
pub mod keymap;
pub mod layout_migration;
pub mod menus;
pub mod native_menu;
pub mod opening;
pub mod palette;
mod panel_docking;
pub mod panels;
pub mod picks;
pub mod place;
pub mod placing;
pub mod plugin_runs;
pub mod prefs_dialog;
pub mod print;
pub mod recovery;
pub mod render_worker;
pub mod screen_color;
mod scrub;
pub mod shortcut_editor;
pub mod shortcuts;
pub mod state;
pub mod stylus;
pub mod sysclip;
pub mod theme;
pub mod thumb_worker;
pub mod titlebar;
pub mod toolbar;
mod tools_float;
pub mod touch;
mod ui_fonts;
pub mod unsaved;
pub mod updates_ui;
pub mod widgets;
pub mod workspaces;

mod document_drag;
#[cfg(test)]
mod tests_adjust;
#[cfg(test)]
mod tests_aisave;
#[cfg(test)]
mod tests_automation;
#[cfg(test)]
mod tests_background;
#[cfg(test)]
mod tests_clipboard;
#[cfg(test)]
mod tests_contextmenu;
#[cfg(test)]
mod tests_cut;
#[cfg(test)]
mod tests_distortkeys;
#[cfg(test)]
mod tests_docsetup;
#[cfg(test)]
mod tests_document_drag;
#[cfg(test)]
mod tests_font_menu;
#[cfg(test)]
mod tests_fonts;
#[cfg(test)]
mod tests_guide_escape;
#[cfg(test)]
mod tests_home;
#[cfg(test)]
mod tests_labels;
#[cfg(test)]
mod tests_lifecycle;
#[cfg(test)]
mod tests_nativemenu;
#[cfg(test)]
mod tests_nativeoptions;
#[cfg(test)]
mod tests_numfields;
#[cfg(test)]
mod tests_overprint;
#[cfg(test)]
mod tests_paintchips;
#[cfg(test)]
mod tests_pastechords;
#[cfg(test)]
mod tests_pathtype;
#[cfg(test)]
mod tests_pdfoutput;
#[cfg(test)]
mod tests_picks;
#[cfg(test)]
mod tests_place;
#[cfg(test)]
mod tests_plugin_runs;
#[cfg(test)]
mod tests_plugins;
#[cfg(test)]
mod tests_printps;
#[cfg(test)]
mod tests_printtiling;
#[cfg(test)]
mod tests_puppetwarp;
#[cfg(test)]
mod tests_recolor;
#[cfg(test)]
mod tests_recovery;
#[cfg(test)]
mod tests_removeanchors;
#[cfg(test)]
mod tests_rotateview;
#[cfg(test)]
mod tests_ruler_origin;
#[cfg(test)]
mod tests_save;
#[cfg(test)]
mod tests_saveext;
#[cfg(test)]
mod tests_screenmode;
#[cfg(test)]
mod tests_screenshots;
#[cfg(test)]
mod tests_script_check;
#[cfg(test)]
mod tests_scrub;
#[cfg(test)]
mod tests_selectall;
#[cfg(test)]
mod tests_slices;
#[cfg(test)]
mod tests_svg;
#[cfg(test)]
mod tests_svgsave;
#[cfg(test)]
mod tests_synthetic;
#[cfg(test)]
mod tests_sysclip;
#[cfg(test)]
mod tests_sysclip_emf;
#[cfg(test)]
mod tests_sysclip_probe;
#[cfg(test)]
mod tests_tab_overflow;
#[cfg(test)]
mod tests_taskbar;
#[cfg(test)]
mod tests_transparencygrid;
#[cfg(test)]
mod tests_widthtool;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use vectorcraft_engine::cmd::fileio;
use vectorcraft_engine::file_access::{self, AutomationRoots};
use vectorcraft_engine::{Session, ViewInfo};

pub use control::{ControlRequest, ControlResponse, Ticket as ControlTicket};
pub use state::{UiState, View};
pub use sysclip::{ClipboardProbeFactory, SystemClipboard};

/// A file type a dialog filters by: its label, which shows translated, and its extensions without
/// the dot.
pub type FileFilter = (String, Vec<String>);

/// What a file dialog shows: a suggested file name (save dialogs), the folder to start in and the
/// file-type filters.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FilePick {
    pub name: String,
    pub folder: Option<String>,
    /// The default type first; empty = every file.
    pub filters: Vec<FileFilter>,
}

impl FilePick {
    /// A save dialog suggesting `name`.
    pub fn named(name: &str) -> Self {
        Self { name: name.to_string(), ..Default::default() }
    }

    /// The file type `label` with `extensions`.
    pub fn filter(label: &str, extensions: &[&str]) -> FileFilter {
        (label.to_owned(), extensions.iter().map(|e| (*e).to_owned()).collect())
    }

    /// The file types of `list`, as the engine lists them: `(label, extensions)`.
    pub fn filters<'a>(list: impl IntoIterator<Item = (&'a str, &'a [&'a str])>) -> Vec<FileFilter> {
        list.into_iter().map(|(label, extensions)| Self::filter(label, extensions)).collect()
    }
}

pub type PickOpen = Box<dyn FnMut(&FilePick) -> Option<String>>;
pub type PickSave = Box<dyn FnMut(&FilePick) -> Option<String>>;
pub type ReadFn = Box<dyn Fn(&str) -> Result<Vec<u8>, String>>;
pub type WriteFn = Box<dyn FnMut(&str, &[u8]) -> Result<(), String>>;
pub type ExistsFn = Box<dyn Fn(&str) -> bool>;
pub type Inbox = Arc<Mutex<Vec<(String, Vec<u8>)>>>;
/// Starts a browser download of the bytes under a file name; an error when it couldn't be started
/// (the save or export then fails as a refused write does: #1693).
pub type DownloadFn = Box<dyn FnMut(&str, &[u8]) -> Result<(), String>>;

/// Opens a URL in the system browser.
pub type OpenUrlFn = Box<dyn FnMut(&str)>;

/// Whether a URL from a document may open in the browser: an `http:`, `https:` or `mailto:` URL
/// without control characters. The system hands other schemes (`file:`, application schemes) to
/// other programs.
pub(crate) fn browsable_url(url: &str) -> Result<(), String> {
    let scheme = url.split_once(':').map_or("", |(s, _)| s);
    if ["http", "https", "mailto"].iter().any(|k| scheme.eq_ignore_ascii_case(k)) && !url.chars().any(char::is_control) {
        Ok(())
    } else {
        Err(format!("only http, https and mailto URLs open in the browser, not “{url}”"))
    }
}

/// Shows a file in the system file manager, or opens it in its app.
pub type RevealFn = Box<dyn FnMut(&str) -> Result<(), String>>;

/// Platform services injected by the host app (desktop or web).
#[derive(Default)]
pub struct Services {
    /// Show an open dialog; returns a path.
    pub pick_open: Option<PickOpen>,
    /// Show a save dialog (suggested name, folder, file types); returns a path.
    pub pick_save: Option<PickSave>,
    pub read: Option<ReadFn>,
    pub write: Option<WriteFn>,
    /// Is there a file or folder at a path (desktop)? The app asks before replacing a file that a
    /// save panel's name names once the app gives it an extension ([`picks::ask_to_replace`]).
    /// Without it, no file counts as there.
    pub exists: Option<ExistsFn>,
    /// Files that arrived asynchronously (web open / drops): (name, bytes).
    pub inbox: Option<Inbox>,
    /// Web: trigger a browser download instead of writing a path.
    pub download: Option<DownloadFn>,
    /// Web: start an async open (bytes arrive via `inbox`).
    pub open_async: Option<Box<dyn FnMut()>>,
    /// Read the system clipboard's text (desktop). Without it, pasted text only arrives with
    /// egui's Paste event (web, and keyboard paste everywhere).
    pub clipboard_read: Option<Box<dyn FnMut() -> Option<String>>>,
    /// Open a URL in the system browser (desktop). Without it, egui opens it (a new tab on the web).
    pub open_url: Option<OpenUrlFn>,
    /// Show an open dialog for several files (File → Place…); returns their paths (none: cancelled).
    pub pick_open_multi: Option<Box<dyn FnMut() -> Vec<String>>>,
    /// Web: start an async pick of files to place (they arrive via `place_inbox`).
    pub place_async: Option<Box<dyn FnMut()>>,
    /// Files that arrived asynchronously to be placed (web: picked for Place, or dropped on the
    /// canvas).
    pub place_inbox: Option<place::PlaceInbox>,
    /// The system clipboard with all its formats (desktop): Copy offers text, SVG, PDF and PNG,
    /// Paste takes SVG, PDF, text and bitmaps from other apps. Without it, SVG text only (through
    /// egui and `clipboard_read`).
    pub system_clipboard: Option<Box<dyn SystemClipboard>>,
    /// A second system-clipboard handle, for checking whether Paste has something to take on a
    /// background thread (Linux: an X11 clipboard owner that never answers would freeze the UI).
    /// Taken on the first frame. Without it the check runs on the UI thread.
    pub clipboard_probe: Option<ClipboardProbeFactory>,
    /// File → Show in Folder: select a file in the system file manager (desktop).
    pub reveal: Option<RevealFn>,
    /// Write a file from any thread (desktop): lets Background Save and Export write off the UI
    /// thread ([`background`]). Without it they run at once.
    pub write_shared: Option<background::SharedWriteFn>,
    /// Write a file from any thread with what a filler writes into it (desktop): native saves
    /// stream into the file as they are encoded instead of being held whole in memory first.
    /// Without it they are encoded, then written with `write` / `write_shared`.
    pub stream_shared: Option<background::SharedStreamFn>,
    /// Open a file (or a folder) in the system's default app for it (desktop: Edit Original, Show
    /// Package). Without it (the web) those answer with an error.
    pub open_file: Option<RevealFn>,
    /// Show a folder picker; returns its path (desktop: Relink to Folder, Package).
    pub pick_folder: Option<Box<dyn FnMut() -> Option<String>>>,
    /// Show the system's color sampler, which picks a color anywhere on the screen (macOS, and
    /// browsers with an EyeDropper): Sample Color from Screen ([`screen_color`]). Without it that
    /// command answers with an error and the Color Picker's eyedropper button is gray.
    pub screen_color: Option<screen_color::StartSampler>,
    /// Where Data Recovery keeps its copies when not in a folder (the web's browser storage;
    /// tests): installed into the session ([`vectorcraft_engine::cmd::recovery`]).
    pub recovery_store: Option<std::sync::Arc<dyn vectorcraft_engine::cmd::recovery::RecoveryStore>>,
    /// File → Print: the system's printers and print queue (desktop), the browser's print dialog
    /// (web). Without it Print saves the job as a PDF.
    pub print: Option<Box<dyn print::PrintService>>,
    /// The macOS menu bar, when the desktop app installed one: the in-window menus are hidden then.
    pub native_menu: Option<native_menu::NativeMenu>,
    /// Show file dialogs off the UI thread (desktop Linux, where a dialog in line holds the window
    /// and the compositor finds it not answering, #592) or as sheets on the window (macOS, where a
    /// dialog in line aborts the app, #867): what asked runs again with the answer ([`picks`]).
    /// Without it they are shown in line.
    pub start_pick: Option<picks::StartPick>,
    /// [`Self::start_pick`]'s dialogs are sheets on the window (macOS): while one is open the app
    /// neither quits nor closes documents, as under the modal dialog before.
    pub dialogs_are_sheets: bool,
    /// The window is on Wayland: the tool cursors are painted into it instead of handed to the
    /// compositor as bitmaps ([`cursors::os_cursors`]).
    pub wayland: bool,
    /// On Wayland, no cursor theme can be loaded, so winit sets no pointer and the compositor's
    /// default one would never come back once the canvas hid it: the canvas keeps the system
    /// pointer visible next to the tool glyph ([`cursors::hide_system_pointer`], #2450).
    pub no_cursor_theme: bool,
    /// The pen pressure and pen end the host reads from the system itself (desktop Linux under
    /// X11 and XWayland, macOS and Windows). The canvas uses the pressure when a frame's events carry no touch force,
    /// and the end to switch to the Eraser tool and back. Without it, pen pressure comes from
    /// touch events only ([`stylus`]).
    pub stylus: Option<stylus::StylusFeed>,
    /// The host's rewrites of a window's raw input (pen barrel button, closing the macOS menu
    /// bar's open menu) for floating panels in OS windows of their own: eframe runs the host's
    /// `raw_input_hook` for the main window only.
    pub native_input_hook: Option<NativeInputHook>,
}

/// Rewrites one window's raw input before egui reads it ([`Services::native_input_hook`]).
pub type NativeInputHook = std::sync::Arc<dyn Fn(&mut egui::RawInput) + Send + Sync>;

/// Cached canvas raster.
pub struct CanvasCache {
    pub renderer: vectorcraft_render::Renderer,
    pub texture: Option<egui::TextureHandle>,
    pub key: Option<CacheKey>,
    /// The document the frame shown was rendered from, and the options it was rendered with, with
    /// [`Self::key`]: when only the document changes, the canvas renders again only where it
    /// changed ([`vectorcraft_render::changed`]).
    pub shown: Option<(std::sync::Arc<vectorcraft_doc::Document>, vectorcraft_render::RenderOptions)>,
    pub last_ms: f64,
    /// The frame shown drew raster effects from rasters made before their objects moved, or at a
    /// drag's lower resolution ([`vectorcraft_render::FrameStats::moved`]): the next frame outside
    /// an interaction runs them again, on the worker when there is one.
    pub moved: bool,
    /// The frame shown was rendered on the worker: an interaction's frames stay there
    /// ([`canvas::show`]).
    pub from_worker: bool,
    /// Set in the web build, which has no render worker: while the view changes on a document whose
    /// last render took more than 8 ms, the canvas draws the frame shown reprojected and renders
    /// once the view has been still for [`canvas::STILL_SECS`].
    pub waits_for_still: bool,
    /// The cache key the canvas last asked for and when it first asked for it (seconds, egui's
    /// input time), kept while [`Self::waits_for_still`] is set.
    pub still: Option<(CacheKey, f64)>,
    pub worker: Option<render_worker::Worker>,
    pub worker_started: bool,
    /// The slices as laid out for (document uid, revision): the canvas draws them every frame.
    pub slices: Option<SliceCache>,
    /// The print tiling's pages for (document uid, revision) (View → Show Print Tiling).
    pub print_tiling: Option<PrintTilingCache>,
    /// The gap lines View → Show Live Paint Gaps draws for (document uid, revision).
    pub live_paint_gaps: Option<LivePaintGapsCache>,
    /// The marks Type → Show Hidden Characters drew on each text object in the last frame. A frame
    /// that does not draw them drops them ([`canvas::show`]).
    pub hidden_chars: canvas::HiddenChars,
    /// [`VectorcraftApp::selection_box`] for (document uid, revision, Use Preview Bounds).
    pub selection_box: Option<((u64, u64, bool), Option<vectorcraft_doc::OrientedBox>)>,
    /// [`VectorcraftApp::selection_bounds`] for (document uid, revision).
    pub selection_bounds: Option<((u64, u64), Option<vectorcraft_geom::Rect>)>,
    /// [`VectorcraftApp::content_bounds`] for (document uid, revision).
    pub content_bounds: Option<((u64, u64), Option<vectorcraft_geom::Rect>)>,
    /// The tools' cursors as OS cursor bitmaps.
    pub cursors: cursors::Images,
    /// The view the canvas last drew and when it last changed (egui's input time): while it moves,
    /// the canvas doesn't hit-test the art under the pointer ([`canvas::hover_wait`], #2452).
    pub view_moved: Option<(canvas::ViewKey, f64)>,
    /// The cursor the active tool last gave over the canvas, with the tool and the modifier keys
    /// it was for: kept while the view moves.
    pub hover_cursor: Option<(&'static str, vectorcraft_tools::Mods, vectorcraft_tools::Cursor)>,
}

/// [`CanvasCache::slices`]: the layout of the slices of (document uid, revision).
pub type SliceCache = ((u64, u64), std::sync::Arc<Vec<vectorcraft_doc::SliceArea>>);

/// [`CanvasCache::live_paint_gaps`]: the gap lines of the selected Live Paint groups of (document
/// uid, revision).
pub type LivePaintGapsCache = ((u64, u64), std::sync::Arc<Vec<vectorcraft_tools::Overlay>>);

/// [`CanvasCache::print_tiling`]: the print tiling of (document uid, revision).
pub type PrintTilingCache = ((u64, u64), std::sync::Arc<Vec<vectorcraft_pdf::TilingPage>>);

#[derive(Clone, Debug, PartialEq)]
pub struct CacheKey {
    pub doc: usize,
    pub revision: u64,
    /// An interaction is in progress ([`vectorcraft_render::RenderOptions::interacting`]).
    pub interacting: bool,
    /// A tool's drag on the canvas is in progress ([`vectorcraft_render::RenderOptions::dragging`]).
    pub dragging: bool,
    pub zoom: f64,
    pub cx: f64,
    pub cy: f64,
    pub w: u32,
    pub h: u32,
    pub outline: bool,
    pub trim: bool,
    /// View › Hide Template: template layers left out ([`vectorcraft_render::RenderOptions::hide_templates`]).
    pub templates: bool,
    /// View › Show/Hide Guides: whether guide paths are visible in the artwork raster.
    pub guides: bool,
    pub ppp: f32,
    pub hidden: Vec<u64>,
    pub rot: f64,
    /// General › Anti-aliased Artwork.
    pub anti_alias: bool,
    /// Type › Highlight Alternate Glyphs (#394).
    pub highlight_alternate_glyphs: bool,
    /// Type › Highlight Substituted Fonts.
    pub highlight_substituted_fonts: bool,
    /// [`vectorcraft_render::placed_document::generation`]: placed documents' bitmaps made since.
    pub placed: u64,
    /// View › Pixel Preview: the document pixels rendered (x0, y0, x1, y1), one per point, shown
    /// with hard edges. None: the art is rendered for the screen.
    pub pixel: Option<[i64; 4]>,
    /// Images sampled smoothly ([`vectorcraft_render::RenderOptions::smooth_images`]).
    pub smooth_images: bool,
    /// Isolation mode's group or layer ([`vectorcraft_render::RenderOptions::isolated`]).
    pub isolated: Option<u64>,
    /// Appearance of Black › On Screen › Display All Blacks as Rich Black
    /// ([`vectorcraft_render::RenderOptions::rich_black`]).
    pub rich_black: bool,
    /// Colour and style of guide paths ([`vectorcraft_render::RenderOptions::guide`]).
    pub guide: ([u8; 3], bool),
    /// The Image Trace objects drawn with their source image
    /// ([`vectorcraft_render::RenderOptions::trace_source`]).
    pub trace_source: Option<std::sync::Arc<Vec<vectorcraft_doc::NodeId>>>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Perf {
    pub frame_ms: f64,
    pub render_ms: f64,
    pub fps: f64,
}

pub struct VectorcraftApp {
    pub session: Session,
    pub ui: UiState,
    pub views: Vec<View>,
    pub services: Services,
    pub canvas: CanvasCache,
    pub perf: Perf,
    /// Docking state that lives for the session only.
    pub(crate) dock_rt: panel_docking::Runtime,
    /// macOS: the room the app bar keeps at its left for the window buttons over it, in points
    /// (0 where the OS draws no buttons over the app bar).
    pub titlebar_inset: f32,
    /// Last applied effect (Effect → Apply Last Effect).
    pub last_effect: Option<(String, serde_json::Value)>,
    /// Last confirmed Warp Options (bend, distortion, orientation) for the next new warp dialog.
    pub last_warp: Option<serde_json::Value>,
    /// Commands run through [`Self::run`] so far: the native menu bar reads its rows again when it
    /// moves ([`native_menu::sync`]).
    run_count: u64,
    control_rx: Option<Receiver<ControlRequest>>,
    /// Window captures asked for and not answered yet.
    pending_screenshots: Vec<PendingScreenshot>,
    /// Admitted captures whose settled frame has not come yet (issued one at a time).
    queued_screenshots: Vec<QueuedScreenshot>,
    screenshot_token: u64,
    /// Bytes reserved by admitted captures (checked `width * height * 4`).
    screenshot_reserved: usize,
    /// Synthetic input events (from the control channel) injected one step per frame.
    pub synthetic: Vec<egui::Event>,
    styled: bool,
    fonts_ready: bool,
    /// The egui context the UI's textures were uploaded to (0: none yet; see
    /// [`Self::adopt_context`]).
    context: u64,
    /// The current context, for the focus part of an injected batch's generation (set by
    /// [`Self::adopt_context`]).
    ui_ctx: Option<egui::Context>,
    /// Installed fonts added to the UI's for characters its own fonts lack (CJK names…).
    ui_fonts: ui_fonts::UiFonts,
    pub(crate) updates_rt: updates_ui::Runtime,
    /// The status bar's frame rate ([`frame_rate`]).
    frame_rate: frame_rate::FrameRate,
    /// Canvas rect of the last frame (screen points), for control-channel coordinate mapping.
    pub canvas_rect: Option<egui::Rect>,
    /// Hover position in document coordinates.
    pub hover_doc: Option<vectorcraft_geom::Point>,
    /// System clipboard: SVG to publish next frame, the last SVG we published (so pasting it back
    /// uses the lossless internal clipboard) and what came with a paste (a Paste event's text, a
    /// picture or file the host read: [`Self::paste_from_host`]).
    clipboard_out: Option<String>,
    clipboard_published: Option<String>,
    pub(crate) clipboard_in: Option<vectorcraft_engine::cmd::clipboard::Flavour>,
    /// The Copy or Cut whose formats go to the system clipboard ([`Self::publish_clipboard`]).
    clipboard_publishing: sysclip::Publishing,
    /// A URL to open through egui next frame (when the host has no `open_url` service).
    pending_url: Option<String>,
    /// Windows and Linux: the window has no OS decorations, so the app bar is the title bar (drag,
    /// double-click to maximize, caption buttons) and invisible edge zones resize the window.
    pub custom_titlebar: bool,
    /// Last window title sent to the OS (`ViewportCommand::Title`, see `chrome::sync_window_title`):
    /// sent again only when it changes, so idle frames don't spam the backend.
    pub(crate) last_window_title: String,
    /// The graphics adapter the window renders with ("name (backend)"), as the host reports it:
    /// shown in Help › About and `ui.inspect` for GPU bug reports. `None` when unknown.
    pub graphics_adapter: Option<String>,
    /// File → Place: picked files, the place cursor's thumbnails, the Control bar's image details.
    pub place: place::PlaceState,
    /// The Paste commands can paste from the system clipboard alone: it holds something to paste
    /// (SVG without a `system_clipboard`) while the internal clipboard is empty. It enables the
    /// Paste menu items.
    pub(crate) system_paste: bool,
    /// The modifiers held when an egui `PasteImage` asked the rich system clipboard to be read.
    /// Consumed once by the next `tick`; storing the paste-time chord keeps a later pass's stale
    /// modifiers from choosing Paste in Place instead of Paste (or the reverse).
    pub(crate) clipboard_image_request: Option<egui::Modifiers>,
    /// When `system_paste` was last checked in line (app time, s; at most once per frame).
    system_paste_at: f64,
    /// The thread that checks the system clipboard for `system_paste` instead
    /// ([`Services::clipboard_probe`]).
    clipboard_probe: Option<clipboard_probe::Probe>,
    /// File dialogs shown off the UI thread and what runs again with their answers.
    pub(crate) picks: picks::Picks,
    /// The screen color sampler shown and not answered yet, and where its color goes.
    pub(crate) screen_sampler: Option<screen_color::Waiting>,
    /// The look for fonts installed or removed meanwhile, running
    /// ([`Self::refresh_installed_fonts`]): whether they were, and whether to wake the app for
    /// the answer.
    font_check: Option<(std::sync::mpsc::Receiver<bool>, bool)>,
    /// When installed fonts were last checked in line (app time, s).
    #[cfg(not(target_arch = "wasm32"))]
    font_install_check_at: f64,
    /// Keyboard pastes of something other than text (see [`shortcuts::PasteChord`]).
    pub(crate) paste_chord: shortcuts::PasteChord,
    /// Runs the frame's key presses against the keymap ([`shortcuts::handle`]).
    pub(crate) dispatcher: craft_keymap::egui::Dispatcher,
    /// Saves and exports running in the background (Preferences → File Handling).
    pub background: background::Background,
    /// Documents being read off the UI thread ([`opening`]).
    pub opening: opening::Opening,
    /// Heavy plug-in filters running off the UI thread ([`plugin_runs`]).
    pub plugin_runs: plugin_runs::PluginRuns,
    /// Files being read off the UI thread for Place ([`placing`]).
    pub placing: placing::Placing,
    /// Data Recovery's timer and startup question ([`recovery`]).
    pub recovery: recovery::Timer,
    /// Modifiers the keyboard holds (from the host's input), given back after synthetic input.
    host_modifiers: egui::Modifiers,
    /// Key, text and clipboard events a floating panel's own OS window forwarded, for the main
    /// window's next frame ([`Self::raw_input_hook`]), and the modifiers held in that window.
    pub(crate) forwarded: Vec<egui::Event>,
    pub(crate) forwarded_modifiers: Option<egui::Modifiers>,
    /// The last frame held the forwarded modifiers: this one goes back to the keyboard's.
    forwarded_modifiers_held: bool,
    /// This host iteration's input carries forwarded keys: they are the main window's although
    /// its OS window doesn't have the keyboard (see `shortcuts::handle`).
    pub(crate) forwarded_frame: bool,
    /// Synthetic input set the modifiers egui holds (see [`Self::raw_input_hook`]).
    synthetic_modifiers: bool,
    /// The marked text the system IME last sent (`None` once it commits or clears). When the Type
    /// tool stops composing on its own (a click, a tool switch), the IME is told to drop it.
    pub(crate) ime_marked: Option<String>,
    /// Owner/generation/change-cause bookkeeping for the outgoing IME surrounding-text producer
    /// (one tracker serves every viewport; see the crate's IME producer module).
    pub(crate) ime_context: egui::ime_context::ImeContextTracker,
    /// Incoming composition ownership is separate from outgoing receipt/generation bookkeeping.
    /// Interrupted input stays stale until the exact request-bound native fence arrives.
    pub(crate) ime_compositions: egui::ViewportIdMap<ime::IncomingComposition>,
    pub(crate) ime_resets: egui::ViewportIdMap<ime::ResetGuard>,
    /// The Type tool's IME caret geometry for this pass, recorded by the canvas. The descriptor
    /// itself is written at the end of the pass ([`ime::end_of_pass`]), after dialogs, floating
    /// panels and the palette, because any of them can move the owner, edit the text or cover the
    /// canvas after the canvas drew.
    pub(crate) ime_caret_rect: Option<(egui::Rect, egui::Rect)>,
    /// The IME must drop its marked text (see [`Self::take_ime_discard`]).
    pub(crate) ime_discard: bool,
    /// Preferences were confirmed (see [`Self::take_prefs_changed`]).
    pub(crate) prefs_changed: bool,
    /// Whether the last frame's input reported a touch screen (Preferences › Devices › Touch Workspace).
    pub touch_screen: bool,
    /// A numeric field is being scrubbed: the document's edits meanwhile are one undo step
    /// ([`scrub::begin_frame`]).
    scrub_group: bool,
    /// The undo step of the scrub in progress ([`vectorcraft_engine::Session::begin_undo_scope`]).
    scrub_scope: Option<vectorcraft_engine::UndoScope>,
    /// The folders control requests, and the input they inject, may read and write
    /// (`--automation-read-root`, `--automation-write-root`); `None`: anywhere. See
    /// [`Self::with_automation_roots`].
    automation_roots: Option<Arc<AutomationRoots>>,
    /// This frame carries input the control channel injected ([`Self::raw_input_hook`]): it runs
    /// confined to [`Self::automation_roots`].
    synthetic_frame: bool,
    /// The pen end the canvas saw last and the tool its eraser end stands in for
    /// ([`stylus::sync_eraser_tool`]).
    pub(crate) pen_end: stylus::PenEnd,
    /// Host iterations (full or logic-only), advanced once in [`Self::begin_host_iteration`].
    pub(crate) host_iteration: u64,
    /// A test's clock for [`Self::tick`], replacing the monotonic [`now_secs`] so idle scheduling
    /// can be driven one simulated frame at a time (production always leaves this `None`).
    #[cfg(test)]
    pub(crate) now_override: Option<f64>,
    /// The host iteration [`Self::tick`] ran in (`None`: never): once per iteration, never per pass.
    tick_iteration: Option<u64>,
    /// The context, viewport and frame whose UI-phase input was already taken in this host
    /// iteration: a discarded rerun and a second viewport pass must not replay a frame's events.
    input_frames: Vec<InputFrame>,
    /// The context, viewport and frame `shortcuts::handle` already took in this host iteration.
    /// Interleaving root and auxiliary viewport passes cannot make an older frame look new again.
    pub(crate) shortcuts_frames: Vec<InputFrame>,
    /// A synthetic batch is in the host's raw input and no settled UI pass has taken it yet.
    synthetic_injected: bool,
    /// The exact events of [`Self::synthetic_injected`], to remove them if the state they belong
    /// to changed before a UI pass could take them.
    injected_events: Vec<egui::Event>,
    /// The document, dialog and focus an injected batch belongs to.
    injected_generation: InputGeneration,
    /// The end-of-pass observer was installed on the context.
    injected_settle_hook: bool,
    /// A settled UI pass consumed the injected batch (read by the next host iteration).
    injected_settled: Arc<AtomicBool>,
    /// A change of the window's content geometry (resize, DPI, zoom or surface), even back again.
    geometry_epoch: u64,
    /// The geometry stamp [`Self::geometry_epoch`] was last advanced for.
    geometry_stamp: Option<GeometryStamp>,
}

/// One UI pass key: the context identity, the viewport and that viewport's cumulative frame.
pub(crate) type InputFrame = (u64, egui::ViewportId, u64);

/// Record `frame` unless this host iteration already took it. A discarded rerun and any
/// interleaved pass order stay exact-once because the key, not the last-seen slot, is compared.
/// The list is cleared at every host-iteration boundary, so it stays as small as that iteration's
/// pass count.
pub(crate) fn take_frame_once(frames: &mut Vec<InputFrame>, frame: InputFrame) -> bool {
    if frames.contains(&frame) {
        return false;
    }
    frames.push(frame);
    true
}

/// The document, dialog and focus an injected batch belongs to: if any changed before a UI pass
/// took the batch, the clicks or text no longer belong to the new state and are dropped.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct InputGeneration {
    document: u64,
    dialog: Option<egui::Id>,
    focus: Option<egui::Id>,
}

/// The window geometry a capture belongs to (logical and screen bounds, DPI and zoom).
#[derive(Clone, Copy, Debug, PartialEq)]
struct GeometryStamp {
    viewport: egui::ViewportId,
    content: egui::Rect,
    inner: Option<egui::Rect>,
    pixels_per_point: f32,
    zoom: f32,
}

/// What the issued capture must still match when it completes.
#[derive(Clone, Copy, Debug)]
struct CaptureEpoch {
    geometry: u64,
    viewport: egui::ViewportId,
    /// The active document (uid, revision) at issuance: a screenshot never answers with pixels of
    /// a document, undo state or selection that replaced it while the capture was in flight.
    document: Option<(u64, u64)>,
    /// The image size the issuing geometry predicts (`width * height`, checked).
    width: usize,
    height: usize,
}

/// An admitted capture waiting for its settled frame (`frames`) and animation delay (`due_ms`).
struct QueuedScreenshot {
    id: u64,
    due_ms: f64,
    frames: u32,
}

/// A window capture asked for through the control channel (`ui.screenshot`), answered once a
/// frame is presented; a window that isn't presented never delivers one, hence the deadline.
struct PendingScreenshot {
    id: u64,
    path: Option<String>,
    /// Send the PNG back as `pngBase64`.
    data: bool,
    reply: Sender<ControlResponse>,
    /// When to give up (ms, as [`now_ms`], monotonic).
    deadline_ms: f64,
    /// The checked bytes this request reserved before it was admitted.
    reserved_bytes: usize,
    /// The active document at admission: switching documents before issuance fails the request
    /// rather than capturing a different document.
    document: Option<(u64, u64)>,
    /// The live one-shot completion slot while the command is outstanding.
    slot: Option<craft_ui::capture::CaptureSlot>,
    /// The geometry and expected image size at issuance.
    issued: Option<CaptureEpoch>,
    /// Fresh slots made after a geometry change (a resize while the capture was in flight).
    attempts: u8,
}

/// At most this many captures admitted (pending or queued) at once.
const MAX_PENDING_SCREENSHOTS: usize = 4;
/// The largest single capture admitted (128 MiB: a 4K viewport at 2x).
const MAX_SCREENSHOT_BYTES: usize = 128 * 1024 * 1024;
/// The aggregate budget of admitted captures (256 MiB).
const MAX_SCREENSHOT_TOTAL_BYTES: usize = 256 * 1024 * 1024;
/// Settle frames a capture waits for after a control request (and the animation time).
const SCREENSHOT_SETTLE_FRAMES: u32 = 3;
/// How long after settling an issued capture may take (ms).
const SCREENSHOT_TIMEOUT_MS: f64 = 8000.0;
/// The timeout's answer: no file, no second reply.
const SCREENSHOT_TIMEOUT_ERROR: &str =
    "no frame was presented (screen locked, window minimized or fully covered); ui.render still renders the artboard";

/// Seconds between two looks at the system clipboard for [`VectorcraftApp::system_paste`].
const SYSTEM_CLIPBOARD_POLL: f64 = 0.25;
/// Seconds between looks for fonts a font manager activated while the app stays in front (#2576).
#[cfg(not(target_arch = "wasm32"))]
const FONT_INSTALL_POLL: f64 = 2.0;

impl VectorcraftApp {
    /// The app around `session`, reaching the platform through `services` (file pickers, the
    /// clipboard, files and the like). It starts cataloguing the installed fonts at once, gives
    /// Data Recovery the host's store and makes a view for each open document.
    pub fn new(mut session: Session, services: Services) -> Self {
        // The font menus and the first file opened need the installed fonts: catalog them now.
        vectorcraft_text::FontDb::global().scan_in_background();
        if let Some(store) = &services.recovery_store {
            session.recovery.set_store(store.clone());
        }
        // The place cursor's clicks and drags place their files through the app
        // ([`place::place_from_cursor`]).
        session.place_cursor_requests = true;
        let views = session.documents().iter().map(View::of).collect();
        Self {
            session,
            ui: UiState::default(),
            views,
            services,
            canvas: CanvasCache {
                renderer: vectorcraft_render::Renderer::new(),
                texture: None,
                key: None,
                shown: None,
                last_ms: 0.0,
                moved: false,
                from_worker: false,
                waits_for_still: cfg!(target_arch = "wasm32"),
                still: None,
                worker: None,
                worker_started: false,
                slices: None,
                print_tiling: None,
                live_paint_gaps: None,
                hidden_chars: Default::default(),
                selection_box: None,
                selection_bounds: None,
                content_bounds: None,
                cursors: Default::default(),
                view_moved: None,
                hover_cursor: None,
            },
            perf: Perf::default(),
            dock_rt: Default::default(),
            titlebar_inset: 0.0,
            last_effect: None,
            last_warp: None,
            run_count: 0,
            clipboard_out: None,
            clipboard_published: None,
            clipboard_in: None,
            clipboard_publishing: Default::default(),
            pending_url: None,
            control_rx: None,
            pending_screenshots: vec![],
            queued_screenshots: vec![],
            screenshot_token: 0,
            screenshot_reserved: 0,
            synthetic: vec![],
            styled: false,
            fonts_ready: false,
            context: 0,
            ui_ctx: None,
            ui_fonts: Default::default(),
            updates_rt: Default::default(),
            frame_rate: Default::default(),
            canvas_rect: None,
            hover_doc: None,
            custom_titlebar: false,
            last_window_title: String::new(),
            graphics_adapter: None,
            place: Default::default(),
            system_paste: false,
            clipboard_image_request: None,
            system_paste_at: f64::NEG_INFINITY,
            clipboard_probe: None,
            picks: picks::Picks::default(),
            screen_sampler: None,
            font_check: None,
            #[cfg(not(target_arch = "wasm32"))]
            font_install_check_at: f64::NEG_INFINITY,
            paste_chord: Default::default(),
            // Held keys repeat their command (Cmd+= keeps zooming in).
            dispatcher: craft_keymap::egui::Dispatcher::new().with_repeat(true),
            background: Default::default(),
            opening: Default::default(),
            plugin_runs: Default::default(),
            placing: Default::default(),
            recovery: Default::default(),
            host_modifiers: Default::default(),
            forwarded: Vec::new(),
            forwarded_modifiers: None,
            forwarded_modifiers_held: false,
            forwarded_frame: false,
            synthetic_modifiers: false,
            ime_marked: None,
            ime_context: Default::default(),
            ime_compositions: Default::default(),
            ime_resets: Default::default(),
            ime_caret_rect: None,
            ime_discard: false,
            prefs_changed: false,
            touch_screen: false,
            scrub_group: false,
            scrub_scope: None,
            automation_roots: None,
            synthetic_frame: false,
            pen_end: Default::default(),
            host_iteration: 0,
            #[cfg(test)]
            now_override: None,
            tick_iteration: None,
            input_frames: vec![],
            shortcuts_frames: vec![],
            synthetic_injected: false,
            injected_events: vec![],
            injected_generation: InputGeneration::default(),
            injected_settle_hook: false,
            injected_settled: Default::default(),
            geometry_epoch: 0,
            geometry_stamp: None,
        }
    }

    pub fn with_control(mut self, rx: Receiver<ControlRequest>) -> Self {
        self.control_rx = Some(rx);
        self
    }

    /// Confine what the control channel does to `roots` (#832): its requests, the input it
    /// injects (in the frames that carry it) and the saves and exports they start read and write
    /// only inside them ([`file_access`]). The host's file services check every path they're given
    /// against the roots in force, and so does the engine; outside those scopes the person at the
    /// keyboard works as usual.
    pub fn with_automation_roots(mut self, roots: Option<Arc<AutomationRoots>>) -> Self {
        let Some(roots) = roots else { return self };
        self.automation_roots = Some(roots);
        let s = &mut self.services;
        if let Some(read) = s.read.take() {
            s.read = Some(Box::new(move |p: &str| file_access::check_read(p).and_then(|()| read(p))));
        }
        if let Some(mut write) = s.write.take() {
            s.write = Some(Box::new(move |p: &str, b: &[u8]| file_access::check_write(p).and_then(|()| write(p, b))));
        }
        if let Some(write) = s.write_shared.take() {
            s.write_shared = Some(Arc::new(move |p: &str, b: &[u8]| file_access::check_write(p).and_then(|()| write(p, b))));
        }
        if let Some(stream) = s.stream_shared.take() {
            s.stream_shared = Some(Arc::new(move |p: &str, fill: &mut dyn FnMut(&mut dyn std::io::Write) -> std::io::Result<()>| {
                file_access::check_write(p).and_then(|()| stream(p, fill))
            }));
        }
        // Edit Original and Show in Folder hand a file to another app: only one automation may read.
        for service in [&mut s.open_file, &mut s.reveal] {
            if let Some(mut open) = service.take() {
                *service = Some(Box::new(move |p: &str| file_access::check_read(p).and_then(|()| open(p))));
            }
        }
        self
    }

    /// Keep `views` aligned with the session's documents (a new one starts at its saved view).
    pub fn sync_views(&mut self) {
        let docs = self.session.documents();
        self.views.truncate(docs.len());
        self.views.extend(docs.iter().skip(self.views.len()).map(View::of));
        for (view, doc) in self.views.iter_mut().zip(docs) {
            view.artboard = view.artboard.min(doc.doc.artboards.len().saturating_sub(1));
        }
        // Undo can remove the Artboard tool's active or additionally selected boards. Repair only
        // invalid indices: valid multiselection, drag state and tool options must survive history.
        if self.session.tool_id() != "artboard" || self.session.tool_busy() {
            return;
        }
        let Some(n) = self.session.active().map(|d| d.doc.artboards.len()).filter(|n| *n > 0) else { return };
        let Some(view) = self.session.active_index().and_then(|i| self.views.get_mut(i)) else { return };
        let options = self.session.tool_options();
        let Some(selected) = options["selected"].as_array() else { return };
        let mut valid: Vec<usize> = selected.iter().filter_map(Value::as_u64).filter_map(|i| usize::try_from(i).ok()).filter(|i| *i < n).collect();
        if valid.len() != selected.len() {
            if valid.is_empty() {
                valid.push(view.artboard);
            }
            if let Some(active) = valid.first() {
                view.artboard = *active;
            }
            self.session.set_tool_option("selected", &serde_json::json!(valid));
        }
    }

    /// The active document's uid, how many times the tool has started afresh
    /// ([`vectorcraft_engine::Session::tool_resets`]) and the artboards the Artboard tool has
    /// selected (null with another tool), which [`Self::follow_artboard_tool`] compares before and
    /// after a command.
    fn artboard_tool_state(&self) -> (Option<u64>, u64, Value) {
        let selected = if self.session.tool_id() == "artboard" { self.session.tool_options()["selected"].clone() } else { Value::Null };
        (self.session.active().map(|d| d.uid), self.session.tool_resets(), selected)
    }

    /// After a command run with the Artboard tool, given [`Self::artboard_tool_state`] from before
    /// it: when the command chose another document, or started the tool afresh in the same one
    /// (File › Revert, a `command.batch` or `action.batch` that reverts it or switches to another
    /// document and back), the tool takes the document's active artboard, as choosing its tab
    /// does. When the command changed the artboards the tool has selected (Paste and Cut with the
    /// tool, `tool.setOption`), also after starting it afresh (a batch step after the revert or the
    /// switch), the tool's active artboard becomes the active one (the view's), as a click with the
    /// tool makes it.
    fn follow_artboard_tool(&mut self, before: (Option<u64>, u64, Value)) {
        self.sync_views();
        if self.session.tool_id() != "artboard" {
            return;
        }
        let Some(n) = self.session.active().map(|d| d.doc.artboards.len()).filter(|n| *n > 0) else { return };
        let (doc, resets, selected) = self.artboard_tool_state();
        // The tool started afresh during the command, and no later step (a Paste or Cut) set its options.
        let reset = resets != before.1;
        let fresh = reset && !self.session.tool_set_since_reset();
        if fresh || (doc != before.0 && !reset) {
            panels::artboards::select(self, panels::artboards::selected(self, n));
        } else if (reset || selected != before.2)
            && let Some(i) = self.session.tool_options()["active"].as_u64().and_then(|i| usize::try_from(i).ok())
            && i < n
            && let Some(v) = self.view_mut()
        {
            v.artboard = i;
        }
    }

    pub fn view(&self) -> Option<&View> {
        self.session.active_index().and_then(|i| self.views.get(i))
    }

    /// The zero point shown by rulers and position fields: the document's with global rulers, the
    /// active artboard's with artboard rulers. Command coordinates stay absolute.
    pub(crate) fn ruler_origin(&self) -> vectorcraft_geom::Point {
        let artboard = self.view().map_or(0, |v| v.artboard);
        self.session.active().map_or(vectorcraft_geom::Point::ZERO, |d| d.doc.ruler_origin_for(artboard))
    }
    pub fn view_mut(&mut self) -> Option<&mut View> {
        self.sync_views();
        let i = self.session.active_index()?;
        self.views.get_mut(i)
    }

    pub fn view_info(&self) -> ViewInfo {
        ViewInfo {
            zoom: self.view().map(|v| v.zoom).unwrap_or(1.0),
            outline: self.ui.view.outline,
            smart_guides: self.ui.view.smart_guides,
            guides: self.ui.view.guides,
            snap_to_grid: self.ui.view.snap_to_grid,
            snap_to_pixel: self.ui.view.snap_to_pixel,
            // View › Hide Edges: the canvas draws neither the bounding box (with the type widget)
            // nor the Live Corners widgets, and the tools do not answer the pointer on them.
            show_bbox: self.ui.view.bounding_box && self.ui.view.edges,
            snap_to_point: self.ui.view.snap_to_point,
            snap_to_glyph: self.ui.view.snap_to_glyph,
            corner_widgets: self.ui.view.corner_widgets && self.ui.view.edges,
            screen: self.screen_frame(),
            artboard: self.view().map_or(0, |v| v.artboard),
        }
    }

    /// The host iteration count: one increment per `App::logic`/`App::ui` iteration (or a
    /// logic-only one), for panel state that asks "was this drawn in the last iteration?".
    pub(crate) fn frame_iteration(&self) -> u64 {
        self.host_iteration
    }

    /// The canvas on screen in document coordinates (none before it is laid out).
    pub fn screen_frame(&self) -> Option<vectorcraft_tools::ScreenFrame> {
        let (rect, view) = (self.canvas_rect?, self.view()?);
        let xf = canvas::Xf::new(rect, view);
        let px = |x: f32, y: f32| xf.delta_to_doc(egui::vec2(x, y));
        Some(vectorcraft_tools::ScreenFrame {
            origin: xf.to_doc(rect.left_top()),
            right: px(1.0, 0.0),
            down: px(0.0, 1.0),
            size: (f64::from(rect.width()), f64::from(rect.height())),
        })
    }

    /// Run a UI or engine command by id. The single entry point for every frontend path.
    pub fn run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        // A file dialog it shows off the UI thread runs it again with the path picked.
        if self.picks.is_entry_free() {
            let entry = picks::Entry::Command(id.to_string(), params.clone());
            return picks::as_entry(self, move || entry, |app| app.run_now(id, params));
        }
        self.run_now(id, params)
    }

    /// [`Self::run`] it, inside what asks for file dialogs.
    fn run_now(&mut self, id: &str, params: Value) -> Result<Value, String> {
        crate::state::sync_zoom_stops_display_print_size(self.session.prefs.display_print_size);
        self.run_count = self.run_count.wrapping_add(1);
        // In debug builds the app's own calls are held to the params descriptions too (#1442): a
        // panel passing a key its command doesn't read shows up in the tests.
        if cfg!(debug_assertions) {
            menus::check_params(&self.session, id, &params)?;
        }
        let artboards = self.artboard_tool_state();
        if let Some(r) = menus::run_ui_command(self, id, &params) {
            self.follow_artboard_tool(artboards);
            return r;
        }
        let mut params = params;
        // Paste in place, in front and in back (#693), Select › All on Active Artboard (#1006), and Align,
        // Distribute Objects and Distribute Spacing with Align to Artboard act on the active artboard, the
        // view's, which the app passes as `artboard`. A batch passes it on to those steps.
        if (vectorcraft_engine::ON_ACTIVE_ARTBOARD.contains(&id) || id == "command.batch")
            && params.get("artboard").is_none()
            && self.view().is_some()
            && let Some(n) = self.session.active().map(|d| d.doc.artboards.len())
            && let Some(p) = params.as_object_mut()
        {
            p.insert("artboard".into(), serde_json::json!(panels::artboards::selected(self, n)));
        }
        // Object › Artboards › Fit to Artwork Bounds resizes the selected artboards (the Artboard
        // tool's, else the active one), which the app passes as its `indices`, and Fit to Selected
        // Art resizes the active artboard, which the app passes as its `index`.
        if vectorcraft_engine::FIT_ACTIVE_ARTBOARD.contains(&id)
            && params.get("index").is_none()
            && params.get("indices").is_none()
            && self.view().is_some()
            && let Some(n) = self.session.active().map(|d| d.doc.artboards.len())
            && let Some(p) = params.as_object_mut()
        {
            if id == "artboard.fitToArt" {
                p.insert("indices".into(), serde_json::json!(panels::artboards::picked(self, n)));
            } else {
                p.insert("index".into(), serde_json::json!(panels::artboards::selected(self, n)));
            }
        }
        if id.starts_with("edit.paste") {
            if let Err(e) = self.adopt_system_clipboard() {
                self.ui.status = e.clone();
                return Err(e);
            }
            // Paste (also without formatting) goes to the centre of the view.
            if matches!(id, "edit.paste" | "edit.pasteWithoutFormatting")
                && ["center", "dx", "dy"].iter().all(|k| params.get(k).is_none())
                && let Some(c) = self.view().filter(|v| v.fitted).map(|v| v.center)
                && let Some(p) = params.as_object_mut()
            {
                p.insert("center".into(), serde_json::json!([c.x, c.y]));
            }
            if let Some(r) = dialogs::swatch_conflict::ask(self, id, &params) {
                return r;
            }
        }
        // Native files carry the view they reopen at.
        if fileio::SaveMode::of(id).is_some() {
            io::remember_view(self);
        }
        // A command that changes the edited text is an application edit (undo, a panel or menu
        // edit, a control request). Capture the bounded committed context around it so `Other`
        // dominates the frame even if an IME receipt follows. Selection/view commands that leave
        // the committed text and caret unchanged record nothing.
        let ime_view = self.view_info();
        let ime_target = crate::ime::target(&mut self.session, ime_view);
        let ime_before = crate::ime::stamp(&mut self.session, ime_view);
        let r = self.session.execute(id, &params).map_err(|e| e.to_string());
        if r.is_ok()
            && let Some(ctx) = self.ui_ctx.clone()
        {
            crate::ime::note_application_edit(self, &ctx, ime_view, ime_target, ime_before);
        }
        if r.is_ok() && matches!(id, "edit.copy" | "edit.cut") {
            self.publish_clipboard();
        }
        self.sync_views();
        // File › Revert replaces the document in its tab, keeps the tab's view and starts the tool afresh.
        self.follow_artboard_tool(artboards);
        match &r {
            Err(e) => self.ui.status = e.clone(),
            Ok(v) => {
                // Image Trace's notes (a large image traces more slowly, or was scaled down).
                if id.starts_with("imageTrace.make")
                    && let Some(w) = v["warnings"].as_array().and_then(|w| w.first()).and_then(Value::as_str)
                {
                    self.ui.status = w.to_string();
                }
                if id == "file.new" {
                    self.ui.status.clear();
                    // New Document's Pixel preview mode (Overprint Preview is the engine's).
                    if v["previewMode"] == "pixel" {
                        self.ui.view.pixel_preview = true;
                    }
                }
                // Choosing a data set, or stepping to one, clears the status bar. The reply's `warning`
                // below then shows when no object is bound to a variable the data set has a value for.
                if matches!(id, "dataset.select" | "dataset.next" | "dataset.prev") {
                    self.ui.status.clear();
                }
                if matches!(id, "text.setStyle" | "text.setRangeStyle")
                    && let Some(font) = params.get("font").and_then(Value::as_str)
                {
                    let r = &mut self.ui.recent_fonts;
                    r.retain(|f| !f.eq_ignore_ascii_case(font));
                    r.insert(0, font.to_string());
                    r.truncate(MAX_RECENT_FONTS);
                }
                // A command's warning (Rasterize's when an image couldn't be decoded) shows in the
                // status bar, as a tool preview's does.
                if let Some(w) = v.get("warning").and_then(Value::as_str) {
                    self.ui.status = w.to_string();
                }
            }
        }
        r
    }

    /// Open a link in the browser (Help → Discord, website, GitHub…).
    pub fn open_url(&mut self, url: &str) {
        match self.services.open_url.as_mut() {
            Some(open) => open(url),
            None => self.pending_url = Some(url.to_string()),
        }
        self.ui.status = format!("Opened {url}");
    }

    /// Open `url`, which came from a document (an object's URL attribute), in the browser:
    /// [`browsable_url`]s only.
    pub(crate) fn open_document_url(&mut self, url: &str) -> Result<(), String> {
        browsable_url(url)?;
        self.open_url(url);
        Ok(())
    }

    /// Refuse UI command `id`, which hands something to another app, when automation sent it (a
    /// control or MCP request, or a frame of synthetic input): the automation roots don't confine
    /// what that app does.
    pub(crate) fn refuse_automation(&self, id: &str) -> Result<(), String> {
        if self.ui.answering_control || self.synthetic_frame {
            return Err(format!("{id} opens another app: only the person at the keyboard can run it"));
        }
        Ok(())
    }

    /// Run a Help link command and open the URL it returns.
    pub fn open_link(&mut self, id: &str) {
        if let Ok(v) = self.run(id, serde_json::json!({}))
            && let Some(u) = v["url"].as_str()
        {
            let u = u.to_string();
            self.open_url(&u);
        }
    }

    /// Select a tool (also used by the toolbar and shortcuts).
    pub fn select_tool(&mut self, id: &str) {
        // The tip of a pen whose eraser end selected the Eraser doesn't undo this choice.
        self.pen_end.forget_tool();
        self.activate_tool(id, true);
    }

    /// Go back to tool `id` after a tool that stood in for it (the pen's eraser end, [`stylus`]):
    /// a perspective grid hidden meanwhile stays hidden.
    pub(crate) fn return_to_tool(&mut self, id: &str) {
        self.activate_tool(id, false);
    }

    /// Make tool `id` active: chosen afresh (`fresh`, [`vectorcraft_engine::Session::select_tool`]),
    /// or back from a tool that stood in for it ([`vectorcraft_engine::Session::switch_tool`]).
    fn activate_tool(&mut self, id: &str, fresh: bool) {
        let v = self.view_info();
        let before = self.session.tool_id();
        let switched = if fresh { self.session.select_tool(id, v) } else { self.session.switch_tool(id, v) };
        if let Err(e) = switched {
            self.ui.status = e.to_string();
        }
        // A newly chosen Artboard tool starts on the active artboard (the view's).
        if before != "artboard"
            && self.session.tool_id() == "artboard"
            && let Some(n) = self.session.active().map(|d| d.doc.artboards.len())
        {
            panels::artboards::select(self, panels::artboards::selected(self, n));
        }
        if let Some(g) = vectorcraft_tools::catalog::group_of(id)
            && let Some(slot) = self.ui.group_tool.get_mut(g)
        {
            *slot = id.to_string();
        }
        toolbar::remember(self, id);
        self.ui.flyout = None;
    }

    fn drain_control(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.control_rx.take() else { return };
        let mut handled = 0;
        while handled < control::MAX_CONTROL_REQUESTS_PER_FRAME {
            let Ok(req) = rx.try_recv() else { break };
            // Its caller stopped waiting (the app was busy or its window not drawn for too long)
            // and was told it won't run: an edit applied now would come from nowhere (#1725).
            if !req.claim() {
                continue;
            }
            handled += 1;
            let reply = req.reply.clone();
            // Guarded per request: a panic must not drop the channel (taken out of `self` above).
            let roots = self.automation_roots.clone();
            self.ui.answering_control = true;
            let answer = file_access::confine(roots.as_ref(), || vectorcraft_engine::guard::catch_panic(|| control::handle(self, ctx, &req)));
            self.ui.answering_control = false;
            let outcome = match answer {
                Ok(outcome) => outcome,
                Err(msg) => {
                    self.picks.recover();
                    control::err(format!("internal error: {msg} (please report this bug)"))
                }
            };
            match outcome {
                control::Outcome::Done(v) => {
                    let _ = reply.send(v);
                }
                control::Outcome::Screenshot { path, data } => {
                    // Admit with the shared ceilings (4 requests, 128 MiB each, 256 MiB together)
                    // before issue, reserving the checked bytes this viewport would need.
                    match self.screenshot_pixels(ctx) {
                        Ok((width, height, bytes)) if bytes > MAX_SCREENSHOT_BYTES => {
                            let _ = reply.send(json!({"ok": false, "error": format!(
                                "screenshot would use {bytes} bytes ({width}×{height}), over the {MAX_SCREENSHOT_BYTES}-byte limit"
                            )}));
                        }
                        Ok((width, height, bytes)) if data && path.is_none() && bytes > control::MAX_SCREENSHOT_PNG_REPLY_BYTES => {
                            let _ = reply.send(json!({"ok": false, "error": format!(
                                "a PNG reply of this {width}×{height} screenshot ({bytes} bytes of pixels) would exceed the {}-byte reply-encoding limit; write it to a path instead",
                                control::MAX_SCREENSHOT_PNG_REPLY_BYTES
                            )}));
                        }
                        Ok((_, _, _)) if self.pending_screenshots.len() >= MAX_PENDING_SCREENSHOTS => {
                            let _ = reply.send(json!({"ok": false, "error": format!(
                                "already {MAX_PENDING_SCREENSHOTS} screenshots are waiting; wait for one to finish"
                            )}));
                        }
                        Ok((width, height, bytes)) if self.screenshot_reserved.saturating_add(bytes) > MAX_SCREENSHOT_TOTAL_BYTES => {
                            let _ = reply.send(json!({
                                "ok": false,
                                "error": format!(
                                    "screenshots already reserved {} bytes; {bytes} more ({width}×{height}) would pass the {MAX_SCREENSHOT_TOTAL_BYTES}-byte limit",
                                    self.screenshot_reserved
                                )
                            }));
                        }
                        Ok((_, _, bytes)) => {
                            self.screenshot_token += 1;
                            let id = self.screenshot_token;
                            let settle = ctx.global_style().animation_time as f64 * 2000.0 + 80.0;
                            let due_ms = now_ms() + settle;
                            self.screenshot_reserved = self.screenshot_reserved.saturating_add(bytes);
                            self.queued_screenshots.push(QueuedScreenshot { id, due_ms, frames: 0 });
                            let document = self.document_identity();
                            self.pending_screenshots.push(PendingScreenshot {
                                id,
                                path,
                                data,
                                reply,
                                deadline_ms: due_ms + SCREENSHOT_TIMEOUT_MS,
                                reserved_bytes: bytes,
                                document,
                                slot: None,
                                issued: None,
                                attempts: 0,
                            });
                        }
                        Err(e) => {
                            let _ = reply.send(json!({"ok": false, "error": e}));
                        }
                    }
                }
            }
        }
        if handled == control::MAX_CONTROL_REQUESTS_PER_FRAME {
            // Leave later requests on the channel and schedule another frame to serve them.
            ctx.request_repaint();
        }
        self.control_rx = Some(rx);
    }

    /// The active document's identity: its process-unique uid and revision. A capture records it
    /// at admission and issuance and refuses to answer with a different document or undo state.
    fn document_identity(&self) -> Option<(u64, u64)> {
        self.session.active().map(|document| (document.uid, document.revision))
    }

    /// The image size (pixels) and checked byte count a capture of the current viewport would
    /// have: the window's content rect times the current DPI/zoom. The viewport's own `inner_rect`
    /// is the fresh source; `content_rect` is only a fallback for hosts that do not report one
    /// (Wayland, headless), and a logic-only pass leaves it from the last shown frame. Issuance in
    /// a real UI pass recomputes this and re-checks the budget, so a stale admission size cannot
    /// let an oversized image through.
    fn screenshot_pixels(&self, ctx: &egui::Context) -> Result<(usize, usize, usize), String> {
        let rect = ctx.input(|i| i.viewport().inner_rect).unwrap_or_else(|| ctx.content_rect());
        let scale = f64::from(ctx.pixels_per_point());
        let pixels = |points: f32, what: &str| -> Result<usize, String> {
            let value = f64::from(points) * scale;
            if !value.is_finite() || !(1.0..=100_000.0).contains(&value) {
                return Err(format!("the window's {what} ({points} points × {scale} DPI) cannot size a screenshot"));
            }
            Ok(value.round() as usize)
        };
        let width = pixels(rect.width(), "width")?;
        let height = pixels(rect.height(), "height")?;
        let bytes = width
            .checked_mul(height)
            .and_then(|n| n.checked_mul(4))
            .ok_or_else(|| format!("a {width}×{height} screenshot's byte count overflows"))?;
        Ok((width, height, bytes))
    }

    /// Issue the first due capture of a settled UI pass, at most one in flight: the one-shot
    /// callback makes correlation trivial, and serializing bounds readback memory.
    fn issue_screenshots(&mut self, ctx: &egui::Context) {
        if self.pending_screenshots.iter().any(|p| p.slot.is_some()) {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
            return;
        }
        for q in &mut self.queued_screenshots {
            q.frames = q.frames.saturating_add(1);
        }
        let now = now_ms();
        let Some(pos) = self.queued_screenshots.iter().position(|q| now >= q.due_ms && q.frames >= SCREENSHOT_SETTLE_FRAMES) else {
            if !self.queued_screenshots.is_empty() {
                ctx.request_repaint_after(std::time::Duration::from_millis(16));
            }
            return;
        };
        let queued = self.queued_screenshots.remove(pos);
        let Some(index) = self.pending_screenshots.iter().position(|p| p.id == queued.id) else { return };
        let document = self.document_identity();
        if self.pending_screenshots[index].document != document {
            self.fail_screenshot(index, "the document changed while the screenshot was pending");
            return;
        }
        let (width, height, bytes) = match self.screenshot_pixels(ctx) {
            Ok(v) => v,
            Err(e) => {
                self.fail_screenshot(index, &e);
                return;
            }
        };
        let (data, path, previous) =
            (self.pending_screenshots[index].data, self.pending_screenshots[index].path.is_some(), self.pending_screenshots[index].reserved_bytes);
        if data && !path && bytes > control::MAX_SCREENSHOT_PNG_REPLY_BYTES {
            let e = format!(
                "a PNG reply of this {width}×{height} screenshot ({bytes} bytes of pixels) no longer fits the {}-byte reply-encoding limit; write it to a path instead",
                control::MAX_SCREENSHOT_PNG_REPLY_BYTES
            );
            self.fail_screenshot(index, &e);
            return;
        }
        if bytes > MAX_SCREENSHOT_BYTES || self.screenshot_reserved.saturating_sub(previous).saturating_add(bytes) > MAX_SCREENSHOT_TOTAL_BYTES {
            let e = format!("the window grew since the screenshot was admitted: {width}×{height} no longer fits the capture budget");
            self.fail_screenshot(index, &e);
            return;
        }
        self.screenshot_reserved = self.screenshot_reserved.saturating_sub(previous).saturating_add(bytes);
        let viewport = ctx.viewport_id();
        let (slot, callback) = craft_ui::capture::CaptureSlot::new(ctx, viewport, MAX_SCREENSHOT_BYTES);
        {
            let pending = &mut self.pending_screenshots[index];
            pending.reserved_bytes = bytes;
            pending.issued = Some(CaptureEpoch { geometry: self.geometry_epoch, viewport, document, width, height });
            pending.slot = Some(slot);
        }
        // A pending `Screenshot` asks for the paint it needs, even while hidden.
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(callback));
        ctx.request_repaint();
    }

    /// Answer a pending request with an error and release its reservation.
    fn fail_screenshot(&mut self, index: usize, error: &str) {
        if index >= self.pending_screenshots.len() {
            return;
        }
        let pending = self.pending_screenshots.remove(index);
        self.queued_screenshots.retain(|q| q.id != pending.id);
        self.screenshot_reserved = self.screenshot_reserved.saturating_sub(pending.reserved_bytes);
        if let Some(slot) = pending.slot {
            slot.cancel();
        }
        let _ = pending.reply.send(json!({"ok": false, "error": error}));
    }

    /// Take completed captures on the UI thread only, verify they still match the document and
    /// geometry they were issued for, and write them under the automation roots. A late completion
    /// (after a timeout or close) has already been canceled with its slot, so it writes nothing.
    ///
    /// This runs in `tick`, as both hosts call logic and the UI on the same thread. `save_screenshot`
    /// and `file_access::confine` here, and the `&mut self` write, must move with it if logic ever
    /// runs off the UI thread.
    fn collect_screenshots(&mut self, ctx: &egui::Context) {
        if self.pending_screenshots.is_empty() {
            return;
        }
        let mut completed: Vec<(usize, Result<Arc<egui::ColorImage>, craft_ui::capture::CaptureError>)> = vec![];
        for (index, pending) in self.pending_screenshots.iter().enumerate() {
            if let Some(slot) = &pending.slot
                && let Some(result) = slot.take()
            {
                completed.push((index, result));
            }
        }
        let now = now_ms();
        let document = self.document_identity();
        for (index, result) in completed.into_iter().rev() {
            let Some((id, issued, deadline)) = self.pending_screenshots.get(index).map(|p| (p.id, p.issued, p.deadline_ms)) else { continue };
            match result {
                // The frame came only after the request's deadline: it is late even though the
                // callback had stored it before this pass took it. Do not encode or write it.
                _ if now >= deadline => {
                    self.fail_screenshot(index, SCREENSHOT_TIMEOUT_ERROR);
                }
                Ok(_) if issued.is_some_and(|e| e.document != document) => {
                    self.fail_screenshot(index, "the document changed while the screenshot was pending");
                }
                Ok(image) => {
                    let size = (image.size[0], image.size[1]);
                    let stale =
                        issued.is_none_or(|e| e.geometry != self.geometry_epoch || e.viewport != ctx.viewport_id() || (e.width, e.height) != size);
                    if stale {
                        // Resize, DPI or zoom changed while the capture was in flight: a fresh slot
                        // and a fresh frame, not the old pixels. Attempts are bounded.
                        let retry = self.pending_screenshots[index].attempts == 0;
                        self.pending_screenshots[index].attempts = self.pending_screenshots[index].attempts.saturating_add(1);
                        self.pending_screenshots[index].slot = None;
                        self.pending_screenshots[index].issued = None;
                        if retry {
                            self.queued_screenshots.insert(0, QueuedScreenshot { id, due_ms: now_ms(), frames: 0 });
                        } else {
                            self.fail_screenshot(index, "the window changed while the screenshot was in flight");
                        }
                        continue;
                    }
                    let pending = self.pending_screenshots.remove(index);
                    self.screenshot_reserved = self.screenshot_reserved.saturating_sub(pending.reserved_bytes);
                    let roots = self.automation_roots.clone();
                    let _ = pending
                        .reply
                        .send(file_access::confine(roots.as_ref(), || control::save_screenshot(self, &image, pending.path.as_deref(), pending.data)));
                }
                Err(e) => {
                    let error = format!("the screenshot could not be used: {e}");
                    self.fail_screenshot(index, &error);
                }
            }
        }
        let timed_out: Vec<usize> = self.pending_screenshots.iter().enumerate().filter(|(_, p)| now >= p.deadline_ms).map(|(i, _)| i).collect();
        for index in timed_out.into_iter().rev() {
            self.fail_screenshot(index, SCREENSHOT_TIMEOUT_ERROR);
        }
        if !self.pending_screenshots.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }

    /// Did the Type tool end an IME composition on its own this frame? Interrupting the IME
    /// through egui (`should_interrupt_composition`) doesn't reach the macOS input context, which
    /// keeps the marked text and types it again into the next composition: the host discards it.
    pub fn take_ime_discard(&mut self) -> bool {
        std::mem::take(&mut self.ime_discard)
    }

    /// Was Preferences confirmed since the last call? The host then writes the preferences file,
    /// so what the user just set survives a session that doesn't end with a normal quit (#1747).
    pub fn take_prefs_changed(&mut self) -> bool {
        std::mem::take(&mut self.prefs_changed)
    }

    /// Linked files another app changed while their document is open: every two seconds, and as
    /// soon as the window comes back to the front, stamp the active document's linked files on a
    /// worker thread and act on the changed ones as Preferences › File Handling › Update Links
    /// says ([`vectorcraft_engine::link_watch`]). No look starts while a dialog is open, so an
    /// Ask When Modified question never goes unseen.
    #[cfg(not(target_arch = "wasm32"))]
    fn tick_link_updates(&mut self, ctx: &egui::Context, now: f64) {
        if let Some(r) = self.session.poll_link_scan() {
            match r {
                Ok(r) => {
                    let n = |k: &str| r[k].as_array().map_or(0, Vec::len);
                    if n("updated") > 0 {
                        self.status(format!("Updated {} linked file(s) changed on disk", n("updated")));
                    }
                    if n("ask") > 0 {
                        dialogs::missing_links::ask_update_changed(self, r["ask"].as_array().cloned().unwrap_or_default());
                    }
                    if n("modified") > 0 {
                        self.status(format!("{} linked image(s) changed on disk: Update Links shows the new versions", n("modified")));
                    }
                }
                Err(e) => self.status(e.to_string()),
            }
        }
        let (last_key, focus_key) = (egui::Id::unique("linkWatch.lastScan"), egui::Id::unique("linkWatch.focused"));
        let focused = ctx.input(|i| i.focused);
        let was_focused: bool = ctx.data(|d| d.get_temp(focus_key)).unwrap_or(focused);
        ctx.data_mut(|d| d.insert_temp(focus_key, focused));
        if self.ui.dialog.is_some() || self.session.active().is_none() {
            return;
        }
        let last: f64 = ctx.data(|d| d.get_temp(last_key)).unwrap_or(f64::NEG_INFINITY);
        // The active document as the last look saw it: its uid and revision.
        let (seen_key, doc) = (egui::Id::unique("linkWatch.seen"), self.session.active().map(|d| (d.uid, d.revision)));
        if now - last >= 2.0 || (focused && !was_focused) {
            // Walking the document's links costs a pass over it: only when a look is due.
            // `start_link_scan` starts none for a document without links, which then doesn't
            // wake the app for looks of its own (any frame still looks once one is due).
            ctx.data_mut(|d| d.insert_temp(last_key, now));
            self.session.start_link_scan();
            let links = self.session.link_scan.is_some();
            ctx.data_mut(|d| {
                d.insert_temp(egui::Id::unique("linkWatch.links"), links);
                d.insert_temp(seen_key, doc);
            });
        }
        if self.session.link_scan.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        } else if ctx.data(|d| d.get_temp::<bool>(egui::Id::unique("linkWatch.links"))).unwrap_or(false) {
            ctx.request_repaint_after(std::time::Duration::from_millis(2000));
        } else if ctx.data(|d| d.get_temp::<Option<(u64, u64)>>(seen_key)).is_some_and(|seen| seen != doc) {
            // The document changed (a file placed into one without links) or another came to the
            // front since that look: wake for the next one even if the app sits idle until then,
            // or a file placed and changed meanwhile is first seen after the change, which is then
            // never acted on.
            ctx.request_repaint_after(std::time::Duration::from_secs_f64((last + 2.0 - now).clamp(0.0, 2.0)));
        }
    }

    /// A file dialog is open off the UI thread (a sheet on macOS, [`picks`]): what asked for it runs
    /// again when it answers.
    pub fn file_dialog_open(&self) -> bool {
        self.picks.is_waiting()
    }

    /// A file dialog is open as a sheet on the window ([`Services::dialogs_are_sheets`]): nothing
    /// closes or quits until it answers.
    pub fn file_sheet_open(&self) -> bool {
        self.services.dialogs_are_sheets && self.file_dialog_open()
    }

    /// Show a transient status message.
    pub fn status(&mut self, s: impl Into<String>) {
        self.ui.status = s.into();
    }

    fn drain_inbox(&mut self) {
        let arrived: Vec<(String, Vec<u8>)> =
            self.services.inbox.as_ref().map(|q| std::mem::take(&mut *q.lock().unwrap_or_else(|e| e.into_inner()))).unwrap_or_default();
        for (name, bytes) in arrived {
            if let Err(e) = io::open_bytes(self, &name, &bytes, None) {
                io::report_open_error(self, &name, &e);
            }
        }
        place::drain(self);
    }
}

/// Seconds since the process started, from a monotonic clock (`web_time::Instant`, which maps to
/// `performance.now()` in the browser and `std::time::Instant` natively). Background work, poll
/// pacing and deadlines use this, not egui's `InputState::time`: a logic-only pass replays the
/// last shown frame's time, and a wall clock jumps with the system's.
pub fn now_secs() -> f64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::sync::LazyLock;
        use std::time::Instant;
        static START: LazyLock<Instant> = LazyLock::new(Instant::now);
        START.elapsed().as_secs_f64()
    }
    #[cfg(target_arch = "wasm32")]
    {
        use std::sync::LazyLock;
        use web_time::Instant;
        static START: LazyLock<Instant> = LazyLock::new(Instant::now);
        START.elapsed().as_secs_f64()
    }
}

/// [`now_secs`] in milliseconds, for frame timing and capture deadlines.
pub fn now_ms() -> f64 {
    now_secs() * 1000.0
}

/// The most fonts Type › Recent Fonts lists (Preferences › Type › Number of Recent Fonts).
pub const MAX_RECENT_FONTS: usize = 15;

impl VectorcraftApp {
    /// Type › Recent Fonts: the fonts used last, newest first, as many as Preferences › Type ›
    /// Number of Recent Fonts says.
    pub fn recent_fonts(&self) -> &[String] {
        let n = usize::try_from(self.session.prefs.recent_fonts_count).unwrap_or(MAX_RECENT_FONTS).clamp(1, MAX_RECENT_FONTS);
        self.ui.recent_fonts.get(..n).unwrap_or(&self.ui.recent_fonts)
    }

    /// The selection's bounding box, rotated with rotated objects ([`Session::transform_box`]); with
    /// anchors selected with Direct Selection, the box around them square to the page, which
    /// `object.setBounds` then edits (#1824). The canvas and the transform fields read it every
    /// frame: it is measured once per revision.
    pub fn selection_box(&mut self) -> Option<vectorcraft_doc::OrientedBox> {
        let st = self.session.active()?;
        let key = (st.uid, st.revision, self.session.prefs.use_preview_bounds);
        if let Some((k, b)) = self.canvas.selection_box
            && k == key
        {
            return b;
        }
        let b = match vectorcraft_effects::preview::anchor_bounds(&st.selection, &st.doc, self.session.prefs.use_preview_bounds) {
            Some(r) => Some(vectorcraft_doc::OrientedBox::aligned(r)),
            None => self.session.transform_box(&st.selection.objects),
        };
        self.canvas.selection_box = Some((key, b));
        b
    }

    /// The selection's visual bounds (strokes included, effects left out), square to the page:
    /// measured once per revision, as the canvas reads it every frame (a traced photo selects a
    /// group of hundreds of thousands of paths).
    pub fn selection_bounds(&mut self) -> Option<vectorcraft_geom::Rect> {
        let st = self.session.active()?;
        let key = (st.uid, st.revision);
        if let Some((k, b)) = self.canvas.selection_bounds
            && k == key
        {
            return b;
        }
        let b = st.doc.bounds_of(&st.selection.objects, true);
        self.canvas.selection_bounds = Some((key, b));
        b
    }

    /// The art and the artboards together, the span of the canvas's scroll bars: measured once
    /// per revision, as the canvas reads it every frame.
    pub fn content_bounds(&mut self) -> Option<vectorcraft_geom::Rect> {
        let st = self.session.active()?;
        let key = (st.uid, st.revision);
        if let Some((k, b)) = self.canvas.content_bounds
            && k == key
        {
            return b;
        }
        let boards = st.doc.artboards.iter().map(|a| a.rect).reduce(|r, a| r.union(a));
        let b = vectorcraft_geom::union_opt(st.doc.art_bounds(), boards);
        self.canvas.content_bounds = Some((key, b));
        b
    }
}

/// eframe isn't a dependency of this crate (the host owns the event loop); these entry points are
/// called from the host's `eframe::App` impl.
impl VectorcraftApp {
    /// The language the UI is drawn in: the Preferences dialog's choice while it is open (so a
    /// change shows before OK), else the `interfaceLanguage` preference (`auto` = the system's).
    pub fn ui_language(&self) -> i18n::Lang {
        let editing =
            self.ui.dialog.as_ref().filter(|d| d.kind == "preferences").and_then(|d| d.fields.get("interfaceLanguage")).and_then(Value::as_str);
        i18n::Lang::from_pref(editing.unwrap_or(&self.session.prefs.interface_language))
    }

    /// The host starts one iteration: called from both hosts' `raw_input_hook`, before the host
    /// decides whether a UI pass follows. It counts the iteration; it is not a frame or pass count
    /// and does not promise that `ui` will run (a hidden window gets `tick` only).
    pub fn begin_host_iteration(&mut self) {
        self.host_iteration = self.host_iteration.wrapping_add(1);
        // Frame keys identify input already taken in this host iteration. Each iteration starts
        // fresh; this is what makes a viewport's discarded pass a duplicate, while two viewports'
        // frame 0 both count.
        self.input_frames.clear();
        self.shortcuts_frames.clear();
    }

    /// Background work, once per host iteration (full or logic-only): control requests, background
    /// saves/exports/opens, plug-ins, recovery, font and clipboard polls, capture completion. It
    /// reads only viewport state from `ctx.input` — never events or `i.time`: a logic-only pass
    /// leaves the last shown frame's input in place.
    ///
    /// A bug that panics costs one iteration and shows an error, instead of closing the app with
    /// unsaved work.
    pub fn tick(&mut self, ctx: &egui::Context) {
        let confined = self.synthetic_frame.then(|| self.automation_roots.clone()).flatten();
        if let Err(msg) = file_access::confine(confined.as_ref(), || vectorcraft_engine::guard::catch_panic(|| self.tick_frame(ctx))) {
            self.picks.recover();
            self.status(format!("Internal error: {msg} (please report this bug)"));
        }
    }

    /// [`Self::tick`]'s work, once per host iteration: a second call in the same iteration (a
    /// discarded rerun shares one hook call) does nothing.
    fn tick_frame(&mut self, ctx: &egui::Context) {
        if self.tick_iteration == Some(self.host_iteration) {
            return;
        }
        self.tick_iteration = Some(self.host_iteration);
        let dialog_was_open = self.ui.dialog.is_some();
        let lang = self.ui_language();
        i18n::set_current(lang);
        // The engine gives new type the Japanese defaults while the UI is in Japanese.
        if self.session.ui_language.as_deref() != Some(lang.code()) {
            self.session.ui_language = Some(lang.code().to_string());
        }
        self.install_injected_settle_hook(ctx);
        self.advance_geometry_epoch(ctx);
        // Monotonic host time, never egui's `InputState::time`: a logic-only (hidden) iteration
        // replays the last shown pass's time, freezing recovery, clipboard polling and deadlines.
        // Tests inject a clock to drive idle scheduling deterministically.
        #[cfg(test)]
        let now = self.now_override.unwrap_or_else(now_secs);
        #[cfg(not(test))]
        let now = now_secs();
        self.perf.fps = self.frame_rate.frame(now, self.perf.frame_ms / 1000.0);
        self.sync_views();
        updates_ui::tick(self, ctx);
        // Read the system clipboard only when that alone decides whether Paste is enabled. A
        // background thread reads it where the host installs one (an unresponsive owner must never
        // stall the frame loop); otherwise, or once that thread is gone, it is read here, at most a
        // few times a second (opening it locks it against other apps on some systems).
        let wanted = self.session.clipboard.is_empty() && self.session.active().is_some();
        if let Some(make) = self.services.clipboard_probe.take() {
            self.clipboard_probe = clipboard_probe::Probe::start(make, ctx.clone());
        }
        if let Some(pasteable) = self.clipboard_probe.as_mut().and_then(|p| p.pasteable(wanted)) {
            self.system_paste = pasteable;
        } else if !(0.0..SYSTEM_CLIPBOARD_POLL).contains(&(now - self.system_paste_at)) {
            self.clipboard_probe = None;
            self.system_paste_at = now;
            self.system_paste = wanted && self.system_clipboard_pasteable();
        }
        #[cfg(not(target_arch = "wasm32"))]
        if !(0.0..FONT_INSTALL_POLL).contains(&(now - self.font_install_check_at)) {
            self.font_install_check_at = now;
            // Read on the next frame the app draws anyway: an idle app isn't woken for it.
            self.refresh_installed_fonts(false);
        }
        self.poll_font_check(ctx);
        picks::poll(self, ctx);
        // A Paste the host read before this iteration's formats were written must still wait for
        // them, even if the worker below finishes them during this same tick.
        ctx.data_mut(|d| d.insert_temp(egui::Id::unique("clipboard.pending-at-tick"), self.clipboard_publishing.is_pending()));
        background::poll(self);
        screen_color::poll(self, ctx);
        opening::poll(self);
        plugin_runs::poll(self);
        placing::poll(self);
        #[cfg(not(target_arch = "wasm32"))]
        self.tick_link_updates(ctx, now);
        if !self.background.jobs.is_empty()
            || !self.opening.pending.is_empty()
            || !self.plugin_runs.running.is_empty()
            || !self.placing.pending.is_empty()
        {
            // Keep the status bar's progress moving and pick the result up when it arrives.
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        // The window's close button (or the system quitting the app) asks about unsaved documents,
        // once the saves running in the background are done. Not while a file dialog's sheet is on
        // the window: what asked for it runs again when it answers.
        if ctx.input(|i| i.viewport().close_requested()) {
            // This native viewport is going away: release its IME context. No request is
            // allocated here: the close prompt or file sheet publishes no custom descriptor, so a
            // token would survive for a later stock field. A close the user then cancels starts a
            // fresh generation on the next pass (never inherits a composition across the prompt).
            ime::close_viewport(self, ctx);
            self.ime_context.end_viewport(ctx.viewport_id());
            background::wait_all(self);
            if self.file_sheet_open() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.status(picks::busy());
            } else if unsaved::any_dirty(self) {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                if let Err(e) = unsaved::close_all(self, "quit") {
                    self.status(e);
                }
            } else {
                // Quitting with nothing unsaved: no copies to leave behind.
                vectorcraft_engine::cmd::recovery::forget_all(&mut self.session);
            }
        }
        if let Some(wait) = recovery::frame(self, now) {
            // The timer and heartbeat run in an idle window too.
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait));
        }
        crate::keymap::note_os(ctx);
        shortcut_editor::sync(&self.ui);
        prefs_dialog::apply_runtime(self, ctx);
        // The OS title bar (and the taskbar / Alt-Tab entry) follows the active file; with the
        // system title bar this is where the document name lives, as the in-app mark is hidden.
        crate::chrome::sync_window_title(self, ctx);
        self.drain_control(ctx);
        // An egui `PasteImage` with a rich system clipboard: run the paste command now, once. It
        // goes through `run`, whose `edit.paste` branch calls `adopt_system_clipboard` (which clears
        // the request) to read the formats and then pastes them. A text paste in the same UI pass
        // never sets the request, so the two cannot both paste.
        if let Some(held) = self.clipboard_image_request.take() {
            crate::menus::invoke(self, crate::shortcuts::paste_command(held), json!({}));
        }
        self.collect_screenshots(ctx);
        if let Some(u) = self.pending_url.take() {
            ctx.open_url(egui::OpenUrl::new_tab(u));
        }
        self.flush_clipboard_out(ctx);
        self.drain_inbox();
        if self.viewport_hidden(ctx) && (self.needs_hidden_paint() || (!dialog_was_open && self.ui.dialog.is_some())) {
            // A plain repaint runs only logic in a hidden window: the ui that applies injected
            // input, shows a queued dialog or issues a capture needs this command. egui-winit
            // handles it; the 0.37 web backend does not (a pending fork fix), so a hidden browser
            // tab cannot be claimed as qualified.
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaintWhileHidden);
        }
        if !self.synthetic.is_empty() || self.synthetic_injected || !self.pending_screenshots.is_empty() {
            ctx.request_repaint();
        }
    }

    /// The 0.36-style entry point, kept for the crate's tests, which call one `logic` inside a
    /// `run_ui` pass and draw a panel afterwards (a UI phase). Production hosts call
    /// `raw_input_hook`, then `tick` in `App::logic` and `ui` in `App::ui`; `tick` alone never
    /// takes input, so a hidden logic-only iteration cannot replay the last shown frame's events.
    ///
    /// Outside tests this is just [`Self::tick`] with an iteration boundary.
    pub fn logic(&mut self, ctx: &egui::Context) {
        self.begin_host_iteration();
        self.tick(ctx);
        #[cfg(test)]
        {
            let confined = self.synthetic_frame.then(|| self.automation_roots.clone()).flatten();
            let _ = file_access::confine(confined.as_ref(), || {
                vectorcraft_engine::guard::catch_panic(|| {
                    self.ui_setup(ctx);
                    self.handle_input_once(ctx);
                })
            });
        }
        self.finish_ime_logic(ctx);
    }

    /// Install once during host Context creation, before its first run_ui.
    pub fn install_ime_reset_hooks(ctx: &egui::Context) {
        ime::install_hooks(ctx);
    }

    /// Host logic wrappers call this after their last application mutation. Full-pass logic
    /// leaves final output observation to UI; standalone logic retires unavailable publication.
    pub fn finish_ime_logic(&mut self, ctx: &egui::Context) {
        ime::finish_logic(self, ctx);
    }

    /// First UI-pass setup, in a real `run_ui` phase: adopt the context, install the interface
    /// fonts and theme, and the image loaders. 0.37's `set_fonts` applies at once when no text has
    /// been laid out, so the same pass can draw; a hidden logic-only iteration never installs them.
    fn ui_setup(&mut self, ctx: &egui::Context) {
        // Hosts install before their first pass. Installing here covers later passes only: egui
        // has already snapshotted this pass's plugin list, so a direct harness that calls `logic`
        // inside its first `run_ui` must call `install_ime_reset_hooks` before that `run_ui`
        // (as `tests_lifecycle::ime_frame` does). This is not a first-pass bootstrap fallback.
        ime::install_hooks(ctx);
        self.adopt_context(ctx);
        if !self.styled {
            theme::apply(ctx, self.ui.brightness);
            egui_extras::install_image_loaders(ctx);
            self.styled = true;
        }
        if !self.fonts_ready {
            // `set_fonts` applies at once only while no text was laid out in this pass; when it
            // defers, the next pass applies it and this leaves input and drawing to that pass.
            let fonts = theme::interface_fonts();
            ctx.set_fonts(fonts.clone());
            self.fonts_ready = ctx.fonts(|applied| applied.definitions() == &fonts);
        }
    }

    /// The first UI pass of a host iteration takes the host's input, once per context, viewport and
    /// frame: clipboard formats, native-menu clicks, shortcuts and dropped files. A later pass
    /// (`request_discard` rerun) or an interleaved second viewport pass with the same key draws
    /// again but does not replay them; a logic-only iteration never reaches this.
    fn handle_input_once(&mut self, ctx: &egui::Context) {
        let frame = (graphics::context_tag(ctx), ctx.viewport_id(), ctx.cumulative_frame_nr_for(ctx.viewport_id()));
        if !take_frame_once(&mut self.input_frames, frame) {
            return;
        }
        self.write_formats_before_text_input(ctx);
        native_menu::run(self, ctx);
        if self.fonts_ready {
            shortcuts::handle(self, ctx);
        }
        // A Copy taken in this UI phase (an egui copy event, or a command it runs) is published in
        // this same pass: waiting for the next `tick` can leave the command unissued when an idle
        // web host has no further frame to run it in. Native rich-format Copy does not use this
        // path; it writes through `SystemClipboard` and leaves `clipboard_out` empty.
        self.flush_clipboard_out(ctx);
        // Native only: the web host reads dropped files asynchronously and feeds the inboxes.
        #[cfg(not(target_arch = "wasm32"))]
        self.take_dropped_files(ctx);
    }

    /// Hand a queued text copy to egui. Called after `tick`, after the UI phase's input and at the
    /// end of the pass; the later calls are what an input- or menu-driven fallback Copy needs, and
    /// `take` keeps it single-shot.
    fn flush_clipboard_out(&mut self, ctx: &egui::Context) {
        if let Some(t) = self.clipboard_out.take() {
            ctx.copy_text(t);
        }
    }

    /// Is the window minimized or fully occluded (as far as the host reports)?
    fn viewport_hidden(&self, ctx: &egui::Context) -> bool {
        ctx.input(|i| {
            let v = i.viewport();
            v.minimized == Some(true) || v.occluded == Some(true)
        })
    }

    /// Is there work that only a UI pass can do, so a hidden window must be asked to paint?
    fn needs_hidden_paint(&self) -> bool {
        !self.synthetic.is_empty()
            || self.synthetic_injected
            || !self.queued_screenshots.is_empty()
            || self.pending_screenshots.iter().any(|p| p.slot.is_none())
    }

    /// Capture the viewport identity and geometry (content and inner rectangles, pixels per point and
    /// zoom) whose change advances the screenshot epoch.
    fn geometry_stamp(ctx: &egui::Context) -> GeometryStamp {
        GeometryStamp {
            viewport: ctx.viewport_id(),
            content: ctx.content_rect(),
            inner: ctx.input(|i| i.viewport().inner_rect),
            pixels_per_point: ctx.pixels_per_point(),
            zoom: ctx.zoom_factor(),
        }
    }

    /// Files dropped on the window: documents, libraries, presets, plug-ins and shortcut sets
    /// opened, pictures and text placed on the canvas ([`Self::drop_target`]).
    #[cfg(not(target_arch = "wasm32"))]
    fn take_dropped_files(&mut self, ctx: &egui::Context) {
        let (dropped, shift) = ctx.input(|i| (i.raw.dropped_files.clone(), i.modifiers.shift));
        self.open_dropped_files(dropped, place::drag_pos(ctx), shift);
    }

    /// Files dropped on the main window or on a floating panel's own OS window, opened or placed
    /// by kind; `pos` is the drop point in the main window, where the backend reports one.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn open_dropped_files(&mut self, dropped: Vec<egui::DroppedFileHandle>, pos: Option<egui::Pos2>, shift: bool) {
        if dropped.is_empty() {
            return;
        }
        let mut files = vec![];
        for f in dropped {
            let path = Some(f.path().to_string_lossy().to_string()).filter(|s| !s.is_empty());
            let name = path.as_deref().map_or_else(|| "dropped".into(), vectorcraft_engine::cmd::fileio::file_name);
            // Native file drops usually carry a path. Use the same file reader as File → Open
            // (and File → Place), even if the drop handle can't supply the bytes itself.
            let bytes = if path.is_some() && self.services.read.is_some() { Ok(vec![]) } else { f.bytes() };
            match bytes {
                Ok(bytes) => {
                    // A backend may give bytes without a path or extension. Sniff their format
                    // before deciding: an SVG opens as a document, a raster image is placed.
                    let target = if path.is_none() && fileio::detect(&name, &bytes).is_none_or(|f| !f.raster) {
                        place::DropTarget::Open
                    } else {
                        self.drop_target(&name, pos, shift)
                    };
                    files.push((target, (name, path, bytes)));
                }
                Err(e) => self.status(format!("Couldn't read {name}: {e}")),
            }
        }
        place::drop_files(self, files);
    }

    /// Fonts installed or removed while the app runs are listed without Refresh Font List: a look
    /// at the font folders and the platform's font list on a timer and when the window comes back,
    /// a scan only when they changed.
    /// The look runs on another thread (asking DirectWrite for the fonts font services loaded
    /// meanwhile takes tens of milliseconds on Windows, #579), [`Self::poll_font_check`] scans;
    /// `wake` wakes the app for the answer.
    fn refresh_installed_fonts(&mut self, wake: bool) {
        if let Some((_, w)) = &mut self.font_check {
            *w |= wake;
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let look = move || {
            // The UI gone meanwhile has nothing to refresh.
            let _ = tx.send(vectorcraft_text::FontDb::global().installed_fonts_changed());
        };
        let spawned =
            if cfg!(target_arch = "wasm32") { None } else { std::thread::Builder::new().name("font-check".into()).spawn(look.clone()).ok() };
        if spawned.is_none() {
            look();
        }
        self.font_check = Some((rx, wake));
    }

    /// Rescan the fonts once [`Self::refresh_installed_fonts`]'s look says they changed.
    fn poll_font_check(&mut self, ctx: &egui::Context) {
        let Some((check, wake)) = &self.font_check else { return };
        match check.try_recv() {
            Ok(changed) => {
                self.font_check = None;
                if changed {
                    // A failure shows in the status bar, as the menu item's does.
                    let _ = self.run("text.rescanFonts", json!({}));
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                if *wake {
                    ctx.request_repaint_after(std::time::Duration::from_millis(50));
                }
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.font_check = None,
        }
    }

    /// Advance the capture geometry epoch when any part of the window geometry changed, even back
    /// to a previous value: a capture issued before the change must not be saved as if it showed
    /// the new one.
    fn advance_geometry_epoch(&mut self, ctx: &egui::Context) {
        let stamp = Self::geometry_stamp(ctx);
        if self.geometry_stamp != Some(stamp) {
            self.geometry_stamp = Some(stamp);
            self.geometry_epoch = self.geometry_epoch.wrapping_add(1);
        }
    }

    /// Identify the document, modal dialog and focused widget that the current accepted input batch
    /// belongs to.
    fn input_generation(&self) -> InputGeneration {
        InputGeneration {
            document: self.session.active().map_or(0, |document| document.uid),
            dialog: self.ui.dialog.as_ref().map(|dialog| egui::Id::unique(&dialog.kind)),
            focus: self.ui_ctx.as_ref().and_then(|ctx| ctx.memory(|memory| memory.focused())),
        }
    }

    /// Install the end-of-pass observer that tells the next host iteration a settled UI pass took
    /// the injected batch (a font-gated or discarded pass does not). The shipped hosts have only
    /// the root viewport; a multi-viewport host would need pending batches and settlement keyed
    /// per viewport instead of this root-only acknowledgement.
    fn install_injected_settle_hook(&mut self, ctx: &egui::Context) {
        if self.injected_settle_hook {
            return;
        }
        let settled = self.injected_settled.clone();
        ctx.on_end_pass(
            "vectorcraft injected input settlement",
            Arc::new(move |ui| {
                if ui.ctx().viewport_id() == egui::ViewportId::ROOT && !ui.ctx().will_discard() {
                    settled.store(true, Ordering::Relaxed);
                }
            }),
        );
        self.injected_settle_hook = true;
    }

    /// The settled pass took the injected batch: allow the next step at the next host iteration.
    fn acknowledge_injected_batch(&mut self) {
        self.synthetic_injected = false;
        self.injected_events.clear();
        self.synthetic_frame = false;
    }

    /// The document, dialog or focus changed before a UI pass took the batch: remove exactly the
    /// events we appended (they still live in eframe's pending raw input) so a forced hidden paint
    /// cannot replay stale clicks or text into the new state.
    fn discard_stale_injected_batch(&mut self, raw: &mut egui::RawInput) {
        if !self.synthetic_injected || self.input_generation() == self.injected_generation {
            return;
        }
        let events = std::mem::take(&mut self.injected_events);
        let mut removed = false;
        if !events.is_empty()
            && let Some(start) = raw.events.windows(events.len()).rposition(|window| window == events.as_slice())
        {
            raw.events.drain(start..start + events.len());
            removed = true;
        }
        if !events.is_empty() && !removed {
            // The host's raw input no longer contains the batch as we injected it (another
            // handler rewrote or interleaved events). We cannot tell stale synthetic input from
            // the person's, so fail closed: deliver nothing this pass rather than replay a stale
            // click or text into the state that replaced it, and cancel the rest of that
            // synthetic sequence instead of injecting its next step after this one.
            raw.events.clear();
            self.synthetic.clear();
        }
        // The discarded batch may have carried the modifiers it set; the host's modifiers are
        // what the next real event has, so do not leave a synthetic latch behind.
        self.synthetic_modifiers = false;
        self.synthetic_injected = false;
        self.synthetic_frame = false;
    }

    /// Inject synthetic events (one press/release step, wheel turn or short key sequence per host
    /// iteration), carrying control provenance until a settled UI pass has taken the batch. The
    /// batch belongs to the one root viewport the shipped hosts draw (see
    /// `install_injected_settle_hook`); a multi-viewport host would key it by viewport. A
    /// hidden logic-only iteration may see this hook run without a UI pass: nothing more is
    /// injected until the batch was actually applied, so a burst of hidden ticks cannot queue
    /// several steps of a drag or replay them together later.
    pub fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        // Both hosts call this hook once per host iteration, before deciding whether a UI pass
        // follows; it is the iteration boundary, not a UI-phase marker.
        self.begin_host_iteration();
        // The observer lives on the context so it also settles a harness that consumes raw input
        // without calling `VectorcraftApp::ui`.
        self.install_injected_settle_hook(ctx);
        // The native menu's key equivalents become the input their keys make, ahead of what came
        // after them (see `native_menu`).
        let keys = self.services.native_menu.as_mut().map(native_menu::NativeMenu::take_keys).unwrap_or_default();
        // The menu bar does nothing while a file dialog is open (`native_menu`): a key that raced it
        // going inert is dropped.
        if !keys.is_empty() && !self.file_dialog_open() {
            let events: Vec<egui::Event> = keys.into_iter().flat_map(|k| native_menu::key_events(k, || self.system_clipboard_text())).collect();
            raw.events.splice(0..0, events);
        }
        for e in &raw.events {
            match e {
                egui::Event::ModifiersChanged(m) => self.host_modifiers = *m,
                egui::Event::WindowFocused(false) => self.host_modifiers = egui::Modifiers::NONE,
                egui::Event::WindowFocused(true) => self.refresh_installed_fonts(true),
                _ => {}
            }
        }
        // Keys a floating panel's own OS window left over run here, as if typed in this window,
        // under the modifiers held there for this frame only.
        if std::mem::take(&mut self.forwarded_modifiers_held) {
            raw.events.insert(0, egui::Event::ModifiersChanged(self.host_modifiers));
        }
        self.forwarded_frame = !self.forwarded.is_empty();
        if !self.forwarded.is_empty() {
            if let Some(m) = self.forwarded_modifiers.take() {
                raw.events.push(egui::Event::ModifiersChanged(m));
                self.forwarded_modifiers_held = true;
            }
            raw.events.append(&mut self.forwarded);
        }
        // The previous pass settled: the batch it took is done, so the next step may go.
        if self.injected_settled.swap(false, Ordering::Relaxed) {
            self.acknowledge_injected_batch();
        }
        self.discard_stale_injected_batch(raw);
        // While the previous batch is unapplied (it lives in eframe's pending raw input), inject
        // nothing more: one step per settled UI pass, not per hidden logic tick.
        if self.synthetic_injected {
            return;
        }
        let Some(first) = self.synthetic.first() else {
            if std::mem::take(&mut self.synthetic_modifiers) {
                raw.events.push(egui::Event::ModifiersChanged(self.host_modifiers));
            }
            return;
        };
        // Pointer events go one per frame so egui sees presses, drags and releases as real input;
        // keyboard sequences go up to the key release.
        let n = match first {
            egui::Event::PointerMoved(_) | egui::Event::PointerButton { .. } | egui::Event::MouseWheel { .. } => 1,
            _ => self.synthetic.iter().position(|e| matches!(e, egui::Event::Key { pressed: false, .. })).map_or(self.synthetic.len(), |i| i + 1),
        };
        let (now, later) = self.synthetic.split_at(n.min(self.synthetic.len()));
        let mut batch: Vec<egui::Event> = Vec::with_capacity(n + 2);
        if let egui::Event::PointerMoved(p) | egui::Event::PointerButton { pos: p, .. } = first {
            batch.push(egui::Event::PointerMoved(*p));
        }
        // This frame's key or button, else the button a drag holds down (released later).
        let held = now
            .iter()
            .find_map(|e| match e {
                egui::Event::Key { modifiers, .. } | egui::Event::PointerButton { modifiers, .. } | egui::Event::MouseWheel { modifiers, .. } => {
                    Some(*modifiers)
                }
                _ => None,
            })
            .or_else(|| match later.iter().find(|e| matches!(e, egui::Event::PointerButton { .. })) {
                Some(egui::Event::PointerButton { pressed: false, modifiers, .. }) => Some(*modifiers),
                _ => None,
            });
        match held {
            Some(m) => {
                batch.push(egui::Event::ModifiersChanged(m));
                self.synthetic_modifiers = true;
            }
            None if std::mem::take(&mut self.synthetic_modifiers) => batch.push(egui::Event::ModifiersChanged(self.host_modifiers)),
            None => {}
        }
        batch.extend(now.iter().cloned());
        // `batch` holds the n drained events plus the pointer/modifier companions that make the
        // step read like real input.
        self.synthetic.drain(..n);
        self.injected_generation = self.input_generation();
        self.injected_events = batch.clone();
        self.synthetic_injected = true;
        self.synthetic_frame = true;
        raw.events.extend(batch);
    }

    /// The canvas panel's body: the document tabs, the canvas and the isolation bar over it.
    pub(crate) fn canvas_area(&mut self, ui: &mut egui::Ui) {
        let t = theme::Tokens::get(ui.ctx());
        ui.painter().rect_filled(ui.max_rect(), 0.0, t.pasteboard);
        if self.ui.screen_mode < 3 {
            chrome::doc_tabs(self, ui);
        }
        canvas::show(self, ui);
        // Over the canvas inside the rulers, so the rulers and scroll bars stay uncovered.
        if self.ui.screen_mode < 3
            && !menus::home_showing(self)
            && let Some(area) = self.canvas_rect
        {
            chrome::isolation_bar(self, ui, area);
        }
    }

    /// Lay out the whole window: the first UI pass of a host iteration sets the UI up (fonts,
    /// theme, image loaders) and takes the input; later passes (a `request_discard` rerun) draw
    /// only.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ime::begin_ui(self, &ctx);
        let confined = self.synthetic_frame.then(|| self.automation_roots.clone()).flatten();
        if let Err(msg) = file_access::confine(confined.as_ref(), || {
            vectorcraft_engine::guard::catch_panic(|| {
                self.ui_setup(&ctx);
                self.handle_input_once(&ctx);
                self.ui_frame(ui);
            })
        }) {
            self.picks.recover();
            self.status(format!("Internal error: {msg} (please report this bug)"));
        }
        ime::finish_ui(self, &ctx);
    }

    /// [`Self::ui`]'s work, inside its guard: the bars, panels and canvas, then the dialogs and
    /// progress windows over them.
    fn ui_frame(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if !self.fonts_ready {
            // Do not draw (or run the app's input handling) before the interface fonts are the
            // context's: a deferred `set_fonts` applies at the start of the next pass.
            ctx.request_repaint();
            return;
        }
        crate::state::sync_zoom_stops_display_print_size(self.session.prefs.display_print_size);
        let t0 = now_ms();
        widgets::set_dialog_open(&ctx, self.ui.dialog.is_some());
        // The canvas records this pass's IME caret geometry; a pass that draws no canvas has none.
        self.ime_caret_rect = None;
        scrub::begin_frame(self, &ctx);
        font_menu::end_stale_preview(self, &ctx);
        tools_float::track(self, &ctx);
        // A selection move released over a panel ends before any panel sees the release (#2826).
        canvas::release_over_panel(self, &ctx);
        let t = theme::Tokens::get(&ctx);
        if self.ui.screen_mode < 2 {
            chrome::app_bar(self, ui);
            if self.ui.control_bar {
                chrome::control_bar(self, ui);
            }
        }
        if self.ui.status_bar && self.ui.screen_mode < 3 {
            chrome::status_bar(self, ui);
            chrome::hint_bar(self, ui);
        } else {
            // Keeps the previous frame's value valid for menus drawn above, before the bars.
            widgets::set_menu_floor(&ctx, ctx.content_rect().bottom());
        }
        if self.ui.toolbar && self.ui.screen_mode < 3 {
            toolbar::show(self, ui);
        }
        panels::library_panel::tidy(self);
        // The canvas and the panels: one docking layout. Tab hides the panels but not the canvas.
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.pasteboard)).show(ui, |ui| panel_docking::show(self, ui));
        dialogs::show(self, &ctx);
        updates_ui::show(self, &ctx);
        export_progress::show(self, &ctx);
        plugin_runs::show(self, &ctx);
        palette::show(self, &ctx);
        if self.custom_titlebar {
            titlebar::resize_zones(ui);
        }
        self.ui_fonts.frame(&ctx);
        scrub::end_frame(self, &ctx);
        native_menu::sync(self, &ctx);
        // Observe once, after all of this pass's input, dialogs, floating panels, palette and
        // commands: the latest completed pass is authoritative for owner, text and cause.
        ime::end_of_pass(self, &ctx);
        self.drop_formats_after_egui_copy(&ctx);
        // A command run by a menu, panel or dialog in this pass may have queued its text copy
        // after the UI phase's input flush. Publish it now for the same reason as there.
        self.flush_clipboard_out(&ctx);
        if opening::long_operation_busy(self) {
            ctx.set_cursor_icon(egui::CursorIcon::Progress);
        }
        // A pass egui will discard and rerun is not a presented frame: it must not become the
        // frame-time sample, and it must not issue a capture of what it drew.
        if !ctx.will_discard() {
            self.perf.frame_ms = now_ms() - t0;
            self.issue_screenshots(&ctx);
        }
        let _ = json!(null);
    }
}
