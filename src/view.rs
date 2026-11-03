//! Live browser view in a terminal pane (herdr desktop and the herdr TUI): CDP
//! screencast frames drawn by `render`, an address bar, a clickable toolbar, and
//! mouse and keys forwarded to the page.

use crate::agent::{self, SendKind};
use crate::cdp::Cdp;
use crate::icons::{self, Icon, Rgb};
use crate::render::{self, Cells, IMAGE_ID, Renderer, SYNC_OFF, SYNC_ON};
use crate::state::{self, Config, Mode};
use crate::{R, chrome, page};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use crossterm::{cursor, execute, queue, terminal};
use serde_json::{Value, json};
use std::io::{Write, stdout};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

enum Msg {
    Frame { data: String, w: f64, h: f64 },
    Gone(String),
}

/// What a stream shows: the page's emulated viewport (CSS px, headless only) and the
/// largest frame wanted (device px; small for half blocks, which can't use more).
#[derive(Debug, Clone, Copy, PartialEq)]
struct StreamSize {
    viewport: (u32, u32),
    dpr: f64,
    max_frame: (u32, u32),
}

/// Screencast `target` on its own connection until `stop`; acks every frame.
fn stream(url: String, target: String, size: StreamSize, headless: bool, tx: Sender<Msg>, stop: Arc<AtomicBool>) {
    let run = || -> R<()> {
        let mut c = Cdp::connect(&url)?;
        let r = c.call("Target.attachToTarget", json!({ "targetId": target, "flatten": true }), None)?;
        let sid = r["sessionId"].as_str().ok_or("no session")?.to_string();
        if headless {
            // Session-scoped: the page gets the pane's size only while this view watches it.
            c.call(
                "Emulation.setDeviceMetricsOverride",
                json!({ "width": size.viewport.0, "height": size.viewport.1, "deviceScaleFactor": size.dpr, "mobile": false }),
                Some(&sid),
            )?;
        }
        c.call(
            "Page.startScreencast",
            json!({ "format": "png", "maxWidth": size.max_frame.0, "maxHeight": size.max_frame.1, "everyNthFrame": 1 }),
            Some(&sid),
        )?;
        while !stop.load(Ordering::Relaxed) {
            let Some(m) = c.next_message(Duration::from_millis(200))? else { continue };
            match m["method"].as_str() {
                Some("Page.screencastFrame") => {
                    let p = &m["params"];
                    c.send("Page.screencastFrameAck", json!({ "sessionId": p["sessionId"] }), Some(&sid))?;
                    let msg = Msg::Frame {
                        data: p["data"].as_str().unwrap_or_default().to_string(),
                        w: p.pointer("/metadata/deviceWidth").and_then(Value::as_f64).unwrap_or(size.viewport.0 as f64),
                        h: p.pointer("/metadata/deviceHeight").and_then(Value::as_f64).unwrap_or(size.viewport.1 as f64),
                    };
                    if tx.send(msg).is_err() {
                        return Ok(());
                    }
                }
                Some("Target.detachedFromTarget" | "Inspector.detached") => return Err("tab closed".into()),
                _ => {}
            }
        }
        Ok(())
    };
    if let Err(e) = run() {
        let _ = tx.send(Msg::Gone(e.to_string()));
    }
}

/// Address-bar editing. Starts with the whole URL selected, like a browser:
/// typing replaces it, an arrow key or click drops the selection.
#[derive(Debug, Clone, PartialEq)]
pub struct UrlEdit {
    buf: Vec<char>,
    cur: usize,
    all: bool,
}

#[derive(Debug, PartialEq)]
pub enum Edit {
    Editing,
    Go(String),
    Cancel,
}

impl UrlEdit {
    pub fn new(url: &str) -> UrlEdit {
        let buf: Vec<char> = url.chars().collect();
        UrlEdit { cur: buf.len(), buf, all: !url.is_empty() }
    }

    fn take_selection(&mut self) -> bool {
        let had = self.all;
        if had {
            self.buf.clear();
            self.cur = 0;
            self.all = false;
        }
        had
    }

    pub fn key(&mut self, k: KeyEvent) -> Edit {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match k.code {
            KeyCode::Esc => return Edit::Cancel,
            KeyCode::Enter => return Edit::Go(self.text()),
            KeyCode::Char('a') if ctrl => (self.cur, self.all) = (0, false),
            KeyCode::Char('e') if ctrl => (self.cur, self.all) = (self.buf.len(), false),
            KeyCode::Char('u') if ctrl => {
                if !self.take_selection() {
                    self.buf.drain(..self.cur);
                    self.cur = 0;
                }
            }
            KeyCode::Char('w') if ctrl => {
                if !self.take_selection() {
                    let end = self.cur;
                    while self.cur > 0 && self.buf[self.cur - 1] == ' ' {
                        self.cur -= 1;
                    }
                    while self.cur > 0 && !"/ .?&=#:".contains(self.buf[self.cur - 1]) {
                        self.cur -= 1;
                    }
                    self.buf.drain(self.cur..end);
                }
            }
            KeyCode::Char(c) if !ctrl => {
                self.take_selection();
                self.buf.insert(self.cur, c);
                self.cur += 1;
            }
            KeyCode::Backspace => {
                if !self.take_selection() && self.cur > 0 {
                    self.cur -= 1;
                    self.buf.remove(self.cur);
                }
            }
            KeyCode::Delete => {
                if !self.take_selection() && self.cur < self.buf.len() {
                    self.buf.remove(self.cur);
                }
            }
            KeyCode::Left => (self.cur, self.all) = (self.cur.saturating_sub(1), false),
            KeyCode::Right => (self.cur, self.all) = ((self.cur + 1).min(self.buf.len()), false),
            KeyCode::Home => (self.cur, self.all) = (0, false),
            KeyCode::End => (self.cur, self.all) = (self.buf.len(), false),
            _ => {}
        }
        Edit::Editing
    }

    pub fn text(&self) -> String {
        self.buf.iter().collect()
    }

    /// First visible char so the cursor stays inside a `width`-wide field.
    fn scroll(&self, width: usize) -> usize {
        (self.cur + 1).saturating_sub(width.max(1))
    }

