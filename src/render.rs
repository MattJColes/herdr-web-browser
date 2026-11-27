//! Terminal graphics for the live view: Kitty images where the terminal supports
//! them (herdr desktop; the herdr TUI inside Ghostty, kitty or WezTerm), else
//! truecolor text: octants (2x4 pixels a cell) on Windows Terminal, half blocks
//! (1x2) anywhere else.

use crate::R;
use crate::state::Config;
use base64::Engine;
#[cfg(unix)]
use std::io::Read;
use std::io::{Write, stdout};
use std::time::Duration;

pub const IMAGE_ID: u32 = 4711;
/// Synchronized output (DEC 2026): the terminal shows an update all at once.
pub const SYNC_ON: &str = "\x1b[?2026h";
pub const SYNC_OFF: &str = "\x1b[?2026l";

/// Cell size in device px and the device pixel ratio guessed from it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cells {
    pub w: f64,
    pub h: f64,
    pub dpr: f64,
    /// Width of one image column. herdr desktop places Kitty images on a whole-pixel
    /// grid (8 px) while its text cells are fractional (8.4 px), so images come out narrow.
    pub img_w: f64,
}

impl Cells {
    /// Used when the terminal doesn't answer the size query.
    pub const FALLBACK: Cells = Cells { w: 9.0, h: 18.0, dpr: 1.0, img_w: 9.0 };

    pub fn from_px(w: f64, h: f64) -> Cells {
        // Logical cells are ~14-22 px tall; twice that means a 2x (Retina) display.
        Cells { w, h, dpr: if h >= 28.0 { 2.0 } else { 1.0 }, img_w: w }
    }

    /// Image columns that cover `cols` text columns.
    pub fn img_cols(&self, cols: u16) -> u16 {
        (cols as f64 * self.w / self.img_w).round() as u16
    }

    pub fn aspect(&self) -> f64 {
        self.h / self.w
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Renderer {
    Kitty,
    Blocks,
    /// Unicode 16 octants: 2x4 pixels a cell in two colours. Windows Terminal draws them
    /// itself (no font needed); herdr passes no image protocol through to it.
    Octants,
}

/// Parse `CSI 6 ; height ; width t` (reply to `CSI 16 t`) out of raw terminal input.
pub fn parse_cell_reply(buf: &[u8]) -> Option<(f64, f64)> {
    let s = String::from_utf8_lossy(buf);
    let start = s.find("\x1b[6;")? + 4;
    let rest = &s[start..];
    let body = &rest[..rest.find('t')?];
    let mut it = body.split(';').map(|n| n.parse::<f64>().ok());
    let (h, w) = (it.next()??, it.next()??);
    (w > 0.0 && h > 0.0).then_some((w, h))
}

/// A Kitty terminal answers the probe image query with `ESC _ G i=31;OK`.
pub fn parse_kitty_reply(buf: &[u8]) -> bool {
    String::from_utf8_lossy(buf).contains("\x1b_Gi=31;OK")
}

/// herdr desktop draws terminals itself while herdr-core answers `CSI 16 t` with a fixed
/// 10x20, so derive cells from the app's font size (cells measured 8.4x22 px at 14 pt),
/// in device pixels so Retina screens get 2x frames rather than 1x ones stretched.
fn desktop_cells(cfg: &Config) -> Option<Cells> {
    let st = crate::host::run("herdr-app", &["status"]).ok()?;
    // `font_size` is what the window draws, zoom included: font 14 at zoom 1.4 reports 19.6.
    let fs = st["font_size"].as_f64()?;
    let settings = std::fs::read_to_string(crate::desktop_home().join("settings.json")).ok();
    let setting = |key: &str| {
        settings
            .as_deref()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .and_then(|v| v[key].as_f64())
            .unwrap_or(1.0)
            .clamp(0.8, 2.0)
    };
    let scale = cfg.scale.filter(|s| (0.5..=4.0).contains(s)).or_else(screen_scale).unwrap_or(1.0);
    let (line_height, cell_width) = (setting("terminal_line_height"), setting("terminal_cell_width"));
    let font = settings.as_deref().and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok()).and_then(|v| v["font"].as_str().map(String::from));
    if let Some((advance, line)) = font.as_deref().and_then(|f| font_metrics(f, fs)) {
        return Some(measured_cell_size(advance, line, line_height, cell_width, scale));
    }
    Some(desktop_cell_size(fs, line_height, cell_width, scale))
}

/// herdr desktop's cell from the configured font's own metrics: its advance and its line
/// height (ascender + descender + leading), each times herdr's multiplier and the screen's
/// scale, rounded to whole device px as herdr draws them. Measured on macOS at 2x: FiraCode
/// Nerd Font Mono 14 (8.615 x 17.23 pt) draws 17 x 34 px cells, Menlo 14 (8.43 x 16.30 pt)
/// 17 x 33, JetBrains Mono 14 (8.4 x 18.48 pt) 17 x 37.
pub fn measured_cell_size(advance: f64, line: f64, line_height: f64, cell_width: f64, scale: f64) -> Cells {
    let (w, h) = ((advance * cell_width * scale).round(), (line * line_height * scale).round());
    Cells { w, h, dpr: scale, img_w: w }
}

/// `font`'s advance and line height in points at `size`, from AppKit. None off macOS, or when
/// the family isn't installed: herdr's bundled JetBrains Mono isn't, and falls back to
/// `desktop_cell_size`.
fn font_metrics(font: &str, size: f64) -> Option<(f64, f64)> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let js = format!(
        "ObjC.import('AppKit'); var f = $.NSFontManager.sharedFontManager.fontWithFamilyTraitsWeightSize({}, 0, 5, {size}); \
         f.isNil() ? '' : [f.advancementForGlyph(f.glyphWithName('zero')).width, f.ascender - f.descender + f.leading].join(' ')",
        serde_json::to_string(font).ok()?
    );
    let out = std::process::Command::new("osascript").args(["-l", "JavaScript", "-e", &js]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut it = text.split_whitespace().map(|n| n.parse::<f64>().ok());
    let (advance, line) = (it.next()??, it.next()??);
    (advance > 0.0 && line > 0.0).then_some((advance, line))
}

/// herdr desktop's terminal cell in device px, from its font size (zoom included), its
/// `terminal_line_height` / `terminal_cell_width` multipliers and the screen's scale.
/// The device pixel ratio is the screen's scale, never guessed from the cell size: a zoomed
/// or large font has tall cells on a 1x screen too.
/// Uses herdr's default JetBrains Mono (0.6 em advance, 1.32 em line), rounded to whole device
/// px: 8 x 18 px at 14 px on a 1x Linux screen, 17 x 37 px at 14 pt on a 2x Mac.
pub fn desktop_cell_size(font_size: f64, line_height: f64, cell_width: f64, scale: f64) -> Cells {
    measured_cell_size(font_size * 0.6, font_size * 1.32, line_height, cell_width, scale)
}

/// The screen's backing scale (2 on Retina); herdr desktop only reports logical sizes.
/// ponytail: macOS's main screen, not necessarily the one herdr's window is on.
fn screen_scale() -> Option<f64> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let js = "ObjC.import('AppKit'); $.NSScreen.mainScreen.backingScaleFactor";
    let out = std::process::Command::new("osascript").args(["-l", "JavaScript", "-e", js]).output().ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok().filter(|s: &f64| (1.0..=4.0).contains(s))
}

/// Read terminal replies until DA1 arrives or `timeout` passes, without leaving a reader
/// behind: a thread blocked on stdin would later swallow the user's keys and clicks.
#[cfg(unix)]
fn read_replies(timeout: Duration) -> Vec<u8> {
    let deadline = std::time::Instant::now() + timeout;
    let mut buf = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return buf;
        }
        let mut pfd = libc::pollfd { fd: libc::STDIN_FILENO, events: libc::POLLIN, revents: 0 };
        // SAFETY: one valid pollfd for the duration of the call.
        let ready = unsafe { libc::poll(&mut pfd, 1, left.as_millis().min(i32::MAX as u128) as i32) };
        if ready <= 0 {
            return buf;
        }
        let mut chunk = [0u8; 256];
        let n = match std::io::stdin().lock().read(&mut chunk) {
            Ok(n) if n > 0 => n,
            _ => return buf,
        };
        buf.extend_from_slice(&chunk[..n]);
        let s = String::from_utf8_lossy(&buf);
        if let Some(i) = s.find("\x1b[?")
            && s[i..].contains('c')
        {
            return buf; // DA1 reply: ESC [ ? ... c
        }
    }
}

/// Windows has no poll on a console stdin, and a pending ReadConsole would eat keys later,
/// so don't probe: herdr desktop sizes cells from `herdr-app status` instead, and
/// `view_renderer = "kitty"` forces the Kitty renderer elsewhere.
#[cfg(not(unix))]
fn read_replies(_timeout: Duration) -> Vec<u8> {
    Vec::new()
}