    /// Put the cursor under field column `col`.
    pub fn click(&mut self, col: usize, width: usize) {
        self.all = false;
        self.cur = (self.scroll(width) + col).min(self.buf.len());
    }
}

/// Output for the writer thread. Frames are large (hundreds of KB), so writing them
/// on the main loop would make keys, clicks and scrolling wait behind every frame.
enum Out {
    Bytes(Vec<u8>),
    Frame { pre: Vec<u8>, b64: String, cols: u16, img_cols: u16, rows: u16, renderer: Renderer },
}

/// Write output in order, but when frames pile up show only the newest. Owns the Kitty
/// double-buffer flip so a dropped frame can't make the next one replace the visible image.
fn spawn_writer() -> (Sender<Out>, std::thread::JoinHandle<()>) {
    let (tx, rx) = mpsc::channel::<Out>();
    let handle = std::thread::spawn(move || {
        let mut flip = false;
        while let Ok(first) = rx.recv() {
            let mut batch = vec![first];
            batch.extend(rx.try_iter());
            let newest = batch.iter().rposition(|o| matches!(o, Out::Frame { .. }));
            let mut buf = Vec::new();
            for (i, item) in batch.into_iter().enumerate() {
                match item {
                    Out::Bytes(b) => buf.extend_from_slice(&b),
                    Out::Frame { pre, b64, cols, img_cols, rows, renderer } if Some(i) == newest => {
                        buf.extend_from_slice(SYNC_ON.as_bytes());
                        buf.extend_from_slice(&pre);
                        let drawn = match renderer {
                            Renderer::Kitty => {
                                let (id, old) = if flip { (IMAGE_ID + 1, IMAGE_ID) } else { (IMAGE_ID, IMAGE_ID + 1) };
                                flip = !flip;
                                render::kitty_swap(&mut buf, &b64, id, old, img_cols, rows)
                            }
                            Renderer::Blocks => render::blocks(&mut buf, &b64, 1, cols, rows),
                        };
                        if drawn.is_err() {
                            buf.extend_from_slice(b"\x1b[0m");
                        }
                        buf.extend_from_slice(SYNC_OFF.as_bytes());
                    }
                    Out::Frame { .. } => {} // superseded by a newer frame
                }
            }
            let mut so = stdout().lock();
            if so.write_all(&buf).and_then(|_| so.flush()).is_err() {
                return;
            }
        }
    });
    (tx, handle)
}

struct Term;

impl Term {
    fn enter() -> R<Term> {
        terminal::enable_raw_mode()?;
        execute!(stdout(), terminal::EnterAlternateScreen, cursor::Hide, event::EnableMouseCapture)?;
        Ok(Term)
    }
}

impl Drop for Term {
    fn drop(&mut self) {
        let mut out = stdout();
        render::kitty_clear(&mut out);
        for id in chrome_ids() {
            let _ = write!(out, "\x1b_Ga=d,d=I,i={id},q=2\x1b\\");
        }
        let _ = execute!(out, event::DisableMouseCapture, cursor::Show, terminal::LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}

/// Chrome colors. On herdr desktop they come from its live theme (synced by the
/// `herdr-web-browser-theme` mod), so the view matches the panes around it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Palette {
    bar: Rgb,
    hover: Rgb,
    fg: Rgb,
    muted: Rgb,
    dim: Rgb,
    field: Rgb,
    field_fg: Rgb,
    selection: Rgb,
    ok: Rgb,
    err: Rgb,
}

impl Palette {
    /// herdr desktop's default dark look, for terminals without a synced theme.
    const DEFAULT: Palette = Palette {
        bar: (33, 37, 43),
        hover: (53, 57, 62),
        fg: (238, 238, 239),
        muted: (156, 158, 161),
        dim: (111, 113, 117),
        field: (52, 56, 61),
        field_fg: (238, 238, 239),
        selection: (50, 56, 68),
        ok: (97, 175, 239),
        err: (224, 108, 117),
    };