/// Ask for cell size and Kitty support in one round trip. DA1 (`CSI c`) goes last because
/// every terminal answers it, so the read ends as soon as the others would have arrived.
fn query_terminal() -> (Cells, bool) {
    let mut out = stdout();
    let probe = "\x1b[16t\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\\x1b[c";
    if write!(out, "{probe}").and_then(|_| out.flush()).is_err() {
        return (Cells::FALLBACK, false);
    }
    let buf = read_replies(Duration::from_millis(500));
    (parse_cell_reply(&buf).map(|(w, h)| Cells::from_px(w, h)).unwrap_or(Cells::FALLBACK), parse_kitty_reply(&buf))
}

/// Cell metrics and the renderer to use; `view_renderer` / `cell_aspect` override.
pub fn detect(cfg: &Config) -> (Cells, Renderer) {
    let desktop = std::env::var_os("HC_TERMINAL_ID").is_some();
    let (mut cells, kitty) = match desktop.then(|| desktop_cells(cfg)).flatten() {
        Some(c) => (c, true),
        None => query_terminal(),
    };
    if let Some(k) = cfg.cell_aspect.filter(|k| *k > 0.5 && *k < 5.0) {
        cells.h = cells.w * k;
    }
    // herdr desktop on Windows on ARM (x64 under emulation, herdr-core 0.1.21) runs its
    // terminals on Windows' own conhost, which drops Kitty images, and its font has no
    // octants: half blocks are what shows there.
    let desktop_without_images = desktop && cfg!(all(windows, target_arch = "aarch64"));
    let renderer = match cfg.view_renderer.as_deref() {
        Some("kitty") => Renderer::Kitty,
        Some("blocks") => Renderer::Blocks,
        Some("octants") => Renderer::Octants,
        _ if desktop_without_images => Renderer::Blocks,
        _ if kitty => Renderer::Kitty,
        // Windows Terminal (the Windows 11 default) draws octants; elsewhere a font may not.
        _ if cfg!(windows) => Renderer::Octants,
        _ => Renderer::Blocks,
    };
    (cells, renderer)
}

/// Cells (cols, rows) that fit a `w`×`h` frame into `cols`×`rows`, keeping aspect;
/// `k` is cell height / cell width.
pub fn fit(w: f64, h: f64, cols: u16, rows: u16, k: f64) -> (u16, u16) {
    let want_rows = (cols as f64 * h / (w * k)).round().max(1.0);
    if want_rows <= rows as f64 { (cols, want_rows as u16) } else { (((rows as f64 * k * w / h).round().max(1.0) as u16).min(cols), rows) }
}

/// Kitty graphics, double-buffered: upload the PNG (already base64) as image `id`, place it
/// at the cursor, then delete the previous image `old`. Replacing one id in place blanks
/// the picture while the new one uploads and decodes, which flickers on every frame.
pub fn kitty_swap(out: &mut Vec<u8>, b64: &str, id: u32, old: u32, cols: u16, rows: u16) -> R<()> {
    let chunks: Vec<&str> = b64.as_bytes().chunks(4096).map(|c| std::str::from_utf8(c).unwrap_or("")).collect();
    for (i, chunk) in chunks.iter().enumerate() {
        let more = u8::from(i + 1 < chunks.len());
        if i == 0 {
            write!(out, "\x1b_Ga=t,f=100,i={id},q=2,m={more};{chunk}\x1b\\")?;
        } else {
            write!(out, "\x1b_Gm={more};{chunk}\x1b\\")?;
        }
    }
    write!(out, "\x1b_Ga=p,i={id},p=1,q=2,C=1,c={cols},r={rows}\x1b\\")?;
    write!(out, "\x1b_Ga=d,d=I,i={old},q=2\x1b\\")?;
    Ok(())
}

/// The smallest box (x0, y0, x1, y1), exclusive ends, holding every pixel that differs
/// between two same-size RGBA frames; None when they're identical.
pub fn dirty_rect(a: &[u8], b: &[u8], w: usize, h: usize) -> Option<(usize, usize, usize, usize)> {
    let row = w * 4;
    let differs = |y: usize| a[y * row..(y + 1) * row] != b[y * row..(y + 1) * row];
    let y0 = (0..h).find(|&y| differs(y))?;
    let y1 = (y0..h).rev().find(|&y| differs(y))? + 1;
    let (mut x0, mut x1) = (w, 0);
    for y in y0..y1 {
        let (ra, rb) = (&a[y * row..(y + 1) * row], &b[y * row..(y + 1) * row]);
        if let Some(x) = (0..x0).find(|&x| ra[x * 4..x * 4 + 4] != rb[x * 4..x * 4 + 4]) {
            x0 = x;
        }
        if let Some(x) = (x1..w).rev().find(|&x| ra[x * 4..x * 4 + 4] != rb[x * 4..x * 4 + 4]) {
            x1 = x + 1;
        }
    }
    Some((x0, y0, x1, y1))
}

/// Write one Kitty command whose payload is zlib'd `rgba`, chunked as the protocol requires.
fn kitty_rgba(out: &mut Vec<u8>, keys: &str, rgba: &[u8]) -> R<()> {
    let z = miniz_oxide::deflate::compress_to_vec_zlib(rgba, 1);
    let b64 = base64::engine::general_purpose::STANDARD.encode(z);
    let chunks: Vec<&[u8]> = b64.as_bytes().chunks(4096).collect();
    for (i, chunk) in chunks.iter().enumerate() {
        let more = u8::from(i + 1 < chunks.len());
        let head = if i == 0 { format!("{keys},f=32,o=z,q=2,m={more}") } else { format!("m={more}") };
        write!(out, "\x1b_G{head};")?;
        out.extend_from_slice(chunk);
        out.extend_from_slice(b"\x1b\\");
    }
    Ok(())
}

/// The frame on screen, for diffing the next one against.
struct Shown {
    id: u32,
    w: usize,
    h: usize,
    cols: u16,
    rows: u16,
    rgba: Vec<u8>,
}

/// Frames as raw RGBA. The first (and any after a resize) goes up whole, double-buffered
/// like `kitty_swap`; after that only the box that changed is sent, as a Kitty frame edit
/// (`a=f`) on the image already on screen. Measured on scrolling and typing, that's about
/// 57% fewer bytes per frame than passing Chrome's PNG through, and nothing for herdr to
/// decode but zlib. Needs a terminal that does frame edits: herdr desktop does.
#[derive(Default)]
pub struct KittyStream {
    flip: bool,
    shown: Option<Shown>,
}

impl KittyStream {
    /// Draw `b64` (a PNG or JPEG frame) at the cursor over `cols` x `rows` cells.
    /// `full` forces a whole frame, as after the screen was cleared.
    pub fn draw(&mut self, out: &mut Vec<u8>, b64: &str, cols: u16, rows: u16, full: bool) -> R<()> {
        let (w, h, rgba) = decode_rgba(&base64::engine::general_purpose::STANDARD.decode(b64)?)?;
        if let Some(s) = self.shown.as_mut().filter(|s| !full && (s.w, s.h, s.cols, s.rows) == (w, h, cols, rows)) {
            if let Some((x0, y0, x1, y1)) = dirty_rect(&s.rgba, &rgba, w, h) {
                let boxed: Vec<u8> = (y0..y1).flat_map(|y| &rgba[(y * w + x0) * 4..(y * w + x1) * 4]).copied().collect();
                let keys = format!("a=f,i={},r=1,X=1,x={x0},y={y0},s={},v={}", s.id, x1 - x0, y1 - y0);
                kitty_rgba(out, &keys, &boxed)?;
            }
            s.rgba = rgba;
            return Ok(());
        }
        let (id, old) = if self.flip { (IMAGE_ID + 1, IMAGE_ID) } else { (IMAGE_ID, IMAGE_ID + 1) };
        self.flip = !self.flip;
        kitty_rgba(out, &format!("a=t,i={id},s={w},v={h}"), &rgba)?;
        write!(out, "\x1b_Ga=p,i={id},p=1,q=2,C=1,c={cols},r={rows}\x1b\\")?;
        write!(out, "\x1b_Ga=d,d=I,i={old},q=2\x1b\\")?;
        self.shown = Some(Shown { id, w, h, cols, rows, rgba });
        Ok(())
    }
}

/// Upload a PNG (base64) as image `id` without showing it.
pub fn kitty_upload(out: &mut Vec<u8>, b64: &str, id: u32) -> R<()> {
    let chunks: Vec<&str> = b64.as_bytes().chunks(4096).map(|c| std::str::from_utf8(c).unwrap_or("")).collect();
    for (i, chunk) in chunks.iter().enumerate() {
        let more = u8::from(i + 1 < chunks.len());
        if i == 0 {
            write!(out, "\x1b_Ga=t,f=100,i={id},q=2,m={more};{chunk}\x1b\\")?;
        } else {
            write!(out, "\x1b_Gm={more};{chunk}\x1b\\")?;
        }
    }
    Ok(())
}