    fn from_theme(t: &Value) -> Palette {
        let d = Palette::DEFAULT;
        // The terminal's own background, so the bars blend into the pane's padding
        // instead of showing as darker strips; the field and hover sit slightly raised.
        let bar = color(t, "background", (0, 0, 0)).unwrap_or(d.bar);
        let field = color(t, "selected", bar).unwrap_or(d.field);
        Palette {
            bar,
            hover: color(t, "raised", bar).unwrap_or(d.hover),
            fg: color(t, "foreground", bar).unwrap_or(d.fg),
            muted: color(t, "muted", bar).unwrap_or(d.muted),
            dim: color(t, "dim", bar).unwrap_or(d.dim),
            field,
            field_fg: color(t, "foreground", field).unwrap_or(d.field_fg),
            selection: color(t, "selection", field).unwrap_or(d.selection),
            ok: color(t, "done", bar).unwrap_or(d.ok),
            err: color(t, "blocked", bar).unwrap_or(d.err),
        }
    }
}

/// `#rrggbb`, or `#rrggbbaa` blended over `base` (herdr's hover/border are translucent).
fn parse_hex(s: &str, base: Rgb) -> Option<Rgb> {
    let h = s.strip_prefix('#')?;
    let byte = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
    let (r, g, b) = (byte(0)?, byte(2)?, byte(4)?);
    let a = if h.len() == 8 { byte(6)? as f32 / 255.0 } else { 1.0 };
    let mix = |c: u8, b: u8| (c as f32 * a + b as f32 * (1.0 - a)).round() as u8;
    Some((mix(r, base.0), mix(g, base.1), mix(b, base.2)))
}

fn color(t: &Value, key: &str, base: Rgb) -> Option<Rgb> {
    parse_hex(t[key].as_str()?, base)
}

const ICON_BASE: u32 = 6000;
const ICON_HOVER_BASE: u32 = 6100;
const PILL_BASE: u32 = 6200;
const CAP_LEFT: u32 = 6300;
const CAP_RIGHT: u32 = 6301;

fn icon_index(icon: Icon) -> u32 {
    icons::ALL.iter().position(|x| *x == icon).unwrap_or(0) as u32
}

fn icon_id(icon: Icon, hovered: bool) -> u32 {
    if hovered { ICON_HOVER_BASE + icon_index(icon) } else { ICON_BASE + icon_index(icon) }
}

/// Every chrome image id, for deleting placements and cleanup.
fn chrome_ids() -> Vec<u32> {
    let mut ids: Vec<u32> = icons::ALL.iter().flat_map(|i| [icon_id(*i, false), icon_id(*i, true), PILL_BASE + icon_index(*i)]).collect();
    ids.extend([CAP_LEFT, CAP_RIGHT]);
    ids
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Act {
    Back,
    Forward,
    Reload,
    Field,
    TabPrev,
    TabNext,
    SendPage,
    SendSelection,
    Screenshot,
    Mode,
    Quit,
}

#[derive(Debug, Clone, Copy)]
struct Hit {
    row: u16,
    x0: u16,
    x1: u16,
    act: Act,
}

/// A chrome image slot: (col, row, image id, width in text columns).
type Slot = (u16, u16, u32, u16);

/// One chrome row being painted, with its clickable spans and icon slots.
struct Line {
    buf: Vec<u8>,
    row: u16,
    col: u16,
    hover: Option<(u16, u16)>,
    hits: Vec<Hit>,
    pal: Palette,
    /// Draw icons as Kitty images (else glyphs).
    images: bool,
    /// (col, row, icon, hovered) for each image slot.
    icons: Vec<Slot>,
}

impl Line {
    fn new(row: u16, hover: Option<(u16, u16)>, pal: Palette, images: bool) -> Line {
        let mut buf = Vec::new();
        let _ = queue!(buf, cursor::MoveTo(0, row));
        Line { buf, row, col: 0, hover, hits: Vec::new(), pal, images, icons: Vec::new() }
    }

    fn text(&mut self, s: &str, fg: Rgb, bg: Rgb) {
        let _ = write!(self.buf, "\x1b[38;2;{};{};{}m\x1b[48;2;{};{};{}m{s}", fg.0, fg.1, fg.2, bg.0, bg.1, bg.2);
        self.col += s.chars().count() as u16;
    }

    fn hovered(&self, x0: u16, x1: u16) -> bool {
        self.hover.is_some_and(|(c, r)| r == self.row && (x0..x1).contains(&c))
    }

    /// A borderless herdr-style icon button (4 columns) with a hover highlight;
    /// its name shows in the toolbar while hovered.
    fn button(&mut self, icon: Icon, act: Act) {
        let x0 = self.col;
        let hot = self.hovered(x0, x0 + 4);
        if self.images {
            // Hovered: one rounded pill image over all 4 cells; else the bare icon.
            if hot {
                self.icons.push((x0, self.row, PILL_BASE + icon_index(icon), 4));
            } else {
                self.icons.push((x0 + 1, self.row, icon_id(icon, false), 2));
            }
            self.text("    ", self.pal.fg, self.pal.bar);
            self.hits.push(Hit { row: self.row, x0, x1: self.col, act });
            return;
        }
        let (fg, bg) = if hot { (self.pal.fg, self.pal.hover) } else { (self.pal.muted, self.pal.bar) };
        self.text(" ", fg, bg);
        {
            self.text(&format!("{} ", icon.glyph()), fg, bg);
        }
        self.text(" ", fg, bg);
        self.hits.push(Hit { row: self.row, x0, x1: self.col, act });
    }

    fn gap(&mut self) {
        self.text(" ", self.pal.fg, self.pal.bar);
    }

    fn fill(&mut self, to: u16) {
        if to > self.col {
            self.text(&" ".repeat((to - self.col) as usize), self.pal.fg, self.pal.bar);
        }
    }

    fn finish(mut self) -> (Vec<u8>, Vec<Hit>, Vec<Slot>) {
        self.buf.extend_from_slice(b"\x1b[0m");
        (self.buf, self.hits, self.icons)
    }
}

fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(n.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

/// A running screencast; stopping joins it so its session's emulation can't
/// clear the next stream's (Chrome resets overrides when a session detaches).
struct Stream {
    target: String,
    size: StreamSize,
    stop: Arc<AtomicBool>,
    thread: std::thread::JoinHandle<()>,
}

impl Stream {
    fn stop(self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.thread.join();
    }
}

/// Header: a gap, back/forward/reload (4 columns each), a gap, then the address field.
const FIELD_START: u16 = 14;

struct View {
    cfg: Config,
    cells: Cells,
    renderer: Renderer,
    owner: Option<String>,
    /// Tab shown when there is no owner.
    tab: Option<String>,
    page: Option<page::Page>,
    streaming: Option<Stream>,
    tx: Sender<Msg>,
    out: Sender<Out>,
    frame: Option<(f64, f64, u16, u16)>,
    /// Last frame received, redrawn after the screen is cleared.
    last_frame: Option<(String, f64, f64)>,
    /// Pane size the screen was last laid out for.
    laid_out: (u16, u16),
    /// Which of the two Kitty image ids holds the frame on screen.
    pal: Palette,
    /// Icons uploaded as Kitty images for the current palette.
    icons_ready: bool,
    /// Modification time of the synced theme file in use.
    theme_at: Option<std::time::SystemTime>,
    /// Chrome bytes as last written; redraw only on change.
    drawn: Vec<u8>,
    hits: Vec<Hit>,
    hover: Option<(u16, u16)>,
    /// Wheel events gathered from one input batch: (x, y, deltaY).
    wheel: Option<(f64, f64, f64)>,
    url: String,
    title: String,
    tabs: (usize, usize),
    mode: Mode,
    message: String,
    message_at: Instant,
    url_input: Option<UrlEdit>,
    last_click: (Instant, u16, u16),
}

impl View {
    fn size() -> (u16, u16) {
        terminal::size().unwrap_or((80, 24))
    }

    fn area() -> (u16, u16) {
        let (c, r) = Self::size();
        (c, r.saturating_sub(2).max(1))
    }

    fn say(&mut self, msg: impl Into<String>) {
        self.message = msg.into();
        self.message_at = Instant::now();
    }

    /// (Re)attach to the tab this view shows: the owner's tab, else the chosen or focused one.
    fn attach(&mut self) -> R<()> {
        let tab_n = match (&self.owner, &self.tab) {
            (None, Some(id)) => {
                let mut c = Cdp::connect(&chrome::ensure(&self.cfg)?)?;
                page::page_targets(&mut c)?.iter().position(|t| &t.id == id).map(|i| i + 1)
            }
            _ => None,
        };
        let p = page::resolve(&self.cfg, tab_n, self.owner.is_some())?;
        self.tab = Some(p.tab.id.clone());
        self.page = Some(p);
        Ok(())
    }

    fn stream_size(&self) -> StreamSize {
        let (c, r) = Self::area();
        let viewport = ((c as f64 * self.cells.w / self.cells.dpr) as u32, (r as f64 * self.cells.h / self.cells.dpr) as u32);
        let max_frame = match self.renderer {
            Renderer::Kitty => ((viewport.0 as f64 * self.cells.dpr) as u32, (viewport.1 as f64 * self.cells.dpr) as u32),
            // Two pixels per cell; 4x that leaves room for box-averaging.
            Renderer::Blocks => (c as u32 * 4, r as u32 * 8),
        };
        StreamSize { viewport, dpr: self.cells.dpr, max_frame }
    }

    fn ensure_stream(&mut self) -> R<()> {
        if self.page.is_none() {
            self.attach()?;
        }
        let target = self.page.as_ref().map(|p| p.tab.id.clone()).unwrap_or_default();
        let size = self.stream_size();
        if let Some(st) = &self.streaming
            && st.target == target
            && st.size == size
        {
            return Ok(());
        }
        if let Some(st) = self.streaming.take() {
            st.stop();
        }
        let headless = state::load()?.mode == Some(Mode::Headless);
        if let Some(p) = self.page.as_mut() {
            let _ = p.activate(); // hidden (background) tabs don't paint, so no frames
        }
        let stop = Arc::new(AtomicBool::new(false));
        let (url, tx, s2, t2) = (chrome::ensure(&self.cfg)?, self.tx.clone(), stop.clone(), target.clone());
        let thread = std::thread::spawn(move || stream(url, t2, size, headless, tx, s2));
        self.streaming = Some(Stream { target, size, stop, thread });
        Ok(())
    }

    fn reset(&mut self) {
        if let Some(st) = self.streaming.take() {
            st.stop();
        }
        self.page = None;
    }

    /// The owner's mapping can change underneath us (`herdr-web-browser tab n`).
    fn owner_moved(&self) -> bool {
        let (Some(o), Some(p)) = (&self.owner, &self.page) else { return false };
        state::load().ok().and_then(|s| s.tabs.get(o).cloned()).is_some_and(|id| id != p.tab.id)
    }

    fn refresh_header(&mut self) {
        let Some(p) = self.page.as_mut() else { return };
        let info = p.info().unwrap_or(Value::Null);
        let tabs = page::page_targets(&mut p.cdp).unwrap_or_default();
        self.tabs = (tabs.iter().position(|t| t.id == p.tab.id).map_or(0, |i| i + 1), tabs.len());
        self.mode = state::load().ok().and_then(|s| s.mode).unwrap_or_default();
        self.url = info["url"].as_str().unwrap_or("").to_string();
        let title = info["title"].as_str().unwrap_or("").to_string();
        if title != self.title {
            // The pane header (herdr desktop and TUI) shows the terminal title.
            let clean: String = title.chars().filter(|c| !c.is_control()).take(80).collect();
            let name = if clean.is_empty() { "Browser".to_string() } else { format!("Browser · {clean}") };
            self.write(format!("\x1b]2;{name}\x07").into_bytes());
        }
        self.title = title;
    }

    fn tag(&self) -> String {
        let mode = match self.mode {
            Mode::Headed => "window",
            Mode::Headless => "headless",
        };
        format!(" {} · {mode} ", self.owner.as_deref().unwrap_or("focused tab"))
    }

    fn images(&self) -> bool {
        self.renderer == Renderer::Kitty && self.icons_ready
    }

    /// Address field: (first inner column, inner width).
    fn field(&self) -> (u16, usize) {
        let cols = Self::size().0 as usize;
        let tag = self.tag().chars().count();
        (FIELD_START + 1, cols.saturating_sub(FIELD_START as usize + tag + 3).max(1))
    }

    fn header(&self) -> (Vec<u8>, Vec<Hit>, Vec<Slot>) {
        let cols = Self::size().0;
        let p = self.pal;
        let mut l = Line::new(0, self.hover, p, self.images());
        l.gap();
        l.button(Icon::Back, Act::Back);
        l.button(Icon::Forward, Act::Forward);
        l.button(Icon::Reload, Act::Reload);
        l.gap();
        let (start, width) = self.field();
        let x0 = l.col;
        let rounded = l.images;
        if rounded {
            l.icons.push((l.col, 0, CAP_LEFT, 1));
            l.text(" ", p.field_fg, p.bar);
        } else {
            l.text(" ", p.field_fg, p.field);
        }
        match &self.url_input {
            Some(e) => {
                let first = e.scroll(width);
                for i in 0..width {
                    let idx = first + i;
                    let ch = e.buf.get(idx).copied().unwrap_or(' ');
                    let (fg, bg) = if idx == e.cur {
                        (p.field, p.field_fg) // block cursor
                    } else if e.all && idx < e.buf.len() {
                        (p.field_fg, p.selection)
                    } else {
                        (p.field_fg, p.field)
                    };
                    l.text(&ch.to_string(), fg, bg);
                }
            }
            None => {
                let shown = clip(&self.url, width);
                let pad = width - shown.chars().count();
                let fg = if self.hovered_field(start, width) { p.fg } else { p.muted };
                l.text(&format!("{shown}{}", " ".repeat(pad)), fg, p.field);
            }
        }
        if rounded {
            l.icons.push((l.col, 0, CAP_RIGHT, 1));
            l.text(" ", p.field_fg, p.bar);
        } else {
            l.text(" ", p.field_fg, p.field);
        }
        l.hits.push(Hit { row: 0, x0, x1: l.col, act: Act::Field });
        l.gap();
        l.text(&self.tag(), p.dim, p.bar);
        l.fill(cols);
        l.finish()
    }

    fn hovered_field(&self, start: u16, width: usize) -> bool {
        self.hover.is_some_and(|(c, r)| r == 0 && c >= start && ((c - start) as usize) < width)
    }

    fn toolbar(&self) -> (Vec<u8>, Vec<Hit>, Vec<Slot>) {
        let (cols, rows) = Self::size();
        let row = rows.saturating_sub(1);
        let p = self.pal;
        let tabs = format!(" {}/{} ", self.tabs.0, self.tabs.1);
        let mut l = Line::new(row, self.hover, p, self.images());
        l.gap();
        l.button(Icon::TabPrev, Act::TabPrev);
        l.text(&tabs, p.dim, p.bar);
        l.button(Icon::TabNext, Act::TabNext);
        l.text("  ", p.fg, p.bar);
        for (icon, act) in [(Icon::Send, Act::SendPage), (Icon::Selection, Act::SendSelection), (Icon::Camera, Act::Screenshot), (Icon::Window, Act::Mode)] {
            l.button(icon, act);
        }
        // Right side: the latest message for a few seconds, else the page title.
        let quit_at = cols.saturating_sub(5);
        let room = quit_at.saturating_sub(l.col + 2) as usize;
        let fresh = !self.message.is_empty() && self.message_at.elapsed() < Duration::from_secs(6);
        let hovered = self.hover.and_then(|(c, r)| self.hits.iter().find(|h| h.row == r && (h.x0..h.x1).contains(&c)));
        let (text, fg) = if let Some(h) = hovered {
            (self.describe(h.act).to_string(), p.fg)
        } else if fresh {
            let bad = self.message.starts_with("error") || self.message.starts_with("view lost");
            (self.message.clone(), if bad { p.err } else { p.ok })
        } else {
            (self.title.clone(), p.dim)
        };
        let text = clip(&text, room);
        l.fill(quit_at.saturating_sub(text.chars().count() as u16 + 1));
        l.text(&text, fg, p.bar);
        l.fill(quit_at);
        l.button(Icon::Close, Act::Quit);
        l.fill(cols);
        l.finish()
    }

    /// What a control does, shown while it's hovered.
    fn describe(&self, act: Act) -> &'static str {
        match act {
            Act::Back => "Back",
            Act::Forward => "Forward",
            Act::Reload => "Reload",
            Act::Field => "Edit address",
            Act::TabPrev => "Previous tab",
            Act::TabNext => "Next tab",
            Act::SendPage => "Send page to agent",
            Act::SendSelection => "Send selection to agent",
            Act::Screenshot => "Send screenshot to agent",
            Act::Mode if self.mode == Mode::Headed => "Run headless",
            Act::Mode => "Open in a window",
            Act::Quit => "Close view",
        }
    }

    /// Upload every icon, plain and hovered, once per palette.
    fn upload_icons(&mut self) -> R<()> {
        if self.renderer != Renderer::Kitty {
            return Ok(());
        }
        use base64::Engine;
        let (w, h) = ((2.0 * self.cells.w * 2.0).round() as u32, (self.cells.h * 2.0).round() as u32);
        let mut out = Vec::new();
        let b64 = |png: Vec<u8>| base64::engine::general_purpose::STANDARD.encode(png);
        let cell_w = (self.cells.w * 2.0).round() as u32;
        for icon in icons::ALL {
            for hot in [false, true] {
                let png = icon.png(if hot { self.pal.fg } else { self.pal.muted }, w, h, 0.62)?;
                render::kitty_upload(&mut out, &b64(png), icon_id(icon, hot))?;
            }
            let pill = icons::pill_png(icon, self.pal.fg, self.pal.hover, cell_w * 4, h)?;
            render::kitty_upload(&mut out, &b64(pill), PILL_BASE + icon_index(icon))?;
        }
        render::kitty_upload(&mut out, &b64(icons::cap_png(true, self.pal.field, cell_w, h)?), CAP_LEFT)?;
        render::kitty_upload(&mut out, &b64(icons::cap_png(false, self.pal.field, cell_w, h)?), CAP_RIGHT)?;
        self.write(out);
        self.icons_ready = true;
        self.drawn.clear();
        Ok(())
    }

    fn draw_chrome(&mut self) -> R<()> {
        let (h, mut hits, mut slots) = self.header();
        let (t, th, ts) = self.toolbar();
        hits.extend(th);
        slots.extend(ts);
        self.hits = hits;
        let mut out = Vec::with_capacity(h.len() + t.len() + 64 * slots.len() + 16);
        out.extend_from_slice(SYNC_ON.as_bytes());
        out.extend_from_slice(&h);
        out.extend_from_slice(&t);
        if !slots.is_empty() {
            // Re-place every icon: drop old placements (keeping the images), put new ones.
            for id in chrome_ids() {
                write!(out, "\x1b_Ga=d,d=i,i={id},q=2\x1b\\")?;
            }
            for (n, (col, row, id, cols)) in slots.iter().enumerate() {
                queue!(out, cursor::MoveTo(*col, *row))?;
                write!(out, "\x1b_Ga=p,i={id},p={},c={},r=1,C=1,q=2\x1b\\", n + 1, self.cells.img_cols(*cols))?;
            }
        }
        out.extend_from_slice(SYNC_OFF.as_bytes());
        if out == self.drawn {
            return Ok(());
        }
        self.write(out.clone());
        self.drawn = out;
        Ok(())
    }

    /// Pick up a changed herdr desktop theme (light/dark switch, theme change).
    fn reload_theme(&mut self) -> R<()> {
        let path = state::state_dir().join("theme.json");
        let Ok(modified) = std::fs::metadata(&path).and_then(|m| m.modified()) else { return Ok(()) };
        if self.theme_at == Some(modified) {
            return Ok(());
        }
        self.theme_at = Some(modified);
        let t: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
        let pal = Palette::from_theme(&t);
        if pal != self.pal || !self.icons_ready {
            self.pal = pal;
            self.upload_icons()?;
        }
        Ok(())
    }

    /// Re-lay everything out when the pane size changed. Panes are often resized right
    /// after they open, sometimes before a Resize event can be read, so poll for it.
    fn relayout_if_resized(&mut self) -> R<()> {
        let size = Self::size();
        if size == self.laid_out {
            return Ok(());
        }
        self.laid_out = size;
        let mut clear = Vec::new();
        render::kitty_clear(&mut clear);
        queue!(clear, terminal::Clear(terminal::ClearType::All))?;
        self.write(clear);
        self.frame = None;
        self.drawn.clear();
        // Clearing the screen frees images nothing shows, icons included: upload them again.
        if self.icons_ready {
            self.upload_icons()?;
        }
        self.draw_chrome()?;
        if let Some((data, w, h)) = self.last_frame.clone() {
            self.draw_frame(&data, w, h)?;
        }
        Ok(())
    }

    fn write(&self, bytes: Vec<u8>) {
        let _ = self.out.send(Out::Bytes(bytes));
    }

    fn draw_frame(&mut self, data: &str, w: f64, h: f64) -> R<()> {
        let (cols, rows) = Self::area();
        let (c, r) = render::fit(w, h, cols, rows, self.cells.aspect());
        let mut pre = Vec::new();
        if self.frame.is_some_and(|(_, _, pc, pr)| (pc, pr) != (c, r)) {
            for row in 1..=rows {
                queue!(pre, cursor::MoveTo(0, row), terminal::Clear(terminal::ClearType::CurrentLine))?;
            }
        }
        queue!(pre, cursor::MoveTo(0, 1))?;
        let _ = self.out.send(Out::Frame {
            pre,
            b64: data.to_string(),
            cols: c,
            img_cols: self.cells.img_cols(c),
            rows: r,
            renderer: self.renderer,
        });
        self.frame = Some((w, h, c, r));
        if self.last_frame.as_ref().is_none_or(|(d, _, _)| d != data) {
            self.last_frame = Some((data.to_string(), w, h));
        }
        Ok(())
    }

    /// Terminal cell -> page CSS px, if the cell is on the frame.
    fn to_page(&self, col: u16, row: u16) -> Option<(f64, f64)> {
        let (w, h, c, r) = self.frame?;
        let y = row.checked_sub(1)?;
        (col < c && y < r).then(|| ((col as f64 + 0.5) / c as f64 * w, (y as f64 + 0.5) / r as f64 * h))
    }

    fn navigate(&mut self, url: &str) {
        if let Some(p) = self.page.as_mut() {
            match p.navigate(url) {
                Ok(_) => self.message.clear(),
                Err(e) => self.say(format!("error: {e}")),
            }
        }
        self.refresh_header();
    }

    fn history(&mut self, delta: i32) {
        if let Some(p) = self.page.as_mut() {
            let r = match delta {
                0 => p.reload(),
                d if d < 0 => p.back(),
                _ => p.forward(),
            };
            if let Err(e) = r {
                self.say(format!("error: {e}"));
            }
        }
        self.refresh_header();
    }

    fn mouse(&mut self, kind: &str, x: f64, y: f64, buttons: u8, clicks: u8, wheel: f64) -> R<()> {
        let Some(p) = self.page.as_mut() else { return Ok(()) };
        let mut params = json!({ "type": kind, "x": x, "y": y, "button": if buttons > 0 || kind != "mouseMoved" { "left" } else { "none" }, "buttons": buttons, "clickCount": clicks });
        if kind == "mouseWheel" {
            params = json!({ "type": kind, "x": x, "y": y, "deltaX": 0, "deltaY": wheel });
        }
        let sid = p.session.clone();
        p.cdp.call("Input.dispatchMouseEvent", params, Some(&sid))?;
        Ok(())
    }

    fn key(&mut self, k: KeyEvent) -> R<()> {
        let Some(p) = self.page.as_mut() else { return Ok(()) };
        let sid = p.session.clone();
        let named = |key: &str, code: &str, vk: u32| json!({ "key": key, "code": code, "windowsVirtualKeyCode": vk });
        let base = match k.code {
            KeyCode::Enter => json!({ "key": "Enter", "code": "Enter", "windowsVirtualKeyCode": 13, "text": "\r" }),
            KeyCode::Backspace => named("Backspace", "Backspace", 8),
            KeyCode::Tab => named("Tab", "Tab", 9),
            KeyCode::Esc => named("Escape", "Escape", 27),
            KeyCode::Left => named("ArrowLeft", "ArrowLeft", 37),
            KeyCode::Up => named("ArrowUp", "ArrowUp", 38),
            KeyCode::Right => named("ArrowRight", "ArrowRight", 39),
            KeyCode::Down => named("ArrowDown", "ArrowDown", 40),
            KeyCode::Delete => named("Delete", "Delete", 46),
            KeyCode::Home => named("Home", "Home", 36),
            KeyCode::End => named("End", "End", 35),
            KeyCode::PageUp => named("PageUp", "PageUp", 33),
            KeyCode::PageDown => named("PageDown", "PageDown", 34),
            KeyCode::Char(c) => {
                let s = c.to_string();
                json!({ "key": s, "text": s, "unmodifiedText": s })
            }
            _ => return Ok(()),
        };
        let mut mods = 0;
        if k.modifiers.contains(KeyModifiers::ALT) {
            mods |= 1;
        }
        if k.modifiers.contains(KeyModifiers::SHIFT) {
            mods |= 8;
        }
        for kind in ["keyDown", "keyUp"] {
            let mut ev = base.clone();
            ev["type"] = json!(kind);
            ev["modifiers"] = json!(mods);
            if kind == "keyUp" {
                ev.as_object_mut().map(|o| o.remove("text"));
            }
            p.cdp.call("Input.dispatchKeyEvent", ev, Some(&sid))?;
        }
        Ok(())
    }

    fn switch_tab(&mut self, delta: isize) -> R<()> {
        let mut c = Cdp::connect(&chrome::ensure(&self.cfg)?)?;
        let tabs = page::page_targets(&mut c)?;
        if tabs.is_empty() {
            return Ok(());
        }
        let cur = self.page.as_ref().and_then(|p| tabs.iter().position(|t| t.id == p.tab.id)).unwrap_or(0) as isize;
        let next = tabs[(cur + delta).rem_euclid(tabs.len() as isize) as usize].id.clone();
        match &self.owner {
            Some(o) => state::update(|s| s.tabs.insert(o.clone(), next.clone())).map(drop)?,
            None => self.tab = Some(next),
        }
        self.reset();
        Ok(())
    }

    fn send(&mut self, kind: SendKind) {
        let label = match kind {
            SendKind::Page => "page",
            SendKind::Selection => "selection",
            SendKind::Screenshot => "screenshot",
        };
        self.say(format!("sending {label}…"));
        let _ = self.draw_chrome();
        // A view opened beside an agent sends to that agent (never to a plain shell).
        let host = crate::host::host();
        let agent = self.owner.clone().filter(|o| {
            crate::host::agents(host).is_ok_and(|(list, _)| list.iter().any(|a| &a.id == o || &a.pane_id == o))
        });
        // Without an owner the view may show any tab; send that one.
        let tab = match (&self.owner, &self.tab) {
            (None, Some(id)) => Cdp::connect(&chrome::ensure(&self.cfg).unwrap_or_default())
                .and_then(|mut c| page::page_targets(&mut c))
                .ok()
                .and_then(|tabs| tabs.iter().position(|t| &t.id == id).map(|i| i + 1)),
            _ => None,
        };
        match agent::send_from_browser(kind, tab, agent.as_deref(), &self.cfg) {
            Ok(t) => self.say(format!("✓ sent {label} to {t}")),
            Err(e) => self.say(format!("error: {e}")),
        }
    }

    fn toggle_mode(&mut self) -> R<()> {
        self.say("switching mode…");
        self.draw_chrome()?;
        self.reset();
        let next = match state::load()?.mode.unwrap_or_default() {
            Mode::Headed => Mode::Headless,
            Mode::Headless => Mode::Headed,
        };
        match chrome::switch_mode(next, &self.cfg) {
            Ok(()) => self.say(format!("✓ now {}", if next == Mode::Headless { "headless" } else { "in a window" })),
            Err(e) => self.say(format!("error: {e}")),
        }
        Ok(())
    }

    /// Returns false to quit.
    fn act(&mut self, act: Act, col: u16) -> R<bool> {
        match act {
            Act::Back => self.history(-1),
            Act::Forward => self.history(1),
            Act::Reload => self.history(0),
            Act::Field => {
                let (start, width) = self.field();
                match self.url_input.as_mut() {
                    Some(e) => e.click(col.saturating_sub(start) as usize, width),
                    None => self.url_input = Some(UrlEdit::new(&self.url)),
                }
            }
            Act::TabPrev => self.switch_tab(-1)?,
            Act::TabNext => self.switch_tab(1)?,
            Act::SendPage => self.send(SendKind::Page),
            Act::SendSelection => self.send(SendKind::Selection),
            Act::Screenshot => self.send(SendKind::Screenshot),
            Act::Mode => self.toggle_mode()?,
            Act::Quit => return Ok(false),
        }
        Ok(true)
    }

    /// Returns false to quit.
    fn on_key(&mut self, k: KeyEvent) -> R<bool> {
        if let Some(edit) = self.url_input.as_mut() {
            match edit.key(k) {
                Edit::Editing => {}
                Edit::Cancel => self.url_input = None,
                Edit::Go(url) => {
                    self.url_input = None;
                    if !url.trim().is_empty() {
                        self.navigate(url.trim());
                    }
                }
            }
            return Ok(true);
        }
        if k.modifiers.contains(KeyModifiers::CONTROL)
            && let KeyCode::Char(c) = k.code
        {
            let act = match c {
                'q' => Act::Quit,
                'l' => Act::Field,
                'r' => Act::Reload,
                'b' => Act::Back,
                'f' => Act::Forward,
                'n' => Act::TabNext,
                'p' => Act::TabPrev,
                'g' => Act::SendPage,
                's' => Act::SendSelection,
                'x' => Act::Screenshot,
                'e' => Act::Mode,
                _ => {
                    self.key(k)?;
                    return Ok(true);
                }
            };
            return self.act(act, 0);
        }
        self.key(k)?;
        Ok(true)
    }

    /// Returns false to quit.
    fn on_mouse(&mut self, m: event::MouseEvent) -> R<bool> {
        let rows = Self::size().1;
        let on_chrome = m.row == 0 || m.row + 1 == rows;
        if matches!(m.kind, MouseEventKind::Moved | MouseEventKind::Drag(_)) {
            self.hover = on_chrome.then_some((m.column, m.row));
        }
        if on_chrome {
            if m.kind == MouseEventKind::Down(MouseButton::Left)
                && let Some(h) = self.hits.iter().find(|h| h.row == m.row && (h.x0..h.x1).contains(&m.column)).copied()
            {
                return self.act(h.act, m.column);
            }
            return Ok(true);
        }
        if matches!(m.kind, MouseEventKind::Down(_)) {
            self.url_input = None; // clicking the page leaves the address bar
        }
        let Some((x, y)) = self.to_page(m.column, m.row) else { return Ok(true) };
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let (t, c, r) = self.last_click;
                let clicks = if t.elapsed() < Duration::from_millis(400) && (c, r) == (m.column, m.row) { 2 } else { 1 };
                self.last_click = (Instant::now(), m.column, m.row);
                self.mouse("mouseMoved", x, y, 0, 0, 0.0)?;
                self.mouse("mousePressed", x, y, 1, clicks, 0.0)?;
            }
            MouseEventKind::Up(MouseButton::Left) => self.mouse("mouseReleased", x, y, 0, 1, 0.0)?,
            MouseEventKind::Drag(MouseButton::Left) => self.mouse("mouseMoved", x, y, 1, 0, 0.0)?,
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                let d = if m.kind == MouseEventKind::ScrollDown { 120.0 } else { -120.0 };
                let sum = self.wheel.map_or(0.0, |w| w.2);
                self.wheel = Some((x, y, sum + d));
            }
            _ => {}
        }
        Ok(true)
    }
}