pub fn kitty_clear(out: &mut impl Write) {
    let _ = write!(out, "\x1b_Ga=d,d=I,i={IMAGE_ID},q=2\x1b\\\x1b_Ga=d,d=I,i={},q=2\x1b\\", IMAGE_ID + 1);
}

/// Decode a PNG or JPEG frame into (width, height, RGBA bytes).
pub fn decode_rgba(bytes: &[u8]) -> R<(usize, usize, Vec<u8>)> {
    if bytes.starts_with(&[0xff, 0xd8]) {
        use zune_jpeg::zune_core::{bytestream::ZCursor, colorspace::ColorSpace, options::DecoderOptions};
        let opts = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGBA);
        let mut dec = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(bytes), opts);
        let px = dec.decode().map_err(|e| format!("JPEG frame: {e:?}"))?;
        let (w, h) = dec.dimensions().ok_or("JPEG frame has no size")?;
        return Ok((w, h, px));
    }
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = dec.read_info()?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or("PNG frame too large")?];
    let info = reader.next_frame(&mut buf)?;
    let ch = info.color_type.samples();
    let (w, h) = (info.width as usize, info.height as usize);
    let mut rgba = Vec::with_capacity(w * h * 4);
    for px in buf[..info.buffer_size()].chunks(ch) {
        match ch {
            1 => rgba.extend_from_slice(&[px[0], px[0], px[0], 255]),
            2 => rgba.extend_from_slice(&[px[0], px[0], px[0], px[1]]),
            3 => rgba.extend_from_slice(&[px[0], px[1], px[2], 255]),
            _ => rgba.extend_from_slice(&px[..4]),
        }
    }
    Ok((w, h, rgba))
}

/// Decode a base64 frame into (width, height, RGB bytes).
fn decode_rgb(b64: &str) -> R<(usize, usize, Vec<u8>)> {
    let (w, h, rgba) = decode_rgba(&base64::engine::general_purpose::STANDARD.decode(b64)?)?;
    Ok((w, h, rgba.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect()))
}

/// Box-average an RGB image down to `gw`×`gh`.
pub fn downsample(w: usize, h: usize, rgb: &[u8], gw: usize, gh: usize) -> Vec<[u8; 3]> {
    let mut out = Vec::with_capacity(gw * gh);
    for gy in 0..gh {
        let (y0, y1) = (gy * h / gh, ((gy + 1) * h / gh).max(gy * h / gh + 1).min(h));
        for gx in 0..gw {
            let (x0, x1) = (gx * w / gw, ((gx + 1) * w / gw).max(gx * w / gw + 1).min(w));
            let mut sum = [0u32; 3];
            let mut n = 0u32;
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = (y * w + x) * 3;
                    for c in 0..3 {
                        sum[c] += rgb[i + c] as u32;
                    }
                    n += 1;
                }
            }
            let n = n.max(1);
            out.push([(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8]);
        }
    }
    out
}

/// Half-block rendering: each cell shows two pixels (`▀` top in fg, bottom in bg).
pub fn blocks(out: &mut Vec<u8>, b64: &str, top_row: u16, cols: u16, rows: u16) -> R<()> {
    let (w, h, rgb) = decode_rgb(b64)?;
    let (gw, gh) = (cols as usize, rows as usize * 2);
    let px = downsample(w, h, &rgb, gw, gh);
    for y in 0..rows as usize {
        write!(out, "\x1b[{};1H", top_row as usize + y + 1)?;
        let (mut fg, mut bg) = ([0u8; 3], [0u8; 3]);
        for x in 0..gw {
            let (t, b) = (px[2 * y * gw + x], px[(2 * y + 1) * gw + x]);
            if x == 0 || t != fg {
                write!(out, "\x1b[38;2;{};{};{}m", t[0], t[1], t[2])?;
                fg = t;
            }
            if x == 0 || b != bg {
                write!(out, "\x1b[48;2;{};{};{}m", b[0], b[1], b[2])?;
                bg = b;
            }
            out.extend_from_slice("▀".as_bytes());
        }
        out.extend_from_slice(b"\x1b[0m");
    }
    Ok(())
}