pub fn run(cfg: Config) -> R<()> {
    let (tx, rx): (Sender<Msg>, Receiver<Msg>) = mpsc::channel();
    let _term = Term::enter()?;
    let (cells, renderer) = render::detect(&cfg);
    let (out, writer) = spawn_writer();
    let mut v = View {
        cfg,
        cells,
        renderer,
        // Only an explicit --owner: the view's own terminal isn't an agent worth a tab.
        owner: page::OWNER.get().cloned(),
        tab: None,
        page: None,
        streaming: None,
        tx,
        out,
        frame: None,
        last_frame: None,
        laid_out: (0, 0),
        pal: Palette::DEFAULT,
        icons_ready: false,
        theme_at: None,
        drawn: Vec::new(),
        hits: Vec::new(),
        hover: None,
        wheel: None,
        url: String::new(),
        title: "starting browser…".into(),
        tabs: (0, 0),
        mode: state::load().ok().and_then(|s| s.mode).unwrap_or_default(),
        message: String::new(),
        message_at: Instant::now(),
        url_input: None,
        last_click: (Instant::now(), u16::MAX, u16::MAX),
    };
    if v.renderer == Renderer::Kitty {
        let _ = v.upload_icons();
    }
    let result = event_loop(&mut v, &rx);
    // Let the writer finish before the terminal is restored, so output doesn't interleave.
    drop(v);
    let _ = writer.join();
    result
}

fn event_loop(v: &mut View, rx: &Receiver<Msg>) -> R<()> {
    if std::env::var_os("HC_TERMINAL_ID").is_some() {
        // Ask herdr desktop for its theme; the mod writes theme.json, picked up below.
        std::thread::spawn(|| drop(crate::host::run("herdr-app", &["do", "mod.open", "--name", "herdr-web-browser-theme"])));
    }
    let mut last_tick = Instant::now() - Duration::from_secs(10);
    let mut last_gc = Instant::now();
    loop {
        v.relayout_if_resized()?;
        if last_tick.elapsed() >= Duration::from_secs(1) {
            last_tick = Instant::now();
            if let Err(e) = v.reload_theme() {
                v.say(format!("error: theme: {e}"));
            }
            if v.owner_moved() {
                v.reset();
            }
            match v.ensure_stream() {
                Ok(()) => v.refresh_header(),
                Err(e) => {
                    v.reset();
                    v.say(format!("view lost: {e}; reconnecting"));
                }
            }
            if last_gc.elapsed() >= Duration::from_secs(15) {
                last_gc = Instant::now();
                if let Some(p) = v.page.as_mut() {
                    let _ = page::gc(&mut p.cdp);
                }
            }
            v.draw_chrome()?;
        }
        let mut latest = None;
        while let Ok(m) = rx.try_recv() {
            match m {
                Msg::Frame { data, w, h } => latest = Some((data, w, h)),
                Msg::Gone(e) => {
                    v.reset();
                    v.say(format!("view lost: {e}; reattaching"));
                }
            }
        }
        if let Some((data, w, h)) = latest
            && let Err(e) = v.draw_frame(&data, w, h)
        {
            v.say(format!("error: {e}"));
        }
        if !event::poll(Duration::from_millis(30))? {
            continue;
        }
        // Take everything already queued, so a fast scroll becomes one wheel event.
        loop {
            let res = match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => v.on_key(k),
                Event::Mouse(m) => v.on_mouse(m),
                Event::Resize(..) => {
                    // relayout_if_resized repaints; the tick restarts the stream at the new size.
                    last_tick = Instant::now() - Duration::from_secs(10);
                    Ok(true)
                }
                _ => Ok(true),
            };
            match res {
                Ok(false) => return Ok(()),
                Ok(true) => {}
                Err(e) => {
                    v.say(format!("error: {e}"));
                    v.reset();
                }
            }
            if !event::poll(Duration::ZERO)? {
                break;
            }
        }
        if let Some((x, y, d)) = v.wheel.take()
            && let Err(e) = v.mouse("mouseWheel", x, y, 0, 0, d)
        {
            v.say(format!("error: {e}"));
        }
        v.draw_chrome()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn typing_replaces_the_selected_url() {
        let mut e = UrlEdit::new("https://example.com/");
        e.key(k(KeyCode::Char('x')));
        assert_eq!(e.key(k(KeyCode::Enter)), Edit::Go("x".into()));
    }

    #[test]
    fn arrows_drop_the_selection_and_edit_in_place() {
        let mut e = UrlEdit::new("https://a.com/");
        e.key(k(KeyCode::Left));
        e.key(k(KeyCode::Backspace));
        e.key(k(KeyCode::Char('x')));
        assert_eq!(e.text(), "https://a.cox/");
    }

    #[test]
    fn ctrl_w_deletes_one_url_segment() {
        let mut e = UrlEdit::new("https://a.com/path");
        e.key(k(KeyCode::End));
        e.key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
        assert_eq!(e.text(), "https://a.com/");
    }

    #[test]
    fn long_urls_scroll_to_keep_the_cursor_visible() {
        let mut e = UrlEdit::new("abcdefghij");
        e.key(k(KeyCode::End));
        assert_eq!(e.scroll(4), 7); // shows "hij" plus the cursor cell
        e.click(0, 4);
        assert_eq!(e.cur, 7);
    }

    #[test]
    fn buttons_record_their_click_spans() {
        let mut l = Line::new(0, None, Palette::DEFAULT, true);
        l.gap();
        l.button(Icon::Back, Act::Back);
        l.button(Icon::Send, Act::SendPage);
        let (_, hits, slots) = l.finish();
        assert_eq!((hits[0].x0, hits[0].x1, hits[0].act), (1, 5, Act::Back));
        assert_eq!((hits[1].x0, hits[1].x1), (5, 9));
        assert_eq!(slots.iter().map(|s| (s.0, s.3)).collect::<Vec<_>>(), [(2, 2), (6, 2)]);
    }

    #[test]
    fn translucent_theme_colors_blend_over_their_base() {
        assert_eq!(parse_hex("#ffffff00", (10, 20, 30)), Some((10, 20, 30)));
        assert_eq!(parse_hex("#ff0000", (0, 0, 0)), Some((255, 0, 0)));
        assert_eq!(parse_hex("#ffffff80", (0, 0, 0)), Some((128, 128, 128)));
    }

    #[test]
    fn clip_marks_truncation() {
        assert_eq!(clip("abcdef", 4), "abc…");
        assert_eq!(clip("abc", 4), "abc");
    }
}