/// The character for a 2x4 pattern: bit `2 * row + col` set means that pixel takes the
/// foreground colour. Unicode 16 (U+1CD00..U+1CDE5) encodes the 230 patterns that had no
/// character yet, in pattern order; the other 26 are older block elements.
pub fn octant_char(bits: u8) -> char {
    static TABLE: std::sync::OnceLock<[char; 256]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        const OLDER: [(u8, u32); 26] = [
            (0x00, 0x20),    // space
            (0xFF, 0x2588),  // full block
            (0x0F, 0x2580),  // upper half
            (0xF0, 0x2584),  // lower half
            (0x55, 0x258C),  // left half
            (0xAA, 0x2590),  // right half
            (0x05, 0x2598),  // quadrant upper left
            (0x0A, 0x259D),  // quadrant upper right
            (0x50, 0x2596),  // quadrant lower left
            (0xA0, 0x2597),  // quadrant lower right
            (0xA5, 0x259A),  // upper left and lower right
            (0x5A, 0x259E),  // upper right and lower left
            (0xF5, 0x2599),  // all but upper right
            (0x5F, 0x259B),  // all but lower right
            (0xAF, 0x259C),  // all but lower left
            (0xFA, 0x259F),  // all but upper left
            (0x03, 0x1FB82), // upper one quarter
            (0xC0, 0x2582),  // lower one quarter
            (0x3F, 0x1FB85), // upper three quarters
            (0xFC, 0x2586),  // lower three quarters
            (0x14, 0x1FBE6), // middle left one quarter
            (0x28, 0x1FBE7), // middle right one quarter
            (0x01, 0x1CEA8), // left half upper one quarter
            (0x02, 0x1CEAB), // right half upper one quarter
            (0x40, 0x1CEA3), // left half lower one quarter
            (0x80, 0x1CEA0), // right half lower one quarter
        ];
        let mut t = [' '; 256];
        let mut next = 0x1CD00;
        for (bits, slot) in t.iter_mut().enumerate() {
            let cp = match OLDER.iter().find(|(b, _)| *b as usize == bits) {
                Some(&(_, cp)) => cp,
                None => {
                    next += 1;
                    next - 1
                }
            };
            *slot = char::from_u32(cp).unwrap_or(' ');
        }
        t
    })[bits as usize]
}

fn dist2(a: [u8; 3], b: [u8; 3]) -> u32 {
    (0..3).map(|i| (a[i] as i32 - b[i] as i32).pow(2) as u32).sum()
}

fn mean(px: &[[u8; 3]]) -> [u8; 3] {
    let n = px.len().max(1) as u32;
    let s = px.iter().fold([0u32; 3], |s, p| [s[0] + p[0] as u32, s[1] + p[1] as u32, s[2] + p[2] as u32]);
    [(s[0] / n) as u8, (s[1] / n) as u8, (s[2] / n) as u8]
}

/// Split one cell's 8 pixels into two colours: seed with the two most different pixels,
/// give each pixel to the nearer seed, then use each group's mean. Returns (bits, fg, bg)
/// with `bits` marking the fg pixels; a flat cell is all bg (bits 0).
pub fn split_cell(px: &[[u8; 3]; 8]) -> (u8, [u8; 3], [u8; 3]) {
    let (mut a, mut b, mut far) = (0, 0, 0);
    for i in 0..8 {
        for j in i + 1..8 {
            let d = dist2(px[i], px[j]);
            if d > far {
                (a, b, far) = (i, j, d);
            }
        }
    }
    // Below ~6 levels a channel the eye can't tell the halves apart: draw it flat.
    if far < 3 * 6 * 6 {
        return (0, [0; 3], mean(px));
    }
    let mut bits = 0u8;
    for (i, p) in px.iter().enumerate() {
        if dist2(*p, px[b]) < dist2(*p, px[a]) {
            bits |= 1 << i;
        }
    }
    let fg: Vec<[u8; 3]> = (0..8).filter(|i| bits & (1 << i) != 0).map(|i| px[i]).collect();
    let bg: Vec<[u8; 3]> = (0..8).filter(|i| bits & (1 << i) == 0).map(|i| px[i]).collect();
    (bits, mean(&fg), mean(&bg))
}

/// Octant rendering: each cell shows 2x4 pixels in its best two colours.
pub fn octants(out: &mut Vec<u8>, b64: &str, top_row: u16, cols: u16, rows: u16) -> R<()> {
    let (w, h, rgb) = decode_rgb(b64)?;
    let (gw, gh) = (cols as usize * 2, rows as usize * 4);
    let px = downsample(w, h, &rgb, gw, gh);
    let mut utf8 = [0u8; 4];
    for y in 0..rows as usize {
        write!(out, "\x1b[{};1H", top_row as usize + y + 1)?;
        let (mut fg, mut bg) = (None, None);
        for x in 0..cols as usize {
            let mut cell = [[0u8; 3]; 8];
            for (i, c) in cell.iter_mut().enumerate() {
                *c = px[(4 * y + i / 2) * gw + 2 * x + i % 2];
            }
            let (bits, f, b) = split_cell(&cell);
            if bits != 0 && fg != Some(f) {
                write!(out, "\x1b[38;2;{};{};{}m", f[0], f[1], f[2])?;
                fg = Some(f);
            }
            if bg != Some(b) {
                write!(out, "\x1b[48;2;{};{};{}m", b[0], b[1], b[2])?;
                bg = Some(b);
            }
            out.extend_from_slice(octant_char(bits).encode_utf8(&mut utf8).as_bytes());
        }
        out.extend_from_slice(b"\x1b[0m");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_frames_fill_the_width() {
        // 900x450 CSS px at 9x18 px cells is 100x25 cells.
        assert_eq!(fit(900.0, 450.0, 100, 40, 2.0), (100, 25));
    }

    #[test]
    fn tall_frames_are_bound_by_rows() {
        assert_eq!(fit(900.0, 1800.0, 100, 20, 2.0), (20, 20));
    }

    #[test]
    fn desktop_images_get_extra_columns_for_whole_pixel_cells() {
        let c = Cells { w: 8.4, h: 22.0, dpr: 1.0, img_w: 8.0 };
        assert_eq!(c.img_cols(80), 84); // 80 x 8.4 px = 672 px = 84 x 8 px
    }

    #[test]
    fn linux_desktop_cells_match_what_herdr_draws() {
        // herdr desktop on Linux, JetBrains Mono 14 at zoom 1: 235 columns span 1880 px,
        // 49 rows span 882 px.
        let c = desktop_cell_size(14.0, 1.0, 1.0, 1.0);
        assert_eq!((c.w, c.h, c.dpr), (8.0, 18.0, 1.0));
    }

    #[test]
    fn zoom_is_not_mistaken_for_a_retina_screen() {
        // Zoom 1.4 makes cells taller than 28 px; the screen is still 1x.
        let c = desktop_cell_size(19.6, 1.0, 1.0, 1.0);
        assert_eq!(c.dpr, 1.0);
        assert_eq!((c.w, c.h), (12.0, 26.0));
    }

    #[test]
    fn desktop_cells_follow_scale_and_multipliers() {
        let c = desktop_cell_size(14.0, 1.2, 1.0, 2.0);
        assert_eq!((c.w, c.h, c.dpr), (17.0, 44.0, 2.0));
        // herdr desktop on macOS at 2x, JetBrains Mono 14: 17 x 37 px cells.
        let mac = desktop_cell_size(14.0, 1.0, 1.0, 2.0);
        assert_eq!((mac.w, mac.h, mac.dpr), (17.0, 37.0, 2.0));
    }

    #[test]
    fn measured_cells_round_to_whole_device_pixels() {
        let fira = measured_cell_size(8.615, 17.231, 1.0, 1.0, 2.0);
        assert_eq!((fira.w, fira.h, fira.dpr, fira.img_w), (17.0, 34.0, 2.0, 17.0));
        let menlo = measured_cell_size(8.429, 16.297, 1.0, 1.0, 2.0);
        assert_eq!((menlo.w, menlo.h), (17.0, 33.0));
        let taller = measured_cell_size(8.615, 17.231, 1.2, 1.0, 2.0);
        assert_eq!(taller.h, 41.0);
    }

    #[test]
    fn cell_reply_is_parsed_from_mixed_input() {
        assert_eq!(parse_cell_reply(b"\x1b[6;34;16t\x1b[?62;22c"), Some((16.0, 34.0)));
        assert_eq!(parse_cell_reply(b"\x1b[?62;22c"), None);
    }

    #[test]
    fn octants_fill_the_unicode_range_in_pattern_order() {
        // The first new pattern is octant 3 alone, the last is all but octant 8 ... in order,
        // skipping the 26 that older block elements already draw.
        assert_eq!(octant_char(0x04), '\u{1CD00}');
        assert_eq!(octant_char(0x06), '\u{1CD01}'); // 0x05 is the upper left quadrant
        assert_eq!(octant_char(0xFE), '\u{1CDE5}');
        let new = (0..=255u8).filter(|&b| ('\u{1CD00}'..='\u{1CDE5}').contains(&octant_char(b))).count();
        assert_eq!(new, 230);
        assert_eq!((octant_char(0x00), octant_char(0xFF), octant_char(0x0F), octant_char(0x55)), (' ', '█', '▀', '▌'));
    }

    #[test]
    fn a_cell_splits_into_its_two_colours() {
        let (ink, paper) = ([20, 30, 200], [250, 250, 250]);
        // Ink down the left column only.
        let mut px = [paper; 8];
        for i in [0, 2, 4, 6] {
            px[i] = ink;
        }
        let (bits, fg, bg) = split_cell(&px);
        let (bits, fg, bg) = if fg == ink { (bits, fg, bg) } else { (!bits, bg, fg) };
        assert_eq!((bits, fg, bg), (0x55, ink, paper));
        assert_eq!(octant_char(bits), '▌');
    }

    #[test]
    fn a_flat_cell_is_drawn_as_background() {
        let (bits, _, bg) = split_cell(&[
            [100, 100, 100],
            [102, 101, 100],
            [100, 100, 100],
            [101, 100, 100],
            [100, 100, 100],
            [100, 100, 100],
            [100, 100, 100],
            [100, 100, 102],
        ]);
        assert_eq!(bits, 0);
        assert!(dist2(bg, [100, 100, 100]) < 4);
    }

    #[test]
    fn kitty_support_is_read_from_the_probe_reply() {
        assert!(parse_kitty_reply(b"\x1b_Gi=31;OK\x1b\\\x1b[?62c"));
        assert!(!parse_kitty_reply(b"\x1b[?62;22c"));
    }

    #[test]
    fn downsample_averages_each_box() {
        // 2x1 image: black and white -> one grey pixel.
        let rgb = [0, 0, 0, 255, 255, 255];
        assert_eq!(downsample(2, 1, &rgb, 1, 1), vec![[127, 127, 127]]);
    }

    #[test]
    fn dirty_rect_boxes_only_what_changed() {
        let a = vec![0u8; 4 * 4 * 4];
        let mut b = a.clone();
        assert_eq!(dirty_rect(&a, &b, 4, 4), None);
        b[(4 + 1) * 4] = 9; // (1, 1)
        b[(2 * 4 + 2) * 4 + 3] = 9; // (2, 2), alpha only
        assert_eq!(dirty_rect(&a, &b, 4, 4), Some((1, 1, 3, 3)));
    }

    #[test]
    fn kitty_stream_sends_a_whole_frame_then_only_the_change() {
        let frame = |px: [u8; 4]| {
            let mut bytes = Vec::new();
            let mut enc = png::Encoder::new(&mut bytes, 2, 1);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.write_header().unwrap().write_image_data(&[[1, 1, 1, 255], px].concat()).unwrap();
            base64::engine::general_purpose::STANDARD.encode(&bytes)
        };
        let mut ks = KittyStream::default();
        let mut out = Vec::new();
        ks.draw(&mut out, &frame([2, 2, 2, 255]), 10, 5, false).unwrap();
        let first = String::from_utf8_lossy(&out).to_string();
        assert!(first.contains(&format!("a=t,i={IMAGE_ID},s=2,v=1,f=32,o=z")) && first.contains("a=p,"));
        out.clear();
        ks.draw(&mut out, &frame([3, 3, 3, 255]), 10, 5, false).unwrap();
        assert!(String::from_utf8_lossy(&out).starts_with(&format!("\x1b_Ga=f,i={IMAGE_ID},r=1,X=1,x=1,y=0,s=1,v=1,f=32,o=z")));
        out.clear();
        ks.draw(&mut out, &frame([3, 3, 3, 255]), 10, 5, false).unwrap();
        assert!(out.is_empty(), "an unchanged frame sends nothing");
        ks.draw(&mut out, &frame([3, 3, 3, 255]), 10, 5, true).unwrap();
        assert!(String::from_utf8_lossy(&out).contains("a=t,"), "full forces a whole frame");
    }

    #[test]
    fn decode_rgb_drops_alpha_from_a_png() {
        let mut bytes = Vec::new();
        let mut enc = png::Encoder::new(&mut bytes, 2, 1);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(&[255, 0, 0, 255, 0, 128, 255, 10]).unwrap();
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        assert_eq!(decode_rgb(&b64).unwrap(), (2, 1, vec![255, 0, 0, 0, 128, 255]));
    }
}
